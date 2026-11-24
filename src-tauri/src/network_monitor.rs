#[cfg(windows)]
use crate::etw::{EtwHealth, TrafficCollector};
use crate::types::{CaptureHealth, ConnectionStats, ProcessSnapshot, ProcessUsage};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use sysinfo::{Pid, System};

#[cfg(windows)]
use windows::Win32::Foundation::{BOOLEAN, NO_ERROR};
#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetPerTcp6ConnectionEStats, GetPerTcpConnectionEStats,
    SetPerTcp6ConnectionEStats, SetPerTcpConnectionEStats, TCP_ESTATS_DATA_ROD_v0,
    TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
    MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
};
#[cfg(windows)]
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};

// MIB_TCP_STATE: only track connections that are at/after ESTABLISHED (5)
// so we skip CLOSED, LISTEN, SYN_SENT, SYN_RCVD rows.
#[cfg(windows)]
const TCP_STATE_ESTABLISHED: u32 = 5;

// Unique identity of a TCP connection (4-tuple). Ports are kept in the raw
// network-byte-order values from the table; equality is all we need.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ConnKey {
    V4 {
        local_addr: u32,
        local_port: u32,
        remote_addr: u32,
        remote_port: u32,
    },
    V6 {
        local_addr: [u8; 16],
        local_scope: u32,
        local_port: u32,
        remote_addr: [u8; 16],
        remote_scope: u32,
        remote_port: u32,
    },
}

#[cfg(windows)]
struct ConnStatsState {
    pid: u32,
    prev_bytes_in: u64,
    prev_bytes_out: u64,
}

pub struct NetworkMonitor {
    system: System,
    // True when the ETW TCP+UDP collector is running; capture() then uses the
    // continuous ETW totals. When false it falls back to per-connection TCP
    // EStats polling (which loses the tail bytes of closed connections).
    #[cfg(windows)]
    etw_active: bool,
    #[cfg(windows)]
    connection_stats: HashMap<ConnKey, ConnStatsState>,
    session_totals: HashMap<u32, (u64, u64)>,
    // Nama dan start time terakhir per PID. Dipakai supaya proses yang sudah
    // mati tidak berubah menjadi "PID_<n>" dan PID reuse tidak mencampur total.
    pid_names: HashMap<u32, String>,
    pid_start_times: HashMap<u32, u64>,
    #[cfg(windows)]
    etw_health: Arc<EtwHealth>,
    // Cumulative per-PID (sent, recv) TCP+UDP bytes fed by the ETW collector.
    #[cfg(windows)]
    etw_totals: Arc<Mutex<HashMap<u32, (u64, u64)>>>,
    // Snapshot of etw_totals taken at the last capture, for delta accounting.
    #[cfg(windows)]
    etw_prev: HashMap<u32, (u64, u64)>,
    // Rate byte/detik hasil capture terakhir. Dihitung backend supaya UI tidak
    // perlu menyimpan baseline sendiri (yang reset tiap pindah tab).
    process_rates: HashMap<u32, (u64, u64)>,
    last_capture_ts: Option<i64>,
    // Stops the ETW session when the monitor is dropped.
    #[cfg(windows)]
    _etw_collector: Option<TrafficCollector>,
}

