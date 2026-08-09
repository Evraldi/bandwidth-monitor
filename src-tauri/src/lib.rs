mod types;
mod db;
mod network_monitor;
mod platform;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State, WindowEvent,
};
use tokio::time;
use chrono::Timelike;

use crate::types::{
    ProcessUsage, DailySummary, MonthlySummary, DailyDataPoint, ProcessSnapshot, Settings,
};
use crate::db::Database;
use crate::network_monitor::NetworkMonitor;

pub struct AppState {
    monitor: Arc<Mutex<NetworkMonitor>>,
    db: Arc<Mutex<Database>>,
}

#[tauri::command]
async fn get_realtime_usage(state: State<'_, AppState>) -> Result<Vec<ProcessUsage>, String> {
    // Read-only: never triggers poll(), so it can't corrupt the capture loop's
    // delta accounting. Bandwidth is derived by the frontend from consecutive reads.
    let mut monitor = state.monitor.lock().map_err(|e| e.to_string())?;
    Ok(monitor.get_realtime_processes())
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

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_settings().map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_settings(settings: Settings, state: State<'_, AppState>) -> Result<Settings, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.save_settings(&settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
async fn is_admin() -> bool {
    platform::is_admin()
}

fn show_main_window<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn notify_spikes(
    app_handle: &tauri::AppHandle,
    settings: &Settings,
    snapshots: &[ProcessSnapshot],
    last_spike: &mut HashMap<String, i64>,
) {
    use tauri_plugin_notification::NotificationExt;

    let threshold = settings.spike_threshold_mb.saturating_mul(1024 * 1024);
    let now = chrono::Utc::now().timestamp();

    for snapshot in snapshots {
        let transferred = snapshot.bytes_sent + snapshot.bytes_recv;
        if transferred < threshold {
            continue;
        }

        // Rate-limit: at most one notification per process every 10 minutes.
        if let Some(&last) = last_spike.get(&snapshot.exe_name) {
            if now - last < 600 {
                continue;
            }
        }

        last_spike.insert(snapshot.exe_name.clone(), now);

        let mb = transferred as f64 / (1024.0 * 1024.0);
        let _ = app_handle
            .notification()
            .builder()
            .title(format!("Bandwidth spike: {}", snapshot.exe_name))
            .body(format!(
                "Transferred {:.1} MB in {}s (PID {})",
                mb, settings.capture_interval_secs, snapshot.pid
            ))
            .show();
    }
}

async fn monitoring_loop(
    monitor: Arc<Mutex<NetworkMonitor>>,
    db: Arc<Mutex<Database>>,
    app_handle: tauri::AppHandle,
) {
    let mut interval = time::interval(Duration::from_secs(5));
    let mut tick_count: u64 = 0;
    let mut last_spike: HashMap<String, i64> = HashMap::new();

    loop {
        interval.tick().await;

        let settings = match db.lock() {
            Ok(database) => database.get_settings().unwrap_or_default(),
            Err(_) => Settings::default(),
        };

        // Recreate the interval when the capture interval setting changes.
        let wanted = Duration::from_secs(settings.capture_interval_secs.max(1));
        if interval.period() != wanted {
            interval = time::interval(wanted);
            interval.tick().await; // skip the immediate tick after recreation
        }

        // Poll network stats
        let snapshots = {
            let mut mon = match monitor.lock() {
                Ok(m) => m,
                Err(_) => continue,
            };
            mon.capture()
        };

        // Save snapshots to database
        if let Ok(database) = db.lock() {
            for snapshot in &snapshots {
                let _ = database.insert_snapshot(snapshot);
            }
        }

        // Bandwidth spike notifications
        if settings.notifications_enabled {
            notify_spikes(&app_handle, &settings, &snapshots, &mut last_spike);
        }

        // Re-aggregate today's summary periodically (~10 min) so History stays
        // fresh even if the app is killed before the midnight/exit aggregation.
        tick_count += 1;
        if tick_count % 120 == 0 {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            if let Ok(database) = db.lock() {
                let _ = database.aggregate_daily(&today);
            }
        }
    }
}

async fn daily_aggregation_task(db: Arc<Mutex<Database>>) {
    let mut interval = time::interval(Duration::from_secs(3600)); // Check every hour
    interval.tick().await;

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

                // Cleanup old snapshots using the configured retention
                let retention = database
                    .get_settings()
                    .map(|s| s.retention_days)
                    .unwrap_or(7);
                let _ = database.cleanup_old_snapshots(retention);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
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

            // System tray icon
            let show_item =
                MenuItem::with_id(app, "show", "Show Bandwidth Monitor", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().expect("missing app icon").clone())
                .tooltip("Bandwidth Monitor")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app_handle, event| match event.id().as_ref() {
                    "show" => show_main_window(app_handle),
                    "quit" => app_handle.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // Spawn monitoring background task
            let monitor_clone = monitor.clone();
            let db_clone = db.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                monitoring_loop(monitor_clone, db_clone, handle).await;
            });

            // Spawn daily aggregation task
            let db_clone = db.clone();
            tauri::async_runtime::spawn(async move {
                daily_aggregation_task(db_clone).await;
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Minimize to tray instead of closing when the X button is pressed
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_realtime_usage,
            get_daily_summary,
            get_monthly_summary,
            get_history_chart,
            aggregate_today,
            get_settings,
            update_settings,
            is_admin,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        // Aggregate today's data before the app closes so nothing is lost
        // when the user quits before midnight.
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<AppState>() {
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                if let Ok(db) = state.db.lock() {
                    let _ = db.aggregate_daily(&today);
                }
            }
        }
    });
}
