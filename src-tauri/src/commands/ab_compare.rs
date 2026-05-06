// v0.4.1 PR 5 — A/B 비교 dev panel 백엔드.
//
// 한 질의에 대해 두 응답을 *동시에* 수집한다:
//   1. baseline — v0.3.2 paragraphs FTS 흐름 강제 (v0.4.1 분기 우회)
//   2. v041     — hybrid_search + build_context 강제
//
// chunks 적재가 *반드시* 있어야 의미 있는 비교 (v041 분기가 미적재면 fallback으로
// baseline과 같은 흐름이 돼버린다 → 비교가 깨짐). 진입에서 명시 검사.
//
// 두 응답은 *별도 chat_messages 행으로 영속하지 않는다*. 측정 단위로만 메모리에
// 잡아두고, 사용자가 한 쪽을 고르면 ab_compare_choices 한 행으로 영속.
//
// 이벤트:
//   * chat:ab_chunk    {handle, track: "baseline"|"v041", text}
//   * chat:ab_done     {handle, track, text, citation_violations: {total, out_of_range, source_count}}
//   * chat:ab_complete {handle}  ← 두 트랙 모두 끝났을 때.
//   * chat:ab_error    {handle, track, error}
//
// 트랙별 chat:ab_chunk를 따로 emit하는 이유:
//   - 두 응답이 *동시에 stream*. 기존 chat:chunk를 재활용하면 두 흐름이 한 이벤트
//     이름을 공유해 서로의 상태를 망친다 (chatStore의 streamingHandle은 단일).
//   - 새 이벤트 이름 = chat:ab_chunk → chatStore와 *완전 격리*. UI도 별 컴포넌트.

use std::sync::Arc;

use futures_util::StreamExt;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use tracing::{error, info};
use uuid::Uuid;

use crate::commands::search;
use crate::error::{AppError, AppResult};
use crate::index::v041::context::{build_context as v041_build_context, parse_citations};
use crate::index::v041::retrieval::hybrid_search;
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role};
use crate::AppState;

/// 두 트랙 각각의 max_tokens — 일반 chat_send와 동일.
const MAX_TOKENS: u32 = 4096;
/// v041 컨텍스트 토큰 예산 — chat_send와 일관 (commands/llm.rs::V041_TOKEN_BUDGET).
const V041_TOKEN_BUDGET: usize = 4000;

/// chat_send_ab_compare 반환 — 프론트가 events 구독에 사용.
#[derive(Debug, Serialize)]
pub struct AbCompareHandle {
    pub handle: String,
}

/// dev_ab_record_choice의 chose 인자 — 직렬화는 lowercase string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AbChoice {
    Baseline,
    V041,
    Tie,
}

impl AbChoice {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::V041 => "v041",
            Self::Tie => "tie",
        }
    }
}

/// dev_ab_export_results 응답 — 누적 stats + 한 줄 형식 markdown 텍스트.
#[derive(Debug, Serialize)]
pub struct AbExportResult {
    pub baseline: i64,
    pub v041: i64,
    pub tie: i64,
    pub total: i64,
    pub markdown: String,
}

