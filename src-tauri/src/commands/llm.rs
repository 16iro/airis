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
use crate::index::v041::context::{
    build_context as v041_build_context, build_context_from_merged as v041_build_context_from_merged,
    parse_citations,
};
use crate::index::v041::retrieval::{hybrid_search, RetrievedChunk};
use crate::index::v042::active_index::read_active_index;
use crate::index::v042::manifest::IndexKind;
use crate::index::v042::retrieval::{hybrid_search_active, RetrievalEmbedder};
use crate::index::v042::worker::{IndexingWorker, PauseReason, Tier};
use crate::index::v043::post_retrieval::{
    expand_sentence_window, merge_parents, mmr_dedupe, MergedChunk, MMR_LAMBDA_DEFAULT,
};
use crate::index::v043::rewriter::{HistoryTurn, QueryRewriter, RewritePolicy, HISTORY_WINDOW_TURNS};
use crate::jobs::{self, ChatPayload, FailedJob};
use crate::llm::{CacheBreakpoint, ChatEvent, ChatRequest, LlmProvider, Message, Role, Usage};
use crate::power_monitor::priority::{can_auto_resume, should_override};
use crate::settings::SearchStrength;
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
///
/// v0.4.1 PR 3: 새 RAG 엔진(hybrid retrieval) 사용 시 `v041_chunks`를 추가로 채운다.
/// 기존 v0.3.2 흐름(active_section / fts / current_file / none)은 `v041_chunks=None`로
/// *완전 무파괴* — 직렬화·역직렬화 모두 기존 row와 호환.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatContextSummary {
    /// "active_section" | "fts" | "current_file" | "v041_hybrid" | "none"
    pub kind: String,
    pub hits: Vec<ChatContextHit>,
    /// v0.4.1 PR 3: 인용 마커 [Sx] → chunks.id 매핑. v0.3.2 흐름은 None.
    /// 프론트의 ChatContextChip 클릭 점프(PR 4)가 이 mapping을 사용한다.
    /// `serde(default, skip_serializing_if = "Option::is_none")` 으로 기존 row 호환.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v041_chunks: Option<Vec<ChatV041ChunkRef>>,
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

/// v0.4.1 PR 3 — 응답에 박힌 [Sx] 마커가 가리키는 chunk 식별자.
/// 프론트가 [Sx] 칩 클릭 시 BookViewer 섹션·페이지로 점프(PR 4 책임).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatV041ChunkRef {
    /// "S1", "S2", ...
    pub marker: String,
    /// chunks.id.
    pub chunk_id: i64,
    /// chunks.page (PDF는 1-base, MD/HTML은 None).
    pub page: Option<i64>,
    /// chunks.section_path (`Ch04/§State` 또는 `p.42`).
    pub section_path: Option<String>,
}

