use crate::time_range::local_day_bounds;
use crate::types::{DailyDataPoint, DailySummary, MonthlySummary, ProcessSnapshot, Settings};
use crate::validation::{
    parse_date, sanitize_settings, validate_date_range, validate_days, validate_year_month,
};
use chrono::{Duration as ChronoDuration, Local};
use rusqlite::{params, Connection, OptionalExtension, Result};
use std::path::PathBuf;
use std::time::Duration;

/// History ringkasan harian sengaja disimpan 10 tahun. Jangan samakan dengan
/// `retention_days` (7 hari) yang hanya berlaku untuk snapshot mentah.
pub const SUMMARY_RETENTION_DAYS: u32 = 3650;

/// Batas baris untuk query ringkasan. UI tidak bisa menampilkan jutaan baris
/// sekaligus; tanpa LIMIT, export/JSON IPC bisa menghabiskan memori.
pub const MAX_SUMMARY_ROWS: i64 = 10_000;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        // WAL + synchronous NORMAL: much faster for the snapshot-heavy insert
        // pattern while keeping crash-safety (and letting readers/History
        // queries run without blocking on the writer).
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;

        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS usage_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                exe_name TEXT NOT NULL,
                pid INTEGER NOT NULL,
                bytes_sent INTEGER NOT NULL,
                bytes_recv INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS daily_summary (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL,
                exe_name TEXT NOT NULL,
                total_sent INTEGER NOT NULL,
                total_recv INTEGER NOT NULL,
                UNIQUE(date, exe_name)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_snapshots_ts ON usage_snapshots(timestamp)",
            [],
        )?;

        // `UNIQUE(date, exe_name)` sudah membuat indeks otomatis dengan
        // `date` sebagai prefix. Indeks eksplisit lama dibuang (juga untuk DB
        // yang sudah ada) supaya tidak ada duplikat yang memperlambat insert.
        conn.execute("DROP INDEX IF EXISTS idx_daily_date", [])?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_daily_exe_date ON daily_summary(exe_name, date)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Database { conn })
    }

    pub fn insert_snapshots(&self, snapshots: &[ProcessSnapshot]) -> Result<()> {
        if snapshots.is_empty() {
            return Ok(());
        }

        // Single transaction per batch (one capture cycle) instead of one
        // autocommit insert per row — the main write throughput win.
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO usage_snapshots (timestamp, exe_name, pid, bytes_sent, bytes_recv)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for snapshot in snapshots {
                stmt.execute(params![
                    snapshot.timestamp,
                    snapshot.exe_name,
                    snapshot.pid,
                    snapshot.bytes_sent,
                    snapshot.bytes_recv
                ])?;
            }
        }
        tx.commit()
    }

    /// WAL bisa tumbuh terus kalau tidak pernah di-checkpoint (proses lama
    /// yang bisa membaca snapshot). Dipanggil dari cleanup tiap jam.
    /// `busy != 0`
    /// berarti ada reader/write aktif; ini bukan error, cukup dicatat.
    pub fn checkpoint_wal(&self) -> Result<()> {
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            log::warn!("WAL checkpoint tertunda (busy={busy}); dicoba lagi nanti");
        }
        Ok(())
    }

    pub fn get_daily_summary(&self, date: &str) -> Result<Vec<DailySummary>> {
        parse_date(date).map_err(rusqlite::Error::InvalidParameterName)?;
        let mut stmt = self.conn.prepare(
            "SELECT exe_name, total_sent, total_recv 
             FROM daily_summary 
             WHERE date = ?1
             ORDER BY (total_sent + total_recv) DESC
             LIMIT ?2",
        )?;

        let summaries = stmt
            .query_map(params![date, MAX_SUMMARY_ROWS], |row| {
                Ok(DailySummary {
                    exe_name: row.get(0)?,
                    total_sent: row.get(1)?,
                    total_recv: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        Ok(summaries)
    }

    pub fn get_monthly_summary(&self, year: u32, month: u32) -> Result<Vec<MonthlySummary>> {
        let (year, month) =
            validate_year_month(year, month).map_err(rusqlite::Error::InvalidParameterName)?;
        let start_date = format!("{:04}-{:02}-01", year, month);
        let end_date = if month == 12 {
            format!("{:04}-01-01", year + 1)
        } else {
            format!("{:04}-{:02}-01", year, month + 1)
        };

        let mut stmt = self.conn.prepare(
            "SELECT exe_name, SUM(total_sent), SUM(total_recv)
             FROM daily_summary
             WHERE date >= ?1 AND date < ?2
             GROUP BY exe_name
             ORDER BY (SUM(total_sent) + SUM(total_recv)) DESC
             LIMIT ?3",
        )?;

        let summaries = stmt
            .query_map(params![start_date, end_date, MAX_SUMMARY_ROWS], |row| {
                Ok(MonthlySummary {
                    exe_name: row.get(0)?,
                    total_sent: row.get(1)?,
                    total_recv: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        Ok(summaries)
    }

    pub fn get_history_chart(&self, exe_name: &str, days: u32) -> Result<Vec<DailyDataPoint>> {
        let days = validate_days(days).map_err(rusqlite::Error::InvalidParameterName)?;
        let end_date = Local::now().naive_local().date();
        let start_date = end_date
            .checked_sub_signed(ChronoDuration::days(days as i64))
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Rentang tanggal overflow".into())
            })?;

        let mut stmt = self.conn.prepare(
            "SELECT date, total_sent, total_recv
             FROM daily_summary
             WHERE exe_name = ?1 AND date >= ?2 AND date <= ?3
             ORDER BY date ASC
             LIMIT ?4",
        )?;

        let data_points = stmt
            .query_map(
                params![
                    exe_name,
                    start_date.to_string(),
                    end_date.to_string(),
                    i64::from(days) + 1
                ],
                |row| {
                    Ok(DailyDataPoint {
                        date: row.get(0)?,
                        total_sent: row.get(1)?,
                        total_recv: row.get(2)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>>>()?;

        Ok(data_points)
    }

    pub fn get_daily_summary_range(&self, start: &str, end: &str) -> Result<Vec<DailySummary>> {
        let (start, end) =
            validate_date_range(start, end).map_err(rusqlite::Error::InvalidParameterName)?;
        let mut stmt = self.conn.prepare(
            "SELECT exe_name, SUM(total_sent), SUM(total_recv)
             FROM daily_summary
             WHERE date >= ?1 AND date <= ?2
             GROUP BY exe_name
             ORDER BY (SUM(total_sent) + SUM(total_recv)) DESC
             LIMIT ?3",
        )?;

        let summaries = stmt
            .query_map(params![start, end, MAX_SUMMARY_ROWS], |row| {
                Ok(DailySummary {
                    exe_name: row.get(0)?,
                    total_sent: row.get(1)?,
                    total_recv: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        Ok(summaries)
    }

    pub fn get_monthly_daily_totals(&self, year: u32, month: u32) -> Result<Vec<DailyDataPoint>> {
        let (year, month) =
            validate_year_month(year, month).map_err(rusqlite::Error::InvalidParameterName)?;
        let start_date = format!("{:04}-{:02}-01", year, month);
        let end_date = if month == 12 {
            format!("{:04}-01-01", year + 1)
        } else {
            format!("{:04}-{:02}-01", year, month + 1)
        };

        let mut stmt = self.conn.prepare(
            "SELECT date, SUM(total_sent), SUM(total_recv)
             FROM daily_summary
             WHERE date >= ?1 AND date < ?2
             GROUP BY date
             ORDER BY date ASC
             LIMIT ?3",
        )?;

        let data_points = stmt
            .query_map(params![start_date, end_date, MAX_SUMMARY_ROWS], |row| {
                Ok(DailyDataPoint {
                    date: row.get(0)?,
                    total_sent: row.get(1)?,
                    total_recv: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        Ok(data_points)
    }

    pub fn aggregate_daily(&self, date: &str) -> Result<()> {
        let naive_date = parse_date(date).map_err(rusqlite::Error::InvalidParameterName)?;
        let (start_ts, end_ts) = local_day_bounds(naive_date).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("Rentang hari tidak valid (DST)".into())
        })?;

        // Interval setengah terbuka `[start, end)`: tidak ada celah di
        // 23:59:59 dan tidak panic kalau tengah malam ambigu/tidak ada.
        self.conn.execute(
            "INSERT OR REPLACE INTO daily_summary (date, exe_name, total_sent, total_recv)
             SELECT ?1, exe_name, SUM(bytes_sent), SUM(bytes_recv)
             FROM usage_snapshots
             WHERE timestamp >= ?2 AND timestamp < ?3
             GROUP BY exe_name",
            params![date, start_ts, end_ts],
        )?;

        Ok(())
    }

    pub fn cleanup_old_snapshots(&self, days_to_keep: u32) -> Result<()> {
        // Wall-clock: tepat N × 24 jam sebelum sekarang, bukan "tengah malam
        // N hari yang lalu". Pendekatan lama bisa membuang data yang baru
        // berumur N hari (atau gagal di hari dengan transisi DST).
        let days = i64::from(days_to_keep.clamp(1, 365));
        let window = days
            .checked_mul(24 * 60 * 60)
            .ok_or_else(|| rusqlite::Error::InvalidParameterName("Retensi overflow".into()))?;
        let cutoff_ts = Local::now()
            .timestamp()
            .checked_sub(window)
            .ok_or_else(|| rusqlite::Error::InvalidParameterName("Retensi overflow".into()))?;

        self.conn.execute(
            "DELETE FROM usage_snapshots WHERE timestamp < ?1",
            params![cutoff_ts],
        )?;

        // `daily_summary` sengaja tidak ikut terkena retensi 7 hari:
        // ringkasan harian adalah history jangka panjang. Ada batas tersendiri
        // di `cleanup_old_summaries` supaya tetap tidak tumbuh tanpa batas.
        Ok(())
    }

    /// Ringkasan harian dipertahankan jauh lebih lama dari snapshot mentah.
    /// Pemanggilan produksi memakai `SUMMARY_RETENTION_DAYS` (10 tahun), jadi
    /// history bulanan user tidak hilang karena ganti retention_days.
    pub fn cleanup_old_summaries(&self, days_to_keep: u32) -> Result<()> {
        let days = i64::from(days_to_keep.clamp(1, SUMMARY_RETENTION_DAYS));
        let cutoff = Local::now()
            .date_naive()
            .checked_sub_signed(ChronoDuration::days(days))
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Retensi ringkasan overflow".into())
            })?
            .format("%Y-%m-%d")
            .to_string();

        self.conn
            .execute("DELETE FROM daily_summary WHERE date < ?1", params![cutoff])?;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<Settings> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = 'app'", [], |row| {
                row.get(0)
            })
            .optional()?;

        match value {
            Some(raw) => match serde_json::from_str(&raw) {
                Ok(settings) => Ok(sanitize_settings(settings)),
                Err(error) => {
                    // Settings korup tidak boleh reset diam-diam: user perlu
                    // tahu di log kenapa preferensinya kembali ke default.
                    log::warn!("Settings tersimpan korup ({error}); pakai default");
                    Ok(Settings::default())
                }
            },
            None => Ok(Settings::default()),
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let raw = serde_json::to_string(settings)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('app', ?1)",
            params![raw],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn timestamp(date: &str, hour: u32) -> i64 {
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .earliest()
            .unwrap()
            .timestamp()
    }

    fn snapshot(date: &str, hour: u32, exe_name: &str, sent: u64, recv: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            exe_name: exe_name.into(),
            pid: 42,
            bytes_sent: sent,
            bytes_recv: recv,
            timestamp: timestamp(date, hour),
        }
    }

    fn memory_db() -> Database {
        Database::new(PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn agregasi_harian_menjumlah_per_proses() {
        let db = memory_db();
        let date = "2026-11-23";
        db.insert_snapshots(&[
            snapshot(date, 12, "chrome.exe", 100, 200),
            snapshot(date, 13, "chrome.exe", 50, 70),
            snapshot(date, 12, "node.exe", 10, 20),
        ])
        .unwrap();

        db.aggregate_daily(date).unwrap();

        let rows = db.get_daily_summary(date).unwrap();
        assert_eq!(rows.len(), 2);
        let chrome = rows
            .iter()
            .find(|row| row.exe_name == "chrome.exe")
            .unwrap();
        assert_eq!((chrome.total_sent, chrome.total_recv), (150, 270));

        let range = db.get_daily_summary_range(date, date).unwrap();
        assert_eq!(range.len(), 2);
        let monthly = db.get_monthly_summary(2026, 11).unwrap();
        let node = monthly
            .iter()
            .find(|row| row.exe_name == "node.exe")
            .unwrap();
        assert_eq!((node.total_sent, node.total_recv), (10, 20));
    }

    #[test]
    fn konversi_settings_lama_tidak_bikin_panic() {
        let db = memory_db();
        db.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES ('app', ?1)",
                params!["{\"poll_interval_secs\":0,\"capture_interval_secs\":999,\"retention_days\":0,\"dark_mode\":true,\"notifications_enabled\":false,\"spike_threshold_mb\":0}"],
            )
            .unwrap();

        let settings = db.get_settings().unwrap();
        assert_eq!(settings.poll_interval_secs, 1);
        assert_eq!(settings.capture_interval_secs, 60);
        assert_eq!(settings.retention_days, 1);
        // Field lama `spike_threshold_mb` tetap dibaca via serde alias.
        assert_eq!(settings.spike_threshold_mib, 1);
        assert!(settings.dark_mode);
        assert!(!settings.notifications_enabled);
    }

    #[test]
    fn retensi_wall_clock_tidak_menghapus_ringkasan_bulanan() {
        let db = memory_db();
        let now = Local::now().timestamp();

        db.conn
            .execute(
                "INSERT INTO usage_snapshots (timestamp, exe_name, pid, bytes_sent, bytes_recv)
                 VALUES (?1, 'lama.exe', 1, 1, 1), (?2, 'baru.exe', 2, 2, 2)",
                params![now - 8 * 24 * 3600, now - 6 * 24 * 3600],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO daily_summary (date, exe_name, total_sent, total_recv)
                 VALUES ('2026-01-01', 'lama.exe', 10, 10)",
                [],
            )
            .unwrap();

        db.cleanup_old_snapshots(7).unwrap();

        let names: Vec<String> = db
            .conn
            .prepare("SELECT exe_name FROM usage_snapshots")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(names, vec!["baru.exe"]);

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM daily_summary", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "history bulanan tidak boleh ikut terhapus");
    }

    #[test]
    fn ringkasan_lama_dipangkas_tapi_history_bulanan_aman() {
        let db = memory_db();
        db.conn
            .execute(
                "INSERT INTO daily_summary (date, exe_name, total_sent, total_recv)
                 VALUES ('2026-01-01', 'lama.exe', 1, 1), ('2026-11-23', 'baru.exe', 2, 2)",
                [],
            )
            .unwrap();

        // Retensi 7 hari (default snapshot) TIDAK boleh menyentuh ringkasan.
        db.cleanup_old_summaries(SUMMARY_RETENTION_DAYS).unwrap();
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM daily_summary", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2, "history September/Januari harus tetap ada");

        // Batas ringkasan 10 tahun baru membuang yang benar-benar usang.
        db.cleanup_old_summaries(30).unwrap();
        let names: Vec<String> = db
            .conn
            .prepare("SELECT exe_name FROM daily_summary")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(names, vec!["baru.exe"]);
    }

    #[test]
    fn settings_korup_dibersihkan_tanpa_panic() {
        let db = memory_db();
        db.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES ('app', '{json rusuh')",
                [],
            )
            .unwrap();
        let settings = db.get_settings().unwrap();
        assert_eq!(
            settings.spike_threshold_mib,
            Settings::default().spike_threshold_mib
        );
        assert_eq!(
            settings.capture_interval_secs,
            Settings::default().capture_interval_secs
        );
    }

    #[test]
    fn input_invalid_ditolak_sebelum_query() {
        let db = memory_db();
        assert!(db.get_daily_summary("2026-13-01").is_err());
        assert!(db
            .get_daily_summary_range("2026-11-30", "2026-11-01")
            .is_err());
        assert!(db.get_monthly_summary(2026, 0).is_err());
        assert!(db.get_monthly_daily_totals(2026, 13).is_err());
        assert!(db.get_history_chart("chrome.exe", 0).is_err());
        assert!(db.aggregate_daily("besok").is_err());
    }

    #[test]
    fn pragma_dan_indeks_tidak_redundan() {
        let db = memory_db();
        let busy: i64 = db
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy, 5000, "busy_timeout wajib aktif");

        let redundant: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_daily_date'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            redundant, 0,
            "idx_daily_date duplikat UNIQUE(date, exe_name)"
        );

        let kept: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_daily_exe_date'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "indeks (exe_name, date) tetap dipakai");

        db.conn
            .execute(
                "INSERT INTO daily_summary (date, exe_name, total_sent, total_recv)
                 VALUES ('2026-11-24', 'dup.exe', 1, 1)",
                [],
            )
            .unwrap();
        assert!(db
            .conn
            .execute(
                "INSERT INTO daily_summary (date, exe_name, total_sent, total_recv)
                 VALUES ('2026-11-24', 'dup.exe', 2, 2)",
                [],
            )
            .is_err());
    }

    #[test]
    fn wal_checkpoint_bisa_dijalankan_di_file_db() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("bm-wal-{}-{stamp}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));

        let db = Database::new(path.clone()).unwrap();
        let journal: String = db
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal");

        let rows: Vec<ProcessSnapshot> = (0..100)
            .map(|n| snapshot("2026-11-24", 12, "wal.exe", n, n))
            .collect();
        db.insert_snapshots(&rows).unwrap();
        db.checkpoint_wal().unwrap();

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM usage_snapshots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 100, "checkpoint tidak boleh menghilangkan data");
        drop(db);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn query_ringkasan_dan_chart_dibatasi() {
        let db = memory_db();
        db.conn
            .execute(
                "WITH RECURSIVE seq(n) AS (
                     SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1
                 )
                 INSERT INTO daily_summary (date, exe_name, total_sent, total_recv)
                 SELECT '2026-11-24', 'p' || n, n, n FROM seq",
                params![MAX_SUMMARY_ROWS + 1],
            )
            .unwrap();

        let expected = MAX_SUMMARY_ROWS as usize;
        assert_eq!(db.get_daily_summary("2026-11-24").unwrap().len(), expected);
        assert_eq!(
            db.get_daily_summary_range("2026-11-24", "2026-11-24")
                .unwrap()
                .len(),
            expected
        );
        assert_eq!(db.get_monthly_summary(2026, 11).unwrap().len(), expected);

        db.conn
            .execute(
                "WITH RECURSIVE seq(n) AS (
                     SELECT 0 UNION ALL SELECT n + 1 FROM seq WHERE n < 4000
                 )
                 INSERT INTO daily_summary (date, exe_name, total_sent, total_recv)
                 SELECT date('2026-11-24', '-' || n || ' days'), 'chart.exe', n, n FROM seq",
                [],
            )
            .unwrap();
        let points = db.get_history_chart("chart.exe", 3650).unwrap();
        assert!(points.len() <= 3651, "chart dibatasi jumlah hari + 1");
    }

    #[test]
    fn agregasi_menangkap_detik_terakhir_hari() {
        let db = memory_db();
        let date = "2026-11-23";
        let last_second = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_local_timezone(Local)
            .earliest()
            .unwrap()
            .timestamp();
        db.insert_snapshots(&[ProcessSnapshot {
            exe_name: "late.exe".into(),
            pid: 9,
            bytes_sent: 7,
            bytes_recv: 8,
            timestamp: last_second,
        }])
        .unwrap();

        db.aggregate_daily(date).unwrap();
        let rows = db.get_daily_summary(date).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].total_sent, rows[0].total_recv), (7, 8));
    }
}
