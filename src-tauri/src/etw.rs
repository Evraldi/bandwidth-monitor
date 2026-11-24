// Per-process TCP/UDP traffic capture via ETW (NT Kernel Logger).
//
// The NETWORK_TCPIP kernel enable flag emits two classic MOF providers:
// TcpIp and UdpIp, both carrying the owning PID and the transfer size. We
// consume both from a single SYSTEM_LOGGER_MODE session so per-process byte
// totals stay continuous: every datagram and every TCP segment is counted,
// including the final bytes of a connection after it closes. That is exactly
// what the TCP EStats polling fallback in network_monitor.rs misses (tail
// bytes of closed connections), which previously caused downloads to be
// undercounted.
//
// Kernel events cannot be enabled via EnableTraceEx2 on Windows 10 (that
// arrived on Windows 11), so we start a SYSTEM_LOGGER_MODE session. Since
// Windows 8 the kernel providers can be multiplexed across up to 8 sessions,
// so we use our own logger name + GUID instead of the global "NT Kernel
// Logger", avoiding conflicts with whatever else uses the system logger.

use std::collections::HashMap;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, OpenTraceW, ProcessTrace, StartTraceW, CONTROLTRACE_HANDLE,
    EVENT_RECORD, EVENT_TRACE_CONTROL_QUERY, EVENT_TRACE_CONTROL_STOP,
    EVENT_TRACE_FLAG_NETWORK_TCPIP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_LOGFILEW_0,
    EVENT_TRACE_LOGFILEW_1, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    EVENT_TRACE_SYSTEM_LOGGER_MODE, EVENT_TRACE_TYPE_ACCEPT, EVENT_TRACE_TYPE_ACKDUP,
    EVENT_TRACE_TYPE_ACKFULL, EVENT_TRACE_TYPE_ACKPART, EVENT_TRACE_TYPE_CONNECT,
    EVENT_TRACE_TYPE_CONNFAIL, EVENT_TRACE_TYPE_COPY_ARP, EVENT_TRACE_TYPE_COPY_TCP,
    EVENT_TRACE_TYPE_DISCONNECT, EVENT_TRACE_TYPE_RECEIVE, EVENT_TRACE_TYPE_RECONNECT,
    EVENT_TRACE_TYPE_RETRANSMIT, EVENT_TRACE_TYPE_SEND, PROCESSTRACE_HANDLE,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, WNODE_FLAG_TRACED_GUID,
};

// Private logger name + GUID for our kernel session (not the shared NT Kernel
// Logger / SystemTraceControlGuid slot). The GUID must be unique across the
// system: Windows refuses to start a second session that reuses a GUID that is
// already held by a running session.
const SESSION_NAME: &str = "BandwidthMonitorNet";
const SESSION_GUID: GUID = GUID::from_u128(0xf59fa954_901c_4125_8404_e537fee5b789);

// Classic TcpIp class GUID (MOF), emitted by the NETWORK_TCPIP enable flag.
const TCPIP_CLASS_GUID: GUID = GUID::from_u128(0x9a280ac0_c8e0_11d1_84e2_00c04fb998a2);
// Classic UdpIp class GUID (MOF), emitted by the NETWORK_TCPIP enable flag.
const UDPIP_CLASS_GUID: GUID = GUID::from_u128(0xbf3a50c5_a9c9_4988_a005_2df0b7c80f80);

// Event type untuk classic MOF TcpIp/UdpIp. Nilai v4 diambil dari konstanta
// `windows` crate (EVENT_TRACE_TYPE_*), bukan angka hafalan.
const CLASS_TYPE_SEND: u16 = EVENT_TRACE_TYPE_SEND as u16;
const CLASS_TYPE_RECEIVE: u16 = EVENT_TRACE_TYPE_RECEIVE as u16;
const CLASS_TYPE_COPY_TCP: u16 = EVENT_TRACE_TYPE_COPY_TCP as u16;
const CLASS_TYPE_SEND_V6: u16 = 26;
const CLASS_TYPE_RECEIVE_V6: u16 = 27;
const CLASS_TYPE_COPY_TCP_V6: u16 = 34;