impl ChatContextSummary {
    fn none() -> Self {
        Self {
            kind: "none".to_string(),
            hits: Vec::new(),
            v041_chunks: None,
        }
    }
    fn is_empty(&self) -> bool {
        self.hits.is_empty() && self.kind == "none" && self.v041_chunks.is_none()
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

    // v0.4.3 PR 1 (D-086) — 검색 강도 토글에 따라 query rewriting layer 적용.
    //   * Fast      → rewriting skip, 원본 query 그대로.
    //   * Balanced  → 항상 rewriting (default).
    //   * Accurate  → rewriting + (HyDE는 PR 3 자리).
    // 폴백: 어떤 단계에서든 실패하면 원본 query — chat 흐름 보호.
    let provider = state.llm.lock().expect("llm mutex").clone();
    let policy = rewrite_policy_from_settings(&state);
    let history_for_rewrite = if policy.should_rewrite() {
        fetch_recent_history_turns(&state, &study_slug, &query, HISTORY_WINDOW_TURNS)
    } else {
        Vec::new()
    };
    let effective_query = if policy.should_rewrite() {
        let rewritten = QueryRewriter::new()
            .rewrite(&history_for_rewrite, &query, provider.as_ref())
            .await
            .unwrap_or_else(|_| query.clone());
        if rewritten != query {
            info!(
                target: "v043.rewriter",
                handle = "chat_send",
                study = %study_slug,
                "query rewriting applied"
            );
        }
        rewritten
    } else {
        query.clone()
    };

    let payload = ChatPayload {
        query: effective_query.clone(),
        context_section_id: context_section_id.clone(),
    };
    let (request, context_summary) = build_chat_request(&state, &study_slug, &payload);
    let model = request.model.clone();
    // v0.4.2 PR 4 (D-084) — chunk_ids 기반 cache key 추출. v0.4.1 hybrid retrieval에서만 의미 있음.
    let cache_key_meta = response_cache_key_meta(&context_summary, &payload.query, &model);

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

    // v0.4.2 PR 4 — response cache lookup. hit이면 LLM 호출 skip + cache value를 SSE로 직접 emit.
    if let Some(meta) = &cache_key_meta {
        let cached = {
            let response_cache = state.response_cache.clone();
            let db = state.db.lock().expect("db mutex");
            response_cache.get_by_key(db.conn(), &meta.key).ok().flatten()
        };
        if let Some(cached_text) = cached {
            info!(
                target: "llm",
                handle = %handle,
                study = %study_slug,
                "response_cache hit — LLM 호출 skip"
            );
            // cache hit emit — frontend가 dev panel 통계 / "캐시됨" 배지 표시 가능.
            let _ = app.emit(
                "chat:cache_hit",
                serde_json::json!({ "handle": &handle, "source": "response_cache" }),
            );
            let app_for_cache = app.clone();
            let handle_for_cache = handle.clone();
            let model_for_cache = model.clone();
            let study_slug_for_cache = study_slug.clone();
            let context_for_cache = context_summary.clone();
            tokio::spawn(async move {
                emit_cached_response(
                    app_for_cache,
                    handle_for_cache,
                    cached_text,
                    study_slug_for_cache,
                    model_for_cache,
                    context_for_cache,
                )
                .await;
            });
            return Ok(ChatJobHandle { handle });
        }
    }

    let cache_meta_for_task = cache_key_meta;

    // v0.4.2 PR 5 (D-083) — chat 진입 시 활성 T2 인덱싱 worker 모두 cooperative pause.
    // chat 응답 완료/에러 시 run_stream가 자동 resume (user pause는 보호).
    apply_cooperative_pause_for_chat(&state);

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
            cache_meta_for_task,
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

    // v0.4.2 PR 5 (D-083) — retry도 chat과 동등 — cooperative pause 적용.
    apply_cooperative_pause_for_chat(&state);

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
            None, // retry는 cache lookup/put 모두 skip — 명시 retry는 신선한 호출이 의도.
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

/// v0.4.3 PR 1 (D-086) — settings.search_strength → rewriter 정책으로 변환.
/// 별도 함수로 빼두면 chat_send 단위 테스트가 build_chat_request를 우회해 분기 검증 가능.
fn rewrite_policy_from_settings(state: &AppState) -> RewritePolicy {
    let strength = state
        .settings
        .lock()
        .expect("settings mutex")
        .search_strength;
    match strength {
        SearchStrength::Fast => RewritePolicy::Skip,
        SearchStrength::Balanced => RewritePolicy::Rewrite,
        SearchStrength::Accurate => RewritePolicy::RewriteAndHyde,
    }
}

/// v0.4.3 PR 1 (D-086) — query rewriter에 넣을 *최근 N턴* 히스토리 조회.
///
/// 호출 시점에 user 메시지(=현재 query)는 *이미* chat_messages에 INSERT 됐으므로
/// 그 row를 제외해야 한다. 가장 단순하게: created_at 기준 desc로 N턴*2 메시지를
/// 받아 "마지막 user 메시지가 현재 query면 제거" 규칙으로 정리한다. 호출 측이
/// `query`를 함께 넘기는 이유다.
fn fetch_recent_history_turns(
    state: &AppState,
    study_slug: &str,
    current_query: &str,
    turns: usize,
) -> Vec<HistoryTurn> {
    let db = state.db.lock().expect("db mutex");
    fetch_recent_history_turns_from_conn(db.conn(), study_slug, current_query, turns)
}

/// 단위 테스트가 직접 호출할 수 있는 inner — Connection만 받음.
fn fetch_recent_history_turns_from_conn(
    conn: &Connection,
    study_slug: &str,
    current_query: &str,
    turns: usize,
) -> Vec<HistoryTurn> {
    let limit = (turns.saturating_mul(2)) as i64 + 2; // 현재 query row 제외 + 약간 여유.
    let mut stmt = match conn.prepare(
        "SELECT role, content FROM chat_messages \
         WHERE study_slug = ?1 \
         ORDER BY id DESC \
         LIMIT ?2",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "v043.rewriter", error = %e, "history prepare 실패");
            return Vec::new();
        }
    };
    let rows = match stmt.query_map(params![study_slug, limit], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(target: "v043.rewriter", error = %e, "history query 실패");
            return Vec::new();
        }
    };
    let mut buf: Vec<(String, String)> = match rows.collect() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(target: "v043.rewriter", error = %e, "history collect 실패");
            return Vec::new();
        }
    };
    // 현재 query에 해당하는 가장 최근 user 메시지 1개 제거 (chat_send가 방금 INSERT한 row).
    if let Some(idx) = buf
        .iter()
        .position(|(role, content)| role == "user" && content == current_query)
    {
        buf.remove(idx);
    }
    // desc → asc 시간순.
    buf.reverse();
    // 최근 turns*2 메시지로 cap.
    let cap = turns.saturating_mul(2);
    if buf.len() > cap {
        buf = buf.split_off(buf.len() - cap);
    }
    buf.into_iter()
        .filter_map(|(role, content)| {
            let r = match role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            Some(HistoryTurn { role: r, content })
        })
        .collect()
}

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

    // v0.4.2 PR 4 (D-084 + architecture §4.11.2) — prompt prefix cache hooks.
    //   * system 프롬프트 끝(=Memory L1/L2 직후) → CacheBreakpoint::System.
    //   * v041 hybrid retrieval로 sources_block이 user message 앞에 prepended된 경우,
    //     그 user message 앞부분(sources)도 *cache prefix candidate*로 marking 하기 위해
    //     CacheBreakpoint::Message(0) 추가. 어댑터별 활용:
    //       - Anthropic: cache_control={type:"ephemeral"} 박음 (D-036, 5분 ttl).
    //       - OpenAI: 자동 prefix cache라 무시.
    //       - Gemini: cachedContents v0.3+로 이연 → 무시.
    //   * 실제 활용 최적화는 v0.4.3 (CacheBreakpoint 인자 정밀화). 본 PR은 *hook*만.
    let mut cache_breakpoints: Vec<CacheBreakpoint> = Vec::new();
    if !compressed.l1.is_empty() || !compressed.l2.is_empty() {
        cache_breakpoints.push(CacheBreakpoint::System);
    }
    if matches!(context_summary.kind.as_str(), "v041_hybrid") {
        // sources_block이 user message 0 앞에 prepended — 같은 노트북 연속 질문 시 prefix 재사용.
        cache_breakpoints.push(CacheBreakpoint::Message(0));
    }

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
/// 0) v0.4.1 chunks 적재된 책이 활성 스터디에 *있으면* hybrid retrieval 우선 (PR 4 도입)
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
    // (v0.4.1 PR 4) 책에 chunks 적재가 있으면 새 RAG 엔진 경로 — 활성 섹션·FTS5보다 우선.
    if let Some(bundle) = build_v041_block(state, study_slug, &payload.query) {
        return bundle;
    }

    if let Some((block, hit)) = build_active_section_block(state) {
        return (
            block,
            ChatContextSummary {
                kind: "active_section".to_string(),
                hits: vec![hit],
                v041_chunks: None,
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
                v041_chunks: None,
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
                v041_chunks: None,
            },
        );
    }

    (String::new(), ChatContextSummary::none())
}

