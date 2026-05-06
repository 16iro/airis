// IndexingWorker — 배치 단위 트랜잭션 체크포인트 + 일시정지/재개/취소.
//
// architecture §5 (cascade·강건성) 토대 모듈. 핵심 invariant:
//   * 한 배치(=`BATCH_SIZE` 청크) 임베딩 결과는 *단일 트랜잭션*에서 commit 한다.
//     → vectors_tN INSERT + chunks.embed_status_tN='done' + indexing_jobs.progress
//       이 셋이 *원자*. crash 시 마지막 미커밋 배치만 잃는다.
//   * 배치 commit 사이에 pause flag·cancel flag를 점검 — 운영 중 cooperative.
//   * 같은 청크가 3번 연속 실패하면 skip + last_error 기록 + 잡은 계속.
//
// 호출 패턴:
//   * PR 2 (T2 인덱서) — `Worker::new(job_id, Tier::T2BgeM3)`로 만들고 청크/벡터 batch마다
//     `embed_batch`로 commit. embedder 자체는 호출 측이 들고 있고, worker는 *DB
//     트랜잭션 책임*만 진다 (모델 로딩과 분리해 테스트가 단순).
//   * PR 3 — Tauri command가 `Worker::pause(reason)` / `Worker::resume()` /
//     `Worker::cancel()` 호출.
//
// 동기 vs async: 본 worker는 *동기* — DB와 임베딩 둘 다 blocking이라 worker 자체를
// async로 만들 동기가 약하다. 호출 측이 `tokio::task::spawn_blocking`으로 격리하는
// v0.4.1 패턴을 그대로 따른다. pause flag는 std::sync::Mutex+Condvar로 충분.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

use rusqlite::{params, Connection};

use crate::error::{AppError, AppResult};
use crate::index::v041::f32_bytes;

/// architecture §5 권장 배치 크기. 한 배치 = 한 트랜잭션 = 손실 단위 상한.
/// gate 1 (재개) 측정값은 1배치 손실 = 약 1~2초 분량.
pub const BATCH_SIZE: usize = 32;

/// 같은 청크 임베딩 재시도 한계. 초과 시 그 청크는 'failed' 마킹 + skip.
pub const MAX_EMBED_ATTEMPTS: i64 = 3;

/// cascade 단계. 한 잡이 어느 tier를 채우는 중인지 결정 — chunks/vectors 컬럼명을 분기.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// tier 1 = mE5-small (384d). v0.4.1부터 채워진 path.
    T1Me5Small,
    /// tier 2 = BGE-M3 (1024d). v0.4.2 PR 2가 본격 활용.
    T2BgeM3,
}

impl Tier {
    /// chunks.embed_status_t{1|2} 컬럼 이름.
    pub fn embed_status_column(&self) -> &'static str {
        match self {
            Self::T1Me5Small => "embed_status_t1",
            Self::T2BgeM3 => "embed_status_t2",
        }
    }

    /// vectors_t{1|2} 영속 BLOB 테이블 이름.
    pub fn vectors_table(&self) -> &'static str {
        match self {
            Self::T1Me5Small => "vectors_t1",
            Self::T2BgeM3 => "vectors_t2",
        }
    }

    /// indexing_jobs.tier 컬럼 값 (v13 chunks.sql CHECK 0/1/2).
    pub fn db_tier(&self) -> i64 {
        match self {
            Self::T1Me5Small => 1,
            Self::T2BgeM3 => 2,
        }
    }
}

/// 일시정지 사유. D-081 우선순위 정책은 PR 3가 락인.
///
/// v0.4.2 PR 5 (D-083) 추가:
///   * `CooperativeChat` — 사용자 chat 진입 시 자동으로 worker 일시정지. chat 응답
///     완료(또는 에러) 시 자동 resume. 우선순위는 자동 사유 중 *가장 낮음*
///     (battery_low보다도 낮음) — 응답 후 즉시 재개가 디폴트 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    User,
    AppQuit,
    Thermal,
    BatteryLow,
    /// v0.4.2 PR 5 — chat 응답 진행 중 인덱싱 일시정지 (gate 3 백그라운드 점유율).
    CooperativeChat,
}

impl PauseReason {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::BatteryLow => "battery_low",
            Self::Thermal => "thermal",
            Self::AppQuit => "app_quit",
            Self::CooperativeChat => "cooperative_chat",
        }
    }
}

