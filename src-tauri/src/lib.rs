// airis 백엔드 진입점.
// 앱 시작 시 logging·db·settings·llm을 초기화하고 AppState로 공유한다.

// 통합 테스트(`tests/v042_cache_smoke.rs`)에서 cache 모듈을 외부 크레이트 경로로 호출 — pub 필수.
pub mod cache;
mod cli_install;
mod commands;
mod db;
pub mod error;
// 통합 테스트(`tests/v041_chunker_smoke.rs`)에서 v0.4.1 chunker/indexer를 외부 크레이트
// 경로(`airis_lib::index::v041::...`)로 호출. 마찬가지로 markdown::parse도 통합 테스트가
// 직접 사용. 다른 모듈은 외부 노출 필요 없어 그대로 비공개.
pub mod index;
mod jobs;
// 통합 테스트(`tests/v043_rewriter_smoke.rs`)에서 LlmProvider trait + MockProvider를 외부
// 크레이트 경로(`airis_lib::llm::...`)로 호출 — pub 필수.
pub mod llm;
mod logging;
pub mod parsers;
// 통합 테스트(`tests/v042_throttle_smoke.rs`)에서 power_monitor::priority 모듈을
// 외부 크레이트 경로(`airis_lib::power_monitor::priority::...`)로 호출.
pub mod power_monitor;
// 통합 테스트(`tests/v042_throttle_smoke.rs`)에서 runtime::throttle 모듈을 외부 크레이트
// 경로(`airis_lib::runtime::throttle::...`)로 호출.
pub mod runtime;
mod secrets;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};
use tracing_appender::non_blocking::WorkerGuard;