// Untuk classic MOF, identitas event ada di `Opcode`, bukan `Id` (PerfView
// mendaftarkan event ini dengan eventId 0xFFFF). Kalau Opcode tidak dikenal,
// baru fallback ke Id untuk provider manifest-style.
const KNOWN_NETWORK_OPCODES: &[u16] = &[
    CLASS_TYPE_SEND,
    CLASS_TYPE_RECEIVE,
    EVENT_TRACE_TYPE_CONNECT as u16,
    EVENT_TRACE_TYPE_DISCONNECT as u16,
    EVENT_TRACE_TYPE_RETRANSMIT as u16,
    EVENT_TRACE_TYPE_ACCEPT as u16,
    EVENT_TRACE_TYPE_RECONNECT as u16,
    EVENT_TRACE_TYPE_CONNFAIL as u16,
    CLASS_TYPE_COPY_TCP,
    EVENT_TRACE_TYPE_COPY_ARP as u16,
    EVENT_TRACE_TYPE_ACKFULL as u16,
    EVENT_TRACE_TYPE_ACKPART as u16,
    EVENT_TRACE_TYPE_ACKDUP as u16,
    CLASS_TYPE_SEND_V6,
    CLASS_TYPE_RECEIVE_V6,
    28, // connect v6
    29, // disconnect v6
    30, // retransmit v6
    31, // accept v6
    32, // reconnect v6
    CLASS_TYPE_COPY_TCP_V6,
];

// Buffer tuning for the real-time session. TCP generates one event per
// segment (far more than UDP), so generous buffers + a fast flush timer keep
// the consumer from dropping events under heavy downloads.
const MIN_BUFFERS: u32 = 64;
const MAX_BUFFERS: u32 = 512;
const BUFFER_SIZE_KB: u32 = 256;
const FLUSH_TIMER_SECS: u32 = 1;

/// Counter loss ETW yang bisa dibaca kapan saja tanpa menghentikan session.
#[derive(Debug, Default, Clone)]
pub struct EtwHealth {
    pub events_lost: Arc<AtomicU64>,
    pub buffers_lost: Arc<AtomicU64>,
    // Nama yang diambil saat event masuk, sebelum proses sempat keluar dari
    // snapshot sysinfo.
    process_names: Arc<Mutex<HashMap<u32, String>>>,
}

impl EtwHealth {
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.events_lost.load(Ordering::Relaxed),
            self.buffers_lost.load(Ordering::Relaxed),
        )
    }

    pub fn process_name(&self, pid: u32) -> Option<String> {
        self.process_names.lock().ok()?.get(&pid).cloned()
    }

    pub fn forget_process(&self, pid: u32) {
        if let Ok(mut names) = self.process_names.lock() {
            names.remove(&pid);
        }
    }

    fn ensure_process_name(&self, pid: u32) {
        if let Ok(mut names) = self.process_names.lock() {
            if let std::collections::hash_map::Entry::Vacant(entry) = names.entry(pid) {
                if let Some(name) = crate::platform::process_name(pid) {
                    entry.insert(name);
                }
            }
        }
    }
}

struct CollectorContext {
    totals: Arc<Mutex<HashMap<u32, (u64, u64)>>>,
    health: Arc<EtwHealth>,
}

