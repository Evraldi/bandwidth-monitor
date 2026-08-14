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
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, StartTraceW, CONTROLTRACE_HANDLE, EVENT_TRACE_CONTROL_STOP,
    EVENT_TRACE_FLAG_NETWORK_TCPIP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_LOGFILEW_0,
    EVENT_TRACE_LOGFILEW_1, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    EVENT_TRACE_SYSTEM_LOGGER_MODE, EVENT_RECORD, OpenTraceW, PROCESS_TRACE_MODE_EVENT_RECORD,
    PROCESS_TRACE_MODE_REAL_TIME, PROCESSTRACE_HANDLE, ProcessTrace, WNODE_FLAG_TRACED_GUID,
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

// Send/Receive event types shared by TcpIp and UdpIp (v4 and v6 variants).
const CLASS_TYPE_SEND: u16 = 10;
const CLASS_TYPE_RECEIVE: u16 = 11;
const CLASS_TYPE_SEND_V6: u16 = 26;
const CLASS_TYPE_RECEIVE_V6: u16 = 27;

// Buffer tuning for the real-time session. TCP generates one event per
// segment (far more than UDP), so generous buffers + a fast flush timer keep
// the consumer from dropping events under heavy downloads.
const MIN_BUFFERS: u32 = 64;
const MAX_BUFFERS: u32 = 512;
const BUFFER_SIZE_KB: u32 = 256;
const FLUSH_TIMER_SECS: u32 = 1;

pub struct TrafficCollector {
    trace: CONTROLTRACE_HANDLE,
    process_trace: PROCESSTRACE_HANDLE,
    thread: Option<JoinHandle<()>>,
    // Keeps the shared totals allocation alive for the ETW callback, which
    // reaches it through the UserContext pointer.
    _totals: Arc<Mutex<HashMap<u32, (u64, u64)>>>,
}

