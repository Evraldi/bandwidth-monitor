use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessUsage {
    pub exe_name: String,
    pub pid: u32,
    pub upload_bps: u64,      // bytes per second
    pub download_bps: u64,    // bytes per second
    pub total_sent: u64,      // total bytes sent this session
    pub total_recv: u64,      // total bytes received this session
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySummary {
    pub exe_name: String,
    pub total_sent: u64,
    pub total_recv: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthlySummary {
    pub exe_name: String,
    pub total_sent: u64,
    pub total_recv: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyDataPoint {
    pub date: String,         // YYYY-MM-DD
    pub total_sent: u64,
    pub total_recv: u64,
}

#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub pid: u32,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub exe_name: String,
    pub pid: u32,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub timestamp: i64,
}