/// 일시정지/재개를 잡는 토큰. 워커 스레드는 `wait_if_paused`에서 블록되고, UI
/// 스레드는 `pause`/`resume`을 부른다. cancel은 atomic으로 즉시 가시성 보장.
#[derive(Debug, Default)]
pub struct PauseGate {
    state: Mutex<PauseState>,
    cv: Condvar,
}

#[derive(Debug, Default)]
struct PauseState {
    paused: bool,
    last_reason: Option<PauseReason>,
}

impl PauseGate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pause(&self, reason: PauseReason) {
        let mut s = self.state.lock().expect("PauseGate mutex poisoned");
        s.paused = true;
        s.last_reason = Some(reason);
    }

    pub fn resume(&self) {
        let mut s = self.state.lock().expect("PauseGate mutex poisoned");
        s.paused = false;
        s.last_reason = None;
        self.cv.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        self.state
            .lock()
            .expect("PauseGate mutex poisoned")
            .paused
    }

    pub fn last_reason(&self) -> Option<PauseReason> {
        self.state
            .lock()
            .expect("PauseGate mutex poisoned")
            .last_reason
    }

    /// pause 상태일 동안 호출 스레드를 블록. resume 호출 시 깨어남.
    /// 본 PR 단위 테스트에서는 호출하지 않음 — PR 3에서 통합 검증.
    pub fn wait_if_paused(&self) {
        let mut s = self.state.lock().expect("PauseGate mutex poisoned");
        while s.paused {
            s = self.cv.wait(s).expect("PauseGate condvar poisoned");
        }
    }
}

/// 인덱싱 워커 핸들 — UI/외부 이벤트가 잡을 일시정지·재개·취소하는 진입점.
///
/// PR 1은 *생성·플래그 조작* + `embed_batch` 자유 함수만. 실행 루프는 PR 2/3가
/// 만든다 (T2 인덱서가 자기 청크 시퀀스를 갖고 본 worker의 플래그를 점검하며
/// embed_batch를 호출).
pub struct IndexingWorker {
    pub job_id: i64,
    pub tier: Tier,
    pub pause_gate: PauseGate,
    pub cancel: AtomicBool,
}

impl IndexingWorker {
    pub fn new(job_id: i64, tier: Tier) -> Self {
        Self {
            job_id,
            tier,
            pause_gate: PauseGate::new(),
            cancel: AtomicBool::new(false),
        }
    }

    pub fn pause(&self, reason: PauseReason) {
        self.pause_gate.pause(reason);
    }