use cli_install::CliPkg;
use commands::book::ActiveSection;
use commands::pomodoro::PomodoroSlot;
use commands::study::{ensure_active_or_bootstrap_default, StudyMeta};
use cache::embedding::EmbeddingCache;
use cache::response::ResponseCache;
use db::Db;
use error::{AppError, AppResult};
use index::v041::embedder::Embedder;
use index::v042::embedder_t2::EmbedderT2;
use index::v042::worker::IndexingWorker;
use index::v043::reranker::Reranker;
use llm::anthropic::AnthropicProvider;
use llm::claude_cli::ClaudeCliProvider;
use llm::codex_cli::CodexCliProvider;
use llm::gemini::GeminiProvider;
use llm::gemini_cli::GeminiCliProvider;
use llm::openai::OpenAiProvider;
use llm::LlmProvider;
use settings::{AuthMode, Provider, Settings};

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
    /// v0.4.1 PR 4 — fastembed Embedder lazy slot. 첫 reindex/chat에서 init 되고 이후 재사용.
    /// `Mutex`는 lazy init 직렬화용. 본문 메서드(`embed_*`)는 자체 mutex로 추가 직렬화.
    /// Tauri appdata 경로(`<app_data>/models/`)에 모델을 캐시 (D-077).
    pub embedder: Arc<Mutex<Option<Arc<Embedder>>>>,
    /// v0.4.2 PR 3 — fastembed BGE-M3 (T2) lazy slot. 첫 `start_t2_build` 진입 시
    /// `EmbedderT2::new(<app_data>/models/)` 실행 — ~2GB 다운로드 + 로드. 이후 재사용.
    /// T1과 분리된 슬롯 — 두 모델 동시 보유 가능 (RAM 약 ~2.5GB).
    pub embedder_t2: Arc<Mutex<Option<Arc<EmbedderT2>>>>,
    /// v0.4.3 PR 4 (D-090) — fastembed BGE-reranker-v2-m3 (T3) lazy slot. 첫 chat 응답
    /// 인용 검증 시 `Reranker::new(<app_data>/models/)` 실행 — ~600MB 다운로드 + 로드.
    /// 이후 재사용. T1·T2와 분리된 슬롯 — 세 모델 동시 보유 가능 (RAM 약 ~3.0GB).
    /// 다운로드 실패 시 None 유지 + citation_check가 substring 폴백.
    pub reranker: Arc<Mutex<Option<Arc<Reranker>>>>,
    /// v0.4.1 PR 4 — 책 인덱싱 직렬 큐 (D-076). `try_lock`이 실패하면 사용자에게 "다른
    /// 책이 인덱싱 중입니다" 안내. 같은 책 두 번 누름도 자연 차단.
    pub indexer_lock: Arc<Mutex<()>>,
    /// v0.4.2 PR 3 — 진행 중 인덱싱 잡 핸들 레지스트리 (`indexing_jobs.id` ↔
    /// `IndexingWorker`). 사용자 일시정지/재개/취소 명령이 본 맵으로 worker를 lookup.
    /// 잡 종료 시 (완료/취소/실패) commands::book가 row 제거.
    pub indexing_workers: Arc<Mutex<HashMap<i64, Arc<IndexingWorker>>>>,
    /// v0.4.2 PR 3 — OS 전원·시스템 이벤트 모니터(D-081 트리거 통합 진입점).
    /// Linux는 UPower D-Bus, macOS·Windows는 stub(NoopMonitor 동등). startup에서
    /// 한 번 만들고 commands::book가 인덱싱 잡 시작 시 콜백을 등록.
    pub power_monitor: Arc<dyn power_monitor::PowerMonitor>,
    /// v0.4.2 PR 4 (D-084) — 임베딩 텍스트 sha256 → 벡터 cache. SQLite 영속(`embedding_cache`
    /// 테이블) + 인메모리 핫셋 1024 LRU. Connection은 호출 측이 매 메서드 진입에 인자로
    /// 전달 — `state.db.lock()` 안에서 자연스럽게 호출. 인덱서·검색이 공유.
    pub embedding_cache: Arc<EmbeddingCache>,
    /// v0.4.2 PR 4 (D-084) — chat 응답 cache. key = sha256(book_id + rewritten_query +
    /// sorted(retrieved_chunk_ids) + active_model). 7일 TTL + 책 단위 명시 invalidation.
    pub response_cache: Arc<ResponseCache>,
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
            let llm = build_provider(
                settings_data.active_provider,
                settings_data.auth_mode,
                &data_dir,
            )?;

            // v1→v2 마이그 직후 또는 신규 사용자 — 활성 스터디가 없으면
            // 'default'를 자동 생성·활성화해 챗 흐름이 끊기지 않게 한다.
            let active_study = ensure_active_or_bootstrap_default(db.conn_mut())?;
            tracing::info!(target: "study", slug = %active_study.slug, "bootstrap active study");

            // PR 65: 기존 사용자가 디스크에 보유한 `.thumbnails/` 디렉토리를 `thumbnails/`로 이동.
            // v10 SQL이 DB path 문자열을 갱신했으니 디스크 실체도 맞춰준다. 실패해도 startup 자체는 살림.
            if let Err(e) = rename_legacy_thumbnail_dirs(&data_dir) {
                tracing::warn!(target: "thumbnail", error = %e, "legacy .thumbnails rename failed (non-fatal)");
            }

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

            // v0.4.2 PR 3 — OS 전원·시스템 이벤트 모니터. Linux UPower 시도 → 실패 시
            // NoopMonitor 폴백. startup은 절대 막지 않는다.
            let power_monitor: Arc<dyn power_monitor::PowerMonitor> =
                Arc::from(power_monitor::default_monitor());
            tracing::info!(
                target: "power_monitor",
                impl = power_monitor.label(),
                "전원 모니터 초기화"
            );

            // v0.4.2 PR 3 — 비정상 종료 잡 감지 (status='running'으로 남은 잡).
            // 호출 측이 frontend에 `index:abnormal_termination` 이벤트를 emit해 UI에서
            // 재개/취소 다이얼로그 노출. 본 setup 단계는 *조회*만, 사용자 응답은 별도 명령.
            let abnormal_jobs = match index::v042::resume::resume_pending_jobs(db.conn()) {
                Ok(plans) => plans
                    .into_iter()
                    .filter(|p| {
                        matches!(
                            p.status_was,
                            index::v042::resume::ResumeStatusWas::AbnormalRunning
                        )
                    })
                    .collect::<Vec<_>>(),
                Err(e) => {
                    tracing::warn!(
                        target: "index",
                        error = %e,
                        "resume_pending_jobs 호출 실패 — 비정상 종료 감지 skip"
                    );
                    Vec::new()
                }
            };
            if !abnormal_jobs.is_empty() {
                let payloads: Vec<serde_json::Value> = abnormal_jobs
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "job_id": p.job_id,
                            "book_id": p.book_id,
                            "tier": match p.tier {
                                index::v042::worker::Tier::T1Me5Small => 1,
                                index::v042::worker::Tier::T2BgeM3 => 2,
                            },
                            "pending_chunks": p.pending_chunk_ids.len(),
                        })
                    })
                    .collect();
                if let Err(e) = app.emit(
                    "index:abnormal_termination",
                    serde_json::json!({ "jobs": payloads }),
                ) {
                    tracing::warn!(
                        target: "index",
                        error = %e,
                        "index:abnormal_termination emit 실패"
                    );
                } else {
                    tracing::info!(
                        target: "index",
                        count = abnormal_jobs.len(),
                        "비정상 종료 잡 감지 — frontend 알림 emit"
                    );
                }
            }

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
                embedder: Arc::new(Mutex::new(None)),
                embedder_t2: Arc::new(Mutex::new(None)),
                reranker: Arc::new(Mutex::new(None)),
                indexer_lock: Arc::new(Mutex::new(())),
                indexing_workers: Arc::new(Mutex::new(HashMap::new())),
                power_monitor: power_monitor.clone(),
                embedding_cache: Arc::new(EmbeddingCache::new()),
                response_cache: Arc::new(ResponseCache::new()),
                _log_guard: log_guard,
            });

            // v0.4.2 PR 3 — SIGTERM/Ctrl-C → graceful shutdown.
            // 진행 중 IndexingWorker 모두 pause(AppQuit) → DB 커밋 대기 (max 10s) → exit.
            // tokio::signal은 Tauri의 tokio runtime 위에서 spawn.
            let app_handle_for_signal = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                wait_termination_signal().await;
                tracing::info!(target: "shutdown", "termination signal 수신 — graceful shutdown 시작");
                graceful_shutdown(&app_handle_for_signal).await;
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
            commands::srs::srs_add_card,
            commands::srs::srs_list_due,
            commands::srs::srs_review_card,
            commands::srs::srs_delete_card,
            commands::recall::recall_evaluate,
            commands::llm::list_due_jobs,
            commands::updates::check_for_update,
            commands::book::add_main_book,
            commands::book::add_sub_book,
            commands::book::list_books,
            commands::book::remove_book,
            commands::study::set_study_thumbnail,
            commands::study::clear_study_thumbnail,
            commands::study::update_study_info,
            commands::study::open_study_folder,
            commands::book::start_indexing,
            commands::book::book_read_raw,
            commands::book::check_stale,
            commands::book::reindex_book,
            commands::book::set_active_section,
            commands::book::clear_active_section,
            commands::book::get_active_section,
            commands::book::start_t2_build,
            commands::book::pause_indexing_job,
            commands::book::resume_indexing_job,
            commands::book::cancel_indexing_job,
            commands::search::search_sections,
            commands::cli_setup::cli_runtime_detect,
            commands::cli_setup::cli_status,
            commands::cli_setup::cli_install_provider,
            commands::cli_setup::cli_auth_status_claude,
            commands::cli_setup::cli_auth_status_gemini,
            commands::cli_setup::cli_auth_status_codex,
            commands::cli_setup::cli_login,
            commands::ab_compare::chat_send_ab_compare,
            commands::ab_compare::dev_ab_record_choice,
            commands::ab_compare::dev_ab_export_results,
            commands::ab_compare::dev_cache_stats,
            commands::dev_acceptance::dev_simulate_abnormal_shutdown,
            commands::dev_acceptance::dev_inspect_active_index_state,
            commands::dev_acceptance::dev_measure_chat_response_ms,
            commands::dev_acceptance::dev_response_cache_hit_ratio,
            commands::dev_acceptance::dev_measure_citation_accuracy,
            commands::dev_acceptance::dev_measure_followup_skip_rate,
            commands::dev_acceptance::dev_measure_prefix_cache_ratio,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Settings (active_provider + auth_mode) 따라 새 LlmProvider 인스턴스 빌드.
