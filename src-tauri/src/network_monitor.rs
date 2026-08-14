use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use sysinfo::{System, Pid};
use crate::types::{ConnectionStats, ProcessSnapshot, ProcessUsage};
#[cfg(windows)]
use crate::etw::TrafficCollector;

#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetPerTcp6ConnectionEStats, GetPerTcpConnectionEStats,
    SetPerTcp6ConnectionEStats, SetPerTcpConnectionEStats,
    MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
    MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData,
    TCP_TABLE_OWNER_PID_ALL,
};
#[cfg(windows)]
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
#[cfg(windows)]
use windows::Win32::Foundation::{BOOLEAN, NO_ERROR};

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
    // Cumulative per-PID (sent, recv) TCP+UDP bytes fed by the ETW collector.
    #[cfg(windows)]
    etw_totals: Arc<Mutex<HashMap<u32, (u64, u64)>>>,
    // Snapshot of etw_totals taken at the last capture, for delta accounting.
    #[cfg(windows)]
    etw_prev: HashMap<u32, (u64, u64)>,
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

impl NetworkMonitor {
    pub fn new() -> Self {
        #[cfg(windows)]
        let (etw_totals, etw_collector, etw_active) = {
            let totals = Arc::new(Mutex::new(HashMap::new()));
            match TrafficCollector::start(totals.clone()) {
                Ok(c) => (totals, Some(c), true),
                Err(e) => {
                    log::warn!("ETW capture unavailable (falling back to TCP EStats polling): {e}");
                    (totals, None, false)
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
            #[cfg(windows)]
            etw_totals,
            #[cfg(windows)]
            etw_prev: HashMap::new(),
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
            let rows = std::slice::from_raw_parts(
                table.table.as_ptr(),
                table.dwNumEntries as usize,
            );
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
            let rows = std::slice::from_raw_parts(
                table.table.as_ptr(),
                table.dwNumEntries as usize,
            );
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
            let set_result = SetPerTcpConnectionEStats(
                row.cast(),
                TcpConnectionEstatsData,
                as_bytes(&rw),
                0,
                0,
            );
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

            Self::accumulate_delta(key, pid, rod.DataBytesIn, rod.DataBytesOut, conn_stats, pid_deltas);
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

            Self::accumulate_delta(key, pid, rod.DataBytesIn, rod.DataBytesOut, conn_stats, pid_deltas);
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
                Self::poll_connection(
                    row,
                    &key,
                    pid,
                    &mut self.connection_stats,
                    &mut pid_deltas,
                );
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
                Self::poll_connection6(
                    row,
                    &key,
                    pid,
                    &mut self.connection_stats,
                    &mut pid_deltas,
                );
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
                (pid, ConnectionStats { bytes_sent, bytes_recv })
            })
            .collect()
    }

    #[cfg(not(windows))]
    pub fn poll(&mut self) -> HashMap<u32, ConnectionStats> {
        HashMap::new()
    }

    // Capture network deltas and accumulate session totals.
    // Called ONLY by the background monitoring loop so that poll() state is
    // never consumed twice. Snapshots carry per-poll deltas so database SUMs
    // stay accurate.
    pub fn capture(&mut self) -> Vec<ProcessSnapshot> {
        self.system.refresh_all();

        let now = chrono::Local::now().timestamp();

        let mut pid_deltas: HashMap<u32, (u64, u64)> = HashMap::new();

        #[cfg(windows)]
        if self.etw_active {
            // ETW path: continuous per-segment accounting for TCP+UDP. Nothing
            // is lost when connections close between polls, so totals are not
            // biased by the capture interval.
            let etw_totals = self.etw_totals.lock().map(|m| m.clone()).unwrap_or_default();
            for (pid, &(sent, recv)) in &etw_totals {
                // First sighting only establishes the baseline; the delta is
                // counted from the next capture onward.
                if let Some(&(prev_sent, prev_recv)) = self.etw_prev.get(pid) {
                    let d_sent = sent.saturating_sub(prev_sent);
                    let d_recv = recv.saturating_sub(prev_recv);
                    if d_sent > 0 || d_recv > 0 {
                        pid_deltas.insert(*pid, (d_sent, d_recv));
                    }
                }
            }
            self.etw_prev = etw_totals;
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

            let process = self.system.process(Pid::from_u32(pid));
            let exe_name = process
                .map(|p| p.name().to_str().unwrap_or("Unknown").to_string())
                .unwrap_or_else(|| format!("PID_{}", pid));

            // Add combined TCP+UDP deltas to session totals.
            self.session_totals.entry(pid)
                .and_modify(|(sent, recv)| {
                    *sent += bytes_sent;
                    *recv += bytes_recv;
                })
                .or_insert((bytes_sent, bytes_recv));

            snapshots.push(ProcessSnapshot {
                exe_name,
                pid,
                bytes_sent,
                bytes_recv,
                timestamp: now,
            });
        }

        snapshots
    }

    // Read-only view for the UI. Does NOT call poll(), so it never interferes
    // with the capture loop's delta accounting.
    pub fn get_realtime_processes(&mut self) -> Vec<ProcessUsage> {
        self.system.refresh_all();

        let mut usage_list = Vec::new();
        for (pid, &(total_sent, total_recv)) in &self.session_totals {
            if total_sent == 0 && total_recv == 0 {
                continue;
            }

            let process = self.system.process(Pid::from_u32(*pid));
            let exe_name = process
                .map(|p| p.name().to_str().unwrap_or("Unknown").to_string())
                .unwrap_or_else(|| format!("PID_{}", pid));

            usage_list.push(ProcessUsage {
                exe_name,
                pid: *pid,
                upload_bps: 0,
                download_bps: 0,
                total_sent,
                total_recv,
            });
        }

        usage_list
    }
}
