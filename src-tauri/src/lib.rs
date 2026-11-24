mod db;
#[cfg(windows)]
mod etw;
mod logging;
mod network_monitor;
mod platform;
mod scheduler;
mod spike;
mod time_range;
mod types;
mod validation;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State, WindowEvent,
};
use tauri_plugin_log::{FileOpenStrategy, RotationStrategy, Target, TargetKind};
use tokio::time;

use crate::db::{Database, SUMMARY_RETENTION_DAYS};
use crate::network_monitor::NetworkMonitor;
use crate::platform::SingleInstanceGuard;
use crate::scheduler::{exit_aggregation_dates, Periodic};
use crate::spike::{SpikeKey, SpikeLimiter};
use crate::types::{
    CaptureHealth, DailyDataPoint, DailySummary, MonthlySummary, ProcessSnapshot, ProcessUsage,
    Settings,
};
use crate::validation::{
    validate_date, validate_date_range, validate_days, validate_exe_name, validate_export_path,
    validate_settings, validate_year_month,
};

pub struct AppState {
    monitor: Arc<Mutex<NetworkMonitor>>,
    db: Arc<Mutex<Database>>,
    /// Snapshot terbaru yang dipakai UI. command get_realtime_usage tidak
    /// pernah menyentuh mutex monitor, jadi UI tidak bisa memperlambat capture.
    realtime: Arc<RwLock<Vec<ProcessUsage>>>,
}

#[tauri::command]
async fn get_realtime_usage(state: State<'_, AppState>) -> Result<Vec<ProcessUsage>, String> {
    let cache = state.realtime.read().map_err(|e| e.to_string())?;
    Ok(cache.clone())
}