/// v0.4.1 PR 4 — chunks 적재된 책이 있으면 hybrid retrieval로 source 블록을 만든다.
///
/// 동작:
///   1. 활성 스터디의 책 중 chunks 적재 ≥1건인 *첫* 책(가장 자연스럽게는 main role 우선,
///      그 다음 added_at 오름차순)을 골라 hybrid_search 진입.
///   2. retrieval 결과 → context::build_context로 시스템 프롬프트·sources_block·인용 mapping.
///   3. ChatContextSummary { kind: "v041_hybrid", hits: 호환용 + v041_chunks: Some(...) }.
///
/// 빈 결과(검색 hit 0)면 None — 호출 측이 폴백(active_section / FTS5 / current_file)으로 흐른다.
fn build_v041_block(
    state: &AppState,
    study_slug: &str,
    query: &str,
) -> Option<(String, ChatContextSummary)> {
    // 책 + chunks 적재 여부 — 한 번의 SELECT로 chunks≥1인 첫 책 row 찾기.
    let book_row: Option<(String, String)> = {
        let db = state.db.lock().expect("db mutex");
        db.conn()
            .query_row(
                "SELECT b.id, b.title FROM books b \
                 WHERE b.study_slug = ?1 \
                   AND EXISTS(SELECT 1 FROM chunks c WHERE c.book_id = b.id LIMIT 1) \
                 ORDER BY (b.role = 'main') DESC, b.added_at ASC \
                 LIMIT 1",
                params![study_slug],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .ok()
    };
    let (book_id, book_title) = book_row?;

    // v0.4.2 PR 3 — active_index.txt 기반 T1/T2 분기 (D-085·HANDOFF §1.4).
    //   * V1Me5Small (또는 파일 부재 = 디폴트) → T1 임베더로 v041 hybrid_search.
    //   * V2BgeM3 → T2 임베더로 v042 hybrid_search.
    //   * 임베더 슬롯 mismatch면 T1 폴백 (없으면 None) — 챗에서 모델 다운로드를 새로 시작
    //     하면 첫 응답이 비상식적으로 느려지므로 *이미 init된 경우만* 사용.
    let active_kind = read_active_index(&state.data_dir, &book_id).ok()?;

    let embedder_t1_opt = state
        .embedder
        .lock()
        .expect("embedder slot poisoned")
        .as_ref()
        .cloned();
    let embedder_t2_opt = state
        .embedder_t2
        .lock()
        .expect("embedder_t2 slot poisoned")
        .as_ref()
        .cloned();

    let retrieved = match (active_kind, &embedder_t1_opt, &embedder_t2_opt) {
        (IndexKind::V2BgeM3, _, Some(t2)) => {
            let db = state.db.lock().expect("db mutex");
            hybrid_search_active(
                db.conn(),
                RetrievalEmbedder::T2(t2.as_ref()),
                &state.data_dir,
                &book_id,
                query,
                10,
            )
            .ok()?
        }
        (IndexKind::V2BgeM3, Some(t1), None) => {
            // active=T2인데 T2 슬롯 미init — T1 폴백 (active_index 갱신은 본 함수 책임 X).
            tracing::warn!(
                target: "llm",
                book_id = %book_id,
                "active_index=v2_bge-m3인데 T2 임베더 미init — T1 폴백"
            );
            let db = state.db.lock().expect("db mutex");
            hybrid_search(db.conn(), t1.as_ref(), &book_id, query, 10).ok()?
        }
        (IndexKind::V1Me5Small, Some(t1), _) => {
            let db = state.db.lock().expect("db mutex");
            // 책별 active=v1이면 v041 hybrid_search 그대로 (코드 중복 회피).
            hybrid_search(db.conn(), t1.as_ref(), &book_id, query, 10).ok()?
        }
        (IndexKind::V0Bm25, _, _) => {
            // FTS-only — 본 함수의 hybrid retrieval 흐름이 아니라 v0.3.2 fts 폴백으로 흐르도록.
            return None;
        }
        _ => {
            // 임베더 슬롯이 둘 다 없거나 active=V2 + 양쪽 다 없는 케이스.
            return None;
        }
    };
    if retrieved.is_empty() {
        return None;
    }

    // v0.4.3 PR 2 (D-088) — Sentence window 확장 → Auto-merging → MMR 중복 제거 후처리.
    //   * SearchStrength::Fast → 후처리 skip (속도 우선, 원본 retrieval 그대로 패킹).
    //   * Balanced/Accurate → 후처리 ON.
    let bundle = if rewrite_policy_from_settings(state).should_postprocess() {
        let merged = run_v043_post_retrieval(state, &book_id, &retrieved, query)?;
        if merged.is_empty() {
            return None;
        }
        v041_build_context_from_merged(&merged, &book_title, V041_TOKEN_BUDGET)
    } else {
        v041_build_context(&retrieved, &book_title, V041_TOKEN_BUDGET)
    };
    if bundle.citation_index_map.is_empty() {
        return None;
    }

    // ChatContextSummary 호환용 hits(레거시 UI에 표시되도록) + v041_chunks 신규 필드.
    let hits: Vec<ChatContextHit> = bundle
        .citation_index_map
        .iter()
        .map(|e| ChatContextHit {
            book_id: Some(book_id.clone()),
            book_title: Some(book_title.clone()),
            book_role: None,
            section_label: e.section_path.clone(),
            section_path: e.section_path.clone(),
            page: e.page,
        })
        .collect();
    let v041_chunks: Vec<ChatV041ChunkRef> = bundle
        .citation_index_map
        .iter()
        .map(|e| ChatV041ChunkRef {
            marker: e.marker.clone(),
            chunk_id: e.chunk_id,
            page: e.page,
            section_path: e.section_path.clone(),
        })
        .collect();

    // 사용자 메시지 앞에 들어갈 메타 블록 — system 프롬프트(`bundle.system_prompt`)는
    // chat 시스템 프롬프트로 대체되지 않고 *별도로* SYSTEM_PROMPT 위에 얹는 형태로 결합.
    // build_chat_request가 system을 SYSTEM_PROMPT로 통일하므로, 여기서는 sources_block을
    // *user 메시지 앞에 prepend* — chat 흐름의 컨텍스트 블록 컨벤션과 일치.
    let prefix = format!(
        "다음은 등록된 책 *{book_title}*에서 사용자 질문과 관련된 자료입니다. \
         답변에는 [S1], [S2] 형식 인용 마커를 반드시 포함하세요.\n\n[SOURCES]\n{}",
        bundle.sources_block
    );

    Some((
        prefix,
        ChatContextSummary {
            kind: "v041_hybrid".to_string(),
            hits,
            v041_chunks: Some(v041_chunks),
        },
    ))
}

/// v0.4.2 PR 4 (D-084) — response_cache 키 메타. chat_send → run_stream 으로 전달.
#[derive(Debug, Clone)]
struct ResponseCacheMeta {
    key: String,
    book_id: String,
}

/// v0.4.1 hybrid retrieval 결과(=`v041_chunks`)가 있는 경우에만 cache key를 도출.
/// 그 외 흐름(active_section / fts / current_file)은 chunk_ids가 결정적이지 않아 cache 적용 X.
fn response_cache_key_meta(
    context: &ChatContextSummary,
    rewritten_query: &str,
    active_model: &str,
) -> Option<ResponseCacheMeta> {
    let chunks = context.v041_chunks.as_ref()?;
    if chunks.is_empty() {
        return None;
    }
    // book_id는 hits[0]에서 — v041 흐름은 단일 책 검색이라 모든 hit가 같은 book.
    let book_id = context
        .hits
        .iter()
        .filter_map(|h| h.book_id.as_deref())
        .next()?
        .to_string();
    let chunk_ids: Vec<i64> = chunks.iter().map(|c| c.chunk_id).collect();
    let key_str = crate::cache::response::make_response_cache_key(
        &book_id,
        rewritten_query,
        &chunk_ids,
        active_model,
    );
    Some(ResponseCacheMeta {
        key: key_str,
        book_id,
    })
}

/// v0.4.2 PR 5 (D-083) — chat 진입 시 활성 T2 인덱싱 worker 모두 cooperative pause.
///
/// invariant:
///   * 모든 활성 T2 worker (`Tier::T2BgeM3`)에 `pause(CooperativeChat)` 시도.
///   * priority::should_override가 false이면 *덮어쓰지 않음* (user/thermal/battery
///     /app_quit 사유는 보존). cooperative_chat은 자동 사유 중 가장 약함.
///   * T1 worker는 pause하지 않음 — T1은 5분 약속이라 빠르게 끝나야 한다.
///
/// 본 함수는 *동기*. chat이 cache hit 직전 분기에선 호출하지 않으므로 cache hit
/// 케이스는 인덱싱이 그대로 진행된다 (LLM 호출 X = chat resource 압박 X).
fn apply_cooperative_pause_for_chat(state: &AppState) {
    let workers: Vec<Arc<IndexingWorker>> = {
        let map = state
            .indexing_workers
            .lock()
            .expect("indexing_workers mutex");
        map.values()
            .filter(|w| w.tier == Tier::T2BgeM3)
            .cloned()
            .collect()
    };
    for w in workers {
        let current = w.pause_gate.last_reason();
        if should_override(current, PauseReason::CooperativeChat) {
            w.pause(PauseReason::CooperativeChat);
            tracing::debug!(
                target: "llm",
                job_id = w.job_id,
                "cooperative chat pause (D-083)"
            );
        }
    }
}

/// chat 종료 시 cooperative pause 해제. RAII guard에서 호출.
///
/// 자동 resume은 *cooperative_chat 사유로 들어간 worker만*. user/thermal/battery
/// /app_quit는 `can_auto_resume(_)` 결과에 따라 보호.
///
/// 호출은 *비동기 컨텍스트 외부에서도* 가능하게 sync — RAII Drop에서 호출.
fn release_cooperative_pause_for_chat(app: &AppHandle) {
    let state = app.state::<AppState>();
    let workers: Vec<Arc<IndexingWorker>> = {
        let map = state
            .indexing_workers
            .lock()
            .expect("indexing_workers mutex");
        map.values()
            .filter(|w| w.tier == Tier::T2BgeM3)
            .cloned()
            .collect()
    };
    for w in workers {
        let current = w.pause_gate.last_reason();
        // cooperative_chat 사유로 우리가 pause한 worker만 자동 resume.
        // user/thermal 등 다른 사유로 pause된 상태면 *건드리지 않음*.
        if matches!(current, Some(PauseReason::CooperativeChat)) && can_auto_resume(current) {
            w.resume();
            tracing::debug!(
                target: "llm",
                job_id = w.job_id,
                "cooperative chat auto-resume (D-083)"
            );
        }
    }
}

/// cache hit 응답을 SSE 흐름에 *그대로 흘려보낸다*. 단일 chunk(전체 텍스트)로 emit하고 즉시 done.
async fn emit_cached_response(
    app: AppHandle,
    handle: String,
    cached_text: String,
    study_slug: String,
    model: String,
    context_summary: ChatContextSummary,
) {
    // 단일 chunk로 흘림. SSE 진행 표시는 즉시 100%인 셈.
    let _ = app.emit(
        "chat:chunk",
        serde_json::json!({ "handle": &handle, "text": cached_text.clone() }),
    );
    // assistant 메시지 영속 — usage는 0(LLM 호출 X). cache_read_tokens는 실측 0이지만
    // 의미상 "캐시에서 왔다"는 메타로 쓸 수 있다 — 향후 dev panel.
    let usage = Usage::default();
    persist_assistant_message(&app, &study_slug, &cached_text, &model, &usage, &context_summary);
    // 시민 검증/citation 위반은 cache hit에도 emit (UI 일관성).
    emit_violations(&app, &handle, &study_slug, &cached_text);
    emit_citation_violations(&app, &handle, &cached_text, &context_summary);
    let _ = app.emit(
        "chat:done",
        serde_json::json!({ "handle": &handle, "usage": usage }),
    );
}

/// v041 컨텍스트 패킹의 토큰 예산 — Lost in the Middle 회피 + claude-code 입력 한도 안.
const V041_TOKEN_BUDGET: usize = 4000;

/// MMR 후 최종 top-N — context.rs가 받을 후보 수. v041 hybrid_search는 top-10을 반환하고
/// 후처리(merge → MMR)로 최종 *6건*까지 좁힌다. 토큰 예산 4000과 균형 (6 × ~600 토큰 ≈ 3,600).
const V043_MMR_TOP_N: usize = 6;

/// v0.4.3 PR 2 (D-088) — hybrid_search 결과 → sentence window 확장 → auto-merging → MMR.
///
/// 동작:
///   1. expand_sentence_window — chunks 테이블에서 prev/next text를 batched lookup.
///   2. merge_parents — 같은 parent 청크 ≥ 2개 매칭 + 토큰 합 < 800이면 부모로 치환.
///   3. mmr_dedupe — λ=0.5로 다양성 균형, top-N으로 좁힘.
///
/// embedding lookup: vectors_t1 BLOB을 retrieved + 병합 후보 chunks.id에 한해 일괄 SELECT.
/// 누락된 청크는 mmr_dedupe가 *graceful*로 score만 사용. T2(v042)는 본 PR 범위 외 — T2 활성
/// 책에서는 embeddings 누락 폴백이 자연 분기 (v0.4.4에서 T2 vectors_t2 lookup 추가 가능).
fn run_v043_post_retrieval(
    state: &AppState,
    book_id: &str,
    retrieved: &[RetrievedChunk],
    query: &str,
) -> Option<Vec<MergedChunk>> {
    if retrieved.is_empty() {
        return Some(Vec::new());
    }
    // 1) 후처리는 chunks read-only — db.lock 안에서 한 번에.
    let merged = {
        let db = state.db.lock().expect("db mutex");
        let expanded = match expand_sentence_window(db.conn(), retrieved) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "v043.post_retrieval",
                    error = %e,
                    "expand_sentence_window 실패 — 원본 retrieval 사용"
                );
                return None;
            }
        };
        match merge_parents(db.conn(), &expanded) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "v043.post_retrieval",
                    error = %e,
                    "merge_parents 실패 — sentence window 결과만 사용"
                );
                expanded
                    .into_iter()
                    .map(|e| MergedChunk {
                        id: e.core.id,
                        text: e.expanded_text,
                        score: e.core.score,
                        page: e.core.page,
                        section_path: e.core.section_path.clone(),
                        token_count: 0, // pack 단계에서 휴리스틱으로 채움. 0이면 헤더만 잡힘.
                        source_chunks: vec![e.core.id],
                    })
                    .collect()
            }
        }
    };
    if merged.is_empty() {
        return Some(Vec::new());
    }

    // 2) MMR 입력용 임베딩 lookup — v041 vectors_t1 only (v0.4.3 PR 2 범위).
    //    누락(T2 책 / embedding 미적재 race)은 mmr_dedupe가 score 폴백으로 처리.
    let candidate_ids: Vec<i64> = merged.iter().map(|c| c.id).collect();
    let embeddings = {
        let db = state.db.lock().expect("db mutex");
        fetch_embeddings_for_ids(db.conn(), &candidate_ids).unwrap_or_default()
    };

    // 3) query embedding은 v0.4.3 PR 2 범위에선 *생략* — chunk-chunk cosine 만으로 다양성 확보.
    //    relevance는 mmr_dedupe가 score 폴백을 사용해 hybrid_search 점수를 보존.
    //    PR 3(HyDE)에서 query embedding 일관 통합 가능.
    let _ = book_id; // 향후 T2 vectors_t2 lookup 시 사용. 현재는 미사용.
    let _ = query;

    let top = mmr_dedupe(&[], &merged, &embeddings, MMR_LAMBDA_DEFAULT, V043_MMR_TOP_N);
    Some(top)
}

