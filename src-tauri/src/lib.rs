// airis 백엔드 진입점.
// 앱 시작 시 logging·db·settings·llm을 초기화하고 AppState로 공유한다.

mod commands;
mod db;
mod error;
mod index;
mod jobs;
mod llm;
mod logging;
mod parsers;
mod secrets;
mod settings;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::Manager;
use tracing_appender::non_blocking::WorkerGuard;

use commands::book::ActiveSection;
use commands::pomodoro::PomodoroSlot;
use commands::study::{ensure_active_or_bootstrap_default, StudyMeta};
use db::Db;
use error::AppResult;
use llm::anthropic::AnthropicProvider;
use llm::gemini::GeminiProvider;
use llm::openai::OpenAiProvider;
use llm::LlmProvider;
use settings::{Provider, Settings};

/// 모든 Tauri command가 접근하는 공유 상태.
///
/// `_log_guard`는 절대 명시적으로 drop하지 않는다 —
/// drop 되는 순간 비동기 로그 워커가 닫히며 큐가 즉시 flush 되기 때문.
pub struct AppState {
    pub db: Mutex<Db>,
    pub settings: Mutex<Settings>,
    pub settings_path: PathBuf,
    /// 사용자 데이터 루트 — `{app_data_dir}`. 스터디 디렉토리·Overview.md 등이 이 아래에 위치.
    pub data_dir: PathBuf,
    /// 현재 워크스페이스에 열린 파일의 본문. v0.1 단일 파일 모드.
    pub current_file: Mutex<Option<String>>,
    /// 활성 LLM 프로바이더 — Settings.active_provider 따라 빌드. 변경 시 새 instance로 교체.
    /// 진행 중 chat_send는 자기 Arc clone을 spawn task에 옮겼으므로 교체에 영향 X (handoff 결정 #4).
    pub llm: Mutex<Arc<dyn LlmProvider>>,
    /// 활성 스터디 메모리 캐시. source of truth는 `studies.is_active`.
    pub active_study: Mutex<Option<StudyMeta>>,
    /// 활성 섹션 — 사용자가 BookViewer에서 마지막 클릭한 헤딩.
    /// chat_send가 *컨텍스트 우선순위 1*로 사용 (paragraphs WHERE book_id+section_path).
    pub active_section: Mutex<Option<ActiveSection>>,
    /// PDFium binary가 위치한 디렉토리. `scripts/setup-pdfium.sh`가 채운 `resources/pdfium/lib`.
    /// None이면 PDF 인덱싱 비활성 (graceful — MD/HTML은 그대로 작동).
    pub pdfium_lib_dir: Option<PathBuf>,
    /// 진행 중 Pomodoro 세션. 매 호출마다 wall-clock으로 잔여 계산 (PR 20).
    pub pomodoro: PomodoroSlot,
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

            let mut db = Db::open(&data_dir.join("app.db"))?;
            let settings_path = data_dir.join("settings.json");
            let settings_data = settings::read(&settings_path)?;
            let llm = build_provider(settings_data.active_provider)?;

            // v1→v2 마이그 직후 또는 신규 사용자 — 활성 스터디가 없으면
            // 'default'를 자동 생성·활성화해 챗 흐름이 끊기지 않게 한다.
            let active_study = ensure_active_or_bootstrap_default(db.conn_mut())?;
            tracing::info!(target: "study", slug = %active_study.slug, "bootstrap active study");

            // PDFium binary 위치 — Tauri resource_dir/pdfium/lib (`scripts/setup-pdfium.sh` 출력).
            // 디렉토리 부재면 None — PDF 인덱싱은 *명시 에러*로 안내하고 MD/HTML은 그대로 작동.
            let pdfium_lib_dir = app
                .path()
                .resource_dir()
                .ok()
                .map(|r| r.join("resources").join("pdfium").join("lib"))
                .filter(|p| p.is_dir());
            tracing::info!(
                target: "pdf",
                lib_dir = ?pdfium_lib_dir,
                "pdfium lib_dir resolved"
            );

            app.manage(AppState {
                db: Mutex::new(db),
                settings: Mutex::new(settings_data),
                settings_path,
                data_dir: data_dir.clone(),
                current_file: Mutex::new(None),
                llm: Mutex::new(llm),
                active_study: Mutex::new(Some(active_study)),
                active_section: Mutex::new(None),
                pdfium_lib_dir,
                pomodoro: Mutex::new(None),
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
            commands::llm::chat_history,
            commands::llm::retry_failed_job,
            commands::llm::list_failed_jobs,
            commands::llm::delete_failed_job,
            commands::study::list_studies,
            commands::study::create_study,
            commands::study::select_study,
            commands::study::delete_study,
            commands::study::get_active_study,
            commands::study::study_overview_read,
            commands::study::study_overview_write_meta,
            commands::memory::memory_read,
            commands::memory::memory_write,
            commands::memory::memory_detect_triggers,
            commands::memory::memory_apply_trigger,
            commands::pomodoro::start_pomodoro,
            commands::pomodoro::stop_pomodoro,
            commands::pomodoro::get_pomodoro_state,
            commands::book::add_main_book,
            commands::book::add_sub_book,
            commands::book::list_books,
            commands::book::remove_book,
            commands::book::start_indexing,
            commands::book::book_read_raw,
            commands::book::check_stale,
            commands::book::reindex_book,
            commands::book::set_active_section,
            commands::book::clear_active_section,
            commands::book::get_active_section,
            commands::search::search_sections,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Settings.active_provider 따라 새 LlmProvider 인스턴스 빌드.
/// 키 부재는 init 단계엔 검사하지 않음 — chat_send 첫 호출 시 secrets::get가 AuthRequired 반환.
pub fn build_provider(provider: Provider) -> AppResult<Arc<dyn LlmProvider>> {
    match provider {
        Provider::Anthropic => Ok(Arc::new(AnthropicProvider::new()?)),
        Provider::Openai => Ok(Arc::new(OpenAiProvider::new()?)),
        Provider::Gemini => Ok(Arc::new(GeminiProvider::new()?)),
    }
}
