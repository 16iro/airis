// airis 백엔드 진입점.
// 앱 시작 시 logging·db·settings·llm을 초기화하고 AppState로 공유한다.

mod commands;
mod db;
mod error;
mod llm;
mod logging;
mod secrets;
mod settings;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::Manager;
use tracing_appender::non_blocking::WorkerGuard;

use db::Db;
use llm::anthropic::AnthropicProvider;
use llm::LlmProvider;
use settings::Settings;

/// 모든 Tauri command가 접근하는 공유 상태.
///
/// `_log_guard`는 절대 명시적으로 drop하지 않는다 —
/// drop 되는 순간 비동기 로그 워커가 닫히며 큐가 즉시 flush 되기 때문.
pub struct AppState {
    pub db: Mutex<Db>,
    pub settings: Mutex<Settings>,
    pub settings_path: PathBuf,
    /// 현재 워크스페이스에 열린 파일의 본문. v0.1 단일 파일 모드.
    pub current_file: Mutex<Option<String>>,
    /// LLM 프로바이더 — v0.1엔 Anthropic 단일 인스턴스.
    pub llm: Arc<dyn LlmProvider>,
    _log_guard: WorkerGuard,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
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
            let llm: Arc<dyn LlmProvider> = Arc::new(AnthropicProvider::new()?);

            app.manage(AppState {
                db: Mutex::new(db),
                settings: Mutex::new(settings_data),
                settings_path,
                current_file: Mutex::new(None),
                llm,
                _log_guard: log_guard,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::api_key_check,
            commands::settings::api_key_set,
            commands::settings::api_key_delete,
            commands::settings::api_key_present,
            commands::settings::settings_read,
            commands::settings::settings_write,
            commands::file::file_open,
            commands::file::file_close,
            commands::file::file_current_content,
            commands::llm::chat_send,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