impl TrafficCollector {
    pub fn start(totals: Arc<Mutex<HashMap<u32, (u64, u64)>>>) -> Result<TrafficCollector, String> {
        unsafe {
            let name: Vec<u16> = format!("{SESSION_NAME}\0").encode_utf16().collect();

            // Clean up a leftover session from a previous crashed run, if any.
            let mut cleanup_props: EVENT_TRACE_PROPERTIES = std::mem::zeroed();
            cleanup_props.Wnode.BufferSize = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
            let _ = ControlTraceW(
                CONTROLTRACE_HANDLE::default(),
                PCWSTR(name.as_ptr()),
                &mut cleanup_props,
                EVENT_TRACE_CONTROL_STOP,
            );

            // Build the session properties buffer (aligned, with the session
            // name appended right after the struct).
            let total_size = size_of::<EVENT_TRACE_PROPERTIES>() + name.len() * 2;
            let mut buffer = vec![0u64; total_size.div_ceil(8)];
            let base = buffer.as_mut_ptr() as *mut u8;
            let props_ptr = base as *mut EVENT_TRACE_PROPERTIES;

            let mut props: EVENT_TRACE_PROPERTIES = std::mem::zeroed();
            props.Wnode.BufferSize = total_size as u32;
            props.Wnode.Guid = SESSION_GUID;
            props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            props.LogFileMode = EVENT_TRACE_SYSTEM_LOGGER_MODE | EVENT_TRACE_REAL_TIME_MODE;
            props.EnableFlags = EVENT_TRACE_FLAG_NETWORK_TCPIP;
            props.BufferSize = BUFFER_SIZE_KB;
            props.MinimumBuffers = MIN_BUFFERS;
            props.MaximumBuffers = MAX_BUFFERS;
            props.FlushTimer = FLUSH_TIMER_SECS;
            props.LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
            props.LogFileNameOffset = 0;
            props_ptr.write(props);

            let name_offset = props.LoggerNameOffset as usize;
            for (i, &ch) in name.iter().enumerate() {
                *base.add(name_offset + i * 2) = (ch & 0xff) as u8;
                *base.add(name_offset + i * 2 + 1) = (ch >> 8) as u8;
            }

            let mut trace = CONTROLTRACE_HANDLE::default();
            let err = StartTraceW(&mut trace, PCWSTR(name.as_ptr()), props_ptr);
            if err.0 != 0 {
                return Err(format!("StartTraceW failed: 0x{:08X}", err.0));
            }

            let mut logfile: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
            let mut logfile_name: Vec<u16> = format!("{SESSION_NAME}\0").encode_utf16().collect();
            logfile.LoggerName = PWSTR(logfile_name.as_mut_ptr());
            logfile.Anonymous1 = EVENT_TRACE_LOGFILEW_0 {
                ProcessTraceMode: PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD,
            };
            logfile.Anonymous2 = EVENT_TRACE_LOGFILEW_1 {
                EventRecordCallback: Some(on_event),
            };
            logfile.Context = Arc::as_ptr(&totals) as *mut core::ffi::c_void;

            let process_trace = OpenTraceW(&mut logfile);
            if process_trace.Value == u64::MAX {
                // INVALID_TRACEHANDLE = UINTPTR_MAX
                let _ = stop_session(trace);
                return Err("OpenTraceW failed".into());
            }

            let thread = std::thread::Builder::new()
                .name("etw-tcpip-consumer".into())
                .spawn(move || {
                    let _ = ProcessTrace(&[process_trace], None, None);
                })
                .map_err(|e| e.to_string())?;

            Ok(TrafficCollector {
                trace,
                process_trace,
                thread: Some(thread),
                _totals: totals,
            })
        }
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
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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
    let etype = if matches!(
        desc.Id,
        CLASS_TYPE_SEND | CLASS_TYPE_RECEIVE | CLASS_TYPE_SEND_V6 | CLASS_TYPE_RECEIVE_V6
    ) {
        desc.Id
    } else {
        desc.Opcode as u16
    };
    let is_send = match etype {
        CLASS_TYPE_SEND | CLASS_TYPE_SEND_V6 => true,
        CLASS_TYPE_RECEIVE | CLASS_TYPE_RECEIVE_V6 => false,
        // Ignore connect/disconnect/retransmit/accept/reconnect/copy etc.;
        // retransmits must not be counted or we would double-count data that
        // was already credited by the original send/receive event.
        _ => return,
    };

    // TcpIp_TypeGroup1/3 and UdpIp_TypeGroup1/2 all start with { PID(4), size(4) }.
    let data = std::slice::from_raw_parts(rec.UserData as *const u8, rec.UserDataLength as usize);
    let size = u32_at(data, 4);
    let mut pid = u32_at(data, 0);
    if pid == 0 {
        pid = rec.EventHeader.ProcessId;
    }
    if size == 0 || pid == 0 {
        return;
    }

    if rec.UserContext.is_null() {
        return;
    }
    let totals = &*(rec.UserContext as *const Mutex<HashMap<u32, (u64, u64)>>);
    if let Ok(mut map) = totals.lock() {
        let entry = map.entry(pid).or_insert((0, 0));
        if is_send {
            entry.0 = entry.0.saturating_add(size as u64);
        } else {
            entry.1 = entry.1.saturating_add(size as u64);
        }
    }
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

unsafe fn stop_session(trace: CONTROLTRACE_HANDLE) -> WIN32_ERROR {
    let mut props: EVENT_TRACE_PROPERTIES = std::mem::zeroed();
    props.Wnode.BufferSize = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    ControlTraceW(
        trace,
        PCWSTR::null(),
        &mut props,
        EVENT_TRACE_CONTROL_STOP,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    // Requires an elevated process. Blasts loopback TCP+UDP to local listeners
    // and verifies the ETW provider attributes both directions to this PID.
    #[test]
    fn traffic_collector_counts_loopback_traffic() {
        let totals = Arc::new(Mutex::new(HashMap::new()));
        let collector = match TrafficCollector::start(totals.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ETW unavailable, skipping: {e}");
                return;
            }
        };

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
        drop(collector);
    }
}