/// 키 부재는 init 단계엔 검사하지 않음 — chat_send 첫 호출 시 secrets::get가 AuthRequired 반환.
///
/// PR 65: 기존 사용자 디스크의 `.thumbnails/` 디렉토리를 `thumbnails/`로 이동.
/// `<data_dir>/studies/<slug>/.thumbnails` → `<data_dir>/studies/<slug>/thumbnails`.
/// 충돌(이미 존재) 시엔 그냥 둔다 — 새 디렉토리에 새 파일이 들어가면 그걸 사용.
fn rename_legacy_thumbnail_dirs(data_dir: &std::path::Path) -> std::io::Result<()> {
    let studies_root = data_dir.join("studies");
    if !studies_root.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&studies_root)? {
        let entry = entry?;
        let study_dir = entry.path();
        if !study_dir.is_dir() {
            continue;
        }
        let legacy = study_dir.join(".thumbnails");
        let new = study_dir.join("thumbnails");
        if legacy.is_dir() && !new.exists() {
            std::fs::rename(&legacy, &new)?;
            tracing::info!(target: "thumbnail", from = %legacy.display(), to = %new.display(), "renamed legacy .thumbnails");
        }
    }
    Ok(())
}

/// PR 24 (D-066): auth_mode == Cli면 subprocess 어댑터를 우선.
/// PR 28 hotfix: CLI 미설치 등으로 어댑터 build 실패 시 *ApiKey 어댑터로 fallback* — 앱 startup 보장.
/// 사용자가 CLI 설치를 마치면 try_rebuild_llm가 다시 시도해 ClaudeCliProvider로 교체.
pub fn build_provider(
    provider: Provider,
    auth_mode: AuthMode,
    data_dir: &std::path::Path,
) -> AppResult<Arc<dyn LlmProvider>> {
    if auth_mode == AuthMode::Cli {
        match build_cli_provider(provider, data_dir) {
            Ok(Some(p)) => return Ok(p),
            Ok(None) => {
                // PR 24 시점 — Anthropic만 구현된 상태에서 Gemini/Openai 선택 시.
                tracing::info!(
                    target: "llm",
                    provider = provider.as_str(),
                    "CLI 어댑터 미구현 — ApiKey 어댑터로 fallback"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "llm",
                    provider = provider.as_str(),
                    error = %e,
                    "CLI 어댑터 build 실패 — ApiKey 어댑터로 fallback (CLI 설치 후 try_rebuild_llm로 회복)"
                );
            }
        }
    }
    match provider {
        Provider::Anthropic => Ok(Arc::new(AnthropicProvider::new()?)),
        Provider::Openai => Ok(Arc::new(OpenAiProvider::new()?)),
        Provider::Gemini => Ok(Arc::new(GeminiProvider::new()?)),
    }
}

