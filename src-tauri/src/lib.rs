mod types;
mod db;
mod network_monitor;

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Manager, State};
use tokio::time;
use chrono::Timelike;

use crate::types::{ProcessUsage, DailySummary, MonthlySummary, DailyDataPoint};
use crate::db::Database;
use crate::network_monitor::NetworkMonitor;

pub struct AppState {
    monitor: Arc<Mutex<NetworkMonitor>>,
    db: Arc<Mutex<Database>>,
}

#[tauri::command]
async fn get_realtime_usage(state: State<'_, AppState>) -> Result<Vec<ProcessUsage>, String> {
    // Clone session totals before releasing lock
    let session_totals = {
        let monitor = state.monitor.lock().map_err(|e| e.to_string())?;
        monitor.get_session_totals().clone()
    };
    
    let mut usage_list: Vec<ProcessUsage> = Vec::new();
    
    // Get current snapshots
    let snapshots = {
        let mut monitor = state.monitor.lock().map_err(|e| e.to_string())?;
        monitor.get_process_usage()
    };
    
    for snapshot in snapshots {
        let (total_sent, total_recv) = session_totals
            .get(&snapshot.pid)
            .copied()
            .unwrap_or((snapshot.bytes_sent, snapshot.bytes_recv));
        
        usage_list.push(ProcessUsage {
            exe_name: snapshot.exe_name,
            pid: snapshot.pid,
            upload_bps: 0, // Will be calculated by frontend based on polling interval
            download_bps: 0,
            total_sent,
            total_recv,
        });
    }
    
    Ok(usage_list)
}

#[tauri::command]
async fn get_daily_summary(
    date: String,
    state: State<'_, AppState>,
) -> Result<Vec<DailySummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_daily_summary(&date).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_monthly_summary(
    year: u32,
    month: u32,
    state: State<'_, AppState>,
) -> Result<Vec<MonthlySummary>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_monthly_summary(year, month).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_history_chart(
    exe_name: String,
    days: u32,
    state: State<'_, AppState>,
) -> Result<Vec<DailyDataPoint>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_history_chart(&exe_name, days).map_err(|e| e.to_string())
}

#[tauri::command]
async fn aggregate_today(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    db.aggregate_daily(&today).map_err(|e| e.to_string())
}

async fn monitoring_loop(monitor: Arc<Mutex<NetworkMonitor>>, db: Arc<Mutex<Database>>) {
    let mut interval = time::interval(Duration::from_secs(5));
    
    loop {
        interval.tick().await;
        
        // Poll network stats
        let snapshots = {
            let mut mon = match monitor.lock() {
                Ok(m) => m,
                Err(_) => continue,
            };
            mon.get_process_usage()
        };
        
        // Save snapshots to database
        if let Ok(database) = db.lock() {
            for snapshot in snapshots {
                let _ = database.insert_snapshot(&snapshot);
            }
        }
    }
}

async fn daily_aggregation_task(db: Arc<Mutex<Database>>) {
    let mut interval = time::interval(Duration::from_secs(3600)); // Check every hour
    
    loop {
        interval.tick().await;
        
        let now = chrono::Local::now();
        
        // Aggregate at midnight
        if now.hour() == 0 && now.minute() < 5 {
            if let Ok(database) = db.lock() {
                let yesterday = (now - chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string();
                let _ = database.aggregate_daily(&yesterday);
                
                // Cleanup old snapshots (keep 7 days)
                let _ = database.cleanup_old_snapshots(7);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            
            // Initialize database
            let app_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_dir)?;
            let db_path = app_dir.join("bandwidth.db");
            
            let db = Arc::new(Mutex::new(
                Database::new(db_path).expect("Failed to initialize database")
            ));
            
            // Initialize network monitor
            let monitor = Arc::new(Mutex::new(NetworkMonitor::new()));
            
            // Store in app state
            app.manage(AppState {
                monitor: monitor.clone(),
                db: db.clone(),
            });
            
            // Spawn monitoring background task
            let monitor_clone = monitor.clone();
            let db_clone = db.clone();
            tauri::async_runtime::spawn(async move {
                monitoring_loop(monitor_clone, db_clone).await;
            });
            
            // Spawn daily aggregation task
            let db_clone = db.clone();
            tauri::async_runtime::spawn(async move {
                daily_aggregation_task(db_clone).await;
            });
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_realtime_usage,
            get_daily_summary,
            get_monthly_summary,
            get_history_chart,
            aggregate_today,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
