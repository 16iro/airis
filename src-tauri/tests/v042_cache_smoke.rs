//! v0.4.2 PR 4 smoke test — Response cache + Embedding cache (D-084).
//!
//! 검증 (HANDOFF §1.7):
//!   1. v15 마이그레이션 후 embedding_cache·response_cache 테이블 존재 + cache 메서드 round-trip.
//!   2. T2 indexer가 같은 텍스트의 두 번째 호출에서 mock embedder 호출 *횟수가 줄어드는*지
//!      (=cache hit 검증).
//!   3. response_cache put → 같은 key get hit. invalidate_book → 관련 row만 삭제.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};

use airis_lib::cache::embedding::EmbeddingCache;
use airis_lib::cache::response::{make_response_cache_key, ResponseCache};
use airis_lib::error::AppResult;
use airis_lib::index::v042::embedder_t2::EmbedderT2;
use airis_lib::index::v042::indexer_t2::{
    build_t2_for_chunks_with_cache, create_t2_job, PassageEmbedder,
};
use airis_lib::index::v042::worker::{IndexingWorker, Tier};

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

fn register_sqlite_vec_once() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    type AutoExtFn = unsafe extern "C" fn(
        *mut rusqlite::ffi::sqlite3,
        *mut *mut std::os::raw::c_char,
        *const rusqlite::ffi::sqlite3_api_routines,
    ) -> std::os::raw::c_int;
    INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<*const (), AutoExtFn>(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

fn open_with_migrations(path: &Path) -> Connection {
    register_sqlite_vec_once();
    let conn = Connection::open(path).expect("open file db");
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (\
            version INTEGER PRIMARY KEY,\
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
         );",
    )
    .unwrap();
    let current: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let v = (i + 1) as i64;
        if v > current {
            conn.execute_batch(sql).unwrap();
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![v],
            )
            .unwrap();
        }
    }
    conn
}

fn seed_book_and_chunks(conn: &Connection, book_id: &str, texts: &[&str]) -> Vec<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO books (id, study_slug, role, title, source_path, file_format, \
                                       file_size, file_hash, added_at) \
         VALUES (?1,'s','main','B','/x','md',0,'h',datetime('now'))",
        params![book_id],
    )
    .unwrap();
    let mut ids = Vec::with_capacity(texts.len());
    for (i, t) in texts.iter().enumerate() {
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, token_count) VALUES (?1, ?2, ?3, 1)",
            params![book_id, i as i64, t],
        )
        .unwrap();
        ids.push(conn.last_insert_rowid());
    }
    ids
}

/// 1024d 결정적 가짜 임베딩 — 텍스트 첫 글자 코드를 인덱스로.
fn mock_vec_for(text: &str) -> Vec<f32> {
    let idx = text.chars().next().map(|c| c as usize).unwrap_or(0) % 1024;
    let mut v = vec![0.0_f32; 1024];
    v[idx] = 1.0;
    v
}

/// 호출 횟수를 카운트하는 mock — cache hit 시 얼마나 줄어드는지 검증.
struct CountingEmbedder {
    calls: Mutex<usize>,
}

impl CountingEmbedder {
    fn new() -> Self {
        Self {
            calls: Mutex::new(0),
        }
    }
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl PassageEmbedder for CountingEmbedder {
    fn dim(&self) -> usize {
        EmbedderT2::DIM
    }
    fn embed_passages(&self, chunks: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let mut g = self.calls.lock().unwrap();
        *g += chunks.len();
        Ok(chunks.iter().map(|t| mock_vec_for(t)).collect())
    }
}

#[test]
fn cache_tables_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_with_migrations(&dir.path().join("app.db"));
    // embedding_cache + response_cache 테이블 존재.
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('embedding_cache','response_cache')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);

    // round-trip.
    let ec = EmbeddingCache::new();
    ec.put(&conn, "hello", "me5-small", 4, &[1.0, 2.0, 3.0, 4.0]).unwrap();
    let got = ec.get(&conn, "hello", "me5-small").unwrap();
    assert_eq!(got, Some(vec![1.0, 2.0, 3.0, 4.0]));

    let rc = ResponseCache::new();
    rc.put(&conn, "b1", "Q?", &[10, 20], "claude-opus-4-7", "ANSWER")
        .unwrap();
    let got = rc.get(&conn, "b1", "Q?", &[10, 20], "claude-opus-4-7").unwrap();
    assert_eq!(got.as_deref(), Some("ANSWER"));
}

