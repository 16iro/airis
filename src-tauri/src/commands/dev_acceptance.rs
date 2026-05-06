// v0.4.2 PR 5 — acceptance 측정 dev 명령.
//
// 4개 gate (HANDOFF §3) 측정 프레임:
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
// 본 모듈은 *프레임만*. 실제 측정값은 사용자가 1주 dev 빌드 사용 중 수기로 기록 →
// design/v0.4.2_results.md (.gitignore라 로컬 only).

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
}