/// `ControlTraceW` menerima `EVENT_TRACE_PROPERTIES` dengan nama session
/// ditaruh tepat setelah struct. Ini dipakai Start/Query/Stop supaya semua
/// panggilan konsisten dan query loss tidak gagal diam-diam.
unsafe fn named_properties_buffer() -> Vec<u64> {
    let name: Vec<u16> = format!("{SESSION_NAME}\0").encode_utf16().collect();
    let total_size = size_of::<EVENT_TRACE_PROPERTIES>() + name.len() * 2;
    let mut buffer = vec![0u64; total_size.div_ceil(8)];
    let base = buffer.as_mut_ptr() as *mut u8;
    let props_ptr = base as *mut EVENT_TRACE_PROPERTIES;

    let mut props: EVENT_TRACE_PROPERTIES = std::mem::zeroed();
    props.Wnode.BufferSize = total_size as u32;
    props.LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    props_ptr.write(props);

    let offset = props.LoggerNameOffset as usize;
    for (i, &ch) in name.iter().enumerate() {
        *base.add(offset + i * 2) = (ch & 0xff) as u8;
        *base.add(offset + i * 2 + 1) = (ch >> 8) as u8;
    }

    buffer
}

pub struct TrafficCollector {
    trace: CONTROLTRACE_HANDLE,
    process_trace: PROCESSTRACE_HANDLE,
    thread: Option<JoinHandle<()>>,
    // ETW callback memakai UserContext sebagai raw pointer. Box ini menjaga
    // alokasinya tetap hidup sampai consumer thread benar-benar selesai.
    context: Option<Box<CollectorContext>>,
}

impl TrafficCollector {
    pub fn start(
        totals: Arc<Mutex<HashMap<u32, (u64, u64)>>>,
        health: Arc<EtwHealth>,
    ) -> Result<TrafficCollector, String> {
        unsafe {
            let name: Vec<u16> = format!("{SESSION_NAME}\0").encode_utf16().collect();

            // Clean up a leftover session from a previous crashed run, if any.
            let mut cleanup_buffer = named_properties_buffer();
            let cleanup_props = cleanup_buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
            let _ = ControlTraceW(
                CONTROLTRACE_HANDLE::default(),
                PCWSTR(name.as_ptr()),
                cleanup_props,
                EVENT_TRACE_CONTROL_STOP,
            );

            // Build the session properties buffer (aligned, with the session
            // name appended right after the struct).
            let mut buffer = named_properties_buffer();
            let props_ptr = buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
            let props = &mut *props_ptr;
            props.Wnode.Guid = SESSION_GUID;
            props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            props.LogFileMode = EVENT_TRACE_SYSTEM_LOGGER_MODE | EVENT_TRACE_REAL_TIME_MODE;
            props.EnableFlags = EVENT_TRACE_FLAG_NETWORK_TCPIP;
            props.BufferSize = BUFFER_SIZE_KB;
            props.MinimumBuffers = MIN_BUFFERS;
            props.MaximumBuffers = MAX_BUFFERS;
            props.FlushTimer = FLUSH_TIMER_SECS;
            props.LogFileNameOffset = 0;

            let mut trace = CONTROLTRACE_HANDLE::default();
            let err = StartTraceW(&mut trace, PCWSTR(name.as_ptr()), props_ptr);
            if err.0 != 0 {
                return Err(format!("StartTraceW failed: 0x{:08X}", err.0));
            }

            let context = Box::new(CollectorContext {
                totals: totals.clone(),
                health: health.clone(),
            });
            let context_ptr = &*context as *const CollectorContext as *mut core::ffi::c_void;

            let mut logfile: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
            let mut logfile_name: Vec<u16> = format!("{SESSION_NAME}\0").encode_utf16().collect();
            logfile.LoggerName = PWSTR(logfile_name.as_mut_ptr());
            logfile.Anonymous1 = EVENT_TRACE_LOGFILEW_0 {
                ProcessTraceMode: PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD,
            };
            logfile.Anonymous2 = EVENT_TRACE_LOGFILEW_1 {
                EventRecordCallback: Some(on_event),
            };
            logfile.BufferCallback = Some(on_buffer);
            logfile.Context = context_ptr;

            let process_trace = OpenTraceW(&mut logfile);
            if process_trace.Value == u64::MAX {
                // INVALID_TRACEHANDLE = UINTPTR_MAX
                let _ = stop_session(trace);
                return Err("OpenTraceW failed".into());
            }

            let thread = std::thread::Builder::new()
                .name("etw-tcpip-consumer".into())
                .spawn(move || {
                    let err = ProcessTrace(&[process_trace], None, None);
                    if err.0 != 0 {
                        log::warn!("ETW ProcessTrace berhenti: 0x{:08X}", err.0);
                    }
                })
                .map_err(|e| e.to_string())?;

            Ok(TrafficCollector {
                trace,
                process_trace,
                thread: Some(thread),
                context: Some(context),
            })
        }
    }
}