#[test]
fn embedding_cache_skips_fastembed_on_second_indexer_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_with_migrations(&dir.path().join("app.db"));

    let ids = seed_book_and_chunks(&conn, "b1", &["alpha", "beta", "gamma"]);
    let chunks: Vec<(i64, String)> = ids
        .iter()
        .zip(["alpha", "beta", "gamma"])
        .map(|(id, t)| (*id, t.to_string()))
        .collect();

    let cache = EmbeddingCache::new();
    let embedder = CountingEmbedder::new();

    // Run 1 — cold cache. embedder가 모든 청크 호출.
    let job_id_1 = create_t2_job(&conn, "b1", chunks.len()).unwrap();
    let worker1 = IndexingWorker::new(job_id_1, Tier::T2BgeM3);
    let r1 = build_t2_for_chunks_with_cache(
        &mut conn,
        job_id_1,
        &chunks,
        &embedder,
        &worker1,
        Some(&cache),
    )
    .unwrap();
    assert_eq!(r1.embeddings_inserted, 3);
    assert_eq!(embedder.calls(), 3, "cold run = 3 embedder calls");

    // Run 2 — 같은 청크 텍스트 (다른 chunk_id로 가정). 새 청크를 같은 텍스트로 추가.
    let ids2 = seed_book_and_chunks(&conn, "b2", &["alpha", "beta", "gamma"]);
    let chunks2: Vec<(i64, String)> = ids2
        .iter()
        .zip(["alpha", "beta", "gamma"])
        .map(|(id, t)| (*id, t.to_string()))
        .collect();

    let job_id_2 = create_t2_job(&conn, "b2", chunks2.len()).unwrap();
    let worker2 = IndexingWorker::new(job_id_2, Tier::T2BgeM3);
    let r2 = build_t2_for_chunks_with_cache(
        &mut conn,
        job_id_2,
        &chunks2,
        &embedder,
        &worker2,
        Some(&cache),
    )
    .unwrap();
    assert_eq!(r2.embeddings_inserted, 3);
    assert_eq!(
        embedder.calls(),
        3,
        "warm run = 0 추가 embedder calls (모든 텍스트 cache hit) — 누적 3 그대로"
    );

    // 통계 확인.
    let s = cache.stats(&conn).unwrap();
    assert_eq!(s.rows, 3, "embedding_cache row 3개 (alpha/beta/gamma)");
    assert!(s.hit_count >= 3, "두 번째 run에서 3건 hit");
}

#[test]
fn response_cache_invalidate_book_removes_only_target_book() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_with_migrations(&dir.path().join("app.db"));
    let cache = ResponseCache::new();

    cache.put(&conn, "b1", "q1", &[1], "m", "A1").unwrap();
    cache.put(&conn, "b1", "q2", &[2], "m", "A2").unwrap();
    cache.put(&conn, "b2", "q3", &[3], "m", "A3").unwrap();

    let removed = cache.invalidate_book(&conn, "b1").unwrap();
    assert_eq!(removed, 2);

    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM response_cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);

    // b1 lookup — miss.
    assert!(cache.get(&conn, "b1", "q1", &[1], "m").unwrap().is_none());
    // b2 lookup — hit.
    assert_eq!(
        cache.get(&conn, "b2", "q3", &[3], "m").unwrap().as_deref(),
        Some("A3")
    );
}

#[test]
fn response_cache_key_normalizes_chunk_id_order() {
    let k1 = make_response_cache_key("b", "q", &[3, 1, 2], "m");
    let k2 = make_response_cache_key("b", "q", &[1, 2, 3], "m");
    assert_eq!(k1, k2, "정렬된 chunk_ids 셋은 동일 키");
}

#[test]
fn embedding_cache_evict_lru_below_max() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_with_migrations(&dir.path().join("app.db"));
    let cache = EmbeddingCache::new();
    for i in 0..5 {
        cache
            .put(&conn, &format!("t-{i}"), "m", 2, &[i as f32, 0.0])
            .unwrap();
    }
    let removed = cache.evict_lru(&conn, 100).unwrap();
    assert_eq!(removed, 0, "임계 이하 — 삭제 0");
}

/// (선언적 wiring 검증) — invalidate_book 호출 후 *동일 키* lookup이 None.
#[test]
fn invalidate_clears_response_cache_in_memory_and_disk() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_with_migrations(&dir.path().join("app.db"));
    let cache = Arc::new(ResponseCache::new());

    cache.put(&conn, "bX", "q", &[5, 6], "m", "OLD").unwrap();
    // 핫셋 hit 한 번 (put 시 등재됨).
    assert_eq!(
        cache.get(&conn, "bX", "q", &[5, 6], "m").unwrap().as_deref(),
        Some("OLD")
    );

    let _ = cache.invalidate_book(&conn, "bX").unwrap();
    // 핫셋 clear 됐으니 SQLite 조회 — 없음.
    assert!(cache.get(&conn, "bX", "q", &[5, 6], "m").unwrap().is_none());
}