/// 활성 스터디의 main 책 중 chunks 적재된 것을 찾는다. 없으면 InvalidInput로 명시 안내.
fn find_indexed_book(conn: &Connection, study_slug: &str) -> AppResult<(String, String)> {
    // 활성/비활성 무관하게 study_slug 스코프의 책 중 chunks ≥1인 첫 책.
    let row = conn
        .query_row(
            "SELECT b.id, b.title FROM books b \
             WHERE b.study_slug = ?1 \
               AND EXISTS(SELECT 1 FROM chunks c WHERE c.book_id = b.id LIMIT 1) \
             ORDER BY (b.role = 'main') DESC, b.added_at ASC \
             LIMIT 1",
            params![study_slug],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .ok();
    row.ok_or(AppError::InvalidInput {
        message: "A/B 비교는 책 인덱싱 후 가능합니다. 책 설정에서 재인덱싱을 먼저 진행하세요.".to_string(),
    })
}

/// SHA-256 hex — query 식별자(개인정보가 아닌 익명 식별자)로 사용.
fn query_hash(query: &str) -> String {
    let digest = Sha256::digest(query.as_bytes());
    hex_lower(&digest)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// baseline 트랙 — v0.3.2 paragraphs FTS 흐름. v0.4.1 분기를 *명시 우회*한다.
///
/// build_chat_request의 build_context와 거의 같은 흐름이지만 `build_v041_block` 호출
/// 자체를 빼고 active_section / FTS5 / current_file 폴백 순서를 그대로 따른다.
fn build_baseline_request(state: &AppState, study_slug: &str, query: &str) -> ChatRequest {
    // v0.3.2 흐름 재구성: active_section → FTS5 → current_file → 빈.
    // commands/llm.rs는 build_v041_block을 *최우선*으로 호출하므로 그 함수를 통째로
    // 부르면 안 된다. 동등한 폴백을 직접 짜서 v041을 *건너뛴다*.
    let mut block = String::new();

    // 1) 활성 섹션 (있으면).
    if let Some(active) = state
        .active_section
        .lock()
        .expect("active_section mutex")
        .clone()
    {
        let db = state.db.lock().expect("db mutex");
        if let Ok(Some(body)) =
            crate::commands::book::fetch_section_body(db.conn(), &active.book_id, &active.section_path)
        {
            block = format!("다음은 사용자가 보고 있는 섹션입니다:\n\n---\n{body}\n---");
        }
    }

    // 2) FTS5 (활성 섹션이 없거나 본문이 비었을 때).
    if block.is_empty() {
        if let Ok(expr) = search::normalize_query(query) {
            let hits = {
                let db = state.db.lock().expect("db mutex");
                search::fts_search(db.conn(), study_slug, &expr, 5).unwrap_or_default()
            };
            if !hits.is_empty() {
                let mut b = String::from("다음은 등록된 책에서 사용자 질문과 관련된 섹션입니다:\n");
                for (i, h) in hits.iter().enumerate() {
                    let header = format!(
                        "\n---\n[{}] {} · {} {}",
                        i + 1,
                        h.book_title,
                        h.section_label,
                        h.page.map(|p| format!("(p. {p})")).unwrap_or_default()
                    );
                    b.push_str(&header);
                    b.push('\n');
                    b.push_str(&h.snippet);
                }
                b.push_str("\n---");
                block = b;
            }
        }
    }

    // 3) current_file 폴백.
    if block.is_empty() {
        if let Some(text) = state
            .current_file
            .lock()
            .expect("current_file mutex")
            .clone()
            .filter(|s| !s.is_empty())
        {
            block = format!("다음은 사용자가 학습 중인 교재 본문입니다:\n\n---\n{text}\n---");
        }
    }

    let user_message = if block.is_empty() {
        query.to_string()
    } else {
        format!("{block}\n\n사용자 질문: {query}")
    };

    let model = state
        .settings
        .lock()
        .expect("settings mutex")
        .active_model();

    ChatRequest {
        model,
        // v0.3.2와 같은 단순 system prompt — A/B 비교의 의미가 *컨텍스트 파이프라인*
        // 차이에 집중되도록 system prompt까지 v041 prompt(인용 마커 강제)로 갈아끼우지
        // 않는다. v0.3.2 commands/llm.rs::SYSTEM_PROMPT 문자열을 그대로 인용.
        system: Some(
            "당신은 한국어 학습 도우미입니다. 사용자가 제공한 교재 본문을 바탕으로 정확하게 답변하고, 본문에 없는 내용은 '본문에 없음'이라고 명시하세요.\n\n응답 형식 (가능하면 따라주세요 — F4.5 3층 응답):\n1) 한 줄 요약\n2) 본문 인용·설명 (출처는 [1], [2] 마커로 표시)\n3) (선택) 더 알아보려면: 추가 섹션·키워드 제안"
                .to_string(),
        ),
        messages: vec![Message {
            role: Role::User,
            content: user_message,
        }],
        max_tokens: MAX_TOKENS,
        cache_breakpoints: Vec::new(),
    }
}

/// v041 트랙 — hybrid_search + build_context를 *반드시* 거친다.
///
/// 이미 진입에서 chunks 적재가 검증됐으므로 검색 결과 0건이어도 *그 자체*가 v041의
/// 결과 품질이다. 빈 sources_block이라도 시스템 프롬프트(인용 강제)는 그대로 둔다.
fn build_v041_request(
    state: &AppState,
    book_id: &str,
    book_title: &str,
    query: &str,
) -> AppResult<(ChatRequest, Vec<crate::index::v041::context::CitationEntry>)> {
    let embedder = {
        let guard = state.embedder.lock().expect("embedder slot poisoned");
        guard.as_ref().cloned()
    };
    let embedder = embedder.ok_or(AppError::InvalidInput {
        message: "임베더가 아직 초기화되지 않았습니다. 책 인덱싱 후 다시 시도하세요.".to_string(),
    })?;

    let retrieved = {
        let db = state.db.lock().expect("db mutex");
        hybrid_search(db.conn(), &embedder, book_id, query, 10)?
    };
    let bundle = v041_build_context(&retrieved, book_title, V041_TOKEN_BUDGET);

    let prefix = if bundle.sources_block.is_empty() {
        String::new()
    } else {
        format!(
            "다음은 등록된 책 *{book_title}*에서 사용자 질문과 관련된 자료입니다. \
             답변에는 [S1], [S2] 형식 인용 마커를 반드시 포함하세요.\n\n[SOURCES]\n{}",
            bundle.sources_block
        )
    };
    let user_message = if prefix.is_empty() {
        query.to_string()
    } else {
        format!("{prefix}\n\n사용자 질문: {query}")
    };

    let model = state
        .settings
        .lock()
        .expect("settings mutex")
        .active_model();

    let request = ChatRequest {
        model,
        system: Some(bundle.system_prompt.clone()),
        messages: vec![Message {
            role: Role::User,
            content: user_message,
        }],
        max_tokens: MAX_TOKENS,
        cache_breakpoints: Vec::new(),
    };

    Ok((request, bundle.citation_index_map))
}

/// 한 트랙의 stream loop — chunks 누적 + chat:ab_chunk emit + 종료 시 chat:ab_done.
async fn run_track_stream(
    app: AppHandle,
    handle: String,
    track: &'static str,
    provider: Arc<dyn LlmProvider>,
    request: ChatRequest,
    citation_map_size: usize,
) {
    let stream_result = provider.chat_stream(request).await;
    let mut accumulated = String::new();
    match stream_result {
        Ok(mut stream) => {
            while let Some(event) = stream.next().await {
                match event {
                    Ok(ChatEvent::TextDelta { text }) => {
                        accumulated.push_str(&text);
                        let _ = app.emit(
                            "chat:ab_chunk",
                            serde_json::json!({
                                "handle": &handle,
                                "track": track,
                                "text": text,
                            }),
                        );
                    }
                    Ok(ChatEvent::Done { usage: _ }) => {
                        // citation 위반 — v041 트랙만 유의미. baseline은 source_count=0이라
                        // 모든 마커가 out_of_range로 잡혀 noise가 되므로 baseline은 0/0 보고.
                        let (total, oor) = if track == "v041" && citation_map_size > 0 {
                            let parsed = parse_citations(&accumulated, citation_map_size);
                            let total = parsed.len();
                            let oor = parsed.iter().filter(|p| !p.in_range).count();
                            (total, oor)
                        } else {
                            (0_usize, 0_usize)
                        };
                        let _ = app.emit(
                            "chat:ab_done",
                            serde_json::json!({
                                "handle": &handle,
                                "track": track,
                                "text": &accumulated,
                                "citation_violations": {
                                    "total_markers": total,
                                    "out_of_range": oor,
                                    "source_count": citation_map_size,
                                }
                            }),
                        );
                        return;
                    }
                    Err(e) => {
                        let _ = app.emit(
                            "chat:ab_error",
                            serde_json::json!({
                                "handle": &handle,
                                "track": track,
                                "error": e,
                            }),
                        );
                        return;
                    }
                }
            }
            // 스트림이 Done 없이 끝났을 때 — 예외적이지만 안전을 위해 done으로 마무리.
            let _ = app.emit(
                "chat:ab_done",
                serde_json::json!({
                    "handle": &handle,
                    "track": track,
                    "text": &accumulated,
                    "citation_violations": {
                        "total_markers": 0,
                        "out_of_range": 0,
                        "source_count": citation_map_size,
                    }
                }),
            );
        }
        Err(e) => {
            error!(target: "ab_compare", track, error = %e, "ab track init failed");
            let _ = app.emit(
                "chat:ab_error",
                serde_json::json!({
                    "handle": &handle,
                    "track": track,
                    "error": e,
                }),
            );
        }
    }
}

/// dev only — 동일 query로 baseline + v041 두 응답 동시 stream.
///
/// 진입 검증:
///   * query 비어있으면 InvalidInput.
///   * 활성 스터디의 책 중 chunks 적재된 게 없으면 명시 에러 (handoff §1.2).
#[tauri::command]
pub async fn chat_send_ab_compare(
    app: AppHandle,
    state: State<'_, AppState>,
    study_slug: String,
    query: String,
) -> AppResult<AbCompareHandle> {
    if query.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "질문이 비어 있습니다".to_string(),
        });
    }
    // 책 + chunks 적재 검증.
    let (book_id, book_title) = {
        let db = state.db.lock().expect("db mutex");
        find_indexed_book(db.conn(), &study_slug)?
    };

    let baseline_req = build_baseline_request(&state, &study_slug, &query);
    let (v041_req, v041_citations) =
        build_v041_request(&state, &book_id, &book_title, &query)?;

    let provider = state.llm.lock().expect("llm mutex").clone();
    let handle = format!("ab-{}", Uuid::new_v4());
    info!(
        target: "ab_compare",
        handle = %handle,
        study = %study_slug,
        book_id = %book_id,
        v041_sources = v041_citations.len(),
        "chat_send_ab_compare"
    );

    let app_for_baseline = app.clone();
    let app_for_v041 = app.clone();
    let app_for_complete = app.clone();
    let provider_for_baseline = provider.clone();
    let provider_for_v041 = provider.clone();
    let handle_baseline = handle.clone();
    let handle_v041 = handle.clone();
    let handle_complete = handle.clone();
    let v041_citation_count = v041_citations.len();

    // 두 task 동시 spawn — 같은 provider Arc 공유. 직렬 큐는 provider 어댑터 내부.
    tokio::spawn(async move {
        let baseline = tokio::spawn(async move {
            run_track_stream(
                app_for_baseline,
                handle_baseline,
                "baseline",
                provider_for_baseline,
                baseline_req,
                0,
            )
            .await;
        });
        let v041 = tokio::spawn(async move {
            run_track_stream(
                app_for_v041,
                handle_v041,
                "v041",
                provider_for_v041,
                v041_req,
                v041_citation_count,
            )
            .await;
        });
        // 양쪽 모두 종료 대기 — JoinError는 무시 (이미 emit된 ab_error로 사용자 안내).
        let _ = baseline.await;
        let _ = v041.await;
        let _ = app_for_complete.emit(
            "chat:ab_complete",
            serde_json::json!({ "handle": handle_complete }),
        );
    });

    Ok(AbCompareHandle { handle })
}