impl TrafficCollector {
    /// Counter dari session yang masih berjalan. Dipanggil monitoring loop.
    pub fn query_loss(&self) -> Result<(u64, u64), String> {
        unsafe {
            // Handle sudah cukup untuk memilih session, tapi properti tetap
            // butuh layout lengkap + nama seperti saat StartTraceW.
            let mut buffer = named_properties_buffer();
            let props_ptr = buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
            let err = ControlTraceW(
                self.trace,
                PCWSTR::null(),
                props_ptr,
                EVENT_TRACE_CONTROL_QUERY,
            );
            if err.0 != 0 {
                return Err(format!("ControlTraceW query gagal: 0x{:08X}", err.0));
            }
            let props = &*props_ptr;
            Ok((
                props.EventsLost as u64,
                props.LogBuffersLost as u64 + props.RealTimeBuffersLost as u64,
            ))
        }
    }

    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .map(|thread| !thread.is_finished())
            .unwrap_or(false)
    }
}

impl Drop for TrafficCollector {
    fn drop(&mut self) {
        unsafe {
            // Stop the session, then unblock the ProcessTrace loop so the
            // consumer thread can exit.
            let _ = stop_session(self.trace);
            let _ = CloseTrace(self.process_trace);
        }
        let mut context = self.context.take();
        if let Some(thread) = self.thread.take() {
            // Jangan pernah menggantung shutdown kalau driver ETW bandel.
            let deadline = Instant::now() + Duration::from_secs(2);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                log::warn!("ETW consumer thread tidak berhenti dalam 2s; dilepas.");
                // Thread masih memakai raw pointer ke context. Jangan di-drop.
                std::mem::forget(context.take());
            }
        }
        drop(context);
    }
}

/// Dipanggil ETW tiap buffer selesai dibaca. `EventsLost` di sini adalah
/// counter kumulatif session, jadi simpan nilai maksimum yang pernah terlihat.
unsafe extern "system" fn on_buffer(logfile: *mut EVENT_TRACE_LOGFILEW) -> u32 {
    if logfile.is_null() {
        return 1;
    }
    let logfile = &*logfile;
    if !logfile.Context.is_null() {
        let context = &*(logfile.Context as *const CollectorContext);
        context
            .health
            .events_lost
            .fetch_max(logfile.EventsLost as u64, Ordering::Relaxed);
    }
    1 // return 0 would stop processing
}

unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    if record.is_null() {
        return;
    }
    let rec = &*record;
    if rec.UserData.is_null() || rec.UserDataLength < 8 {
        return;
    }
    let provider = rec.EventHeader.ProviderId;
    if provider != UDPIP_CLASS_GUID && provider != TCPIP_CLASS_GUID {
        return;
    }

    let desc = rec.EventHeader.EventDescriptor;
    let Some(kind) = classify_traffic(provider == UDPIP_CLASS_GUID, desc.Id, desc.Opcode) else {
        return;
    };

    // TcpIp_TypeGroup1/3 dan UdpIp_TypeGroup1/2 sama-sama mulai dengan
    // { PID(4), size(4) }.
    let data = std::slice::from_raw_parts(rec.UserData as *const u8, rec.UserDataLength as usize);
    let size = u32_at(data, 4);
    let Some(pid) = resolve_pid(u32_at(data, 0), rec.EventHeader.ProcessId) else {
        return;
    };
    if size == 0 {
        return;
    }

    if rec.UserContext.is_null() {
        return;
    }
    let context = &*(rec.UserContext as *const CollectorContext);
    context.health.ensure_process_name(pid);
    if let Ok(mut map) = context.totals.lock() {
        let entry = map.entry(pid).or_insert((0, 0));
        match kind {
            TrafficKind::Send => entry.0 = entry.0.saturating_add(size as u64),
            TrafficKind::Receive => entry.1 = entry.1.saturating_add(size as u64),
        }
    }
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrafficKind {
    Send,
    Receive,
}

