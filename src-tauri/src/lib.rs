// airis 백엔드 진입점.
// 앱 시작 시 logging·db·settings를 초기화하고 AppState로 공유한다.

mod commands;
mod db;
mod error;
mod logging;
mod secrets;
mod settings;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;
use tracing_appender::non_blocking::WorkerGuard;

use db::Db;
use settings::Settings;

/// 모든 Tauri command가 접근하는 공유 상태.
///
/// `_log_guard`는 절대 명시적으로 drop하지 않는다 —
/// drop 되는 순간 비동기 로그 워커가 닫히며 큐가 즉시 flush 되기 때문.
pub struct AppState {
    pub db: Mutex<Db>,
    pub settings: Mutex<Settings>,
    pub settings_path: PathBuf,
    _log_guard: WorkerGuard,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // app_data_dir은 OS별 경로:
            //   - macOS: ~/Library/Application Support/dev.airis.app
            //   - Linux: ~/.local/share/dev.airis.app
            //   - Windows: %APPDATA%/dev.airis.app
            let data_dir: PathBuf = app
                .path()
                .app_data_dir()
                .expect("app_data_dir is available on supported platforms");
            std::fs::create_dir_all(&data_dir)?;

            let log_guard = logging::init(&data_dir)?;
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "airis startup");

            let db = Db::open(&data_dir.join("app.db"))?;
            let settings_path = data_dir.join("settings.json");
            let settings_data = settings::read(&settings_path)?;

            app.manage(AppState {
                db: Mutex::new(db),
                settings: Mutex::new(settings_data),
                settings_path,
                _log_guard: log_guard,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::settings::api_key_check,
            commands::settings::api_key_set,
            commands::settings::api_key_delete,
            commands::settings::api_key_present,
            commands::settings::settings_read,
            commands::settings::settings_write,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// PR 1 스캐폴드 흔적 — PR 5에서 실제 commands 모듈로 대체 예정.
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}! You've been greeted from Rust!")
}
