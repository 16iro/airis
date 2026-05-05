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
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::{error, info};
use uuid::Uuid;

use crate::commands::book;
use crate::commands::memory;
use crate::commands::search;
use crate::commands::validation;
use crate::error::{AppError, AppResult};
use crate::jobs::{self, ChatPayload, FailedJob};
use crate::llm::{CacheBreakpoint, ChatEvent, ChatRequest, LlmProvider, Message, Role, Usage};
use crate::AppState;

const SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미입니다. 사용자가 제공한 교재 본문을 바탕으로 정확하게 답변하고, 본문에 없는 내용은 '본문에 없음'이라고 명시하세요.\n\n응답 형식 (가능하면 따라주세요 — F4.5 3층 응답):\n1) 한 줄 요약\n2) 본문 인용·설명 (출처는 [1], [2] 마커로 표시)\n3) (선택) 더 알아보려면: 추가 섹션·키워드 제안";
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
    /// v0.3.2 B1: 어시스턴트 응답이 받은 컨텍스트 요약. user/system 메시지는 None.
    pub context: Option<ChatContextSummary>,
}

/// v0.3.2 B1 — 어시스턴트 응답에 어떤 컨텍스트가 주입됐는지 요약.
/// chat:context 이벤트로 emit되고, chat_messages.context_json에 영속.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatContextSummary {
    /// "active_section" | "fts" | "current_file" | "none"
    pub kind: String,
    pub hits: Vec<ChatContextHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatContextHit {
    pub book_id: Option<String>,
    pub book_title: Option<String>,
    pub book_role: Option<String>,
    pub section_label: Option<String>,
    pub section_path: Option<String>,
    pub page: Option<i64>,
}

impl ChatContextSummary {
    fn none() -> Self {
        Self {
            kind: "none".to_string(),
            hits: Vec::new(),
        }
    }
    fn is_empty(&self) -> bool {
        self.hits.is_empty() && self.kind == "none"
    }
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
            None,
        )?;
    }

    let payload = ChatPayload {
        query: query.clone(),
        context_section_id: context_section_id.clone(),
    };
    let (request, context_summary) = build_chat_request(&state, &study_slug, &payload);
    let provider = state.llm.lock().expect("llm mutex").clone();
    let model = request.model.clone();

    let handle = format!("chat-{}", Uuid::new_v4());
    let app_handle = app.clone();
    let handle_for_task = handle.clone();
    let payload_for_task = payload.clone();
    let study_slug_for_task = study_slug.clone();
    let context_for_task = context_summary.clone();

    info!(
        target: "llm",
        handle = %handle,
        study = %study_slug,
        query_len = query.len(),
        context = %context_summary.kind,
        "chat_send"
    );

    // chat:context 이벤트 — stream 시작 직전. 프론트가 진행 중 어시스턴트 메시지에 첨부.
    if let Err(e) = app.emit(
        "chat:context",
        serde_json::json!({ "handle": &handle, "context": &context_summary }),
    ) {
        tracing::warn!(target: "llm", error = %e, "chat:context emit failed");
    }

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
            context_for_task,
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

    let (request, context_summary) = build_chat_request(&state, &study_slug, &payload);
    let provider = state.llm.lock().expect("llm mutex").clone();
    let model = request.model.clone();

    let handle = format!("chat-{}", Uuid::new_v4());
    let app_handle = app.clone();
    let handle_for_task = handle.clone();
    let payload_for_task = payload.clone();
    let study_slug_for_task = study_slug.clone();
    let context_for_task = context_summary.clone();

    info!(
        target: "llm",
        handle = %handle,
        study = %study_slug,
        job_id,
        context = %context_summary.kind,
        "retry_failed_job"
    );

    if let Err(e) = app.emit(
        "chat:context",
        serde_json::json!({ "handle": &handle, "context": &context_summary }),
    ) {
        tracing::warn!(target: "llm", error = %e, "chat:context emit failed");
    }

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
            context_for_task,
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