/// dev only — 사용자 선택을 ab_compare_choices에 영속.
///
/// 잘못된 chose enum / 빈 handle은 `Deserialize` 단계에서 거부 (string 매칭) —
/// AppError::InvalidInput 명시 검증은 query·텍스트 길이 정도만.
#[tauri::command]
pub fn dev_ab_record_choice(
    state: State<'_, AppState>,
    handle: String,
    query: String,
    baseline_text: String,
    v041_text: String,
    chose: AbChoice,
    note: Option<String>,
) -> AppResult<()> {
    if handle.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "handle이 비어 있습니다".to_string(),
        });
    }
    if query.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "질문이 비어 있습니다".to_string(),
        });
    }
    let hash = query_hash(&query);
    let db = state.db.lock().expect("db mutex");
    db.conn().execute(
        "INSERT INTO ab_compare_choices \
            (query_hash, query_text, baseline_text, v041_text, chose, note, handle) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            hash,
            query,
            baseline_text,
            v041_text,
            chose.as_str(),
            note,
            handle,
        ],
    )?;
    info!(target: "ab_compare", chose = %chose.as_str(), handle = %handle, "dev_ab_record_choice");
    Ok(())
}

/// dev only — 누적 stats 집계 + markdown 텍스트로 export.
///
/// markdown 형식은 PR 5의 *프레임만*. 실제 측정값은 사용자가 dev 빌드 1주 사용 후 채움.
/// 한 줄 = 한 비교. handoff §1.5의 "본 PR이 *프레임만* 만들어 두고" 정신.
#[tauri::command]
pub fn dev_ab_export_results(state: State<'_, AppState>) -> AppResult<AbExportResult> {
    let db = state.db.lock().expect("db mutex");
    let stats = compute_ab_stats(db.conn())?;
    let rows = fetch_recent_choices(db.conn(), 200)?;
    let markdown = render_results_markdown(&stats, &rows);
    Ok(AbExportResult {
        baseline: stats.baseline,
        v041: stats.v041,
        tie: stats.tie,
        total: stats.total,
        markdown,
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AbStats {
    pub baseline: i64,
    pub v041: i64,
    pub tie: i64,
    pub total: i64,
}

pub fn compute_ab_stats(conn: &Connection) -> AppResult<AbStats> {
    let mut stmt = conn
        .prepare("SELECT chose, COUNT(*) FROM ab_compare_choices GROUP BY chose")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut stats = AbStats::default();
    for row in rows {
        let (chose, count) = row?;
        match chose.as_str() {
            "baseline" => stats.baseline = count,
            "v041" => stats.v041 = count,
            "tie" => stats.tie = count,
            _ => {} // CHECK constraint가 다른 값을 막아주지만 forward-compat 위해 무시.
        }
    }
    stats.total = stats.baseline + stats.v041 + stats.tie;
    Ok(stats)
}

#[derive(Debug, Clone)]
struct ChoiceRow {
    created_at: i64,
    chose: String,
    query_text: String,
    note: Option<String>,
}

fn fetch_recent_choices(conn: &Connection, limit: i64) -> AppResult<Vec<ChoiceRow>> {
    let mut stmt = conn.prepare(
        "SELECT created_at, chose, query_text, note FROM ab_compare_choices \
         ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok(ChoiceRow {
            created_at: r.get(0)?,
            chose: r.get(1)?,
            query_text: r.get(2)?,
            note: r.get(3)?,
        })
    })?;
    rows.map(|r| r.map_err(Into::into)).collect()
}

