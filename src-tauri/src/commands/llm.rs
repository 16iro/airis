// F4 — LLM 챗 호출 + 실패 큐 + chat_messages 영속.
//
// chat_send 흐름:
//   1) study_slug 검증 (실존 + 활성/임의 둘 다 허용 — 활성 외 검색은 v0.3+ '탭 다중 스터디' 시 의미)
//   2) user 메시지 즉시 INSERT (history 영속 시작)
//   3) handle 반환, spawn task가 SSE 스트리밍
//   4) chat:done 시 assistant 메시지 INSERT + 토큰·모델 메타 기록
//   5) chat:error 시 잡 큐 적재 (assistant 메시지는 영속 X — 큐 항목으로 재시도)

use std::sync::Arc;

use futures_util::StreamExt;
use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::{error, info};
use uuid::Uuid;

use crate::commands::search;
use crate::error::{AppError, AppResult};
use crate::jobs::{self, ChatPayload, FailedJob};
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role, Usage};
use crate::AppState;

const SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미입니다. 사용자가 제공한 교재 본문을 바탕으로 정확하게 답변하고, 본문에 없는 내용은 '본문에 없음'이라고 명시하세요.";
const MAX_TOKENS: u32 = 4096;
const HISTORY_DEFAULT_LIMIT: u32 = 50;
const HISTORY_MAX_LIMIT: u32 = 500;

#[derive(Debug, Serialize)]
pub struct ChatJobHandle {
    pub handle: String,
}

/// chat_history 응답 항목. created_at은 ISO 8601 (DB의 datetime('now')와 호환).
#[derive(Debug, Clone, Serialize)]
pub struct ChatHistoryMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
}

#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    state: State<'_, AppState>,
    study_slug: String,
    query: String,
    context_section_id: Option<String>,
) -> AppResult<ChatJobHandle> {
    if query.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "질문이 비어 있습니다".into(),
        });
    }

    // study_slug가 실존하는지 검증 — 존재하지 않으면 NotFound.
    {
        let db = state.db.lock().expect("db mutex");
        let exists: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM studies WHERE slug = ?1",
            params![&study_slug],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(AppError::NotFound {
                message: format!("스터디 '{study_slug}'를 찾을 수 없습니다"),
            });
        }
    }

    // user 메시지 즉시 영속 — chat_send가 성공 반환했다면 사용자 발화는 *항상* 기록.
    {
        let db = state.db.lock().expect("db mutex");
        insert_chat_message(
            db.conn(),
            &study_slug,
            "user",
            &query,
            ChatMessageMeta::default(),
        )?;
    }

    let payload = ChatPayload {
        query: query.clone(),
        context_section_id: context_section_id.clone(),
    };
    let request = build_chat_request(&state, &study_slug, &payload);
    let provider = state.llm.clone();
    let model = request.model.clone();

    let handle = format!("chat-{}", Uuid::new_v4());
    let app_handle = app.clone();
    let handle_for_task = handle.clone();
    let payload_for_task = payload.clone();
    let study_slug_for_task = study_slug.clone();

    info!(
        target: "llm",
        handle = %handle,
        study = %study_slug,
        query_len = query.len(),
        "chat_send"
    );

    tokio::spawn(async move {
        run_stream(
            app_handle,
            handle_for_task,
            provider,
            request,
            payload_for_task,
            study_slug_for_task,
            model,
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
    let (payload, study_slug) = {
        let db = state.db.lock().expect("db mutex");
        let payload = jobs::fetch_payload(db.conn(), job_id)?;
        let slug = jobs::fetch_study_slug(db.conn(), job_id)?;
        (payload, slug)
    };

    let request = build_chat_request(&state, &study_slug, &payload);
    let provider = state.llm.clone();
    let model = request.model.clone();

    let handle = format!("chat-{}", Uuid::new_v4());
    let app_handle = app.clone();
    let handle_for_task = handle.clone();
    let payload_for_task = payload.clone();
    let study_slug_for_task = study_slug.clone();

    info!(
        target: "llm",
        handle = %handle,
        study = %study_slug,
        job_id,
        "retry_failed_job"
    );

    tokio::spawn(async move {
        run_stream(
            app_handle,
            handle_for_task,
            provider,
            request,
            payload_for_task,
            study_slug_for_task,
            model,
            Some(job_id),
        )
        .await;
    });

    Ok(ChatJobHandle { handle })
}

#[tauri::command]
pub fn chat_history(
    state: State<'_, AppState>,
    study_slug: String,
    limit: Option<u32>,
    before: Option<i64>,
) -> AppResult<Vec<ChatHistoryMessage>> {
    let lim = limit
        .unwrap_or(HISTORY_DEFAULT_LIMIT)
        .min(HISTORY_MAX_LIMIT) as i64;
    let db = state.db.lock().expect("db mutex");
    fetch_chat_history(db.conn(), &study_slug, lim, before)
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

// ---- helpers --------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy)]
struct ChatMessageMeta<'a> {
    model: Option<&'a str>,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
}

fn insert_chat_message(
    conn: &Connection,
    study_slug: &str,
    role: &str,
    content: &str,
    meta: ChatMessageMeta<'_>,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO chat_messages (
            study_slug, role, content, created_at,
            cache_hit_tokens, creation_tokens, output_tokens, model
         )
         VALUES (?1, ?2, ?3, datetime('now'), ?4, ?5, ?6, ?7)",
        params![
            study_slug,
            role,
            content,
            meta.cache_read_tokens,
            meta.input_tokens,
            meta.output_tokens,
            meta.model,
        ],
    )?;
    Ok(())
}

