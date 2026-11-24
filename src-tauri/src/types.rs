use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub poll_interval_secs: u64,
    pub capture_interval_secs: u64,
    pub retention_days: u32,
    pub dark_mode: bool,
    pub notifications_enabled: bool,
    /// Ambang spike dalam MiB **per detik** (bukan per siklus capture).
    #[serde(alias = "spike_threshold_mb")]
    pub spike_threshold_mib: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_interval_secs: 2,
            capture_interval_secs: 5,
            retention_days: 7,
            dark_mode: false,
            notifications_enabled: true,
            spike_threshold_mib: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessUsage {
    pub exe_name: String,
    pub pid: u32,
    /// Byte/detik dihitung backend dari delta capture, bukan diserahkan ke UI.
    pub upload_bps: u64,
    pub download_bps: u64,
    /// Total byte sejak aplikasi berjalan (kumulatif, bukan delta).
    pub total_sent: u64,
    pub total_recv: u64,
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
    pub date: String, // YYYY-MM-DD
    pub total_sent: u64,
    pub total_recv: u64,
}

#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub bytes_sent: u64,
    pub bytes_recv: u64,
}

/// Kesehatan sumber data. Dipakai supaya event loss tidak senyap di release.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureHealth {
    pub etw_active: bool,
    /// ETW consumer thread masih hidup.
    pub session_running: bool,
    /// Kumulatif event yang dibuang ETW.
    pub events_lost: u64,
    /// Kumulatif buffer yang hilang (log + realtime).
    pub buffers_lost: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub exe_name: String,
    pub pid: u32,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub timestamp: i64,
}
