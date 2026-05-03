// F4 — LLM 챗 호출.
// v0.1 PR 4: chat_send 1개. handle 즉시 반환 + Tauri events로 스트리밍.
// 시그니처는 api-contract.md와 동일. v0.1 가드:
//   - study_slug != "default" → InvalidInput
//   - context_section_id.is_some() → InvalidInput

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tracing::{error, info};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::llm::{ChatEvent, ChatRequest, Message, Role};
use crate::AppState;

const SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미입니다. 사용자가 제공한 교재 본문을 바탕으로 정확하게 답변하고, 본문에 없는 내용은 '본문에 없음'이라고 명시하세요.";
const MAX_TOKENS: u32 = 4096;

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
    // v0.1 인자 가드 — 미래 확장에 대비해 시그니처는 유지하되 실값 제약.
    if study_slug != "default" {
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

    let context_text = state
        .current_file
        .lock()
        .expect("current_file mutex")
        .clone();
    let model = state.settings.lock().expect("settings mutex").model.clone();

    let user_message = match context_text {
        Some(text) if !text.is_empty() => format!(
            "다음은 사용자가 학습 중인 교재 본문입니다:\n\n---\n{text}\n---\n\n사용자 질문: {query}"
        ),
        _ => query.clone(),
    };

    let request = ChatRequest {
        model,
        system: Some(SYSTEM_PROMPT.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: user_message,
        }],
        max_tokens: MAX_TOKENS,
    };

    let handle = format!("chat-{}", Uuid::new_v4());
    let provider = state.llm.clone();
    let app_handle = app.clone();
    let handle_for_task = handle.clone();

    info!(target: "llm", handle = %handle, query_len = query.len(), "chat_send");

    tokio::spawn(async move {
        run_stream(app_handle, handle_for_task, provider, request).await;
    });

    Ok(ChatJobHandle { handle })
}

async fn run_stream(
    app: AppHandle,
    handle: String,
    provider: std::sync::Arc<dyn crate::llm::LlmProvider>,
    request: ChatRequest,
) {
    let stream_result = provider.chat_stream(request).await;
    let mut stream = match stream_result {
        Ok(s) => s,
        Err(e) => {
            error!(target: "llm", handle = %handle, error = %e, "chat_stream init failed");
            emit_error(&app, &handle, &e);
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
                    cache_create = usage.cache_creation_input_tokens,
                    cache_read = usage.cache_read_input_tokens,
                    "chat_done"
                );
                let _ = app.emit(
                    "chat:done",
                    serde_json::json!({ "handle": &handle, "usage": usage }),
                );
            }
            Err(e) => {
                error!(target: "llm", handle = %handle, error = %e, "stream error");
                emit_error(&app, &handle, &e);
                return;
            }
        }
    }
}

fn emit_error(app: &AppHandle, handle: &str, err: &AppError) {
    let _ = app.emit(
        "chat:error",
        serde_json::json!({ "handle": handle, "error": err }),
    );
}