/// 자동 워커가 *now ≥ next_retry_at*인 due 잡을 받아 *retry_failed_job*에 흘리는 데 사용.
#[tauri::command]
pub fn list_due_jobs(state: State<'_, AppState>) -> AppResult<Vec<FailedJob>> {
    let db = state.db.lock().expect("db mutex");
    jobs::list_due_jobs(db.conn())
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
    context_json: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO chat_messages (
            study_slug, role, content, created_at,
            cache_hit_tokens, creation_tokens, output_tokens, model,
            context_json
         )
         VALUES (?1, ?2, ?3, datetime('now'), ?4, ?5, ?6, ?7, ?8)",
        params![
            study_slug,
            role,
            content,
            meta.cache_read_tokens,
            meta.input_tokens,
            meta.output_tokens,
            meta.model,
            context_json,
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
                creation_tokens, output_tokens, cache_hit_tokens, context_json
         FROM chat_messages
         WHERE study_slug = ?1
           AND (?2 IS NULL OR id < ?2)
         ORDER BY id DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![study_slug, before, limit], |r| {
        let context_raw: Option<String> = r.get(8)?;
        let context = context_raw
            .and_then(|s| serde_json::from_str::<ChatContextSummary>(&s).ok());
        Ok(ChatHistoryMessage {
            id: r.get(0)?,
            role: r.get(1)?,
            content: r.get(2)?,
            created_at: r.get(3)?,
            model: r.get(4)?,
            input_tokens: r.get(5)?,
            output_tokens: r.get(6)?,
            cache_read_tokens: r.get(7)?,
            context,
        })
    })?;
    let mut items: Vec<ChatHistoryMessage> = rows.collect::<Result<_, _>>()?;
    // 시간순(오름차순)으로 뒤집어 반환 — 프론트는 그대로 push만 하면 됨.
    items.reverse();
    Ok(items)
}

fn build_chat_request(
    state: &AppState,
    study_slug: &str,
    payload: &ChatPayload,
) -> (ChatRequest, ChatContextSummary) {
    let model = state
        .settings
        .lock()
        .expect("settings mutex")
        .active_model();
    let (context_block, context_summary) = build_context(state, study_slug, payload);

    // Memory L1·L2 자동 주입 — D-036/F10.6.
    let compressed = memory::read(&state.data_dir, study_slug, None)
        .ok()
        .map(|r| memory::compress(&r.doc.body))
        .unwrap_or_default();

    let mut system = String::from(SYSTEM_PROMPT);
    if !compressed.l1.is_empty() {
        system.push_str("\n\n## 사용자 누적 선호·교정 (활성)\n");
        system.push_str(&compressed.l1);
    }
    if !compressed.l2.is_empty() {
        system.push_str("\n\n## 학습 진도·메타·목표 (활성)\n");
        system.push_str(&compressed.l2);
    }

    let cache_breakpoints = if !compressed.l1.is_empty() || !compressed.l2.is_empty() {
        vec![CacheBreakpoint::System]
    } else {
        Vec::new()
    };

    let user_message = if context_block.is_empty() {
        payload.query.clone()
    } else {
        format!("{context_block}\n\n사용자 질문: {}", payload.query)
    };

    let request = ChatRequest {
        model,
        system: Some(system),
        messages: vec![Message {
            role: Role::User,
            content: user_message,
        }],
        max_tokens: MAX_TOKENS,
        cache_breakpoints,
    };
    (request, context_summary)
}

