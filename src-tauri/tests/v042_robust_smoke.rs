//! v0.4.2 PR 1 smoke test — DB v15 + IndexingWorker 트랜잭션 체크포인트 + 재개.
//!
//! 검증 (HANDOFF §1.7):
//!   1. v15 마이그레이션이 v1~v14 위에 forward-only로 잘 얹힌다.
//!   2. v0.4.1 적재된 청크는 마이그 안에서 t1='done' 백필.
//!   3. embed_batch가 단일 트랜잭션으로 vectors_t{N}·chunks·indexing_jobs 셋을 atomic commit.
//!   4. 트랜잭션 안에서 실패하면 셋 다 롤백 (FK 위반으로 시뮬).
//!   5. 작은 책 인덱싱 → SIGKILL 시뮬(Db drop) → 재오픈 → resume_pending_jobs가 'running' 잡 회복.
//!
//! v0.4.1 smoke test 패턴(`v041_db_smoke.rs`)을 그대로 따라 마이그를 직접 적용.

use rusqlite::{params, Connection};
use std::path::Path;

use airis_lib::index::v042::resume::{resume_pending_jobs, ResumeStatusWas};
use airis_lib::index::v042::worker::{
    embed_batch, record_embed_failure, EmbedFailureOutcome, Tier, MAX_EMBED_ATTEMPTS,
};

const MIGRATIONS: &[&str] = &[
    include_str!("../src/migrations/v1_initial.sql"),
    include_str!("../src/migrations/v2_studies_and_chat.sql"),
    include_str!("../src/migrations/v3_paragraphs_fts.sql"),
    include_str!("../src/migrations/v4_intervention_and_history.sql"),
    include_str!("../src/migrations/v5_pomodoro_cycles.sql"),
    include_str!("../src/migrations/v6_srs_cards.sql"),
    include_str!("../src/migrations/v7_recall_challenges.sql"),
    include_str!("../src/migrations/v8_book_thumbnail.sql"),
    include_str!("../src/migrations/v9_study_thumbnail.sql"),
    include_str!("../src/migrations/v10_thumbnails_dir_rename.sql"),
    include_str!("../src/migrations/v11_study_description.sql"),
    include_str!("../src/migrations/v12_chat_context.sql"),
    include_str!("../src/migrations/v13_chunks.sql"),
    include_str!("../src/migrations/v14_ab_compare.sql"),
    include_str!("../src/migrations/v15_robustness.sql"),
];

fn open_with_migrations(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open file db");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("foreign_keys ON");
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (\
            version INTEGER PRIMARY KEY,\
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
         );",
    )
    .expect("schema_version bootstrap");
    let current: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (idx, sql) in MIGRATIONS.iter().enumerate() {
        let version = (idx + 1) as i64;
        if version <= current {
            continue;
        }
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("migration v{version} 실행 실패: {e}"));
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![version],
        )
        .unwrap();
    }
    conn
}

fn seed_book(conn: &Connection, book_id: &str) {
    conn.execute(
        "INSERT OR IGNORE INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO books (
            id, study_slug, role, title, source_path, file_format,
            file_size, file_hash, added_at
         ) VALUES (?1,'s','main','B','/tmp/x','md',0,'h',datetime('now'))",
        params![book_id],
    )
    .unwrap();
}

fn insert_chunks(conn: &Connection, book_id: &str, n: usize) -> Vec<i64> {
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, token_count) VALUES (?1, ?2, ?3, 1)",
            params![book_id, i as i64, format!("chunk {i}")],
        )
        .unwrap();
        ids.push(conn.last_insert_rowid());
    }
    ids
}