/// Klasifikasi murni supaya bisa di-test tanpa session ETW.
///
/// `Opcode` diprioritaskan karena event MOF klasik menyimpan event type di
/// sana. `Id` hanya dipakai kalau Opcode bukan event jaringan yang dikenal.
/// `TcpIp_TypeGroup1/3` memakai event 18/34 untuk jalur copy loopback: ini
/// adalah sisi terima, jadi dihitung sebagai download. Mengabaikannya bikin
/// traffic loopback undercount. Retransmit/ACK/connect sengaja diabaikan
/// karena byte-nya sudah tercatat di Send/Recv asli.
fn classify_traffic(is_udp: bool, id: u16, opcode: u8) -> Option<TrafficKind> {
    let opcode = u16::from(opcode);
    let etype = if KNOWN_NETWORK_OPCODES.contains(&opcode) {
        opcode
    } else if KNOWN_NETWORK_OPCODES.contains(&id) {
        id
    } else {
        return None;
    };

    match etype {
        CLASS_TYPE_SEND | CLASS_TYPE_SEND_V6 => Some(TrafficKind::Send),
        CLASS_TYPE_RECEIVE | CLASS_TYPE_RECEIVE_V6 => Some(TrafficKind::Receive),
        // TCPCopy hanya ada di provider TcpIp, bukan UdpIp.
        CLASS_TYPE_COPY_TCP | CLASS_TYPE_COPY_TCP_V6 if !is_udp => Some(TrafficKind::Receive),
        _ => None,
    }
}

/// PID di payload adalah sumber kebenaran. Dokumentasi ETW memperingatkan
/// bahwa `EventHeader.ProcessId` bisa menjadi thread sistem yang melakukan
/// logging, jadi dipakai hanya sebagai fallback saat payload PID = 0.
fn resolve_pid(payload_pid: u32, header_pid: u32) -> Option<u32> {
    match (payload_pid, header_pid) {
        (pid, _) if pid != 0 => Some(pid),
        (0, pid) if pid != 0 => Some(pid),
        _ => None,
    }
}