/// 컨텍스트 우선순위 (D-064 슬라이스 정신, PR 12 갱신):
/// 1) 활성 섹션 (사용자가 BookViewer에서 마지막 클릭한 섹션) — 가장 명시적인 의도
/// 2) FTS5 검색 결과 Top-K — 활성 스터디의 책에서 query와 관련된 섹션
/// 3) 현재 열린 단일 파일 본문 (v0.1 호환 fallback)
///
/// 셋 다 없으면 (빈 문자열, none summary). v0.3.2 B1: prompt block과 함께 어떤 컨텍스트가
/// 주입됐는지 ChatContextSummary로 동시 반환 — chat:context 이벤트 + DB 영속에 사용.
fn build_context(
    state: &AppState,
    study_slug: &str,
    payload: &ChatPayload,
) -> (String, ChatContextSummary) {
    if let Some((block, hit)) = build_active_section_block(state) {
        return (
            block,
            ChatContextSummary {
                kind: "active_section".to_string(),
                hits: vec![hit],
            },
        );
    }

    let hits = match search::normalize_query(&payload.query) {
        Ok(expr) => {
            let db = state.db.lock().expect("db mutex");
            search::fts_search(db.conn(), study_slug, &expr, 5).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    if !hits.is_empty() {
        let mut block = String::from("다음은 등록된 책에서 사용자 질문과 관련된 섹션입니다:\n");
        let mut summary_hits = Vec::with_capacity(hits.len());
        for (i, h) in hits.iter().enumerate() {
            // 부교재일 때 role_note를 헤더에 prepend — LLM이 책별 역할을 인지하고 활용.
            let role_tag = if h.book_role == "sub" {
                match h.book_role_note.as_deref() {
                    Some(note) if !note.trim().is_empty() => {
                        format!(" [부교재 — {note}]")
                    }
                    _ => " [부교재]".to_string(),
                }
            } else {
                String::new()
            };
            let header = format!(
                "\n---\n[{}] {}{} · {} {}",
                i + 1,
                h.book_title,
                role_tag,
                h.section_label,
                h.page.map(|p| format!("(p. {p})")).unwrap_or_default()
            );
            block.push_str(&header);
            block.push('\n');
            block.push_str(&h.snippet);

            summary_hits.push(ChatContextHit {
                book_id: Some(h.book_id.clone()),
                book_title: Some(h.book_title.clone()),
                book_role: Some(h.book_role.clone()),
                section_label: Some(h.section_label.clone()),
                section_path: Some(h.section_path.clone()),
                page: h.page,
            });
        }
        block.push_str("\n---");
        return (
            block,
            ChatContextSummary {
                kind: "fts".to_string(),
                hits: summary_hits,
            },
        );
    }

    if let Some(text) = state
        .current_file
        .lock()
        .expect("current_file mutex")
        .clone()
        .filter(|s| !s.is_empty())
    {
        return (
            format!("다음은 사용자가 학습 중인 교재 본문입니다:\n\n---\n{text}\n---"),
            ChatContextSummary {
                kind: "current_file".to_string(),
                hits: Vec::new(),
            },
        );
    }

    (String::new(), ChatContextSummary::none())
}

/// 활성 섹션이 박혀 있고 paragraphs에 본문이 있으면 (헤더 + 본문 블록, 요약 hit) 반환.
fn build_active_section_block(state: &AppState) -> Option<(String, ChatContextHit)> {
    let active = state
        .active_section
        .lock()
        .expect("active_section mutex")
        .clone()?;
    let db = state.db.lock().expect("db mutex");
    let body = book::fetch_section_body(db.conn(), &active.book_id, &active.section_path)
        .ok()
        .flatten()?;
    let label = book::fetch_section_label(db.conn(), &active.book_id, &active.section_path)
        .ok()
        .flatten();
    let (book_title_opt, section_label_opt) = match &label {
        Some((bt, sl)) => (Some(bt.clone()), Some(sl.clone())),
        None => (None, None),
    };
    let header = match label {
        Some((book_title, section_label)) => {
            format!("다음은 사용자가 보고 있는 *{book_title} · {section_label}* 섹션입니다:")
        }
        None => "다음은 사용자가 보고 있는 섹션입니다:".to_string(),
    };
    let block = format!("{header}\n\n---\n{body}\n---");
    let hit = ChatContextHit {
        book_id: Some(active.book_id.clone()),
        book_title: book_title_opt,
        book_role: None,
        section_label: section_label_opt,
        section_path: Some(active.section_path.clone()),
        page: None,
    };
    Some((block, hit))
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
    context_summary: ChatContextSummary,
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
                persist_assistant_message(
                    &app,
                    &study_slug,
                    &accumulated,
                    &model,
                    &usage,
                    &context_summary,
                );

                // F4.4 응답 검증 — Memory.Corrections active 위반 의심 검출. emit chat:violation.
                emit_violations(&app, &handle, &study_slug, &accumulated);

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

/// chat:done 직후 Memory.Corrections 위반 의심 검사 + chat:violation event emit.
/// 거짓 양성 가능 — UI는 *경고 배너*만, 응답은 그대로 둠 (handoff 결정 #1).
fn emit_violations(app: &AppHandle, handle: &str, study_slug: &str, response: &str) {
    let state = app.state::<AppState>();
    let memory_result = match memory::read(&state.data_dir, study_slug, None) {
        Ok(r) if r.file_existed => r,
        _ => return,
    };
    let hits = validation::detect(response, &memory_result.doc.body);
    if hits.is_empty() {
        return;
    }
    if let Err(e) = app.emit(
        "chat:violation",
        serde_json::json!({ "handle": handle, "violations": hits }),
    ) {
        tracing::warn!(target: "llm", error = %e, "chat:violation emit failed");
    }
}

fn persist_assistant_message(
    app: &AppHandle,
    study_slug: &str,
    content: &str,
    model: &str,
    usage: &Usage,
    context: &ChatContextSummary,
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
    let context_json = if context.is_empty() {
        None
    } else {
        match serde_json::to_string(context) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(target: "llm", error = %e, "serialize chat context failed");
                None
            }
        }
    };
    if let Err(e) = insert_chat_message(
        db.conn(),
        study_slug,
        "assistant",
        content,
        meta,
        context_json.as_deref(),
    ) {
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
        insert_chat_message(db.conn(), "s1", "user", "hello", ChatMessageMeta::default(), None)
            .unwrap();
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
            None,
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
                None,
            )
            .unwrap();
        }
        insert_chat_message(
            db.conn(),
            "b",
            "user",
            "b-only",
            ChatMessageMeta::default(),
            None,
        )
        .unwrap();

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
            insert_chat_message(db.conn(), "s1", "user", "x", ChatMessageMeta::default(), None)
                .unwrap();
        }
        let all = fetch_chat_history(db.conn(), "s1", 50, None).unwrap();
        let middle_id = all[1].id;
        let before = fetch_chat_history(db.conn(), "s1", 50, Some(middle_id)).unwrap();
        // before=middle_id → id < middle_id 만 반환 → 1개.
        assert_eq!(before.len(), 1);
        assert!(before[0].id < middle_id);
    }
}