/// vectors_t1.embedding BLOB → f32 Vec batched lookup.
fn fetch_embeddings_for_ids(
    conn: &Connection,
    ids: &[i64],
) -> AppResult<std::collections::HashMap<i64, Vec<f32>>> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut placeholders = String::with_capacity(ids.len() * 2);
    for i in 0..ids.len() {
        if i > 0 {
            placeholders.push(',');
        }
        placeholders.push('?');
    }
    let sql = format!(
        "SELECT chunk_id, embedding FROM vectors_t1 WHERE chunk_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out: std::collections::HashMap<i64, Vec<f32>> = std::collections::HashMap::new();
    for (id, bytes) in rows {
        if bytes.len() % 4 != 0 {
            continue;
        }
        let n = bytes.len() / 4;
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let off = i * 4;
            v.push(f32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
        }
        out.insert(id, v);
    }
    Ok(out)
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
    cache_meta: Option<ResponseCacheMeta>,
) {
    // 누적 텍스트를 보관 — chat:done 시 assistant 메시지로 영속.
    let mut accumulated = String::new();

    // v0.4.2 PR 5 (D-083) — chat이 *어떤 결말을 맺든* (Done / Err / panic) cooperative
    // pause 해제. 명시 호출 누락 방지를 위해 RAII guard 패턴.
    struct CooperativeResumeGuard {
        app: AppHandle,
    }
    impl Drop for CooperativeResumeGuard {
        fn drop(&mut self) {
            release_cooperative_pause_for_chat(&self.app);
        }
    }
    let _resume_guard = CooperativeResumeGuard { app: app.clone() };

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

                // v0.4.2 PR 4 (D-084) — chunk_ids 기반 cache key가 있으면 응답 영속.
                if let Some(meta) = &cache_meta {
                    if !accumulated.is_empty() {
                        let state = app.state::<AppState>();
                        let response_cache = state.response_cache.clone();
                        let db = state.db.lock().expect("db mutex");
                        if let Err(e) = response_cache.put_by_key(
                            db.conn(),
                            &meta.key,
                            &meta.book_id,
                            &model,
                            &accumulated,
                        ) {
                            tracing::warn!(
                                target: "cache",
                                error = %e,
                                "response_cache put 실패 (non-fatal)"
                            );
                        }
                    }
                }

                // F4.4 응답 검증 — Memory.Corrections active 위반 의심 검출. emit chat:violation.
                emit_violations(&app, &handle, &study_slug, &accumulated);

                // v0.4.1 PR 4 — 응답에 박힌 [Sx] 마커가 source 인덱스 범위 밖인 경우 카운트.
                // architecture §4.9.2: 환각 가드. 별도 이벤트 chat:citation_violations로 분리해
                // Memory.Corrections 위반(chat:violation)과 의미 충돌이 없게 한다.
                emit_citation_violations(&app, &handle, &accumulated, &context_summary);

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

/// v0.4.1 PR 4 — 응답의 [Sx] 마커 중 *source 범위 밖*(=환각·오타) 카운트를 emit.
///
/// `context_summary.v041_chunks`가 None이면 (=v0.3.2 흐름이거나 검색 hit 0) noop.
/// 위반이 0건이어도 *명시적으로 emit*해 UI가 "검증됨" 상태를 표시할 수 있게 한다.
fn emit_citation_violations(
    app: &AppHandle,
    handle: &str,
    response: &str,
    context_summary: &ChatContextSummary,
) {
    let Some(refs) = context_summary.v041_chunks.as_ref() else {
        return;
    };
    let parsed = parse_citations(response, refs.len());
    let total = parsed.len();
    let out_of_range = parsed.iter().filter(|p| !p.in_range).count();
    if let Err(e) = app.emit(
        "chat:citation_violations",
        serde_json::json!({
            "handle": handle,
            "total_markers": total,
            "out_of_range": out_of_range,
            "source_count": refs.len(),
        }),
    ) {
        tracing::warn!(target: "llm", error = %e, "chat:citation_violations emit failed");
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

    // v0.4.1 PR 3: ChatContextSummary 직렬화 호환성 회귀.
    #[test]
    fn context_summary_v041_chunks_optional_serializes_when_none_skips_field() {
        // v0.3.2 흐름 — kind="fts", v041_chunks=None. JSON에 "v041_chunks" 키 자체 없음.
        let s = ChatContextSummary {
            kind: "fts".to_string(),
            hits: vec![ChatContextHit {
                book_id: Some("b1".to_string()),
                book_title: Some("Book".to_string()),
                book_role: Some("main".to_string()),
                section_label: Some("§A".to_string()),
                section_path: Some("Ch01/§A".to_string()),
                page: Some(12),
            }],
            v041_chunks: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"kind\":\"fts\""));
        assert!(!json.contains("v041_chunks"), "None일 땐 키 자체가 직렬화되지 않음");
    }

    #[test]
    fn context_summary_v041_chunks_round_trip_with_some_payload() {
        let s = ChatContextSummary {
            kind: "v041_hybrid".to_string(),
            hits: Vec::new(),
            v041_chunks: Some(vec![
                ChatV041ChunkRef {
                    marker: "S1".to_string(),
                    chunk_id: 42,
                    page: Some(3),
                    section_path: Some("Ch01/§Intro".to_string()),
                },
                ChatV041ChunkRef {
                    marker: "S2".to_string(),
                    chunk_id: 99,
                    page: None,
                    section_path: None,
                },
            ]),
        };
        let json = serde_json::to_string(&s).unwrap();
        // 마커·chunk_id 키 모두 있음.
        assert!(json.contains("\"marker\":\"S1\""));
        assert!(json.contains("\"chunk_id\":42"));
        // round-trip — 재역직렬화 시 동일 구조.
        let back: ChatContextSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, s.kind);
        assert_eq!(back.v041_chunks.as_ref().unwrap().len(), 2);
        assert_eq!(back.v041_chunks.as_ref().unwrap()[0].marker, "S1");
        assert_eq!(back.v041_chunks.as_ref().unwrap()[0].chunk_id, 42);
    }

    #[test]
    fn context_summary_v041_legacy_json_without_v041_chunks_deserializes() {
        // v0.3.2가 영속한 JSON 텍스트(필드 v041_chunks 없음) — v0.4.1이 읽을 수 있어야 함.
        let legacy = r#"{"kind":"fts","hits":[]}"#;
        let parsed: ChatContextSummary = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.kind, "fts");
        assert!(parsed.v041_chunks.is_none(), "legacy JSON은 v041_chunks=None로 해석");
    }

    #[test]
    fn context_summary_none_helper_is_empty() {
        let n = ChatContextSummary::none();
        assert!(n.is_empty());
        assert!(n.v041_chunks.is_none());
    }

    // v0.4.1 PR 4 — 영속 round-trip: ChatContextSummary v041_chunks가 chat_messages 테이블에
    // JSON으로 저장된 뒤, fetch_chat_history가 v041_chunks를 그대로 복원해야 한다.
    #[test]
    fn v041_context_summary_persists_via_chat_messages_and_loads_back() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");

        let summary = ChatContextSummary {
            kind: "v041_hybrid".to_string(),
            hits: vec![ChatContextHit {
                book_id: Some("book1".to_string()),
                book_title: Some("RAG Book".to_string()),
                book_role: None,
                section_label: Some("§Intro".to_string()),
                section_path: Some("Ch01/§Intro".to_string()),
                page: Some(7),
            }],
            v041_chunks: Some(vec![ChatV041ChunkRef {
                marker: "S1".to_string(),
                chunk_id: 11,
                page: Some(7),
                section_path: Some("Ch01/§Intro".to_string()),
            }]),
        };
        let json = serde_json::to_string(&summary).expect("serialize");

        // user 메시지(컨텍스트 미첨부) + assistant 메시지(컨텍스트 첨부) 쌍으로 영속.
        insert_chat_message(db.conn(), "s1", "user", "질문", ChatMessageMeta::default(), None)
            .unwrap();
        insert_chat_message(
            db.conn(),
            "s1",
            "assistant",
            "답변 [S1]",
            ChatMessageMeta {
                model: Some("claude-opus-4-7"),
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
            },
            Some(&json),
        )
        .unwrap();

        let history = fetch_chat_history(db.conn(), "s1", 50, None).unwrap();
        assert_eq!(history.len(), 2);
        let asst = history.iter().find(|m| m.role == "assistant").unwrap();
        let ctx = asst.context.as_ref().expect("v041 context 영속됨");
        assert_eq!(ctx.kind, "v041_hybrid");
        let chunks = ctx.v041_chunks.as_ref().expect("v041_chunks 키 복원됨");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].marker, "S1");
        assert_eq!(chunks[0].chunk_id, 11);
        assert_eq!(chunks[0].page, Some(7));
        assert_eq!(chunks[0].section_path.as_deref(), Some("Ch01/§Intro"));
    }

    // -----------------------------------------------------------------------
    // v0.4.3 PR 1 (D-086) — fetch_recent_history_turns_from_conn 단위
    // -----------------------------------------------------------------------

    #[test]
    fn rewriter_history_returns_recent_turns_excluding_current_query() {
        // 2턴 대화 후 새 질문 INSERT — fetch는 이전 4메시지만 돌려주고 현재 query row는 제거.
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        // turn 1
        insert_chat_message(db.conn(), "s1", "user", "PPU란?", ChatMessageMeta::default(), None)
            .unwrap();
        insert_chat_message(
            db.conn(),
            "s1",
            "assistant",
            "Picture Processing Unit입니다.",
            ChatMessageMeta::default(),
            None,
        )
        .unwrap();
        // turn 2
        insert_chat_message(db.conn(), "s1", "user", "MMU는?", ChatMessageMeta::default(), None)
            .unwrap();
        insert_chat_message(
            db.conn(),
            "s1",
            "assistant",
            "Memory Management Unit입니다.",
            ChatMessageMeta::default(),
            None,
        )
        .unwrap();
        // 현재 query (chat_send가 방금 INSERT한 user row)
        insert_chat_message(
            db.conn(),
            "s1",
            "user",
            "이거 어떻게 구현?",
            ChatMessageMeta::default(),
            None,
        )
        .unwrap();

        let turns = fetch_recent_history_turns_from_conn(db.conn(), "s1", "이거 어떻게 구현?", 4);
        assert_eq!(turns.len(), 4, "이전 2턴 = 4 메시지만 (현재 query row 제외)");
        assert_eq!(turns[0].role, Role::User);
        assert_eq!(turns[0].content, "PPU란?");
        assert_eq!(turns[3].role, Role::Assistant);
        assert_eq!(turns[3].content, "Memory Management Unit입니다.");
    }

    #[test]
    fn rewriter_history_caps_at_window_size() {
        // 6턴 대화 → window=4 시 최근 4턴(8 msg)만.
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        for i in 0..6 {
            insert_chat_message(
                db.conn(),
                "s1",
                "user",
                &format!("U{i}"),
                ChatMessageMeta::default(),
                None,
            )
            .unwrap();
            insert_chat_message(
                db.conn(),
                "s1",
                "assistant",
                &format!("A{i}"),
                ChatMessageMeta::default(),
                None,
            )
            .unwrap();
        }
        // 현재 query — 직전 turn 6 직후 INSERT.
        insert_chat_message(
            db.conn(),
            "s1",
            "user",
            "현재질문",
            ChatMessageMeta::default(),
            None,
        )
        .unwrap();

        let turns = fetch_recent_history_turns_from_conn(db.conn(), "s1", "현재질문", 4);
        assert_eq!(turns.len(), 8);
        // 가장 오래된 = U2, 가장 최근 = A5.
        assert_eq!(turns[0].content, "U2");
        assert_eq!(turns[7].content, "A5");
    }

    #[test]
    fn rewriter_history_isolates_studies() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "a");
        seed_study(db.conn(), "b");
        insert_chat_message(db.conn(), "a", "user", "Aq", ChatMessageMeta::default(), None).unwrap();
        insert_chat_message(db.conn(), "b", "user", "Bq", ChatMessageMeta::default(), None).unwrap();
        let turns = fetch_recent_history_turns_from_conn(db.conn(), "a", "다른q", 4);
        // study a에 user "Aq" 1개 — 현재 query "다른q"와 다르므로 그대로 1턴.
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "Aq");
    }

    #[test]
    fn rewriter_history_empty_for_first_question() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        // 첫 질문 INSERT (chat_send 방금 한 행위).
        insert_chat_message(
            db.conn(),
            "s1",
            "user",
            "첫 질문",
            ChatMessageMeta::default(),
            None,
        )
        .unwrap();
        let turns = fetch_recent_history_turns_from_conn(db.conn(), "s1", "첫 질문", 4);
        assert!(turns.is_empty(), "첫 질문 = 이전 history 없음");
    }
}