    pub fn resume(&self) {
        self.pause_gate.resume();
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.pause_gate.is_paused()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// 배치 단위 트랜잭션 체크포인트.
///
/// 한 배치(`chunk_ids.len() == embeddings.len()`)를 *단일 트랜잭션*에서:
///   1. vectors_t{N}에 INSERT/UPDATE (chunk_id PK 기준 ON CONFLICT REPLACE).
///   2. chunks.embed_status_t{N} = 'done' UPDATE.
///   3. indexing_jobs.progress_chunks += N, updated_at = now() UPDATE.
///
/// commit하면 영구 완료. 실패 시 자동 롤백 — chunks·vectors·jobs 셋 다 미커밋
/// 상태로 일관성 유지.
///
/// 임베딩 자체는 호출 측 책임. worker는 `(chunk_id, embedding_f32)` 쌍만 받는다.
/// 모델 로딩·prefix·tokenization은 PR 2의 T2 인덱서가 담당.
///
/// 차원 검증: 호출 측이 보장. (PR 2가 BGE-M3 인스턴스에서 1024d 받는지 assert.)
///
/// `progress_delta`는 보통 `chunk_ids.len()` 그대로지만, retry로 일부만 새로
/// 'done'된 경우 호출 측이 줄여 부른다. tests/v042_robust_smoke 참조.
pub fn embed_batch(
    conn: &mut Connection,
    job_id: i64,
    tier: Tier,
    chunk_ids: &[i64],
    embeddings: &[Vec<f32>],
) -> AppResult<()> {
    if chunk_ids.len() != embeddings.len() {
        return Err(AppError::Internal {
            message: format!(
                "embed_batch length mismatch: {} ids vs {} vectors",
                chunk_ids.len(),
                embeddings.len()
            ),
        });
    }
    if chunk_ids.is_empty() {
        return Ok(());
    }

    let vectors_table = tier.vectors_table();
    let status_col = tier.embed_status_column();
    let n = chunk_ids.len() as i64;

    let tx = conn.transaction()?;

    // 1. vectors_t{N} upsert.
    {
        let insert_sql = format!(
            "INSERT INTO {vectors_table} (chunk_id, embedding) VALUES (?1, ?2) \
             ON CONFLICT(chunk_id) DO UPDATE SET embedding = excluded.embedding"
        );
        let mut stmt = tx.prepare(&insert_sql)?;
        for (chunk_id, vec) in chunk_ids.iter().zip(embeddings.iter()) {
            stmt.execute(params![chunk_id, f32_bytes(vec)])?;
        }
    }

    // 2. chunks.embed_status_t{N} = 'done'.
    {
        let update_sql = format!("UPDATE chunks SET {status_col} = 'done' WHERE id = ?1");
        let mut stmt = tx.prepare(&update_sql)?;
        for chunk_id in chunk_ids {
            stmt.execute(params![chunk_id])?;
        }
    }

    // 3. indexing_jobs.progress_chunks += N + updated_at = now().
    tx.execute(
        "UPDATE indexing_jobs SET \
            progress_chunks = progress_chunks + ?1, \
            updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
         WHERE id = ?2",
        params![n, job_id],
    )?;

    tx.commit()?;
    Ok(())
}

/// 청크 임베딩 시도가 실패했을 때: attempts++ + last_error 기록.
///
/// `MAX_EMBED_ATTEMPTS` 도달 시 `embed_status_t{N} = 'failed'`로 마킹해 worker 루프가
/// 다음 호출에서 skip할 수 있게 한다.
///
/// 트랜잭션 단위 — 본 함수 자체가 한 청크씩 처리하므로 호출 측에서 묶을 필요 없음.
/// 잡 자체는 *계속* — 일부 청크 skip이 인덱싱 전체를 멈추지 않는다.
pub fn record_embed_failure(
    conn: &Connection,
    chunk_id: i64,
    tier: Tier,
    error_message: &str,
) -> AppResult<EmbedFailureOutcome> {
    let status_col = tier.embed_status_column();

    // 재시도 횟수 +1 + last_error 갱신.
    conn.execute(
        "UPDATE chunks SET \
            embed_attempts = embed_attempts + 1, \
            last_error = ?1 \
         WHERE id = ?2",
        params![error_message, chunk_id],
    )?;

    let attempts: i64 = conn.query_row(
        "SELECT embed_attempts FROM chunks WHERE id = ?1",
        params![chunk_id],
        |r| r.get(0),
    )?;

    if attempts >= MAX_EMBED_ATTEMPTS {
        let mark_sql = format!("UPDATE chunks SET {status_col} = 'failed' WHERE id = ?1");
        conn.execute(&mark_sql, params![chunk_id])?;
        Ok(EmbedFailureOutcome::Skipped)
    } else {
        Ok(EmbedFailureOutcome::WillRetry)
    }
}

/// `record_embed_failure`의 결정 반환. 호출 측이 그 청크를 스킵할지 retry 큐에 다시
/// 넣을지 분기하는 신호.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedFailureOutcome {
    WillRetry,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 마이그레이션 v1~v15 적용된 in-memory DB. v041::indexer 테스트 패턴 그대로.
    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("FK on");
        let migrations: &[&str] = &[
            include_str!("../../migrations/v1_initial.sql"),
            include_str!("../../migrations/v2_studies_and_chat.sql"),
            include_str!("../../migrations/v3_paragraphs_fts.sql"),
            include_str!("../../migrations/v4_intervention_and_history.sql"),
            include_str!("../../migrations/v5_pomodoro_cycles.sql"),
            include_str!("../../migrations/v6_srs_cards.sql"),
            include_str!("../../migrations/v7_recall_challenges.sql"),
            include_str!("../../migrations/v8_book_thumbnail.sql"),
            include_str!("../../migrations/v9_study_thumbnail.sql"),
            include_str!("../../migrations/v10_thumbnails_dir_rename.sql"),
            include_str!("../../migrations/v11_study_description.sql"),
            include_str!("../../migrations/v12_chat_context.sql"),
            include_str!("../../migrations/v13_chunks.sql"),
            include_str!("../../migrations/v14_ab_compare.sql"),
            include_str!("../../migrations/v15_robustness.sql"),
        ];
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (\
                version INTEGER PRIMARY KEY,\
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
             );",
        )
        .unwrap();
        for sql in migrations {
            conn.execute_batch(sql).unwrap();
        }
        conn
    }

    fn seed_book_and_chunks(conn: &Connection, n: usize) -> (String, Vec<i64>) {
        conn.execute(
            "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
            [],
        )
        .unwrap();
        let book_id = "b-test".to_string();
        conn.execute(
            "INSERT INTO books (
                id, study_slug, role, title, source_path, file_format,
                file_size, file_hash, added_at
             ) VALUES (?1,'s','main','B','/x','md',0,'h',datetime('now'))",
            params![book_id],
        )
        .unwrap();
        let mut ids = Vec::with_capacity(n);
        for i in 0..n {
            conn.execute(
                "INSERT INTO chunks (book_id, ord, text, token_count) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![book_id, i as i64, format!("chunk {i}"), 1_i64],
            )
            .unwrap();
            ids.push(conn.last_insert_rowid());
        }
        (book_id, ids)
    }

    fn create_running_job(conn: &Connection, book_id: &str, tier: Tier) -> i64 {
        conn.execute(
            "INSERT INTO indexing_jobs \
                (book_id, status, tier, progress_chunks, started_at) \
             VALUES (?1, 'running', ?2, 0, CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
            params![book_id, tier.db_tier()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn dummy_vec(n: usize, seed: f32) -> Vec<f32> {
        (0..n).map(|i| seed + i as f32 * 0.1).collect()
    }

    #[test]
    fn pause_gate_round_trip_sets_state_and_reason() {
        let g = PauseGate::new();
        assert!(!g.is_paused());
        g.pause(PauseReason::User);
        assert!(g.is_paused());
        assert_eq!(g.last_reason(), Some(PauseReason::User));
        g.resume();
        assert!(!g.is_paused());
        assert_eq!(g.last_reason(), None);
    }

    #[test]
    fn worker_cancel_flag_is_atomic() {
        let w = IndexingWorker::new(1, Tier::T1Me5Small);
        assert!(!w.is_cancelled());
        w.cancel();
        assert!(w.is_cancelled());
    }

    #[test]
    fn tier_metadata_matches_schema() {
        // tier가 컬럼/테이블 이름을 잘못 매핑하면 v15 스키마 깨짐.
        assert_eq!(Tier::T1Me5Small.embed_status_column(), "embed_status_t1");
        assert_eq!(Tier::T2BgeM3.embed_status_column(), "embed_status_t2");
        assert_eq!(Tier::T1Me5Small.vectors_table(), "vectors_t1");
        assert_eq!(Tier::T2BgeM3.vectors_table(), "vectors_t2");
        assert_eq!(Tier::T1Me5Small.db_tier(), 1);
        assert_eq!(Tier::T2BgeM3.db_tier(), 2);
    }

    #[test]
    fn embed_batch_t1_commits_vectors_status_and_progress() {
        let mut conn = fresh_db();
        let (book_id, ids) = seed_book_and_chunks(&conn, 4);
        let job_id = create_running_job(&conn, &book_id, Tier::T1Me5Small);

        // 4개 청크 임베딩 (mE5-small 차원 384은 본 단위 테스트에선 임의 차원으로 충분).
        let vecs: Vec<Vec<f32>> = (0..4).map(|i| dummy_vec(8, i as f32)).collect();
        embed_batch(&mut conn, job_id, Tier::T1Me5Small, &ids, &vecs).unwrap();

        // vectors_t1 4행.
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM vectors_t1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 4);

        // chunks.embed_status_t1 = 'done' 4건.
        let done: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE embed_status_t1 = 'done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(done, 4);

        // indexing_jobs.progress_chunks = 4 + updated_at != NULL.
        let (progress, updated_at): (i64, Option<i64>) = conn
            .query_row(
                "SELECT progress_chunks, updated_at FROM indexing_jobs WHERE id = ?1",
                params![job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(progress, 4);
        assert!(updated_at.is_some(), "updated_at은 마지막 커밋 시점 epoch ms");
    }

    #[test]
    fn embed_batch_t2_writes_to_vectors_t2_and_status_t2() {
        let mut conn = fresh_db();
        let (book_id, ids) = seed_book_and_chunks(&conn, 2);
        let job_id = create_running_job(&conn, &book_id, Tier::T2BgeM3);

        let vecs: Vec<Vec<f32>> = (0..2).map(|i| dummy_vec(8, i as f32)).collect();
        embed_batch(&mut conn, job_id, Tier::T2BgeM3, &ids, &vecs).unwrap();

        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM vectors_t2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        let done: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE embed_status_t2 = 'done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(done, 2);
        // T2 적재해도 t1은 그대로 NULL — 두 단계 독립.
        let t1: Option<String> = conn
            .query_row(
                "SELECT embed_status_t1 FROM chunks WHERE id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(t1, None);
    }

    #[test]
    fn embed_batch_rolls_back_on_failure() {
        // length mismatch는 트랜잭션 시작 *전* 실패 — DB 변경 X.
        // 트랜잭션 안에서 실패하는 시나리오를 검증하려면 chunk_id 위배 (FK 없는 테이블이라
        // chunk_id가 없어도 INSERT 자체는 통과). 대신 chunks.id가 NOT NULL이라 NULL chunk_id로
        // 트랜잭션 내부에서 NOT NULL 위반 유도. 단, vectors_t{N}.chunk_id에 FK가
        // 걸려 있으므로 *존재하지 않는 chunk_id* 1건이 끼면 FK 위반이 트리거된다.
        let mut conn = fresh_db();
        let (book_id, ids) = seed_book_and_chunks(&conn, 2);
        let job_id = create_running_job(&conn, &book_id, Tier::T1Me5Small);

        // 두 번째 청크 ID를 *없는 ID*로 바꾸기 — 트랜잭션 안에서 FK 위반.
        let bad_ids = vec![ids[0], 9_999];
        let vecs: Vec<Vec<f32>> = (0..2).map(|i| dummy_vec(8, i as f32)).collect();

        let result = embed_batch(&mut conn, job_id, Tier::T1Me5Small, &bad_ids, &vecs);
        assert!(result.is_err(), "FK 위반 시 트랜잭션은 실패해야 한다");

        // 롤백 검증 — vectors_t1 0건, chunks.embed_status_t1 NULL 유지, progress 0.
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM vectors_t1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "트랜잭션 롤백으로 vectors_t1은 비어 있어야 한다");
        let done: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE embed_status_t1 = 'done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(done, 0, "트랜잭션 롤백 시 status도 미적용");
        let progress: i64 = conn
            .query_row(
                "SELECT progress_chunks FROM indexing_jobs WHERE id = ?1",
                params![job_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(progress, 0);
    }

    #[test]
    fn embed_batch_empty_input_is_noop() {
        let mut conn = fresh_db();
        let (book_id, _) = seed_book_and_chunks(&conn, 1);
        let job_id = create_running_job(&conn, &book_id, Tier::T1Me5Small);
        embed_batch(&mut conn, job_id, Tier::T1Me5Small, &[], &[]).unwrap();
        let progress: i64 = conn
            .query_row(
                "SELECT progress_chunks FROM indexing_jobs WHERE id = ?1",
                params![job_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(progress, 0);
    }

    #[test]
    fn embed_batch_length_mismatch_returns_internal_error() {
        let mut conn = fresh_db();
        let (book_id, ids) = seed_book_and_chunks(&conn, 2);
        let job_id = create_running_job(&conn, &book_id, Tier::T1Me5Small);
        let vecs = vec![dummy_vec(4, 0.0)];
        let err = embed_batch(&mut conn, job_id, Tier::T1Me5Small, &ids, &vecs).unwrap_err();
        match err {
            AppError::Internal { .. } => {}
            other => panic!("기대 Internal, 받음: {other:?}"),
        }
    }

    #[test]
    fn record_embed_failure_increments_attempts_and_writes_last_error() {
        let conn = fresh_db();
        let (_book_id, ids) = seed_book_and_chunks(&conn, 1);
        let outcome =
            record_embed_failure(&conn, ids[0], Tier::T1Me5Small, "tokenizer panic").unwrap();
        assert_eq!(outcome, EmbedFailureOutcome::WillRetry);
        let (attempts, last_error, status): (i64, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT embed_attempts, last_error, embed_status_t1 FROM chunks WHERE id = ?1",
                params![ids[0]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(last_error.as_deref(), Some("tokenizer panic"));
        assert_eq!(status, None, "1회 실패는 'failed' 마킹 X — retry 후보");
    }

    #[test]
    fn record_embed_failure_marks_failed_after_max_attempts() {
        let conn = fresh_db();
        let (_book_id, ids) = seed_book_and_chunks(&conn, 1);
        for _ in 0..(MAX_EMBED_ATTEMPTS - 1) {
            let r = record_embed_failure(&conn, ids[0], Tier::T1Me5Small, "fail").unwrap();
            assert_eq!(r, EmbedFailureOutcome::WillRetry);
        }
        let last = record_embed_failure(&conn, ids[0], Tier::T1Me5Small, "final").unwrap();
        assert_eq!(last, EmbedFailureOutcome::Skipped);
        let status: Option<String> = conn
            .query_row(
                "SELECT embed_status_t1 FROM chunks WHERE id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status.as_deref(), Some("failed"));
    }
}