fn fetch_chat_history(
    conn: &Connection,
    study_slug: &str,
    limit: i64,
    before: Option<i64>,
) -> AppResult<Vec<ChatHistoryMessage>> {
    // 최신부터 limit개 → 사용자에 보여줄 땐 reverse(시간순). 페이징은 id 기반 cursor.
    let mut stmt = conn.prepare(
        "SELECT id, role, content, created_at, model,
                creation_tokens, output_tokens, cache_hit_tokens
         FROM chat_messages
         WHERE study_slug = ?1
           AND (?2 IS NULL OR id < ?2)
         ORDER BY id DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![study_slug, before, limit], |r| {
        Ok(ChatHistoryMessage {
            id: r.get(0)?,
            role: r.get(1)?,
            content: r.get(2)?,
            created_at: r.get(3)?,
            model: r.get(4)?,
            input_tokens: r.get(5)?,
            output_tokens: r.get(6)?,
            cache_read_tokens: r.get(7)?,
        })
    })?;
    let mut items: Vec<ChatHistoryMessage> = rows.collect::<Result<_, _>>()?;
    // 시간순(오름차순)으로 뒤집어 반환 — 프론트는 그대로 push만 하면 됨.
    items.reverse();
    Ok(items)
}

fn build_chat_request(state: &AppState, study_slug: &str, payload: &ChatPayload) -> ChatRequest {
    let model = state.settings.lock().expect("settings mutex").model.clone();
    let context_block = build_context_block(state, study_slug, payload);

    let user_message = if context_block.is_empty() {
        payload.query.clone()
    } else {
        format!("{context_block}\n\n사용자 질문: {}", payload.query)
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

/// 컨텍스트 우선순위 (D-064 슬라이스 정신):
/// 1) 현재 열린 파일 본문 (v0.1 호환 — 있으면 그대로 주입)
/// 2) FTS5 검색 결과 Top-K — 활성 스터디의 책에서 query와 관련된 섹션
///
/// 둘 다 없으면 빈 문자열.
fn build_context_block(state: &AppState, study_slug: &str, payload: &ChatPayload) -> String {
    if let Some(text) = state
        .current_file
        .lock()
        .expect("current_file mutex")
        .clone()
        .filter(|s| !s.is_empty())
    {
        return format!("다음은 사용자가 학습 중인 교재 본문입니다:\n\n---\n{text}\n---");
    }

    let hits = match search::normalize_query(&payload.query) {
        Ok(expr) => {
            let db = state.db.lock().expect("db mutex");
            search::fts_search(db.conn(), study_slug, &expr, 5).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    if hits.is_empty() {
        return String::new();
    }

    let mut block = String::from("다음은 등록된 책에서 사용자 질문과 관련된 섹션입니다:\n");
    for (i, h) in hits.iter().enumerate() {
        let header = format!(
            "\n---\n[{}] {} · {} {}",
            i + 1,
            h.book_title,
            h.section_label,
            h.page.map(|p| format!("(p. {p})")).unwrap_or_default()
        );
        block.push_str(&header);
        block.push('\n');
        block.push_str(&h.snippet);
    }
    block.push_str("\n---");
    block
}

#[allow(clippy::too_many_arguments)]
async fn run_stream(
    app: AppHandle,
    handle: String,
    provider: Arc<dyn LlmProvider>,
    request: ChatRequest,
    payload: ChatPayload,
    study_slug: String,
    model: String,
    retry_job_id: Option<i64>,
) {
    // 누적 텍스트를 보관 — chat:done 시 assistant 메시지로 영속.
    let mut accumulated = String::new();

    let stream_result = provider.chat_stream(request).await;
    let mut stream = match stream_result {
        Ok(s) => s,
        Err(e) => {
            error!(target: "llm", handle = %handle, error = %e, "chat_stream init failed");
            let job_id = handle_failure(&app, &payload, &study_slug, &e, retry_job_id);
            emit_error(&app, &handle, &e, job_id);
            return;
        }
    };

    while let Some(event) = stream.next().await {
        match event {
            Ok(ChatEvent::TextDelta { text }) => {
                accumulated.push_str(&text);
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
                persist_assistant_message(&app, &study_slug, &accumulated, &model, &usage);

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
                let job_id = handle_failure(&app, &payload, &study_slug, &e, retry_job_id);
                emit_error(&app, &handle, &e, job_id);
                return;
            }
        }
    }
}

fn persist_assistant_message(
    app: &AppHandle,
    study_slug: &str,
    content: &str,
    model: &str,
    usage: &Usage,
) {
    if content.is_empty() {
        // 모델이 빈 응답을 준 경우 — 영속하지 않는다(history에 빈 행 noise만 남음).
        return;
    }
    let state = app.state::<AppState>();
    let db = state.db.lock().expect("db mutex");
    let meta = ChatMessageMeta {
        model: Some(model),
        input_tokens: usage.input_tokens as i64,
        output_tokens: usage.output_tokens as i64,
        cache_read_tokens: usage.cache_read_input_tokens as i64,
    };
    if let Err(e) = insert_chat_message(db.conn(), study_slug, "assistant", content, meta) {
        error!(target: "llm", error = %e, "persist assistant message failed");
    }
}

/// 에러를 받으면 *재시도 가능*한 경우 큐에 적재. 적재된 job_id 반환 (없으면 None).
fn handle_failure(
    app: &AppHandle,
    payload: &ChatPayload,
    study_slug: &str,
    error: &AppError,
    _retry_job_id: Option<i64>,
) -> Option<i64> {
    if !jobs::is_retryable_error(error) {
        return None;
    }

    let state = app.state::<AppState>();
    let db = state.db.lock().expect("db mutex");
    match jobs::enqueue_or_update(db.conn(), study_slug, payload, &error.to_string()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn seed_study(conn: &Connection, slug: &str) {
        conn.execute(
            "INSERT INTO studies (slug, name, created_at) VALUES (?1, ?1, datetime('now'))",
            params![slug],
        )
        .unwrap();
    }

    #[test]
    fn insert_and_fetch_history_returns_chronological() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        insert_chat_message(db.conn(), "s1", "user", "hello", ChatMessageMeta::default()).unwrap();
        insert_chat_message(
            db.conn(),
            "s1",
            "assistant",
            "hi there",
            ChatMessageMeta {
                model: Some("claude-opus-4-7"),
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: 0,
            },
        )
        .unwrap();

        let history = fetch_chat_history(db.conn(), "s1", 50, None).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(history[1].input_tokens, 10);
    }

    #[test]
    fn fetch_history_respects_limit_and_isolates_studies() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "a");
        seed_study(db.conn(), "b");
        for i in 0..5 {
            insert_chat_message(
                db.conn(),
                "a",
                "user",
                &format!("a{i}"),
                ChatMessageMeta::default(),
            )
            .unwrap();
        }
        insert_chat_message(db.conn(), "b", "user", "b-only", ChatMessageMeta::default()).unwrap();

        let only_b = fetch_chat_history(db.conn(), "b", 50, None).unwrap();
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].content, "b-only");

        let limited = fetch_chat_history(db.conn(), "a", 3, None).unwrap();
        assert_eq!(limited.len(), 3);
        // 시간순 — 마지막 3개는 a2, a3, a4.
        assert_eq!(limited[0].content, "a2");
        assert_eq!(limited[2].content, "a4");
    }

    #[test]
    fn fetch_history_before_cursor_excludes_id() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        for _ in 0..3 {
            insert_chat_message(db.conn(), "s1", "user", "x", ChatMessageMeta::default()).unwrap();
        }
        let all = fetch_chat_history(db.conn(), "s1", 50, None).unwrap();
        let middle_id = all[1].id;
        let before = fetch_chat_history(db.conn(), "s1", 50, Some(middle_id)).unwrap();
        // before=middle_id → id < middle_id 만 반환 → 1개.
        assert_eq!(before.len(), 1);
        assert!(before[0].id < middle_id);
    }
}
