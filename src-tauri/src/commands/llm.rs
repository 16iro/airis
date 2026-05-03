// F4 — LLM 챗 호출 + 실패 큐 (PR 6).
// chat_send: 즉시 handle 반환 → spawn task가 스트리밍 + 실패 시 큐 적재.
// retry_failed_job: payload 재시도 → 새 handle 반환. 성공 시 row 삭제, 실패 시 attempts++.

use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::{error, info};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::jobs::{self, ChatPayload, FailedJob};
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role};
use crate::AppState;

const SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미입니다. 사용자가 제공한 교재 본문을 바탕으로 정확하게 답변하고, 본문에 없는 내용은 '본문에 없음'이라고 명시하세요.";
const MAX_TOKENS: u32 = 4096;
const STUDY_SLUG_DEFAULT: &str = "default";

#[derive(Debug, Serialize)]
pub struct ChatJobHandle {
    pub handle: String,
}

#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    state: State<'_, AppState>,
    study_slug: String,
    query: String,
    context_section_id: Option<String>,
) -> AppResult<ChatJobHandle> {
    if study_slug != STUDY_SLUG_DEFAULT {
        return Err(AppError::InvalidInput {
            message: "v0.1: study_slug는 'default'만 지원합니다".into(),
        });
    }
    if context_section_id.is_some() {
        return Err(AppError::InvalidInput {
            message: "v0.1: context_section_id는 v0.2부터 지원됩니다".into(),
        });
    }
    if query.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "질문이 비어 있습니다".into(),
        });
    }

    let payload = ChatPayload {
        query: query.clone(),
        context_section_id,
    };
    let request = build_chat_request(&state, &payload);
    let provider = state.llm.clone();

    let handle = format!("chat-{}", Uuid::new_v4());
    let app_handle = app.clone();
    let handle_for_task = handle.clone();
    let payload_for_task = payload.clone();

    info!(target: "llm", handle = %handle, query_len = query.len(), "chat_send");

    tokio::spawn(async move {
        run_stream(
            app_handle,
            handle_for_task,
            provider,
            request,
            payload_for_task,
            None,
        )
        .await;
    });

    Ok(ChatJobHandle { handle })
}

/// 큐에 적재된 잡 명시 재시도. 새 handle 반환 — 프론트는 일반 chat_send처럼 events 구독.
#[tauri::command]
pub async fn retry_failed_job(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: i64,
) -> AppResult<ChatJobHandle> {
    let payload = {
        let db = state.db.lock().expect("db mutex");
        jobs::fetch_payload(db.conn(), job_id)?
    };

    let request = build_chat_request(&state, &payload);
    let provider = state.llm.clone();

    let handle = format!("chat-{}", Uuid::new_v4());
    let app_handle = app.clone();
    let handle_for_task = handle.clone();
    let payload_for_task = payload.clone();

    info!(target: "llm", handle = %handle, job_id, "retry_failed_job");

    tokio::spawn(async move {
        run_stream(
            app_handle,
            handle_for_task,
            provider,
            request,
            payload_for_task,
            Some(job_id),
        )
        .await;
    });

    Ok(ChatJobHandle { handle })
}

#[tauri::command]
pub fn list_failed_jobs(
    state: State<'_, AppState>,
    study_slug: Option<String>,
) -> AppResult<Vec<FailedJob>> {
    let db = state.db.lock().expect("db mutex");
    jobs::list_jobs(db.conn(), study_slug.as_deref())
}

#[tauri::command]
pub fn delete_failed_job(state: State<'_, AppState>, job_id: i64) -> AppResult<()> {
    let db = state.db.lock().expect("db mutex");
    jobs::delete_job(db.conn(), job_id)?;
    info!(target: "llm", job_id, "delete_failed_job");
    Ok(())
}

fn build_chat_request(state: &AppState, payload: &ChatPayload) -> ChatRequest {
    let context_text = state
        .current_file
        .lock()
        .expect("current_file mutex")
        .clone();
    let model = state.settings.lock().expect("settings mutex").model.clone();

    let user_message = match context_text {
        Some(text) if !text.is_empty() => format!(
            "다음은 사용자가 학습 중인 교재 본문입니다:\n\n---\n{text}\n---\n\n사용자 질문: {}",
            payload.query
        ),
        _ => payload.query.clone(),
    };

    ChatRequest {
        model,
        system: Some(SYSTEM_PROMPT.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: user_message,
        }],
        max_tokens: MAX_TOKENS,
    }
}

async fn run_stream(
    app: AppHandle,
    handle: String,
    provider: Arc<dyn LlmProvider>,
    request: ChatRequest,
    payload: ChatPayload,
    retry_job_id: Option<i64>,
) {
    let stream_result = provider.chat_stream(request).await;
    let mut stream = match stream_result {
        Ok(s) => s,
        Err(e) => {
            error!(target: "llm", handle = %handle, error = %e, "chat_stream init failed");
            let job_id = handle_failure(&app, &payload, &e, retry_job_id);
            emit_error(&app, &handle, &e, job_id);
            return;
        }
    };

    while let Some(event) = stream.next().await {
        match event {
            Ok(ChatEvent::TextDelta { text }) => {
                let _ = app.emit(
                    "chat:chunk",
                    serde_json::json!({ "handle": &handle, "text": text }),
                );
            }
            Ok(ChatEvent::Done { usage }) => {
                info!(
                    target: "llm",
                    handle = %handle,
                    input = usage.input_tokens,
                    output = usage.output_tokens,
                    "chat_done"
                );
                // 재시도 성공 → 큐에서 row 삭제.
                if let Some(id) = retry_job_id {
                    let state = app.state::<AppState>();
                    let db = state.db.lock().expect("db mutex");
                    if let Err(e) = jobs::delete_job(db.conn(), id) {
                        error!(target: "llm", job_id = id, error = %e, "delete_job after retry success failed");
                    }
                }
                let _ = app.emit(
                    "chat:done",
                    serde_json::json!({ "handle": &handle, "usage": usage }),
                );
            }
            Err(e) => {
                error!(target: "llm", handle = %handle, error = %e, "stream error");
                let job_id = handle_failure(&app, &payload, &e, retry_job_id);
                emit_error(&app, &handle, &e, job_id);
                return;
            }
        }
    }
}

/// 에러를 받으면 *재시도 가능*한 경우 큐에 적재. 적재된 job_id 반환 (없으면 None).
fn handle_failure(
    app: &AppHandle,
    payload: &ChatPayload,
    error: &AppError,
    _retry_job_id: Option<i64>,
) -> Option<i64> {
    if !jobs::is_retryable_error(error) {
        return None;
    }

    let state = app.state::<AppState>();
    let db = state.db.lock().expect("db mutex");
    match jobs::enqueue_or_update(db.conn(), STUDY_SLUG_DEFAULT, payload, &error.to_string()) {
        Ok(id) => {
            info!(target: "llm", job_id = id, "queued failed chat job");
            Some(id)
        }
        Err(e) => {
            error!(target: "llm", error = %e, "enqueue failed");
            None
        }
    }
}

fn emit_error(app: &AppHandle, handle: &str, err: &AppError, job_id: Option<i64>) {
    let _ = app.emit(
        "chat:error",
        serde_json::json!({
            "handle": handle,
            "error": err,
            "job_id": job_id,
        }),
    );
}
