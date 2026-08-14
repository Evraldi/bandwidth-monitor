use rusqlite::{Connection, Result, params};
use rusqlite::OptionalExtension;
use chrono::{Local, NaiveDate};
use std::path::PathBuf;
use crate::types::{ProcessSnapshot, DailySummary, MonthlySummary, DailyDataPoint, Settings};

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

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_daily_date ON daily_summary(date)",
            [],
        )?;

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

    pub fn get_daily_summary(&self, date: &str) -> Result<Vec<DailySummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT exe_name, total_sent, total_recv 
             FROM daily_summary 
             WHERE date = ?1 
             ORDER BY (total_sent + total_recv) DESC"
        )?;

        let summaries = stmt.query_map(params![date], |row| {
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
             ORDER BY (SUM(total_sent) + SUM(total_recv)) DESC"
        )?;

        let summaries = stmt.query_map(params![start_date, end_date], |row| {
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
        let end_date = Local::now().naive_local().date();
        let start_date = end_date - chrono::Duration::days(days as i64);

        let mut stmt = self.conn.prepare(
            "SELECT date, total_sent, total_recv
             FROM daily_summary
             WHERE exe_name = ?1 AND date >= ?2 AND date <= ?3
             ORDER BY date ASC"
        )?;

        let data_points = stmt.query_map(
            params![exe_name, start_date.to_string(), end_date.to_string()],
            |row| {
                Ok(DailyDataPoint {
                    date: row.get(0)?,
                    total_sent: row.get(1)?,
                    total_recv: row.get(2)?,
                })
            }
        )?
        .collect::<Result<Vec<_>>>()?;

        Ok(data_points)
    }

    pub fn get_daily_summary_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<DailySummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT exe_name, SUM(total_sent), SUM(total_recv)
             FROM daily_summary
             WHERE date >= ?1 AND date <= ?2
             GROUP BY exe_name
             ORDER BY (SUM(total_sent) + SUM(total_recv)) DESC"
        )?;

        let summaries = stmt.query_map(params![start, end], |row| {
            Ok(DailySummary {
                exe_name: row.get(0)?,
                total_sent: row.get(1)?,
                total_recv: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;

        Ok(summaries)
    }

    pub fn get_monthly_daily_totals(
        &self,
        year: u32,
        month: u32,
    ) -> Result<Vec<DailyDataPoint>> {
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
             ORDER BY date ASC"
        )?;

        let data_points = stmt.query_map(params![start_date, end_date], |row| {
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
        // Parse date to get timestamp range
        let naive_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
        
        let start_ts = naive_date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .expect("invalid local start time")
            .timestamp();
        let end_ts = naive_date
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .expect("invalid local end time")
            .timestamp();

        // Aggregate snapshots into daily_summary
        self.conn.execute(
            "INSERT OR REPLACE INTO daily_summary (date, exe_name, total_sent, total_recv)
             SELECT ?1, exe_name, SUM(bytes_sent), SUM(bytes_recv)
             FROM usage_snapshots
             WHERE timestamp >= ?2 AND timestamp <= ?3
             GROUP BY exe_name",
            params![date, start_ts, end_ts],
        )?;

        Ok(())
    }

    pub fn cleanup_old_snapshots(&self, days_to_keep: u32) -> Result<()> {
        let cutoff_date = Local::now().naive_local().date() - chrono::Duration::days(days_to_keep as i64);
        let cutoff_ts = cutoff_date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .expect("invalid local cutoff time")
            .timestamp();

        self.conn.execute(
            "DELETE FROM usage_snapshots WHERE timestamp < ?1",
            params![cutoff_ts],
        )?;

        Ok(())
    }

    pub fn get_settings(&self) -> Result<Settings> {
        let value: Option<String> = self.conn.query_row(
            "SELECT value FROM settings WHERE key = 'app'",
            [],
            |row| row.get(0),
        ).optional()?;

        match value {
            Some(raw) => serde_json::from_str(&raw).or(Ok(Settings::default())),
            None => Ok(Settings::default()),
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let raw = serde_json::to_string(settings).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(e))
        })?;
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('app', ?1)",
            params![raw],
        )?;
        Ok(())
    }
}