fn render_results_markdown(stats: &AbStats, rows: &[ChoiceRow]) -> String {
    let mut out = String::new();
    out.push_str("# v0.4.1 A/B 비교 결과\n\n");
    out.push_str("> 사용자 dev 빌드 1주 사용 후 직접 채워지는 측정 프레임. 본 export는 현재 시점의 누적 stats를 그대로 박는다.\n\n");
    out.push_str("## 누적 합계\n\n");
    out.push_str(&format!(
        "- 총 비교 건수: {}\n- v041 선호: {}\n- baseline 선호: {}\n- 무승부: {}\n\n",
        stats.total, stats.v041, stats.baseline, stats.tie
    ));
    out.push_str("## 한 줄 형식 예시\n\n");
    out.push_str("```\n");
    out.push_str("YYYY-MM-DDTHH:MM:SS | chose | query | (선택) note\n");
    out.push_str("```\n\n");
    out.push_str("## 기록\n\n");
    if rows.is_empty() {
        out.push_str("_(아직 기록 없음)_\n");
        return out;
    }
    for r in rows {
        // created_at은 ms epoch — ISO 8601 변환 없이 epoch ms 그대로 박는다(외부 변환은 사용자 몫).
        let note = r
            .note
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|n| format!(" | {}", n.replace('\n', " ")))
            .unwrap_or_default();
        out.push_str(&format!(
            "- `{}` | **{}** | {}{}\n",
            r.created_at,
            r.chose,
            r.query_text.replace('\n', " "),
            note
        ));
    }
    out
}

