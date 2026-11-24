//! Validasi di boundary command. Jangan pernah percaya nilai yang datang dari
//! renderer, walaupun UI sudah punya atribut min/max.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::types::Settings;

pub const MAX_EXPORT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_HISTORY_DAYS: u32 = 3650;

pub fn parse_date(input: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(input, "%Y-%m-%d")
        .map_err(|_| format!("Tanggal tidak valid: {input}. Pakai format YYYY-MM-DD."))
}

pub fn validate_date(input: &str) -> Result<String, String> {
    parse_date(input)?;
    Ok(input.to_string())
}

pub fn validate_date_range(start: &str, end: &str) -> Result<(String, String), String> {
    let start_date = parse_date(start)?;
    let end_date = parse_date(end)?;
    if start_date > end_date {
        return Err("Tanggal mulai tidak boleh setelah tanggal akhir.".into());
    }
    Ok((start.to_string(), end.to_string()))
}

pub fn validate_year_month(year: u32, month: u32) -> Result<(u32, u32), String> {
    if !(1970..=9990).contains(&year) {
        return Err("Tahun harus antara 1970 dan 9990.".into());
    }
    if !(1..=12).contains(&month) {
        return Err("Bulan harus antara 1 dan 12.".into());
    }
    Ok((year, month))
}

pub fn validate_days(days: u32) -> Result<u32, String> {
    if days == 0 || days > MAX_HISTORY_DAYS {
        return Err(format!(
            "Rentang hari harus antara 1 dan {MAX_HISTORY_DAYS}."
        ));
    }
    Ok(days)
}

pub fn validate_exe_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Nama proses tidak boleh kosong.".into());
    }
    if trimmed.len() > 260 {
        return Err("Nama proses terlalu panjang.".into());
    }
    Ok(())
}

/// Nilai yang keluar dari DB lama/index korup dibungkus ke rentang aman.
/// Beda dengan `validate_settings`, yang menolak input baru dari UI.
pub fn sanitize_settings(settings: Settings) -> Settings {
    Settings {
        poll_interval_secs: settings.poll_interval_secs.clamp(1, 60),
        capture_interval_secs: settings.capture_interval_secs.clamp(1, 60),
        retention_days: settings.retention_days.clamp(1, 365),
        dark_mode: settings.dark_mode,
        notifications_enabled: settings.notifications_enabled,
        spike_threshold_mib: settings.spike_threshold_mib.clamp(1, 10_240),
    }
}

pub fn validate_settings(settings: Settings) -> Result<Settings, String> {
    let clean = sanitize_settings(settings.clone());
    if clean.poll_interval_secs != settings.poll_interval_secs {
        return Err("Interval refresh harus 1-60 detik.".into());
    }
    if clean.capture_interval_secs != settings.capture_interval_secs {
        return Err("Interval capture harus 1-60 detik.".into());
    }
    if clean.retention_days != settings.retention_days {
        return Err("Retensi harus 1-365 hari.".into());
    }
    if clean.spike_threshold_mib != settings.spike_threshold_mib {
        return Err("Threshold spike harus 1-10240 MiB/detik.".into());
    }
    Ok(clean)
}

/// Validasi murni supaya bisa di-test tanpa AppHandle.
///
/// `allowed_roots` harus sudah di-canonicalize. Path yang belum ada
/// divalidasi lewat parent-nya; path yang sudah ada di-canonicalize supaya
/// symlink yang keluar dari root ditolak.
pub fn validate_export_path(
    target: &Path,
    contents: &str,
    allowed_roots: &[PathBuf],
) -> Result<(), String> {
    if contents.len() > MAX_EXPORT_BYTES {
        return Err(format!(
            "Isi export terlalu besar (maks. {} MiB).",
            MAX_EXPORT_BYTES / 1024 / 1024
        ));
    }

    let extension = target
        .extension()
        .and_then(|v| v.to_str())
        .map(|v| v.to_ascii_lowercase())
        .ok_or_else(|| "Export harus berekstensi .csv atau .json.".to_string())?;
    if extension != "csv" && extension != "json" {
        return Err("Export hanya mendukung .csv atau .json.".into());
    }

    if !target.is_absolute() {
        return Err("Path export harus absolut.".into());
    }

    let canonical = if target.exists() {
        target
            .canonicalize()
            .map_err(|e| format!("Gagal memvalidasi path export: {e}"))?
    } else {
        let parent = target
            .parent()
            .ok_or_else(|| "Path export tidak punya direktori induk.".to_string())?;
        let parent = parent
            .canonicalize()
            .map_err(|e| format!("Direktori export tidak ditemukan: {e}"))?;
        let file_name = target
            .file_name()
            .ok_or_else(|| "Nama file export tidak valid.".to_string())?;
        parent.join(file_name)
    };

    let candidate = canonical.to_string_lossy().to_lowercase();
    let allowed = allowed_roots.iter().any(|root| {
        let root = root.to_string_lossy().to_lowercase();
        let root = root.trim_end_matches(std::path::MAIN_SEPARATOR).to_string();
        candidate == root || candidate.starts_with(&format!("{root}{}", std::path::MAIN_SEPARATOR))
    });

    if !allowed {
        return Err(
            "Export hanya boleh disimpan di Downloads, Documents, Desktop, atau folder Export aplikasi."
                .into(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tanggal_dan_rentang_divalidasi() {
        assert!(parse_date("2026-11-24").is_ok());
        assert!(parse_date("2026-2-30").is_err());
        assert!(parse_date("24-11-2026").is_err());
        assert!(validate_date_range("2026-11-01", "2026-11-24").is_ok());
        assert!(validate_date_range("2026-11-25", "2026-11-24").is_err());
    }

    #[test]
    fn tahun_bulan_dan_hari_dibatasi() {
        assert!(validate_year_month(2026, 12).is_ok());
        assert!(validate_year_month(2026, 0).is_err());
        assert!(validate_year_month(2026, 13).is_err());
        assert!(validate_year_month(u32::MAX, 1).is_err());
        assert!(validate_days(3650).is_ok());
        assert!(validate_days(0).is_err());
        assert!(validate_days(u32::MAX).is_err());
    }

    #[test]
    fn settings_ditolak_atau_dibersihkan() {
        let settings = Settings {
            capture_interval_secs: 0,
            ..Settings::default()
        };
        assert!(validate_settings(settings.clone()).is_err());
        assert_eq!(sanitize_settings(settings).capture_interval_secs, 1);

        let settings = Settings::default();
        assert!(validate_settings(settings).is_ok());
    }

    #[test]
    fn export_path_hanya_boleh_di_root_yang_diizinkan() {
        let tmp = std::env::temp_dir().join(format!("bm-export-test-{}", std::process::id()));
        let allowed_dir = tmp.join("allowed");
        let blocked_dir = tmp.join("blocked");
        std::fs::create_dir_all(&allowed_dir).unwrap();
        std::fs::create_dir_all(&blocked_dir).unwrap();

        let roots = vec![allowed_dir.canonicalize().unwrap()];
        let good = allowed_dir.join("laporan.csv");
        let bad = blocked_dir.join("laporan.csv");
        let wrong_ext = allowed_dir.join("laporan.exe");

        assert!(validate_export_path(&good, "a,b\n", &roots).is_ok());
        assert!(validate_export_path(&bad, "a,b\n", &roots).is_err());
        assert!(validate_export_path(&wrong_ext, "a,b\n", &roots).is_err());
        assert!(validate_export_path(&good, &"x".repeat(MAX_EXPORT_BYTES + 1), &roots).is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