/// CLI 어댑터 생성. 바이너리 미설치면 CliMissing 에러를 *반환*해 chat_send가 사용자에게 안내하게 한다.
/// PR 24 = Anthropic, PR 25 = Gemini, PR 26 = OpenAI(Codex). 미구현 프로바이더는 None → ApiKey fallback.
fn build_cli_provider(
    provider: Provider,
    data_dir: &std::path::Path,
) -> AppResult<Option<Arc<dyn LlmProvider>>> {
    match provider {
        Provider::Anthropic => {
            let bin = locate_required(provider)?;
            Ok(Some(Arc::new(ClaudeCliProvider::new(
                bin,
                data_dir.to_path_buf(),
            ))))
        }
        Provider::Gemini => {
            let bin = locate_required(provider)?;
            Ok(Some(Arc::new(GeminiCliProvider::new(
                bin,
                data_dir.to_path_buf(),
            ))))
        }
        Provider::Openai => {
            let bin = locate_required(provider)?;
            Ok(Some(Arc::new(CodexCliProvider::new(
                bin,
                data_dir.to_path_buf(),
            ))))
        }
    }
}

fn locate_required(provider: Provider) -> AppResult<std::path::PathBuf> {
    let pkg = CliPkg::for_provider(provider);
    cli_install::locate_binary(pkg.binary)?.ok_or_else(|| AppError::CliMissing {
        provider: provider.as_str().into(),
    })
}