#[cfg(windows)]
fn as_bytes<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((val as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

#[cfg(windows)]
fn as_mut_bytes<T: Sized>(val: &mut T) -> &mut [u8] {
    unsafe {
        std::slice::from_raw_parts_mut((val as *mut T).cast::<u8>(), std::mem::size_of::<T>())
    }
}

/// Konversi delta satu siklus jadi byte/detik. Siklus pertama sengaja 0:
/// first-sight ETW menghitung total kumulatif sejak app start, jadi kalau
/// dijadikan rate langsung akan muncul spike palsu.
pub(crate) fn rates_from_delta(
    first_cycle: bool,
    bytes: (u64, u64),
    elapsed_secs: u64,
) -> (u64, u64) {
    if first_cycle {
        (0, 0)
    } else {
        (
            crate::spike::rate_bps(bytes.0, elapsed_secs),
            crate::spike::rate_bps(bytes.1, elapsed_secs),
        )
    }
}

/// ETW counters adalah milik aplikasi ini (dimulai saat app launch), jadi
/// first sighting boleh dihitung penuh. Kalau counter turun, itu PID reuse /
/// reset dan nilai saat ini dianggap sesi baru.
pub(crate) fn compute_etw_deltas(
    current: &HashMap<u32, (u64, u64)>,
    previous: &HashMap<u32, (u64, u64)>,
) -> HashMap<u32, (u64, u64)> {
    current
        .iter()
        .filter_map(|(&pid, &(sent, recv))| {
            let (sent, recv) = match previous.get(&pid) {
                None => (sent, recv),
                Some(&(prev_sent, prev_recv)) if sent >= prev_sent && recv >= prev_recv => {
                    (sent - prev_sent, recv - prev_recv)
                }
                Some(_) => {
                    log::warn!("ETW counter reset untuk PID {pid}; dihitung sebagai sesi baru");
                    (sent, recv)
                }
            };
            (sent > 0 || recv > 0).then_some((pid, (sent, recv)))
        })
        .collect()
}

/// Susun delta satu siklus ETW. Counter yang turun dibersihkan dari baseline
/// dan total sesi, TAPI nilai saat ini tetap dihitung sebagai sesi baru.
/// Menghapus `current` di sini akan membuang byte reset tersebut.
pub(crate) fn apply_etw_snapshot(
    current: &HashMap<u32, (u64, u64)>,
    previous: &mut HashMap<u32, (u64, u64)>,
    session_totals: &mut HashMap<u32, (u64, u64)>,
) -> (HashMap<u32, (u64, u64)>, Vec<u32>) {
    let mut reset_pids: Vec<u32> = current
        .iter()
        .filter(|(&pid, &(sent, recv))| {
            previous
                .get(&pid)
                .is_some_and(|&(prev_sent, prev_recv)| sent < prev_sent || recv < prev_recv)
        })
        .map(|(&pid, _)| pid)
        .collect();
    reset_pids.sort_unstable();

    for pid in &reset_pids {
        log::warn!("ETW counter reset untuk PID {pid}; total sesi dihitung ulang");
        previous.remove(pid);
        session_totals.remove(pid);
    }

    let deltas = compute_etw_deltas(current, previous);
    *previous = current.clone();
    (deltas, reset_pids)
}

impl NetworkMonitor {
    pub fn new() -> Self {
        #[cfg(windows)]
        let (etw_totals, etw_collector, etw_active, etw_health) = {
            let totals = Arc::new(Mutex::new(HashMap::new()));
            let health = Arc::new(EtwHealth::default());
            match TrafficCollector::start(totals.clone(), health.clone()) {
                Ok(c) => (totals, Some(c), true, health),
                Err(e) => {
                    log::warn!("ETW capture unavailable (falling back to TCP EStats polling): {e}");
                    (totals, None, false, health)
                }
            }
        };

        NetworkMonitor {
            system: System::new_all(),
            #[cfg(windows)]
            etw_active,
            #[cfg(windows)]
            connection_stats: HashMap::new(),
            session_totals: HashMap::new(),
            pid_names: HashMap::new(),
            pid_start_times: HashMap::new(),
            #[cfg(windows)]
            etw_health,
            #[cfg(windows)]
            etw_totals,
            #[cfg(windows)]
            etw_prev: HashMap::new(),
            process_rates: HashMap::new(),
            last_capture_ts: None,
            #[cfg(windows)]
            _etw_collector: etw_collector,
        }
    }

    #[cfg(windows)]
    fn snapshot_tcp4_rows() -> Option<Vec<MIB_TCPROW_OWNER_PID>> {
        unsafe {
            let mut size: u32 = 0;
            GetExtendedTcpTable(
                None,
                &mut size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
            if size == 0 {
                return None;
            }

            let mut buffer = vec![0u8; size as usize];
            let result = GetExtendedTcpTable(
                Some(buffer.as_mut_ptr() as *mut _),
                &mut size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
            if result != NO_ERROR.0 {
                return None;
            }

            let table = &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
            let rows =
                std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
            Some(rows.to_vec())
        }
    }

    #[cfg(windows)]
    fn snapshot_tcp6_rows() -> Option<Vec<MIB_TCP6ROW_OWNER_PID>> {
        unsafe {
            let mut size: u32 = 0;
            GetExtendedTcpTable(
                None,
                &mut size,
                false,
                AF_INET6.0 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
            if size == 0 {
                return None;
            }

            let mut buffer = vec![0u8; size as usize];
            let result = GetExtendedTcpTable(
                Some(buffer.as_mut_ptr() as *mut _),
                &mut size,
                false,
                AF_INET6.0 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
            if result != NO_ERROR.0 {
                return None;
            }

            let table = &*(buffer.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
            let rows =
                std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
            Some(rows.to_vec())
        }
    }

    #[cfg(windows)]
    fn accumulate_delta(
        key: &ConnKey,
        pid: u32,
        bytes_in: u64,
        bytes_out: u64,
        conn_stats: &mut HashMap<ConnKey, ConnStatsState>,
        pid_deltas: &mut HashMap<u32, (u64, u64)>,
    ) {
        let mut delta_in = 0u64;
        let mut delta_out = 0u64;

        if let Some(prev) = conn_stats.get(key) {
            // Only count deltas when the connection still belongs to the same
            // PID; on a PID change treat it as a fresh connection (baseline).
            if prev.pid == pid {
                delta_in = bytes_in.saturating_sub(prev.prev_bytes_in);
                delta_out = bytes_out.saturating_sub(prev.prev_bytes_out);
            }
        }

        if delta_in > 0 || delta_out > 0 {
            let entry = pid_deltas.entry(pid).or_insert((0, 0));
            entry.0 += delta_in;
            entry.1 += delta_out;
        }

        conn_stats.insert(
            *key,
            ConnStatsState {
                pid,
                prev_bytes_in: bytes_in,
                prev_bytes_out: bytes_out,
            },
        );
    }

    #[cfg(windows)]
    fn poll_connection(
        row: *const MIB_TCPROW_OWNER_PID,
        key: &ConnKey,
        pid: u32,
        conn_stats: &mut HashMap<ConnKey, ConnStatsState>,
        pid_deltas: &mut HashMap<u32, (u64, u64)>,
    ) {
        unsafe {
            // Enable extended data collection (idempotent, required once).
            let rw = TCP_ESTATS_DATA_RW_v0 {
                EnableCollection: BOOLEAN(1),
            };
            let set_result =
                SetPerTcpConnectionEStats(row.cast(), TcpConnectionEstatsData, as_bytes(&rw), 0, 0);
            if set_result != NO_ERROR.0 {
                return; // Not elevated / unavailable -> no per-connection stats
            }

            // Read cumulative data bytes for this connection.
            let mut rod = TCP_ESTATS_DATA_ROD_v0::default();
            let get_result = GetPerTcpConnectionEStats(
                row.cast(),
                TcpConnectionEstatsData,
                None,
                0,
                None,
                0,
                Some(as_mut_bytes(&mut rod)),
                0,
            );
            if get_result != NO_ERROR.0 {
                return;
            }

            Self::accumulate_delta(
                key,
                pid,
                rod.DataBytesIn,
                rod.DataBytesOut,
                conn_stats,
                pid_deltas,
            );
        }
    }

    #[cfg(windows)]
    fn poll_connection6(
        row: *const MIB_TCP6ROW_OWNER_PID,
        key: &ConnKey,
        pid: u32,
        conn_stats: &mut HashMap<ConnKey, ConnStatsState>,
        pid_deltas: &mut HashMap<u32, (u64, u64)>,
    ) {
        unsafe {
            let rw = TCP_ESTATS_DATA_RW_v0 {
                EnableCollection: BOOLEAN(1),
            };
            let set_result = SetPerTcp6ConnectionEStats(
                row.cast(),
                TcpConnectionEstatsData,
                as_bytes(&rw),
                0,
                0,
            );
            if set_result != NO_ERROR.0 {
                return;
            }

            let mut rod = TCP_ESTATS_DATA_ROD_v0::default();
            let get_result = GetPerTcp6ConnectionEStats(
                row.cast(),
                TcpConnectionEstatsData,
                None,
                0,
                None,
                0,
                Some(as_mut_bytes(&mut rod)),
                0,
            );
            if get_result != NO_ERROR.0 {
                return;
            }

            Self::accumulate_delta(
                key,
                pid,
                rod.DataBytesIn,
                rod.DataBytesOut,
                conn_stats,
                pid_deltas,
            );
        }
    }

    #[cfg(windows)]
    pub fn poll(&mut self) -> HashMap<u32, ConnectionStats> {
        let mut pid_deltas: HashMap<u32, (u64, u64)> = HashMap::new();
        let mut seen_keys: HashSet<ConnKey> = HashSet::new();

        if let Some(rows) = Self::snapshot_tcp4_rows() {
            for row in &rows {
                let pid = row.dwOwningPid;
                if pid == 0 || row.dwState < TCP_STATE_ESTABLISHED {
                    continue;
                }
                let key = ConnKey::V4 {
                    local_addr: row.dwLocalAddr,
                    local_port: row.dwLocalPort,
                    remote_addr: row.dwRemoteAddr,
                    remote_port: row.dwRemotePort,
                };
                Self::poll_connection(row, &key, pid, &mut self.connection_stats, &mut pid_deltas);
                seen_keys.insert(key);
            }
        }

        if let Some(rows) = Self::snapshot_tcp6_rows() {
            for row in &rows {
                let pid = row.dwOwningPid;
                if pid == 0 || row.dwState < TCP_STATE_ESTABLISHED {
                    continue;
                }
                let key = ConnKey::V6 {
                    local_addr: row.ucLocalAddr,
                    local_scope: row.dwLocalScopeId,
                    local_port: row.dwLocalPort,
                    remote_addr: row.ucRemoteAddr,
                    remote_scope: row.dwRemoteScopeId,
                    remote_port: row.dwRemotePort,
                };
                Self::poll_connection6(row, &key, pid, &mut self.connection_stats, &mut pid_deltas);
                seen_keys.insert(key);
            }
        }

        // Forget connections that disappeared (closed) so their next sighting
        // is treated as a fresh baseline. The tail bytes of closed connections
        // are intentionally not captured until an ETW-based approach is added.
        self.connection_stats
            .retain(|key, _| seen_keys.contains(key));

        pid_deltas
            .into_iter()
            .map(|(pid, (bytes_recv, bytes_sent))| {
                (
                    pid,
                    ConnectionStats {
                        bytes_sent,
                        bytes_recv,
                    },
                )
            })
            .collect()
    }

    #[cfg(not(windows))]
    pub fn poll(&mut self) -> HashMap<u32, ConnectionStats> {
        HashMap::new()
    }

    #[cfg(windows)]
    fn etw_running(&self) -> bool {
        self._etw_collector
            .as_ref()
            .map(|collector| collector.is_running())
            .unwrap_or(false)
    }

    pub fn capture_health(&self) -> CaptureHealth {
        #[cfg(windows)]
        {
            let (events_lost, buffers_lost) = self.etw_health.snapshot();
            let (queried_events, queried_buffers) = self
                ._etw_collector
                .as_ref()
                .and_then(|collector| collector.query_loss().ok())
                .unwrap_or((0, 0));
            CaptureHealth {
                etw_active: self.etw_active,
                session_running: self.etw_running(),
                events_lost: events_lost.max(queried_events),
                buffers_lost: buffers_lost.max(queried_buffers),
            }
        }
        #[cfg(not(windows))]
        {
            CaptureHealth {
                etw_active: false,
                session_running: false,
                events_lost: 0,
                buffers_lost: 0,
            }
        }
    }

    fn live_pids(&self) -> HashSet<u32> {
        self.system
            .processes()
            .keys()
            .map(|pid| pid.as_u32())
            .collect()
    }

    fn process_name(&mut self, pid: u32) -> String {
        if let Some(process) = self.system.process(Pid::from_u32(pid)) {
            let name = process.name().to_string_lossy().into_owned();
            if !name.is_empty() {
                self.pid_names.insert(pid, name.clone());
                return name;
            }
        }
        #[cfg(windows)]
        if let Some(name) = self.etw_health.process_name(pid) {
            self.pid_names.insert(pid, name.clone());
            return name;
        }

        self.pid_names
            .get(&pid)
            .cloned()
            .unwrap_or_else(|| format!("PID_{pid}"))
    }

    /// Deteksi PID reuse dari process start time. Tanpa ini, byte milik proses
    /// lama dan baru nyampur di satu entri.
    fn sync_process_identity(&mut self) {
        let identities: Vec<(u32, u64)> = self
            .system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                let pid = pid.as_u32();
                if pid == 0 {
                    return None;
                }
                Some((pid, process.start_time()))
            })
            .collect();

        let mut reused = Vec::new();
        for (pid, start_time) in &identities {
            if self
                .pid_start_times
                .get(pid)
                .is_some_and(|&previous| previous != *start_time)
            {
                reused.push(*pid);
            }
        }

        for pid in reused {
            log::warn!("PID {pid} dipakai ulang; total sesi direset");
            self.session_totals.remove(&pid);
            self.pid_names.remove(&pid);
            self.process_rates.remove(&pid);
            #[cfg(windows)]
            {
                self.etw_prev.remove(&pid);
                self.etw_health.forget_process(pid);
                if let Ok(mut totals) = self.etw_totals.lock() {
                    totals.remove(&pid);
                }
            }
        }

        self.pid_start_times = identities.into_iter().collect();
    }

    fn evict_dead_pids(&mut self, live: &HashSet<u32>) {
        // Kumpulkan dulu: setelah `retain`, PID mati sudah hilang dari
        // `pid_names` dan loop di bawah tidak akan pernah menjalankan
        // `forget_process`.
        #[cfg(windows)]
        let dead_pids: Vec<u32> = self
            .pid_names
            .keys()
            .copied()
            .filter(|pid| !live.contains(pid))
            .collect();

        self.pid_names.retain(|pid, _| live.contains(pid));
        self.pid_start_times.retain(|pid, _| live.contains(pid));
        self.session_totals.retain(|pid, _| live.contains(pid));
        self.process_rates.retain(|pid, _| live.contains(pid));
        #[cfg(windows)]
        {
            for pid in dead_pids {
                self.etw_health.forget_process(pid);
            }
            self.etw_prev.retain(|pid, _| live.contains(pid));
            if let Ok(mut totals) = self.etw_totals.lock() {
                totals.retain(|pid, _| live.contains(pid));
            }
        }
    }

    // Capture network deltas and accumulate session totals.
    // Called ONLY by the background monitoring loop so that poll() state is
    // never consumed twice. Snapshots carry per-poll deltas so database SUMs
    // stay accurate.
    pub fn capture(&mut self) -> Vec<ProcessSnapshot> {
        self.system.refresh_all();
        self.sync_process_identity();

        let now = chrono::Local::now().timestamp();
        let first_cycle = self.last_capture_ts.is_none();
        let elapsed_secs = self
            .last_capture_ts
            .map(|previous| now.saturating_sub(previous).max(1) as u64)
            .unwrap_or(0);
        self.last_capture_ts = Some(now);

        let live = self.live_pids();
        let mut pid_deltas: HashMap<u32, (u64, u64)> = HashMap::new();

        #[cfg(windows)]
        if self.etw_active && !self.etw_running() {
            log::error!("ETW consumer berhenti tidak terduga; beralih ke TCP EStats");
            self.etw_active = false;
        }

        #[cfg(windows)]
        if self.etw_active {
            // ETW path: continuous per-segment accounting for TCP+UDP. First
            // sighting dihitung penuh karena counter dimulai bareng app.
            // Jangan clone seluruh map tiap capture: callback ETW bisa
            // menambah ribuan entri dan clone itu membebani memori/CPU.
            // Pinjam guard-nya langsung; field lain dipinjam terpisah sehingga
            // borrow checker tetap puas.
            let etw_totals = match self.etw_totals.lock() {
                Ok(guard) => guard,
                Err(error) => {
                    log::error!("Mutex etw_totals rusak: {error}");
                    return Vec::new();
                }
            };

            // `apply_etw_snapshot` sudah menyamakan `etw_prev` dengan snapshot
            // `etw_totals` saat ini. Jangan hapus entri dari `etw_totals` di
            // sini: itu akan membuat siklus berikutnya mulai dari nol sementara
            // baseline masih di nilai lama -> delta ikut terpotong.
            let (deltas, _) =
                apply_etw_snapshot(&etw_totals, &mut self.etw_prev, &mut self.session_totals);

            drop(etw_totals);
            for (pid, delta) in deltas {
                pid_deltas.insert(pid, delta);
            }
        } else {
            // Fallback: per-connection TCP EStats polling. Lossy — the tail
            // bytes of connections that close between polls are not captured.
            for (pid, stats) in self.poll() {
                let entry = pid_deltas.entry(pid).or_insert((0, 0));
                entry.0 += stats.bytes_sent;
                entry.1 += stats.bytes_recv;
            }
        }

        let mut snapshots = Vec::new();
        for (pid, (bytes_sent, bytes_recv)) in pid_deltas {
            if bytes_sent == 0 && bytes_recv == 0 {
                continue;
            }

            let exe_name = self.process_name(pid);

            self.session_totals
                .entry(pid)
                .and_modify(|(sent, recv)| {
                    *sent += bytes_sent;
                    *recv += bytes_recv;
                })
                .or_insert((bytes_sent, bytes_recv));

            self.process_rates.insert(
                pid,
                rates_from_delta(first_cycle, (bytes_sent, bytes_recv), elapsed_secs),
            );

            snapshots.push(ProcessSnapshot {
                exe_name,
                pid,
                bytes_sent,
                bytes_recv,
                timestamp: now,
            });
        }

        self.evict_dead_pids(&live);
        snapshots
    }

    /// Read-only view for the UI. Tidak memanggil `refresh_all()` dan tidak
    /// mengubah state, jadi pemanggilan dari command UI tidak bisa menahan
    /// loop capture. Rate sudah dihitung sekali per siklus di `capture()`.
    pub fn get_realtime_processes(&self) -> Vec<ProcessUsage> {
        self.session_totals
            .iter()
            .filter(|(_, &(sent, recv))| sent > 0 || recv > 0)
            .map(|(&pid, &(total_sent, total_recv))| {
                let (upload_bps, download_bps) =
                    self.process_rates.get(&pid).copied().unwrap_or((0, 0));
                let exe_name = self
                    .pid_names
                    .get(&pid)
                    .cloned()
                    .unwrap_or_else(|| format!("PID_{pid}"));
                ProcessUsage {
                    exe_name,
                    pid,
                    upload_bps,
                    download_bps,
                    total_sent,
                    total_recv,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_etw_snapshot, compute_etw_deltas, rates_from_delta};
    use std::collections::HashMap;

    #[test]
    fn rate_siklus_pertama_tidak_spike_palsu() {
        assert_eq!(rates_from_delta(true, (900, 800), 5), (0, 0));
        assert_eq!(rates_from_delta(false, (900, 800), 5), (180, 160));
        assert_eq!(rates_from_delta(false, (900, 800), 0), (0, 0));
    }

    #[test]
    fn first_sight_count_full_and_normal_delta_is_difference() {
        let current = HashMap::from([(1, (100, 200)), (2, (50, 60))]);
        let previous = HashMap::from([(2, (40, 50))]);

        let deltas = compute_etw_deltas(&current, &previous);

        assert_eq!(deltas.get(&1), Some(&(100, 200)));
        assert_eq!(deltas.get(&2), Some(&(10, 10)));
    }

    #[test]
    fn reset_is_treated_as_fresh_session() {
        let current = HashMap::from([(1, (12, 7))]);
        let previous = HashMap::from([(1, (900, 900))]);

        let deltas = compute_etw_deltas(&current, &previous);

        assert_eq!(deltas.get(&1), Some(&(12, 7)));
    }

    #[test]
    fn zero_deltas_are_skipped() {
        let current = HashMap::from([(1, (100, 100))]);
        let previous = HashMap::from([(1, (100, 100))]);

        assert!(compute_etw_deltas(&current, &previous).is_empty());
    }

    #[test]
    fn snapshot_reset_tetap_menghitung_nilai_baru() {
        let current = HashMap::from([(1, (12, 7)), (2, (100, 100))]);
        let mut previous = HashMap::from([(1, (900, 900)), (2, (90, 95))]);
        let mut session_totals = HashMap::from([(1, (900, 900)), (2, (90, 95))]);

        let (deltas, reset_pids) = apply_etw_snapshot(&current, &mut previous, &mut session_totals);

        assert_eq!(reset_pids, vec![1]);
        assert_eq!(deltas.get(&1), Some(&(12, 7)));
        assert_eq!(deltas.get(&2), Some(&(10, 5)));
        assert_eq!(previous.get(&1), Some(&(12, 7)));
        assert!(!session_totals.contains_key(&1));
    }

    #[test]
    fn reset_tidak_memotong_delta_siklus_berikutnya() {
        let first = HashMap::from([(1, (12, 7))]);
        let mut previous = HashMap::from([(1, (900, 900))]);
        let mut session_totals = HashMap::from([(1, (900, 900))]);

        let (d1, resets) = apply_etw_snapshot(&first, &mut previous, &mut session_totals);
        assert_eq!(resets, vec![1]);
        assert_eq!(d1.get(&1), Some(&(12, 7)));

        // Callback terus menambah ke akumulator yang SAMA (12+60, 7+40).
        // Kalau akumulator ikut dihapus saat reset, siklus ini cuma dapat
        // (48, 33) dan byte-nya hilang.
        let second = HashMap::from([(1, (72, 47))]);
        let (d2, resets2) = apply_etw_snapshot(&second, &mut previous, &mut session_totals);
        assert!(resets2.is_empty());
        assert_eq!(d2.get(&1), Some(&(60, 40)));
    }
}