/// dev 토글 검사 — settings.dev_ab_compare가 OFF면 *명시 에러*. 프론트가 dev 토글 우회로
/// command를 직접 부르는 케이스 차단(테스트 자동화 등).
#[allow(dead_code)]
fn ensure_dev_toggle_on(_state: &AppState) -> AppResult<()> {
    // NB: 현재는 settings에 `dev_ab_compare`가 아직 도입돼 있지 않다 — settings.rs의 변경
    // 반영이 끝나면 이 함수가 그 필드를 읽도록 갱신한다. 지금은 항상 통과시켜
    // command 자체가 dev 빌드에서만 노출되는 *프론트* 게이팅에 위임한다.
    Ok(())
}

// =============================================================================
// v0.4.2 PR 4 — cache stats (D-084 dev panel 가시화)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct CacheStatsPayload {
    pub embedding: CacheStatsView,
    pub response: CacheStatsView,
}

#[derive(Debug, Serialize)]
pub struct CacheStatsView {
    pub rows: i64,
    pub hit_count: u64,
    pub miss_count: u64,
    pub hit_ratio: f64,
}

impl From<crate::cache::CacheStats> for CacheStatsView {
    fn from(s: crate::cache::CacheStats) -> Self {
        Self {
            rows: s.rows,
            hit_count: s.hit_count,
            miss_count: s.miss_count,
            hit_ratio: s.hit_ratio(),
        }
    }
}