/// SIGTERM/SIGINT(또는 Windows의 Ctrl-C) 도착까지 await. 이후 호출 측이 graceful
/// shutdown 절차를 실행한다 (현재 진행 중 IndexingWorker pause(AppQuit) → DB 커밋
/// 대기 → exit).
///
/// Unix는 SIGTERM·SIGINT 둘 다 처리. Windows는 Ctrl-C만 (signal-hook 미사용).
async fn wait_termination_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(target: "shutdown", error = %e, "SIGTERM 핸들러 등록 실패");
                return;
            }
        };
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(target: "shutdown", error = %e, "SIGINT 핸들러 등록 실패");
                return;
            }
        };
        tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!(target: "shutdown", "SIGTERM 수신");
            }
            _ = sigint.recv() => {
                tracing::info!(target: "shutdown", "SIGINT 수신");
            }
        }
    }
    #[cfg(not(unix))]
    {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(target: "shutdown", error = %e, "ctrl_c await 실패");
        }
    }
}

/// graceful shutdown — 진행 중 IndexingWorker 모두 pause(AppQuit) + cancel,
/// DB 커밋 대기(최대 10초). 본 함수는 Tauri runtime을 직접 종료하진 않는다 —
/// async task가 끝난 후 OS가 프로세스 자연 종료.
async fn graceful_shutdown(app_handle: &tauri::AppHandle) {
    use std::time::Duration;
    use tokio::time::sleep;

    let state = app_handle.state::<AppState>();
    let workers: Vec<Arc<IndexingWorker>> = {
        let map = state
            .indexing_workers
            .lock()
            .expect("indexing_workers mutex poisoned");
        map.values().cloned().collect()
    };

    if workers.is_empty() {
        tracing::info!(target: "shutdown", "진행 중 인덱싱 잡 없음 — graceful shutdown 즉시 완료");
        return;
    }

    tracing::info!(
        target: "shutdown",
        count = workers.len(),
        "진행 중 잡에 AppQuit pause + cancel — DB 커밋 대기(max 10s)"
    );

    for w in &workers {
        w.pause(index::v042::worker::PauseReason::AppQuit);
        // pause만으로는 활성 batch가 끝날 때까지 대기 — 추가 cancel로 batch 경계에서 즉시 종료.
        w.cancel();
        // wait_if_paused로 블록된 worker가 wakeup되어 cancel 점검할 수 있도록 resume 시그널.
        w.resume();
    }

    // 워커 task가 자연 종료되도록 짧게 대기. 실제 종료는 worker 루프가 cancel 점검 후
    // batch 사이에서 그만두는 시점.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = {
            let map = state
                .indexing_workers
                .lock()
                .expect("indexing_workers mutex poisoned");
            map.len()
        };
        if remaining == 0 || tokio::time::Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    let final_count = {
        let map = state
            .indexing_workers
            .lock()
            .expect("indexing_workers mutex poisoned");
        map.len()
    };
    tracing::info!(
        target: "shutdown",
        remaining = final_count,
        "graceful shutdown 완료 — 남은 잡은 다음 시작 시 비정상 종료 회복 절차로 처리"
    );
}

/// PR 28 hotfix — 현재 settings 기준으로 LLM provider를 *시도* 재구성.
///
/// 성공: state.llm 교체 + Ok(true)
/// 실패: 기존 provider 유지 + Ok(false) + warn log
///
/// settings_write·cli_install_provider·cli_login 등 LLM 어댑터 가용성에 영향 줄 수 있는
/// 모든 명령이 호출. CLI 미설치/미인증 같은 transient 상태로 인한 build 실패가
/// 명령 자체를 깨뜨리지 않도록 *fail-soft* 정책.
pub fn try_rebuild_llm(state: &AppState) -> bool {
    let (provider, auth_mode) = {
        let g = state.settings.lock().expect("settings mutex");
        (g.active_provider, g.auth_mode)
    };
    let data_dir = state.data_dir.clone();
    match build_provider(provider, auth_mode, &data_dir) {
        Ok(new_llm) => {
            *state.llm.lock().expect("llm mutex") = new_llm;
            tracing::info!(
                target: "llm",
                provider = provider.as_str(),
                auth = ?auth_mode,
                "llm provider rebuilt"
            );
            true
        }
        Err(e) => {
            tracing::warn!(
                target: "llm",
                provider = provider.as_str(),
                auth = ?auth_mode,
                error = %e,
                "llm provider rebuild skipped — keep existing"
            );
            false
        }
    }
}