#[tauri::command]
async fn get_daily_summary(
    date: String,
    state: State<'_, AppState>,
) -> Result<Vec<DailySummary>, String> {
    let date = validate_date(&date)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_daily_summary(&date).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_monthly_summary(
    year: u32,
    month: u32,
    state: State<'_, AppState>,
) -> Result<Vec<MonthlySummary>, String> {
    let (year, month) = validate_year_month(year, month)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_monthly_summary(year, month)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_history_chart(
    exe_name: String,
    days: u32,
    state: State<'_, AppState>,
) -> Result<Vec<DailyDataPoint>, String> {
    validate_exe_name(&exe_name)?;
    let days = validate_days(days)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_history_chart(&exe_name, days)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_daily_summary_range(
    start: String,
    end: String,
    state: State<'_, AppState>,
) -> Result<Vec<DailySummary>, String> {
    let (start, end) = validate_date_range(&start, &end)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_daily_summary_range(&start, &end)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_monthly_daily_totals(
    year: u32,
    month: u32,
    state: State<'_, AppState>,
) -> Result<Vec<DailyDataPoint>, String> {
    let (year, month) = validate_year_month(year, month)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_monthly_daily_totals(year, month)
        .map_err(|e| e.to_string())
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
async fn update_settings(
    settings: Settings,
    state: State<'_, AppState>,
) -> Result<Settings, String> {
    let settings = validate_settings(settings)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.save_settings(&settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
async fn is_admin() -> bool {
    platform::is_admin()
}

#[tauri::command]
async fn get_capture_health(state: State<'_, AppState>) -> Result<CaptureHealth, String> {
    Ok(state
        .monitor
        .lock()
        .map_err(|e| e.to_string())?
        .capture_health())
}

#[tauri::command]
async fn write_export_file(
    path: String,
    contents: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let target = PathBuf::from(&path);

    // Root yang diizinkan berasal dari OS, bukan dari renderer. Folder app
    // data sengaja dibatasi ke `exports` supaya renderer tidak bisa menimpa
    // DB/log aplikasi lewat command ini.
    let mut candidates: Vec<Result<PathBuf, String>> = vec![
        app.path().download_dir().map_err(|e| e.to_string()),
        app.path().document_dir().map_err(|e| e.to_string()),
        app.path().desktop_dir().map_err(|e| e.to_string()),
    ];
    if let Ok(app_dir) = app.path().app_data_dir() {
        let export_dir = app_dir.join("exports");
        candidates.push(
            std::fs::create_dir_all(&export_dir)
                .map(|_| export_dir)
                .map_err(|e| format!("Gagal menyiapkan folder export: {e}")),
        );
    }

    let mut roots = Vec::new();
    for root in candidates.into_iter().flatten() {
        if let Ok(root) = root.canonicalize() {
            roots.push(root);
        }
    }
    if roots.is_empty() {
        return Err("Tidak ada direktori export yang valid.".into());
    }

    validate_export_path(&target, &contents, &roots)?;
    std::fs::write(&target, contents).map_err(|e| e.to_string())
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
    limiter: &mut SpikeLimiter,
) {
    use tauri_plugin_notification::NotificationExt;

    // Ambang dalam MiB/detik. Dulu dibandingkan langsung dengan delta
    // per-capture, jadi mengubah capture_interval_secs diam-diam mengubah
    // arti threshold.
    let interval = settings.capture_interval_secs.max(1);
    // Monoton: jam sistem yang mundur tidak boleh mematikan notifikasi.
    let now = Instant::now();

    for snapshot in snapshots {
        let transferred = snapshot.bytes_sent.saturating_add(snapshot.bytes_recv);
        if !spike::is_spike(transferred, interval, settings.spike_threshold_mib) {
            continue;
        }
        let rate = spike::rate_bps(transferred, interval);

        // Bedakan PID walaupun nama exe sama, dan batasi total notifikasi
        // supaya banyak proses yang spike bareng tidak membanjiri tray.
        let key = SpikeKey {
            pid: snapshot.pid,
            exe_name: snapshot.exe_name.clone(),
        };
        if !limiter.allow(key, now) {
            continue;
        }

        if let Err(error) = app_handle
            .notification()
            .builder()
            .title(format!("Bandwidth spike: {}", snapshot.exe_name))
            .body(format!(
                "{} (PID {})",
                spike::rate_label(rate),
                snapshot.pid
            ))
            .show()
        {
            log::warn!(
                "Gagal kirim notifikasi spike {}: {error}",
                snapshot.exe_name
            );
        }
    }
}

async fn monitoring_loop(
    monitor: Arc<Mutex<NetworkMonitor>>,
    db: Arc<Mutex<Database>>,
    realtime: Arc<RwLock<Vec<ProcessUsage>>>,
    app_handle: tauri::AppHandle,
) {
    let mut interval = time::interval(Duration::from_secs(5));
    let mut spike_limiter = SpikeLimiter::new(Duration::from_secs(600), Duration::from_secs(10));
    let mut last_loss: (u64, u64) = (0, 0);
    let mut aggregate_tick = Periodic::new();
    let mut cleanup_tick = Periodic::new();

    loop {
        interval.tick().await;

        let settings = match db.lock() {
            Ok(database) => database.get_settings().unwrap_or_else(|error| {
                log::warn!("Gagal baca settings ({error}); pakai default");
                Settings::default()
            }),
            Err(error) => {
                log::error!("Mutex DB rusak saat baca settings: {error}");
                Settings::default()
            }
        };

        // Recreate the interval when the capture interval setting changes.
        let wanted = Duration::from_secs(settings.capture_interval_secs.max(1));
        if interval.period() != wanted {
            interval = time::interval(wanted);
            interval.tick().await; // skip the immediate tick after recreation
        }

        // Poll network stats. Kunci monitor dilepas segera setelah snapshot
        // diambil supaya command UI tidak pernah mengantri di belakangnya.
        let (snapshots, health, usage) = {
            let mut mon = match monitor.lock() {
                Ok(m) => m,
                Err(error) => {
                    log::error!("Mutex monitor rusak: {error}");
                    continue;
                }
            };
            let snapshots = mon.capture();
            let health = mon.capture_health();
            let usage = mon.get_realtime_processes();
            (snapshots, health, usage)
        };

        match realtime.write() {
            Ok(mut cache) => *cache = usage,
            Err(error) => log::error!("Mutex cache realtime rusak: {error}"),
        }

        // Jangan biarkan event loss jadi angka turun yang tidak terlihat.
        if health.events_lost > last_loss.0 || health.buffers_lost > last_loss.1 {
            log::warn!(
                "{} (+{} event, +{} buffer)",
                logging::capture_summary(
                    health.etw_active,
                    health.session_running,
                    health.events_lost,
                    health.buffers_lost
                ),
                health.events_lost - last_loss.0,
                health.buffers_lost - last_loss.1
            );
            last_loss = (health.events_lost, health.buffers_lost);
        }
        if health.etw_active && !health.session_running {
            log::error!("Session ETW tidak berjalan; data memakai fallback TCP EStats");
        }

        // Save snapshots to database (batched in a single transaction)
        match db.lock() {
            Ok(database) => {
                if let Err(error) = database.insert_snapshots(&snapshots) {
                    log::error!("Gagal simpan {} snapshot: {error}", snapshots.len());
                }
            }
            Err(error) => log::error!("Mutex DB rusak saat simpan snapshot: {error}"),
        }

        // Bandwidth spike notifications
        if settings.notifications_enabled {
            notify_spikes(&app_handle, &settings, &snapshots, &mut spike_limiter);
        }

        // Agregasi & cleanup berbasis waktu berjalan, bukan `tick_count % 120`
        // (yang interval aslinya mengikuti capture_interval_secs) dan bukan
        // jendela 00:00-00:05 (yang terlewat kalau app mati saat itu).
        let now_instant = Instant::now();
        if aggregate_tick.due(now_instant, Duration::from_secs(600)) {
            for date in exit_aggregation_dates(chrono::Local::now().date_naive()) {
                match db.lock() {
                    Ok(database) => {
                        if let Err(error) = database.aggregate_daily(&date) {
                            log::error!("Gagal agregasi {date}: {error}");
                        }
                    }
                    Err(error) => log::error!("Mutex DB rusak saat agregasi: {error}"),
                }
            }
        }

        if cleanup_tick.due(now_instant, Duration::from_secs(3600)) {
            match db.lock() {
                Ok(database) => {
                    if let Err(error) = database.cleanup_old_snapshots(settings.retention_days) {
                        log::error!("Gagal cleanup snapshot: {error}");
                    }
                    // 10 tahun, bukan retention_days: history bulanan aman.
                    if let Err(error) = database.cleanup_old_summaries(SUMMARY_RETENTION_DAYS) {
                        log::error!("Gagal cleanup ringkasan: {error}");
                    }
                    // Setelah hapus besar-besaran, WAL perlu di-truncate supaya
                    // tidak tumbuh terus di disk.
                    if let Err(error) = database.checkpoint_wal() {
                        log::error!("Gagal checkpoint WAL: {error}");
                    }
                }
                Err(error) => log::error!("Mutex DB rusak saat cleanup: {error}"),
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _single_instance = match SingleInstanceGuard::acquire() {
        Some(guard) => guard,
        None => {
            eprintln!("Bandwidth Monitor sudah berjalan; instance ini keluar.");
            return;
        }
    };

    // Autostart mengirim argumen ini. Manual launch tetap menampilkan window.
    let started_by_autostart = std::env::args().any(|arg| arg == "--from-autostart");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // Jendela dibuat hidden. Autostart tetap hidden; manual launch
            // ditampilkan di bawah ini. Tray tetap jalur untuk menampilkannya.
            #[cfg(desktop)]
            app.handle().plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                Some(vec!["--from-autostart"]),
            ))?;

            // Initialize database
            let app_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_dir)?;

            // Log file harus hidup di release juga, kalau tidak event loss dan
            // error capture tidak bisa didiagnosis dari mesin user.
            let level = logging::level_for(cfg!(debug_assertions));
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(level)
                    .clear_targets()
                    .target(Target::new(TargetKind::Stdout))
                    .target(Target::new(TargetKind::Folder {
                        path: app_dir.join("app-logs"),
                        file_name: Some("app".into()),
                    }))
                    .rotation_strategy(RotationStrategy::KeepSome(5))
                    .max_file_size(1_048_576)
                    .file_open_strategy(FileOpenStrategy::Append)
                    .build(),
            )?;

            let db_path = app_dir.join("bandwidth.db");

            // Jangan panic di setup: kalau DB korup, user nella lihat pesan
            // error yang jelas dari Tauri, bukan crash tanpa jejak.
            let db = Arc::new(Mutex::new(Database::new(db_path)?));

            // Initialize network monitor
            let monitor = Arc::new(Mutex::new(NetworkMonitor::new()));
            let realtime = Arc::new(RwLock::new(Vec::new()));

            // Store in app state
            app.manage(AppState {
                monitor: monitor.clone(),
                db: db.clone(),
                realtime: realtime.clone(),
            });

            let retention_days = db
                .lock()
                .ok()
                .and_then(|database| database.get_settings().ok())
                .map(|settings| settings.retention_days)
                .unwrap_or(7);
            log::info!(
                "{}",
                logging::startup_summary(
                    env!("CARGO_PKG_VERSION"),
                    platform::is_admin(),
                    retention_days
                )
            );

            // System tray icon
            let show_item =
                MenuItem::with_id(app, "show", "Show Bandwidth Monitor", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let icon = app
                .default_window_icon()
                .ok_or("Ikon aplikasi tidak ditemukan")?
                .clone();
            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(icon)
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

            if !started_by_autostart {
                show_main_window(app.handle());
            }

            // Spawn monitoring background task
            let monitor_clone = monitor.clone();
            let db_clone = db.clone();
            let realtime_clone = realtime.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                monitoring_loop(monitor_clone, db_clone, realtime_clone, handle).await;
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
            get_daily_summary_range,
            get_monthly_daily_totals,
            aggregate_today,
            get_settings,
            update_settings,
            is_admin,
            get_capture_health,
            write_export_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        // Aggregate today's data before the app closes so nothing is lost
        // when the user quits before midnight.
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<AppState>() {
                match state.db.lock() {
                    Ok(db) => {
                        for date in exit_aggregation_dates(chrono::Local::now().date_naive()) {
                            if let Err(error) = db.aggregate_daily(&date) {
                                log::error!("Gagal agregasi saat exit ({date}): {error}");
                            }
                        }
                    }
                    Err(error) => log::error!("Mutex DB rusak saat exit: {error}"),
                }
            }
        }
    });
}