fn create_running_job(conn: &Connection, book_id: &str, tier: i64) -> i64 {
    conn.execute(
        "INSERT INTO indexing_jobs \
            (book_id, status, tier, progress_chunks, started_at) \
         VALUES (?1, 'running', ?2, 0, CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
        params![book_id, tier],
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn v15_migration_creates_robustness_columns_and_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v15.sqlite");
    let conn = open_with_migrations(&path);

    // 신규 컬럼 (chunks).
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(chunks)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    for col in [
        "embed_status_t1",
        "embed_status_t2",
        "embed_attempts",
        "last_error",
    ] {
        assert!(cols.iter().any(|c| c == col), "chunks.{col} 컬럼 누락");
    }

    let job_cols: Vec<String> = conn
        .prepare("PRAGMA table_info(indexing_jobs)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    for col in ["pause_reason", "updated_at"] {
        assert!(
            job_cols.iter().any(|c| c == col),
            "indexing_jobs.{col} 컬럼 누락"
        );
    }

    // 신규 테이블.
    for tbl in ["vectors_t2", "embedding_cache", "response_cache"] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![tbl],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{tbl} 테이블 누락");
    }
}

#[test]
fn v15_backfills_t1_done_for_existing_vectors_t1_rows() {
    // v0.4.1 시나리오 흉내: v14까지만 일단 적용 → chunks + vectors_t1 INSERT →
    // v15 마이그 적용 → t1='done' 백필 검증.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("backfill.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (\
            version INTEGER PRIMARY KEY,\
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
         );",
    )
    .unwrap();

    // v1~v14만 적용.
    for (idx, sql) in MIGRATIONS.iter().take(14).enumerate() {
        let version = (idx + 1) as i64;
        conn.execute_batch(sql).unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![version],
        )
        .unwrap();
    }

    // v0.4.1 적재 시뮬: 책 + 청크 2개 + vectors_t1 BLOB 1개.
    seed_book(&conn, "b-existing");
    let ids = insert_chunks(&conn, "b-existing", 2);
    conn.execute(
        "INSERT INTO vectors_t1 (chunk_id, embedding) VALUES (?1, ?2)",
        params![ids[0], vec![0u8; 4]],
    )
    .unwrap();

    // v15 적용.
    conn.execute_batch(MIGRATIONS[14]).unwrap();
    conn.execute("INSERT INTO schema_version (version) VALUES (15)", [])
        .unwrap();

    // ids[0]은 done, ids[1]은 NULL.
    let s0: Option<String> = conn
        .query_row(
            "SELECT embed_status_t1 FROM chunks WHERE id = ?1",
            params![ids[0]],
            |r| r.get(0),
        )
        .unwrap();
    let s1: Option<String> = conn
        .query_row(
            "SELECT embed_status_t1 FROM chunks WHERE id = ?1",
            params![ids[1]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(s0.as_deref(), Some("done"), "vectors_t1 적재된 청크는 백필");
    assert_eq!(s1, None, "벡터 없는 청크는 NULL 유지");
}

#[test]
fn embed_batch_is_atomic_across_vectors_status_and_progress() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("atomic.sqlite");
    let mut conn = open_with_migrations(&path);

    seed_book(&conn, "b1");
    let ids = insert_chunks(&conn, "b1", 3);
    let job_id = create_running_job(&conn, "b1", 1);

    let vecs: Vec<Vec<f32>> = (0..3).map(|i| vec![i as f32, 0.5, 1.0, 1.5]).collect();
    embed_batch(&mut conn, job_id, Tier::T1Me5Small, &ids, &vecs).unwrap();

    let n_v: i64 = conn
        .query_row("SELECT COUNT(*) FROM vectors_t1", [], |r| r.get(0))
        .unwrap();
    let n_done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE embed_status_t1 = 'done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let progress: i64 = conn
        .query_row(
            "SELECT progress_chunks FROM indexing_jobs WHERE id = ?1",
            params![job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_v, 3);
    assert_eq!(n_done, 3);
    assert_eq!(progress, 3);
}

#[test]
fn embed_batch_rolls_back_all_three_on_fk_violation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.sqlite");
    let mut conn = open_with_migrations(&path);

    seed_book(&conn, "b1");
    let ids = insert_chunks(&conn, "b1", 1);
    let job_id = create_running_job(&conn, "b1", 1);

    // 두 번째 ID는 chunks에 없는 9999 — FK 위반.
    let bad_ids = vec![ids[0], 9_999];
    let vecs = vec![vec![0.0_f32; 4], vec![1.0_f32; 4]];
    let r = embed_batch(&mut conn, job_id, Tier::T1Me5Small, &bad_ids, &vecs);
    assert!(r.is_err(), "FK 위반은 트랜잭션 실패");

    let n_v: i64 = conn
        .query_row("SELECT COUNT(*) FROM vectors_t1", [], |r| r.get(0))
        .unwrap();
    let n_done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE embed_status_t1 = 'done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let progress: i64 = conn
        .query_row(
            "SELECT progress_chunks FROM indexing_jobs WHERE id = ?1",
            params![job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_v, 0, "롤백으로 vectors_t1 미적재");
    assert_eq!(n_done, 0, "롤백으로 status 미반영");
    assert_eq!(progress, 0, "롤백으로 progress 미증가");
}

#[test]
fn record_embed_failure_marks_failed_at_max_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fail.sqlite");
    let conn = open_with_migrations(&path);
    seed_book(&conn, "b1");
    let ids = insert_chunks(&conn, "b1", 1);
    for i in 0..(MAX_EMBED_ATTEMPTS - 1) {
        let r = record_embed_failure(&conn, ids[0], Tier::T1Me5Small, "boom").unwrap();
        assert_eq!(r, EmbedFailureOutcome::WillRetry, "{i}번째 시도는 retry");
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

#[test]
fn sigkill_simulation_resume_recovers_running_job() {
    // 시나리오:
    //   1. file-backed DB 열고 마이그 적용.
    //   2. 책 1권 + 청크 5개 + 'running' 잡 생성.
    //   3. 첫 배치(2개)만 embed_batch로 commit (트랜잭션 commit = 영구).
    //   4. SIGKILL 흉내 — Connection drop만 하고 status 갱신 X. 'running' 그대로.
    //   5. DB 파일 다시 열기 → resume_pending_jobs가 그 잡을 'AbnormalRunning'으로 회복.
    //   6. 회복된 plan에 *남은 3개* 청크 ID가 들어 있는지 검증 (= gate 1: 손실 ≤ 1배치).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigkill.sqlite");

    // --- pre-crash session ---
    let job_id;
    let all_ids;
    {
        let mut conn = open_with_migrations(&path);
        seed_book(&conn, "b1");
        all_ids = insert_chunks(&conn, "b1", 5);
        job_id = create_running_job(&conn, "b1", 1);

        // 첫 배치 = 처음 2개 commit.
        let first_batch = &all_ids[..2];
        let first_vecs: Vec<Vec<f32>> = (0..2).map(|_| vec![0.0_f32; 4]).collect();
        embed_batch(&mut conn, job_id, Tier::T1Me5Small, first_batch, &first_vecs).unwrap();

        // SIGKILL 흉내 — drop만. status 'running' 남는다.
        drop(conn);
    }

    // --- post-crash session ---
    let conn2 = open_with_migrations(&path);
    let plans = resume_pending_jobs(&conn2).unwrap();
    assert_eq!(plans.len(), 1, "재개 후보 1건");
    let plan = &plans[0];
    assert_eq!(plan.job_id, job_id);
    assert_eq!(plan.tier, Tier::T1Me5Small);
    assert_eq!(
        plan.status_was,
        ResumeStatusWas::AbnormalRunning,
        "'running' 상태로 발견 = 비정상 종료"
    );
    // 첫 2개는 done 상태라 pending에서 빠지고, 나머지 3개가 남아 있어야 한다.
    assert_eq!(
        plan.pending_chunk_ids,
        all_ids[2..].to_vec(),
        "첫 배치 commit된 청크는 재처리 후보 제외"
    );
}
