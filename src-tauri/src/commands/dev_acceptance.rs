// v0.4.2 PR 5 / v0.4.3 PR 5 — acceptance 측정 dev 명령.
//
// v0.4.2 4개 gate (HANDOFF §3) 측정 프레임:
//   * gate 1 (재개) — 인덱싱 강제 종료 시뮬 + 재시작 후 재처리 청크 수 ≤ 32 확인.
//                    실제 SIGKILL은 Tauri context에서 자기 프로세스 종료가 어려우므로
//                    `dev_simulate_abnormal_shutdown` = `status='running'` 잡 row만 두고
//                    chunks를 *미커밋 상태*로 시뮬. resume_pending_jobs 호출이 그 잡을
//                    `pending_chunks ≤ 32`로 분류하는지 검증.
//   * gate 2 (T2 핫스왑) — 명시 측정 X. 사용자 1주 dev 빌드에서 chat 응답 정상 + active_index
//                        일관 확인으로 충분. 본 명령은 *상태 점검*만:
//                        `dev_inspect_active_index_state` = manifest_t2 ready 상태 + active_index
//                        파일 내용 + chunks/vectors_t2 카운트 한 묶음.
//   * gate 3 (백그라운드 점유율) — `dev_measure_chat_response_ms` = 같은 query 5건의 wall-clock
//                              평균 응답 시간 측정. baseline(T2 빌드 X)과 T2 빌드 진행 중 비교는
//                              사용자가 두 번 실행 후 수동 비교 (50% 이내).
//   * gate 4 (Response cache 히트율) — `dev_response_cache_hit_ratio` = 누적 hit/miss로 계산.
//                                  사용자가 같은 5건 호출 후 호출하면 5/5 hit이 정상.
//
// v0.4.3 4개 gate (HANDOFF §3) 측정 프레임:
//   * gate 1 (인용 정확도 ≥ 85/100) — `dev_measure_citation_accuracy`.
//                                 chat_messages.context_json 의 citation_scores를 N건 집계.
//                                 verdict='pass' 비율 + 평균 점수 + verdict 분포.
//   * gate 2 (Follow-up 효율 ≥ 60%) — `dev_measure_followup_skip_rate`.
//                                  연속 user 질의 중 *재검색 없이* 응답된 비율.
//                                  분류: 한국어 follow-up 정규식 + chunks 변화 없음.
//   * gate 3 (prompt prefix cache hit ratio ≥ 70%) — `dev_measure_prefix_cache_ratio`.
//                                                cache_read_tokens / input_tokens 비율 평균.
//                                                Anthropic은 wire에서 직접 옴, OpenAI는 자동 prefix.
//   * gate 4 (체감 품질 ≥ 8/10) — AbComparePanel v0.4.2 vs v0.4.3 누적 stats (별도 명령 X).
//
// 본 모듈은 *프레임만*. 실제 측정값은 사용자가 1주 dev 빌드 사용 중 수기로 기록 →
// design/v0.4.2_results.md / design/v0.4.3_results.md (.gitignore라 로컬 only).

#![allow(dead_code)]

use std::path::Path;

use rusqlite::params;
use serde::Serialize;
use tauri::State;

use crate::error::AppResult;
use crate::index::v042::active_index::read_active_index;
use crate::index::v042::manifest::{
    manifest_path, read_manifest, IndexKind, ManifestStatus,
};
use crate::AppState;

// ---- gate 1: 재개 측정 ----------------------------------------------------

/// gate 1 측정 프레임 — `status='running'` 잡 + 미커밋 chunks 시뮬 결과.
#[derive(Debug, Serialize)]
pub struct AbnormalShutdownSimulationResult {
    pub job_id: i64,
    pub book_id: String,
    /// 시뮬 직후 `running` 상태로 남은 잡이 *재시작 시 비정상 종료로 분류될* 청크 수.
    /// 실제 acceptance gate 1 통과 = 32(BATCH_SIZE) 이하.
    pub pending_chunks_on_restart: i64,
}