unsafe fn stop_session(trace: CONTROLTRACE_HANDLE) -> WIN32_ERROR {
    let mut buffer = named_properties_buffer();
    let props_ptr = buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
    ControlTraceW(trace, PCWSTR::null(), props_ptr, EVENT_TRACE_CONTROL_STOP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    // Requires an elevated process AND a free session. Blasts loopback
    // TCP+UDP to local listeners and verifies ETW attributes both directions.
    // Sengaja `#[ignore]`: kalau dijalankan biasa, jangan pernah dilaporkan
    // "ok" padahal StartTraceW gagal. Jalankan sebagai admin dengan:
    //   cargo test etw_requires_elevation -- --ignored --nocapture
    #[test]
    #[ignore = "butuh admin + session ETW bebas; jalankan dengan --ignored --nocapture"]
    fn etw_requires_elevation_counts_loopback_traffic() {
        let totals = Arc::new(Mutex::new(HashMap::new()));
        let health = Arc::new(EtwHealth::default());
        let collector = TrafficCollector::start(totals.clone(), health.clone()).unwrap_or_else(
            |e| {
                panic!("ETW tidak tersedia: {e}. Jalankan sebagai admin dan matikan instance app lain.")
            },
        );

        std::thread::sleep(Duration::from_millis(500));

        let payload = b"bandwidth-monitor-etw-udp-test";
        let listener = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let dest = listener.local_addr().unwrap();
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.connect(dest).unwrap();
        for _ in 0..50 {
            sock.send(payload).unwrap();
        }
        let mut buf = [0u8; 256];
        for _ in 0..50 {
            listener.recv_from(&mut buf).unwrap();
        }

        let tcp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_dest = tcp_listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = tcp_listener.accept().unwrap();
            let mut received = Vec::new();
            stream.read_to_end(&mut received).unwrap();
            stream.write_all(&received).unwrap();
        });
        let mut client = std::net::TcpStream::connect(tcp_dest).unwrap();
        let blob = vec![0xAAu8; 1 << 20];
        client.write_all(&blob).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let mut echoed = Vec::new();
        client.read_to_end(&mut echoed).unwrap();
        server.join().unwrap();
        assert_eq!(echoed.len(), blob.len());

        std::thread::sleep(Duration::from_secs(2));

        let map = totals.lock().unwrap();
        let mine = map.get(&std::process::id()).copied().unwrap_or((0, 0));
        eprintln!(
            "ETW totals captured for self PID {}: {:?}",
            std::process::id(),
            mine
        );
        assert!(
            mine.0 >= blob.len() as u64,
            "TCP/UDP send traffic was not captured via ETW: {:?}",
            mine
        );
        assert!(
            mine.1 >= blob.len() as u64,
            "TCP/UDP recv traffic was not captured via ETW: {:?}",
            mine
        );
        drop(map);
        let (events_lost, buffers_lost) = health.snapshot();
        assert_eq!(events_lost, 0, "ETW event loss terdeteksi");
        assert_eq!(buffers_lost, 0, "ETW buffer loss terdeteksi");
        drop(collector);
    }

    #[test]
    fn opcode_menang_atas_id_untuk_event_mof() {
        // Classic MOF punya Id = 0xFFFF; beberapa event bisa membawa Id yang
        // kebetulan sama dengan event type lain. Opcode yang menentukan.
        assert_eq!(
            classify_traffic(false, u16::MAX, CLASS_TYPE_SEND as u8),
            Some(TrafficKind::Send)
        );
        assert_eq!(
            classify_traffic(false, CLASS_TYPE_SEND, CLASS_TYPE_COPY_TCP as u8),
            Some(TrafficKind::Receive),
            "TCPCopy tidak boleh salah diklasifikasi jadi Send karena Id"
        );
        assert_eq!(
            classify_traffic(false, 0, CLASS_TYPE_RECEIVE_V6 as u8),
            Some(TrafficKind::Receive)
        );
    }

    #[test]
    fn id_dipakai_hanya_bila_opcode_tidak_dikenal() {
        assert_eq!(
            classify_traffic(false, CLASS_TYPE_RECEIVE, 0),
            Some(TrafficKind::Receive)
        );
        assert_eq!(classify_traffic(false, 0, 0), None);
    }

    #[test]
    fn copy_tcp_dihitung_tapi_event_non_data_diabaikan() {
        assert_eq!(
            classify_traffic(false, 0, CLASS_TYPE_COPY_TCP as u8),
            Some(TrafficKind::Receive)
        );
        assert_eq!(
            classify_traffic(false, 0, CLASS_TYPE_COPY_TCP_V6 as u8),
            Some(TrafficKind::Receive)
        );
        // TCPCopy bukan event UDP.
        assert_eq!(classify_traffic(true, 0, CLASS_TYPE_COPY_TCP as u8), None);
        // Retransmit/connect/ACK sudah tercatat di Send/Recv asli.
        assert_eq!(
            classify_traffic(false, 0, EVENT_TRACE_TYPE_RETRANSMIT as u8),
            None
        );
        assert_eq!(
            classify_traffic(false, 0, EVENT_TRACE_TYPE_CONNECT as u8),
            None
        );
    }

    #[test]
    fn pid_payload_dipakai_header_sebagai_fallback() {
        assert_eq!(resolve_pid(4242, 999), Some(4242));
        assert_eq!(resolve_pid(0, 999), Some(999));
        assert_eq!(resolve_pid(0, 0), None);
    }
}
