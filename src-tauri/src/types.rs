use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub poll_interval_secs: u64,
    pub capture_interval_secs: u64,
    pub retention_days: u32,
    pub dark_mode: bool,
    pub notifications_enabled: bool,
    pub spike_threshold_mb: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_interval_secs: 2,
            capture_interval_secs: 5,
            retention_days: 7,
            dark_mode: false,
            notifications_enabled: true,
            spike_threshold_mb: 100,
        }
    }
}

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
