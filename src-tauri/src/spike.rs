//! Kontrak satuan dan rate-limit notifikasi spike.
//!
//! Semua perhitungan memakai **byte/detik**. `spike_threshold_mib` artinya
//! MiB/detik, bukan MiB per siklus capture. Dengan begitu mengubah
//! `capture_interval_secs` tidak mengubah arti threshold.
//!
//! Rate-limit memakai `Instant` (monoton), bukan `Utc::now()`: kalau jam sistem
//! mundur, notifikasi tidak boleh mati sampai waktu "kejar" timestamp lama.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const MIB: u64 = 1024 * 1024;

/// Rate byte/detik dari delta byte dan lamanya siklus. Siklus 0 detik sengaja
/// menghasilkan 0 (bukan pembagian tak terdefinisi / angka meledak).
pub fn rate_bps(bytes: u64, elapsed_secs: u64) -> u64 {
    bytes.checked_div(elapsed_secs).unwrap_or(0)
}

pub fn threshold_bps(threshold_mib: u64) -> u64 {
    threshold_mib.saturating_mul(MIB)
}

pub fn is_spike(bytes: u64, elapsed_secs: u64, threshold_mib: u64) -> bool {
    rate_bps(bytes, elapsed_secs) >= threshold_bps(threshold_mib)
}

pub fn rate_label(bytes_per_second: u64) -> String {
    format!("{:.1} MiB/s", bytes_per_second as f64 / MIB as f64)
}

/// Identitas notifikasi: PID saja bisa didaur ulang, exe_name saja bikin dua
/// proses dengan nama sama saling menekan. Dua-duanya dipakai.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpikeKey {
    pub pid: u32,
    pub exe_name: String,
}

/// Rate-limit dua lapis:
/// - per proses: maksimal satu notifikasi per `per_process`;
/// - global: maksimal satu notifikasi per `global` supaya 30 proses yang spike
///   bersamaan tidak membanjiri tray.
#[derive(Debug)]
pub struct SpikeLimiter {
    by_process: HashMap<SpikeKey, Instant>,
    last_any: Option<Instant>,
    per_process: Duration,
    global: Duration,
}

impl SpikeLimiter {
    pub fn new(per_process: Duration, global: Duration) -> Self {
        Self {
            by_process: HashMap::new(),
            last_any: None,
            per_process,
            global,
        }
    }

    /// `now` sengaja disuntik supaya bisa di-test. `Instant` tidak bisa mundur,
    /// tapi `saturating_duration_since` juga aman kalau caller salah memberi
    /// nilai lebih kecil (tidak panic, tidak membuka rate-limit).
    pub fn allow(&mut self, key: SpikeKey, now: Instant) -> bool {
        self.by_process
            .retain(|_, last| now.saturating_duration_since(*last) < self.per_process);

        if self
            .last_any
            .is_some_and(|last| now.saturating_duration_since(last) < self.global)
        {
            return false;
        }

        if self
            .by_process
            .get(&key)
            .is_some_and(|last| now.saturating_duration_since(*last) < self.per_process)
        {
            return false;
        }

        self.by_process.insert(key, now);
        self.last_any = Some(now);
        true
    }

    /// Dipakai test untuk memastikan cache lama benar-benar dibuang.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_process.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_memakai_detik_bukan_siklus() {
        assert_eq!(rate_bps(100, 0), 0);
        assert_eq!(rate_bps(100, 2), 50);
        assert_eq!(rate_bps(100, 5), 20);
    }

    #[test]
    fn threshold_mib_adalah_per_detik() {
        assert_eq!(threshold_bps(1), MIB);
        // 100 MiB terkumpul dalam 5 detik = 20 MiB/s -> kena threshold 10 MiB/s.
        assert!(is_spike(100 * MIB, 5, 10));
        // Delta sama tapi 20 detik = 5 MiB/s -> tidak kena.
        assert!(!is_spike(100 * MIB, 20, 10));
        assert_eq!(rate_label(20 * MIB), "20.0 MiB/s");
    }

    #[test]
    fn rate_limit_per_proses_dan_global() {
        let mut limiter = SpikeLimiter::new(Duration::from_secs(600), Duration::from_secs(10));
        let t0 = Instant::now();
        let a = SpikeKey {
            pid: 100,
            exe_name: "app.exe".into(),
        };
        let b = SpikeKey {
            pid: 101,
            exe_name: "app.exe".into(),
        };

        assert!(limiter.allow(a.clone(), t0));
        // Proses lain nama sama tetap kena throttling global.
        assert!(!limiter.allow(b.clone(), t0));
        // Global lewat, tapi proses `a` masih dalam cooldown 10 menit.
        assert!(limiter.allow(b.clone(), t0 + Duration::from_secs(11)));
        assert!(!limiter.allow(a.clone(), t0 + Duration::from_secs(11)));
        // Setelah 10 menit, `a` boleh lagi.
        assert!(limiter.allow(a, t0 + Duration::from_secs(601)));
    }

    #[test]
    fn jam_mundur_tidak_mengubah_keputusan() {
        let mut limiter = SpikeLimiter::new(Duration::from_secs(600), Duration::from_secs(1));
        let t0 = Instant::now();
        let key = SpikeKey {
            pid: 7,
            exe_name: "svc.exe".into(),
        };

        assert!(limiter.allow(key.clone(), t0 + Duration::from_secs(100)));
        // "Waktu" mundur: saturating_duration_since mengembalikan 0, jadi tetap
        // diblokir dan tidak panic. Tidak ada Utc::now() yang bisa macet.
        assert!(!limiter.allow(key.clone(), t0));
        assert!(limiter.allow(key, t0 + Duration::from_secs(701)));
    }

    #[test]
    fn cache_rate_limit_dipangkas() {
        let mut limiter = SpikeLimiter::new(Duration::from_secs(10), Duration::from_secs(1));
        let t0 = Instant::now();
        let old = SpikeKey {
            pid: 1,
            exe_name: "old.exe".into(),
        };
        let new = SpikeKey {
            pid: 2,
            exe_name: "new.exe".into(),
        };

        assert!(limiter.allow(old, t0));
        // Lewati cooldown 10 detik: entri lama harus dibuang saat allow()
        // berikutnya, walaupun proses barinya berbeda.
        assert!(limiter.allow(new, t0 + Duration::from_secs(11)));
        assert_eq!(limiter.len(), 1, "entri lama harus dibuang");
    }
}