/// gate 1 (재개) 측정 dev 명령.
///
/// 동작:
///   1. 활성 스터디의 첫 indexed 책에 임시 indexing_jobs row(status='running')를 생성.
///   2. chunks `embed_status_t2 IS NULL OR 'failed'`인 *시뮬 미커밋* 청크 수 카운트.
///   3. resume_pending_jobs(conn) 결과를 본 잡에 대해 조회 → pending_chunk_ids 길이 반환.
///
/// 본 명령은 *조회·계측*. 실제 SIGKILL 시뮬은 OS 레벨이라 본 명령으로 대체 불가.
/// 사용자는 별도로 `kill -9 <pid>` + 재시작 시 본 측정 호출.
#[tauri::command]
pub fn dev_simulate_abnormal_shutdown(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<AbnormalShutdownSimulationResult> {
    let db = state.db.lock().expect("db mutex");

    // 시뮬 잡 생성 (tier=2/T2BgeM3, status='running' — *비정상 종료* 시뮬).
    db.conn().execute(
        "INSERT INTO indexing_jobs \
            (book_id, status, tier, progress_chunks, started_at, updated_at) \
         VALUES (?1, 'running', 2, 0, \
                 CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
                 CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
        params![book_id],
    )?;
    let job_id = db.conn().last_insert_rowid();

    // pending(=NULL or 'failed') 청크 수 — resume 시 재처리 후보.
    let pending_chunks: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM chunks \
         WHERE book_id = ?1 \
           AND (embed_status_t2 IS NULL OR embed_status_t2 = 'failed')",
        params![book_id],
        |r| r.get(0),
    )?;

    Ok(AbnormalShutdownSimulationResult {
        job_id,
        book_id,
        pending_chunks_on_restart: pending_chunks,
    })
}

// ---- gate 2: 핫스왑 상태 점검 ---------------------------------------------

/// gate 2 (T2 핫스왑) 상태 점검 결과. 측정 자체는 수기 (chat 응답 정상 여부).
#[derive(Debug, Serialize)]
pub struct ActiveIndexInspection {
    pub book_id: String,
    /// "v0_bm25" | "v1_me5-small" | "v2_bge-m3" — active_index.txt 내용.
    pub active_kind: String,
    pub manifest_t1_status: Option<String>,
    pub manifest_t2_status: Option<String>,
    pub chunks_count: i64,
    pub vectors_t1_count: i64,
    pub vectors_t2_count: i64,
}

