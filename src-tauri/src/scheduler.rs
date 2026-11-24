//! Penjadwalan berbasis waktu berjalan (bukan jam dinding).
//!
//! Pendekatan lama `tick_count % 120` mengikat interval agregasi ke
//! `capture_interval_secs`, dan cleanup cuma jalan kalau app kebetulan hidup
//! di menit 00:00-00:05. Di sini semua diukur dengan `Instant`.

use chrono::{Duration as ChronoDuration, NaiveDate};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Periodic {
    last: Option<Instant>,
}

impl Default for Periodic {
    fn default() -> Self {
        Self::new()
    }
}

impl Periodic {
    pub fn new() -> Self {
        Self { last: None }
    }

    /// First call hanya menandai baseline, tidak langsung jalan. Ini mencegah
    /// agregasi/cleanup berulang kali saat aplikasi baru start.
    pub fn due(&mut self, now: Instant, interval: Duration) -> bool {
        match self.last {
            None => {
                self.last = Some(now);
                false
            }
            Some(last) if now.duration_since(last) >= interval => {
                self.last = Some(now);
                true
            }
            Some(_) => false,
        }
    }
}

/// Tanggal yang perlu diagregasi saat exit: hari ini dan kemarin. Kemarin
/// ikut supaya data tidak hilang kalau aplikasi menyala lintas tengah malam
/// dan tidak sempat menjalankan agregasi terjadwal.
pub fn exit_aggregation_dates(today: NaiveDate) -> [String; 2] {
    let yesterday = today
        .checked_sub_signed(ChronoDuration::days(1))
        .unwrap_or(today);
    [
        today.format("%Y-%m-%d").to_string(),
        yesterday.format("%Y-%m-%d").to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_tidak_langsung_jalan() {
        let mut tick = Periodic::new();
        let start = Instant::now();
        assert!(!tick.due(start, Duration::from_secs(600)));
        assert!(!tick.due(start + Duration::from_secs(599), Duration::from_secs(600)));
        assert!(tick.due(start + Duration::from_secs(600), Duration::from_secs(600)));
        // Setelah jalan, baseline direset.
        assert!(!tick.due(start + Duration::from_secs(601), Duration::from_secs(600)));
    }

    #[test]
    fn exit_agregasi_hari_ini_dan_kemarin() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        assert_eq!(
            exit_aggregation_dates(today),
            ["2026-11-24".to_string(), "2026-11-23".to_string()]
        );
    }
}