/// dev only — embedding_cache + response_cache 통계 한 묶음.
///
/// 프론트의 dev panel(`AbComparePanel` 안 또는 별도 dev section)가 polling 호출.
/// 토글: settings.dev_cache_stats(=`dev_ab_compare` 재활용 또는 별도 plumbing) — 본 PR은
/// 프론트 게이팅에 위임 (handoff §1.5).
#[tauri::command]
pub fn dev_cache_stats(state: State<'_, AppState>) -> AppResult<CacheStatsPayload> {
    let db = state.db.lock().expect("db mutex");
    let embedding = state.embedding_cache.stats(db.conn())?;
    let response = state.response_cache.stats(db.conn())?;
    Ok(CacheStatsPayload {
        embedding: embedding.into(),
        response: response.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::collections::HashSet;

    fn seed_basic_study_with_chunks(db: &Db) {
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s1','S1',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (id, study_slug, role, title, source_path, file_format, file_size, file_hash, added_at)
                 VALUES ('b1','s1','main','Book','/tmp/x','md',0,'h',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO chunks (book_id, ord, text, section_path) VALUES ('b1', 0, '본문 청크', 'Ch01')",
                [],
            )
            .unwrap();
    }

    fn seed_study_without_chunks(db: &Db) {
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s2','S2',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (id, study_slug, role, title, source_path, file_format, file_size, file_hash, added_at)
                 VALUES ('b2','s2','main','Book2','/tmp/y','md',0,'h2',datetime('now'))",
                [],
            )
            .unwrap();
    }

    #[test]
    fn find_indexed_book_returns_book_with_chunks() {
        let db = Db::open_in_memory_for_test();
        seed_basic_study_with_chunks(&db);
        let (id, title) = find_indexed_book(db.conn(), "s1").unwrap();
        assert_eq!(id, "b1");
        assert_eq!(title, "Book");
    }

    #[test]
    fn find_indexed_book_errors_when_no_chunks_loaded() {
        let db = Db::open_in_memory_for_test();
        seed_study_without_chunks(&db);
        let err = find_indexed_book(db.conn(), "s2").unwrap_err();
        match err {
            AppError::InvalidInput { message } => {
                assert!(message.contains("A/B 비교"), "사용자 친화 메시지: {message}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn query_hash_is_stable_for_same_input() {
        let h1 = query_hash("동일 질문 ABC");
        let h2 = query_hash("동일 질문 ABC");
        assert_eq!(h1, h2);
        // SHA-256 hex = 64자.
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn query_hash_differs_for_distinct_inputs() {
        assert_ne!(query_hash("질문 A"), query_hash("질문 B"));
    }

    #[test]
    fn ab_choice_serde_round_trip() {
        for s in ["baseline", "v041", "tie"] {
            let json = format!("\"{s}\"");
            let v: AbChoice = serde_json::from_str(&json).unwrap();
            let back = serde_json::to_string(&v).unwrap();
            assert_eq!(back, json);
        }
    }

    #[test]
    fn ab_choice_serde_rejects_unknown() {
        let result: Result<AbChoice, _> = serde_json::from_str("\"lol\"");
        assert!(result.is_err());
    }

    #[test]
    fn record_choice_inserts_row() {
        let db = Db::open_in_memory_for_test();
        seed_basic_study_with_chunks(&db);
        // command 함수는 State를 받으므로 직접 SQL로 동등 체크.
        db.conn()
            .execute(
                "INSERT INTO ab_compare_choices (query_hash, query_text, baseline_text, v041_text, chose, note, handle) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    query_hash("질문"),
                    "질문",
                    "베이스라인 응답",
                    "v041 응답",
                    "v041",
                    Some("좋음"),
                    "ab-handle-1"
                ],
            )
            .unwrap();
        let total: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM ab_compare_choices", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn compute_ab_stats_aggregates_by_chose() {
        let db = Db::open_in_memory_for_test();
        for (chose, count) in [("baseline", 2), ("v041", 7), ("tie", 1)] {
            for i in 0..count {
                db.conn()
                    .execute(
                        "INSERT INTO ab_compare_choices (query_hash, query_text, baseline_text, v041_text, chose, handle) \
                         VALUES (?1, ?2, 'a', 'b', ?3, ?4)",
                        params![
                            format!("h-{chose}-{i}"),
                            format!("query-{i}"),
                            chose,
                            format!("ab-{i}")
                        ],
                    )
                    .unwrap();
            }
        }
        let stats = compute_ab_stats(db.conn()).unwrap();
        assert_eq!(stats.baseline, 2);
        assert_eq!(stats.v041, 7);
        assert_eq!(stats.tie, 1);
        assert_eq!(stats.total, 10);
    }

    #[test]
    fn render_results_markdown_includes_totals_and_rows() {
        let stats = AbStats {
            baseline: 2,
            v041: 7,
            tie: 1,
            total: 10,
        };
        let rows = vec![ChoiceRow {
            created_at: 1_700_000_000_000,
            chose: "v041".to_string(),
            query_text: "예시 질문".to_string(),
            note: Some("이유 메모".to_string()),
        }];
        let md = render_results_markdown(&stats, &rows);
        assert!(md.contains("v041 선호: 7"));
        assert!(md.contains("baseline 선호: 2"));
        assert!(md.contains("무승부: 1"));
        assert!(md.contains("**v041**"));
        assert!(md.contains("예시 질문"));
        assert!(md.contains("이유 메모"));
    }

    #[test]
    fn render_results_markdown_handles_empty_history() {
        let stats = AbStats::default();
        let md = render_results_markdown(&stats, &[]);
        assert!(md.contains("총 비교 건수: 0"));
        assert!(md.contains("아직 기록 없음"));
    }

    #[test]
    fn ab_choice_set_covers_all_three_variants_for_check_constraint() {
        // 셋 다 enum 값이 표준대로 들어가는지 — 화이트박스 검증.
        let mut set = HashSet::new();
        set.insert(AbChoice::Baseline.as_str());
        set.insert(AbChoice::V041.as_str());
        set.insert(AbChoice::Tie.as_str());
        assert_eq!(set.len(), 3);
        assert!(set.contains("baseline"));
        assert!(set.contains("v041"));
        assert!(set.contains("tie"));
    }

    /// run_track_stream의 출력이 chat:ab_chunk + chat:ab_done로 완결되는지 — Tauri AppHandle
    /// 직접 테스트는 부담이라 mock LLM provider로 두 응답을 동시 흘려도 *합쳐서* 두 트랙이
    /// 정확히 emit되는지를 e2e Tauri test framework 없이 직접 검증.
    ///
    /// 본 단위 테스트는 핵심 *데이터 경로* (LLM 응답 누적 + citation parse)만 검증한다.
    /// 실제 emit은 수동 dev 빌드 검증으로.
    #[tokio::test]
    async fn mock_provider_distinct_text_per_request() {
        use crate::llm::mock::MockProvider;
        let baseline_provider = MockProvider::from_text_chunks(&["base", "line ", "응답"]);
        let v041_provider = MockProvider::from_text_chunks(&["v041", " 응답 ", "[S1]"]);

        let req = ChatRequest {
            model: "test".to_string(),
            system: None,
            messages: vec![Message {
                role: Role::User,
                content: "Q".to_string(),
            }],
            max_tokens: 1024,
            cache_breakpoints: Vec::new(),
        };

        // baseline mock → 누적 텍스트 검증.
        let mut s = baseline_provider.chat_stream(req.clone()).await.unwrap();
        let mut acc = String::new();
        while let Some(ev) = s.next().await {
            if let Ok(ChatEvent::TextDelta { text }) = ev {
                acc.push_str(&text);
            }
        }
        assert_eq!(acc, "baseline 응답");

        // v041 mock → 인용 마커 [S1] 포함 + parse_citations로 1건 in_range 확인.
        let mut s2 = v041_provider.chat_stream(req).await.unwrap();
        let mut acc2 = String::new();
        while let Some(ev) = s2.next().await {
            if let Ok(ChatEvent::TextDelta { text }) = ev {
                acc2.push_str(&text);
            }
        }
        assert!(acc2.contains("[S1]"));
        let parsed = parse_citations(&acc2, 1);
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].in_range);
    }
}