#[tauri::command]
pub fn dev_inspect_active_index_state(
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<ActiveIndexInspection> {
    let app_data_dir: &Path = &state.data_dir;
    let active_kind = read_active_index(app_data_dir, &book_id)?;
    let active_kind_label = match active_kind {
        IndexKind::V0Bm25 => "v0_bm25",
        IndexKind::V1Me5Small => "v1_me5-small",
        IndexKind::V2BgeM3 => "v2_bge-m3",
    };

    let manifest_t1_status = read_manifest(&manifest_path(
        app_data_dir,
        &book_id,
        IndexKind::V1Me5Small,
    ))
    .ok()
    .flatten()
    .map(|m| match m.status {
        ManifestStatus::Building => "building".to_string(),
        ManifestStatus::Ready => "ready".to_string(),
        ManifestStatus::Failed => "failed".to_string(),
    });

    let manifest_t2_status = read_manifest(&manifest_path(
        app_data_dir,
        &book_id,
        IndexKind::V2BgeM3,
    ))
    .ok()
    .flatten()
    .map(|m| match m.status {
        ManifestStatus::Building => "building".to_string(),
        ManifestStatus::Ready => "ready".to_string(),
        ManifestStatus::Failed => "failed".to_string(),
    });

    let db = state.db.lock().expect("db mutex");
    let chunks_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE book_id = ?1",
            params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let vectors_t1_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM vectors_t1 v JOIN chunks c ON c.id = v.chunk_id WHERE c.book_id = ?1",
            params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let vectors_t2_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM vectors_t2 v JOIN chunks c ON c.id = v.chunk_id WHERE c.book_id = ?1",
            params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    Ok(ActiveIndexInspection {
        book_id,
        active_kind: active_kind_label.to_string(),
        manifest_t1_status,
        manifest_t2_status,
        chunks_count,
        vectors_t1_count,
        vectors_t2_count,
    })
}

// ---- gate 3: chat 응답 시간 측정 -------------------------------------------

/// gate 3 (백그라운드 점유율) 측정 — *최근 N건* chat 메시지의 wall-clock 응답 시간.
///
/// 동작: chat_messages.created_at에서 user→assistant 페어 시간차를 평균. 정확한 측정은
/// 아니지만(다른 처리 시간 포함) baseline·T2-진행 중 비교에 *상대값*으로 의미 있음.
///
/// 사용자 사용:
///   1. T2 빌드 X 상태에서 같은 5건 질문 → `dev_measure_chat_response_ms` 호출.
///   2. T2 빌드 시작 → 50% 진행 중 같은 5건 → 다시 호출.
///   3. 두 평균값 비교 — 50% 이내 증가가 acceptance.
#[derive(Debug, Serialize)]
pub struct ChatResponseTimingResult {
    pub samples: usize,
    /// user→assistant 시간차 평균 (ms 추정 — datetime 파싱 단순 휴리스틱).
    pub avg_ms: f64,
}

#[tauri::command]
pub fn dev_measure_chat_response_ms(
    state: State<'_, AppState>,
    study_slug: String,
    last_n: u32,
) -> AppResult<ChatResponseTimingResult> {
    let lim = last_n.min(100) as i64;
    let db = state.db.lock().expect("db mutex");

    // user→assistant 페어 — 같은 study에서 *시간 인접 두 행* 차이.
    // SQLite datetime 결과를 strftime '%s%f'로 epoch로 변환. 페어 매칭은 한 user 직후 첫 assistant.
    let mut stmt = db.conn().prepare(
        "WITH ordered AS ( \
            SELECT id, role, created_at, \
                   CAST(strftime('%s', created_at) AS REAL) * 1000.0 AS ms, \
                   ROW_NUMBER() OVER (ORDER BY id ASC) AS rn \
              FROM chat_messages \
             WHERE study_slug = ?1 \
         ) \
         SELECT u.ms, a.ms FROM ordered u \
            JOIN ordered a ON a.rn = u.rn + 1 \
                          AND u.role = 'user' AND a.role = 'assistant' \
          ORDER BY u.id DESC LIMIT ?2",
    )?;
    let rows: Vec<(f64, f64)> = stmt
        .query_map(params![study_slug, lim], |r| {
            Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        return Ok(ChatResponseTimingResult {
            samples: 0,
            avg_ms: 0.0,
        });
    }
    let n = rows.len();
    let sum: f64 = rows.into_iter().map(|(u, a)| (a - u).max(0.0)).sum();
    Ok(ChatResponseTimingResult {
        samples: n,
        avg_ms: sum / n as f64,
    })
}

// ---- gate 4: response cache hit ratio --------------------------------------

#[derive(Debug, Serialize)]
pub struct ResponseCacheHitRatioResult {
    pub rows: i64,
    pub hit_count: u64,
    pub miss_count: u64,
    pub hit_ratio: f64,
}

#[tauri::command]
pub fn dev_response_cache_hit_ratio(
    state: State<'_, AppState>,
) -> AppResult<ResponseCacheHitRatioResult> {
    let db = state.db.lock().expect("db mutex");
    let s = state.response_cache.stats(db.conn())?;
    Ok(ResponseCacheHitRatioResult {
        rows: s.rows,
        hit_count: s.hit_count,
        miss_count: s.miss_count,
        hit_ratio: s.hit_ratio(),
    })
}

// ---- v0.4.3 gate 1: 인용 정확도 -------------------------------------------

/// gate 1 (인용 정확도) 측정 결과 — 최근 N건 chat assistant 메시지의 citation_scores 통계.
///
/// 동작:
///   1. 최근 N건 assistant chat_messages를 study_slug로 필터.
///   2. context_json의 citation_scores(=v0.4.3 PR 4 락인 D-090)를 deserialize.
///   3. 각 메시지의 verdict별 카운트 누적.
///
/// `pass` 비율 ≥ 85% 면 acceptance gate 1 PASS.
#[derive(Debug, Default, Serialize)]
pub struct CitationAccuracyResult {
    /// 검사된 assistant 메시지 수 (citation_scores=Some 인 경우만).
    pub messages: i64,
    /// 누적 마커 수 (verdict 단위, 같은 [Sx]가 메시지 내 여러 번 등장해도 verdict는 1개).
    pub markers: i64,
    /// verdict='pass' 마커 수.
    pub pass: i64,
    /// verdict='low' 마커 수.
    pub low: i64,
    /// verdict='no_match' 마커 수.
    pub no_match: i64,
    /// markers 대비 pass 비율 (0~1). markers=0 이면 0.0.
    pub pass_ratio: f64,
    /// 모든 마커 score 평균. markers=0 이면 0.0.
    pub avg_score: f64,
}

/// citation_scores 의 raw 형태 — JSON에서 직접 deserialize 가능하도록 가벼운 shape.
/// `commands::llm::ChatContextSummary` 와 *비파괴* 호환. citation_scores 키만 추출.
#[derive(serde::Deserialize)]
struct CitationScoresOnly {
    #[serde(default)]
    citation_scores: Option<Vec<RawVerdict>>,
}

#[derive(serde::Deserialize)]
struct RawVerdict {
    #[serde(default)]
    score: f32,
    #[serde(default)]
    verdict: String,
}

#[tauri::command]
pub fn dev_measure_citation_accuracy(
    state: State<'_, AppState>,
    study_slug: String,
    last_n: u32,
) -> AppResult<CitationAccuracyResult> {
    let lim = last_n.min(500) as i64;
    let db = state.db.lock().expect("db mutex");
    let mut stmt = db.conn().prepare(
        "SELECT context_json FROM chat_messages \
         WHERE study_slug = ?1 AND role = 'assistant' AND context_json IS NOT NULL \
         ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![study_slug, lim], |r| r.get::<_, String>(0))?;
    let mut result = CitationAccuracyResult::default();
    let mut score_sum: f64 = 0.0;
    for row in rows {
        let raw = row?;
        let parsed: CitationScoresOnly = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue, // forward-compat: 알 수 없는 shape는 skip.
        };
        let Some(verdicts) = parsed.citation_scores else {
            continue;
        };
        if verdicts.is_empty() {
            continue;
        }
        result.messages += 1;
        for v in verdicts {
            result.markers += 1;
            score_sum += v.score as f64;
            match v.verdict.as_str() {
                "pass" => result.pass += 1,
                "low" => result.low += 1,
                "no_match" => result.no_match += 1,
                _ => {}
            }
        }
    }
    if result.markers > 0 {
        result.pass_ratio = result.pass as f64 / result.markers as f64;
        result.avg_score = score_sum / result.markers as f64;
    }
    Ok(result)
}

// ---- v0.4.3 gate 2: follow-up 효율 ---------------------------------------

/// gate 2 (follow-up 효율) 측정 결과 — 최근 N건 user 메시지를 분류.
///
/// 동작:
///   1. 최근 N건 user 메시지를 study_slug로 fetch (시간 오름차순).
///   2. 각 user 메시지에 follow-up 패턴(한국어 정규식)이 잡히면 followups 카운트.
///   3. 그 follow-up 직전의 assistant 응답이 v041_chunks로 답했고, 해당 follow-up이
///      *재검색 없이* 답해질 가능성이 높은 신호 = follow-up 패턴 hit + 직전 chunks 존재.
///
/// 주의: 본 측정은 *분류 정확도*만 — "재검색 없이 답한" 실측 비율은 chat_send 안에서
/// follow-up 분류 결과를 영속해야 정확. v0.4.3 본 PR은 *프레임*. v0.4.4에서 정밀도 향상.
#[derive(Debug, Default, Serialize)]
pub struct FollowupSkipRateResult {
    /// 분석한 user 메시지 수.
    pub user_messages: i64,
    /// follow-up 패턴이 hit된 user 메시지 수.
    pub followups: i64,
    /// followups 중 직전 assistant 응답이 *컨텍스트 v041_hybrid* 라 재사용 가능 후보.
    pub reusable_followups: i64,
    /// reusable_followups / user_messages 비율 (0~1). user_messages=0 이면 0.0.
    pub skip_rate: f64,
}

/// 한국어 follow-up 패턴. 매칭 = 직전 응답에 대한 후속 질의 가능성 높음.
///
/// 정규식 사용 X — `regex` 크레이트 추가 부담 회피. 단순 contains로 충분.
/// case-insensitive는 NFC 정규화 후 lower로 처리.
const FOLLOWUP_HINTS: &[&str] = &[
    "그러면",
    "그럼",
    "그건 왜",
    "왜 그래",
    "왜 그런",
    "다시 설명",
    "더 자세",
    "예시",
    "예시를",
    "구체적",
    "근거",
    "출처",
    "이전에",
    "방금",
    "방금 답변",
    "그 부분",
    "그것",
    "그게 무슨",
    "어떻게 그",
    "방금 말한",
];

fn is_followup_query(text: &str) -> bool {
    let t = text.trim();
    if t.len() < 2 {
        return false;
    }
    // 짧고 대명사·생략 많은 질의 — 4자 이하 + "왜·어떻게" 포함.
    if t.chars().count() <= 6
        && (t.contains("왜")
            || t.contains("어떻게")
            || t.contains("뭐")
            || t.contains("어디"))
    {
        return true;
    }
    FOLLOWUP_HINTS.iter().any(|h| t.contains(h))
}

#[tauri::command]
pub fn dev_measure_followup_skip_rate(
    state: State<'_, AppState>,
    study_slug: String,
    last_n: u32,
) -> AppResult<FollowupSkipRateResult> {
    let lim = last_n.min(500) as i64;
    let db = state.db.lock().expect("db mutex");
    let mut stmt = db.conn().prepare(
        "SELECT id, role, content, context_json FROM chat_messages \
         WHERE study_slug = ?1 \
         ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![study_slug, lim], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    // 시간 오름차순으로 다시 정렬 (DESC로 가져온 후 reverse).
    let mut all: Vec<(i64, String, String, Option<String>)> =
        rows.collect::<rusqlite::Result<_>>()?;
    all.reverse();

    let mut result = FollowupSkipRateResult::default();
    let mut last_assistant_had_chunks = false;
    for (_, role, content, context_json) in &all {
        match role.as_str() {
            "user" => {
                result.user_messages += 1;
                if is_followup_query(content) {
                    result.followups += 1;
                    if last_assistant_had_chunks {
                        result.reusable_followups += 1;
                    }
                }
            }
            "assistant" => {
                last_assistant_had_chunks = context_json
                    .as_deref()
                    .map(|s| s.contains("\"v041_hybrid\"") || s.contains("v041_chunks"))
                    .unwrap_or(false);
            }
            _ => {}
        }
    }
    if result.user_messages > 0 {
        result.skip_rate = result.reusable_followups as f64 / result.user_messages as f64;
    }
    Ok(result)
}

// ---- v0.4.3 gate 3: prompt prefix cache hit ratio -------------------------

/// gate 3 (prompt prefix cache hit ratio) 측정 결과.
///
/// 동작:
///   1. 최근 N건 assistant chat_messages 의 cache_read_tokens / input_tokens 비율을 평균.
///   2. Anthropic 어댑터는 wire에서 cache_read_input_tokens 를 추출해 chat_messages.cache_read_tokens 에 영속(D-036).
///   3. OpenAI/Gemini는 자체 자동 prefix cache(prompt_tokens_details.cached_tokens / cachedContentTokenCount).
///   4. claude_cli 도 result subtype의 usage.cache_read_input_tokens 를 그대로 영속.
///
/// 주의: `input_tokens` 가 cache_read 를 *포함*하지 않는 어댑터 vs *포함*하는 어댑터 차이가
/// 있을 수 있음 — 본 측정은 어댑터 일관성 가정. Anthropic 표준은 input_tokens = uncached, cache_read = 별도.
#[derive(Debug, Default, Serialize)]
pub struct PrefixCacheRatioResult {
    /// 분석한 assistant 메시지 수 (input_tokens > 0 인 경우만).
    pub messages: i64,
    /// 누적 cache_read_tokens.
    pub cache_read_total: i64,
    /// 누적 input_tokens (cache_read 포함 *어댑터별 정의*).
    pub input_total: i64,
    /// cache_read_total / (cache_read_total + input_total). 어댑터별 정의 차이를 흡수하기
    /// 위해 *분모를 합으로* — 모든 입력 토큰 중 캐시 읽기 비율로 해석.
    /// total=0 이면 0.0.
    pub hit_ratio: f64,
}

#[tauri::command]
pub fn dev_measure_prefix_cache_ratio(
    state: State<'_, AppState>,
    study_slug: String,
    last_n: u32,
) -> AppResult<PrefixCacheRatioResult> {
    let lim = last_n.min(500) as i64;
    let db = state.db.lock().expect("db mutex");
    // chat_messages 컬럼명: creation_tokens(=input), cache_hit_tokens(=cache_read).
    // (v2 스키마 명명 — 본 PR에서 의미 매핑만 사용.)
    let mut stmt = db.conn().prepare(
        "SELECT creation_tokens, cache_hit_tokens FROM chat_messages \
         WHERE study_slug = ?1 AND role = 'assistant' \
         ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![study_slug, lim], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut result = PrefixCacheRatioResult::default();
    for row in rows {
        let (input, cache_read) = row?;
        if input <= 0 && cache_read <= 0 {
            continue; // 메타 누락 (mock provider 등).
        }
        result.messages += 1;
        result.cache_read_total += cache_read.max(0);
        result.input_total += input.max(0);
    }
    let denom = result.cache_read_total + result.input_total;
    if denom > 0 {
        result.hit_ratio = result.cache_read_total as f64 / denom as f64;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn seed_basic_setup(db: &Db) {
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (id, study_slug, role, title, source_path, file_format, file_size, file_hash, added_at) \
                 VALUES ('b1','s','main','B','/x','md',0,'h',datetime('now'))",
                [],
            )
            .unwrap();
        for i in 0..5 {
            db.conn()
                .execute(
                    "INSERT INTO chunks (book_id, ord, text, token_count) VALUES ('b1', ?1, ?2, 1)",
                    params![i as i64, format!("c{i}")],
                )
                .unwrap();
        }
    }

    #[test]
    fn pending_chunks_count_matches_unprocessed() {
        let db = Db::open_in_memory_for_test();
        seed_basic_setup(&db);

        // 5개 청크 모두 t2 미적재 — pending = 5.
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM chunks \
                 WHERE book_id = 'b1' \
                   AND (embed_status_t2 IS NULL OR embed_status_t2 = 'failed')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);

        // 3개를 'done'으로 바꾸면 pending = 2.
        db.conn()
            .execute(
                "UPDATE chunks SET embed_status_t2 = 'done' WHERE book_id = 'b1' AND ord < 3",
                [],
            )
            .unwrap();
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM chunks \
                 WHERE book_id = 'b1' \
                   AND (embed_status_t2 IS NULL OR embed_status_t2 = 'failed')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    // ---- v0.4.3 PR 5 acceptance 측정 unit 테스트 ----------------------------

    fn seed_assistant_with_citations(
        db: &Db,
        study_slug: &str,
        verdicts_json: &str,
    ) {
        let context_json = format!("{{\"kind\":\"v041_hybrid\",\"hits\":[],\"citation_scores\":{verdicts_json}}}");
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at, context_json) \
                 VALUES (?1, 'assistant', 'resp', datetime('now'), ?2)",
                params![study_slug, context_json],
            )
            .unwrap();
    }

    fn seed_study_only(db: &Db, slug: &str) {
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES (?1, ?1, datetime('now'))",
                params![slug],
            )
            .unwrap();
    }

    fn count_citation_metrics(study_slug: &str, db: &Db, last_n: u32) -> CitationAccuracyResult {
        // command 진입을 우회한 *순수 SQL* 동등 — State 의존 회피.
        let lim = last_n.min(500) as i64;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT context_json FROM chat_messages \
                 WHERE study_slug = ?1 AND role = 'assistant' AND context_json IS NOT NULL \
                 ORDER BY id DESC LIMIT ?2",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![study_slug, lim], |r| r.get::<_, String>(0))
            .unwrap();
        let mut result = CitationAccuracyResult::default();
        let mut score_sum: f64 = 0.0;
        for row in rows {
            let raw = row.unwrap();
            let parsed: CitationScoresOnly = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let Some(verdicts) = parsed.citation_scores else {
                continue;
            };
            if verdicts.is_empty() {
                continue;
            }
            result.messages += 1;
            for v in verdicts {
                result.markers += 1;
                score_sum += v.score as f64;
                match v.verdict.as_str() {
                    "pass" => result.pass += 1,
                    "low" => result.low += 1,
                    "no_match" => result.no_match += 1,
                    _ => {}
                }
            }
        }
        if result.markers > 0 {
            result.pass_ratio = result.pass as f64 / result.markers as f64;
            result.avg_score = score_sum / result.markers as f64;
        }
        result
    }

    #[test]
    fn citation_accuracy_aggregates_pass_low_no_match() {
        let db = Db::open_in_memory_for_test();
        seed_study_only(&db, "s");
        // 메시지 1: 마커 2개 (pass + low).
        seed_assistant_with_citations(
            &db,
            "s",
            r#"[{"source_idx":1,"score":0.7,"verdict":"pass"},{"source_idx":2,"score":0.45,"verdict":"low"}]"#,
        );
        // 메시지 2: 마커 1개 (pass).
        seed_assistant_with_citations(
            &db,
            "s",
            r#"[{"source_idx":1,"score":0.85,"verdict":"pass"}]"#,
        );
        // 메시지 3: 마커 1개 (no_match).
        seed_assistant_with_citations(
            &db,
            "s",
            r#"[{"source_idx":3,"score":0.0,"verdict":"no_match"}]"#,
        );

        let r = count_citation_metrics("s", &db, 50);
        assert_eq!(r.messages, 3);
        assert_eq!(r.markers, 4);
        assert_eq!(r.pass, 2);
        assert_eq!(r.low, 1);
        assert_eq!(r.no_match, 1);
        assert!((r.pass_ratio - 0.5).abs() < 1e-9);
        // (0.7 + 0.45 + 0.85 + 0.0) / 4 = 0.5.
        assert!((r.avg_score - 0.5).abs() < 1e-6);
    }

    #[test]
    fn citation_accuracy_skips_messages_without_citation_scores() {
        let db = Db::open_in_memory_for_test();
        seed_study_only(&db, "s");
        // citation_scores 키 없음 — 카운트 X.
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at, context_json) \
                 VALUES ('s', 'assistant', 'r', datetime('now'), '{\"kind\":\"none\",\"hits\":[]}')",
                [],
            )
            .unwrap();
        let r = count_citation_metrics("s", &db, 50);
        assert_eq!(r.messages, 0);
        assert_eq!(r.markers, 0);
        assert_eq!(r.pass_ratio, 0.0);
    }

    #[test]
    fn followup_classifier_recognizes_korean_patterns() {
        // 명시 follow-up 힌트.
        assert!(is_followup_query("그러면 다음은?"));
        assert!(is_followup_query("그건 왜 그렇지"));
        assert!(is_followup_query("다시 설명해줘"));
        assert!(is_followup_query("예시를 들어줘"));
        assert!(is_followup_query("근거가 뭐야"));
        // 짧고 대명사 많은 질의.
        assert!(is_followup_query("왜?"));
        assert!(is_followup_query("어떻게?"));
        // 일반 질의 — 매칭 X.
        assert!(!is_followup_query("Rust의 borrow checker가 뭔가요"));
        assert!(!is_followup_query("PPU 구조 설명"));
        // 너무 짧은 noise — 매칭 X.
        assert!(!is_followup_query(""));
        assert!(!is_followup_query("a"));
    }

    fn count_followup_metrics(
        study_slug: &str,
        db: &Db,
        last_n: u32,
    ) -> FollowupSkipRateResult {
        // 순수 SQL 동등 — command 우회.
        let lim = last_n.min(500) as i64;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, role, content, context_json FROM chat_messages \
                 WHERE study_slug = ?1 \
                 ORDER BY id DESC LIMIT ?2",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![study_slug, lim], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .unwrap();
        let mut all: Vec<(i64, String, String, Option<String>)> =
            rows.collect::<rusqlite::Result<_>>().unwrap();
        all.reverse();
        let mut result = FollowupSkipRateResult::default();
        let mut last_assistant_had_chunks = false;
        for (_, role, content, context_json) in &all {
            match role.as_str() {
                "user" => {
                    result.user_messages += 1;
                    if is_followup_query(content) {
                        result.followups += 1;
                        if last_assistant_had_chunks {
                            result.reusable_followups += 1;
                        }
                    }
                }
                "assistant" => {
                    last_assistant_had_chunks = context_json
                        .as_deref()
                        .map(|s| {
                            s.contains("\"v041_hybrid\"") || s.contains("v041_chunks")
                        })
                        .unwrap_or(false);
                }
                _ => {}
            }
        }
        if result.user_messages > 0 {
            result.skip_rate =
                result.reusable_followups as f64 / result.user_messages as f64;
        }
        result
    }

    #[test]
    fn followup_skip_rate_counts_reusable_after_v041_hybrid() {
        let db = Db::open_in_memory_for_test();
        seed_study_only(&db, "s");
        // user1: 일반 질의.
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at) \
                 VALUES ('s', 'user', 'PPU 구조 설명', datetime('now', '-10 seconds'))",
                [],
            )
            .unwrap();
        // assistant1: v041_hybrid 컨텍스트.
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at, context_json) \
                 VALUES ('s', 'assistant', 'a1', datetime('now', '-9 seconds'), \
                         '{\"kind\":\"v041_hybrid\",\"hits\":[]}')",
                [],
            )
            .unwrap();
        // user2: follow-up.
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at) \
                 VALUES ('s', 'user', '그러면 왜 그렇게 동작하지?', datetime('now', '-8 seconds'))",
                [],
            )
            .unwrap();
        // assistant2.
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at, context_json) \
                 VALUES ('s', 'assistant', 'a2', datetime('now', '-7 seconds'), \
                         '{\"kind\":\"v041_hybrid\",\"hits\":[]}')",
                [],
            )
            .unwrap();
        // user3: follow-up 패턴 X — 분류되지 않음.
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at) \
                 VALUES ('s', 'user', 'CPU와 GPU의 차이는 무엇인가', datetime('now', '-6 seconds'))",
                [],
            )
            .unwrap();

        let r = count_followup_metrics("s", &db, 100);
        assert_eq!(r.user_messages, 3);
        assert_eq!(r.followups, 1);
        assert_eq!(r.reusable_followups, 1);
        // 1/3.
        assert!((r.skip_rate - (1.0 / 3.0)).abs() < 1e-9);
    }

    fn count_prefix_cache_metrics(
        study_slug: &str,
        db: &Db,
        last_n: u32,
    ) -> PrefixCacheRatioResult {
        let lim = last_n.min(500) as i64;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT creation_tokens, cache_hit_tokens FROM chat_messages \
                 WHERE study_slug = ?1 AND role = 'assistant' \
                 ORDER BY id DESC LIMIT ?2",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![study_slug, lim], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap();
        let mut result = PrefixCacheRatioResult::default();
        for row in rows {
            let (input, cache_read) = row.unwrap();
            if input <= 0 && cache_read <= 0 {
                continue;
            }
            result.messages += 1;
            result.cache_read_total += cache_read.max(0);
            result.input_total += input.max(0);
        }
        let denom = result.cache_read_total + result.input_total;
        if denom > 0 {
            result.hit_ratio = result.cache_read_total as f64 / denom as f64;
        }
        result
    }

    #[test]
    fn prefix_cache_ratio_averages_input_vs_cache_read() {
        let db = Db::open_in_memory_for_test();
        seed_study_only(&db, "s");
        // 메시지1: input=400, cache_read=600.
        db.conn()
            .execute(
                "INSERT INTO chat_messages \
                    (study_slug, role, content, created_at, creation_tokens, cache_hit_tokens) \
                 VALUES ('s', 'assistant', 'a1', datetime('now', '-3 seconds'), 400, 600)",
                [],
            )
            .unwrap();
        // 메시지2: input=200, cache_read=800.
        db.conn()
            .execute(
                "INSERT INTO chat_messages \
                    (study_slug, role, content, created_at, creation_tokens, cache_hit_tokens) \
                 VALUES ('s', 'assistant', 'a2', datetime('now', '-2 seconds'), 200, 800)",
                [],
            )
            .unwrap();
        // 메시지3: 메타 누락 (0/0) — skip.
        db.conn()
            .execute(
                "INSERT INTO chat_messages \
                    (study_slug, role, content, created_at, creation_tokens, cache_hit_tokens) \
                 VALUES ('s', 'assistant', 'a3', datetime('now', '-1 seconds'), 0, 0)",
                [],
            )
            .unwrap();

        let r = count_prefix_cache_metrics("s", &db, 50);
        assert_eq!(r.messages, 2);
        assert_eq!(r.cache_read_total, 1400);
        assert_eq!(r.input_total, 600);
        // 1400 / (1400+600) = 0.7.
        assert!((r.hit_ratio - 0.7).abs() < 1e-9);
    }
}
