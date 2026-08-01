use std::collections::HashMap;
use sysinfo::{System, Pid, Networks};
use crate::types::{ConnectionStats, ProcessSnapshot};

#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    GetExtendedUdpTable, MIB_UDPTABLE_OWNER_PID, UDP_TABLE_OWNER_PID,
};
#[cfg(windows)]
use windows::Win32::Networking::WinSock::AF_INET;
#[cfg(windows)]
use windows::Win32::Foundation::NO_ERROR;

pub struct NetworkMonitor {
    system: System,
    networks: Networks,
    previous_system_totals: (u64, u64),
    connection_count: HashMap<u32, u32>,
    session_totals: HashMap<u32, (u64, u64)>,
}

impl NetworkMonitor {
    pub fn new() -> Self {
        NetworkMonitor {
            system: System::new_all(),
            networks: Networks::new_with_refreshed_list(),
            previous_system_totals: (0, 0),
            connection_count: HashMap::new(),
            session_totals: HashMap::new(),
        }
    }

    #[cfg(windows)]
    pub fn poll(&mut self) -> HashMap<u32, ConnectionStats> {
        let mut current_stats: HashMap<u32, ConnectionStats> = HashMap::new();
        
        unsafe {
            let mut size: u32 = 0;
            
            let _ = GetExtendedTcpTable(
                None,
                &mut size,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );

            if size > 0 {
                let mut buffer = vec![0u8; size as usize];
                let result = GetExtendedTcpTable(
                    Some(buffer.as_mut_ptr() as *mut _),
                    &mut size,
                    false,
                    AF_INET.0 as u32,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                );

                if result == NO_ERROR.0 {
                    let table = &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
                    let entries = std::slice::from_raw_parts(
                        &table.table[0],
                        table.dwNumEntries as usize,
                    );

                    for entry in entries {
                        let pid = entry.dwOwningPid;
                        if pid > 0 {
                            *self.connection_count.entry(pid).or_insert(0) += 1;
                        }
                    }
                }
            }

            size = 0;
            let _ = GetExtendedUdpTable(
                None,
                &mut size,
                false,
                AF_INET.0 as u32,
                UDP_TABLE_OWNER_PID,
                0,
            );

            if size > 0 {
                let mut buffer = vec![0u8; size as usize];
                let result = GetExtendedUdpTable(
                    Some(buffer.as_mut_ptr() as *mut _),
                    &mut size,
                    false,
                    AF_INET.0 as u32,
                    UDP_TABLE_OWNER_PID,
                    0,
                );

                if result == NO_ERROR.0 {
                    let table = &*(buffer.as_ptr() as *const MIB_UDPTABLE_OWNER_PID);
                    let entries = std::slice::from_raw_parts(
                        &table.table[0],
                        table.dwNumEntries as usize,
                    );

                    for entry in entries {
                        let pid = entry.dwOwningPid;
                        if pid > 0 {
                            *self.connection_count.entry(pid).or_insert(0) += 1;
                        }
                    }
                }
            }
        }

        self.networks.refresh();
        let mut total_received = 0u64;
        let mut total_transmitted = 0u64;
        
        for (_interface_name, network) in &self.networks {
            total_received += network.received();
            total_transmitted += network.transmitted();
        }

        let (prev_sent, prev_recv) = self.previous_system_totals;
        let delta_sent = total_transmitted.saturating_sub(prev_sent);
        let delta_recv = total_received.saturating_sub(prev_recv);
        
        self.previous_system_totals = (total_transmitted, total_received);

        let total_connections: u32 = self.connection_count.values().sum();
        
        if total_connections > 0 && (delta_sent > 0 || delta_recv > 0) {
            for (pid, &conn_count) in &self.connection_count {
                let weight = conn_count as f64 / total_connections as f64;
                
                let allocated_recv = (delta_recv as f64 * weight) as u64;
                let allocated_sent = (delta_sent as f64 * weight) as u64;

                current_stats.insert(*pid, ConnectionStats {
                    pid: *pid,
                    bytes_sent: allocated_sent,
                    bytes_recv: allocated_recv,
                });
            }
        }

        self.connection_count.clear();

        current_stats
    }

    #[cfg(not(windows))]
    pub fn poll(&mut self) -> HashMap<u32, ConnectionStats> {
        HashMap::new()
    }

    pub fn get_process_usage(&mut self) -> Vec<ProcessSnapshot> {
        self.system.refresh_all();
        
        let current_stats = self.poll();
        let now = chrono::Local::now().timestamp();
        let mut snapshots = Vec::new();

        for (pid, stats) in current_stats {
            let process = self.system.process(Pid::from_u32(pid));
            let exe_name = process
                .map(|p| p.name().to_str().unwrap_or("Unknown").to_string())
                .unwrap_or_else(|| format!("PID_{}", pid));

            // stats already contains deltas from poll(), just add directly to session totals
            self.session_totals.entry(pid)
                .and_modify(|(sent, recv)| {
                    *sent += stats.bytes_sent;
                    *recv += stats.bytes_recv;
                })
                .or_insert((stats.bytes_sent, stats.bytes_recv));

            let (total_sent, total_recv) = self.session_totals.get(&pid).copied().unwrap_or((0, 0));

            snapshots.push(ProcessSnapshot {
                exe_name,
                pid,
                bytes_sent: total_sent,
                bytes_recv: total_recv,
                timestamp: now,
            });
        }

        snapshots
    }

    pub fn get_session_totals(&self) -> &HashMap<u32, (u64, u64)> {
        &self.session_totals
    }
}
