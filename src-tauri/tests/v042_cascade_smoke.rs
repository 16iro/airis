//! v0.4.2 PR 2 smoke test — T2 BGE-M3 인덱서 + manifest + active_index 핫스왑.
//!
//! 검증 (HANDOFF §1.7):
//!   1. v15 마이그레이션 후 작은 청크 시퀀스를 *T1 인덱싱(시뮬)* → manifest_t1 'ready' +
//!      active_index='v1_me5-small'.
//!   2. T2 인덱싱 (mock embedder) — manifest_t2 'building' → 'ready' 전환.
//!   3. active_index.txt 아토믹 rename으로 'v2_bge-m3' 핫스왑.
//!   4. 핫스왑 *전*에는 active_index 읽기가 v1, *후*에는 v2.
//!   5. T2 vec0_t2 + chunks_fts 모두 같은 chunks 위에 적재 가능 (모델 무관 chunks).
//!
//! e2e BGE-M3 다운로드는 env `AIRIS_E2E_T2=1` 게이팅. 본 smoke는 mock embedder.

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

use airis_lib::error::AppResult;
use airis_lib::index::v042::active_index::{read_active_index, write_active_index_atomic};
use airis_lib::index::v042::embedder_t2::EmbedderT2;
use airis_lib::index::v042::indexer_t2::{build_t2_for_chunks, create_t2_job, PassageEmbedder};
use airis_lib::index::v042::manifest::{
    book_dir, ensure_tier_dir, manifest_path, read_manifest, write_manifest_atomic, IndexKind,
    Manifest, ManifestStatus,
};
use airis_lib::index::v042::vector_store_t2::ensure_vec0_t2;
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
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            AutoExtFn,
        >(sqlite_vec::sqlite3_vec_init as *const ())));
    });
}

fn open_with_migrations(path: &Path) -> Connection {
    register_sqlite_vec_once();
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
            params![book_id, i as i64, format!("chunk-{book_id}-{i}")],
        )
        .unwrap();
        ids.push(conn.last_insert_rowid());
    }
    ids
}

fn one_hot_1024(i: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; 1024];
    if i < 1024 {
        v[i] = 1.0;
    }
    v
}

/// 결정적 mock T2 임베더 — 호출 시퀀스 인덱스로 1024d one-hot.
struct MockT2 {
    seq: Mutex<usize>,
}

impl MockT2 {
    fn new() -> Self {
        Self {
            seq: Mutex::new(0),
        }
    }
}

impl PassageEmbedder for MockT2 {
    fn dim(&self) -> usize {
        EmbedderT2::DIM
    }
    fn embed_passages(&self, chunks: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let mut s = self.seq.lock().unwrap();
        let mut out = Vec::with_capacity(chunks.len());
        for _ in chunks {
            out.push(one_hot_1024(*s % 1024));
            *s += 1;
        }
        Ok(out)
    }
}

#[test]
fn t2_indexer_writes_vectors_t2_and_marks_chunks_done() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cascade.sqlite");
    let mut conn = open_with_migrations(&path);
    seed_book(&conn, "b1");
    let ids = insert_chunks(&conn, "b1", 5);
    let job_id = create_t2_job(&conn, "b1", 5).unwrap();
    let chunks: Vec<(i64, String)> = ids.iter().map(|id| (*id, format!("text-{id}"))).collect();

    ensure_vec0_t2(&conn).unwrap();
    let embedder = MockT2::new();
    let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);
    let outcome = build_t2_for_chunks(&mut conn, job_id, &chunks, &embedder, &worker).unwrap();
    assert_eq!(outcome.embeddings_inserted, 5);
    assert!(!outcome.cancelled);

    let n_v: i64 = conn
        .query_row("SELECT COUNT(*) FROM vectors_t2", [], |r| r.get(0))
        .unwrap();
    let n_done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE embed_status_t2 = 'done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_v, 5);
    assert_eq!(n_done, 5);
}

#[test]
fn manifest_round_trip_and_atomic_overwrite() {
    let app_data = tempfile::tempdir().unwrap();
    let _dir = ensure_tier_dir(app_data.path(), "b1", IndexKind::V2BgeM3).unwrap();
    let path = manifest_path(app_data.path(), "b1", IndexKind::V2BgeM3);

    // building.
    let mut m = Manifest::new_building(IndexKind::V2BgeM3, 1_000, Some(1500));
    write_manifest_atomic(&path, &m).unwrap();
    let loaded = read_manifest(&path).unwrap().unwrap();
    assert_eq!(loaded.status, ManifestStatus::Building);

    // 진행 → ready.
    m.update_progress(1500, 1500);
    m.mark_ready(2_000, 1500);
    write_manifest_atomic(&path, &m).unwrap();
    let loaded2 = read_manifest(&path).unwrap().unwrap();
    assert_eq!(loaded2.status, ManifestStatus::Ready);
    assert_eq!(loaded2.built_at, Some(2_000));
    assert_eq!(loaded2.completed_chunks, Some(1500));
}

#[test]
fn active_index_default_then_hot_swap_to_v2() {
    let app_data = tempfile::tempdir().unwrap();
    // T1 인덱싱 직후엔 파일 부재 → 디폴트 v1.
    let kind = read_active_index(app_data.path(), "b1").unwrap();
    assert_eq!(kind, IndexKind::V1Me5Small);

    // T2 빌드 완료 가정 → manifest_t2 ready 기록 후 active_index 핫스왑.
    let path = manifest_path(app_data.path(), "b1", IndexKind::V2BgeM3);
    let mut m = Manifest::new_building(IndexKind::V2BgeM3, 1_000, Some(100));
    m.mark_ready(2_000, 100);
    write_manifest_atomic(&path, &m).unwrap();
    write_active_index_atomic(app_data.path(), "b1", IndexKind::V2BgeM3).unwrap();

    let now = read_active_index(app_data.path(), "b1").unwrap();
    assert_eq!(now, IndexKind::V2BgeM3, "핫스왑 후 v2 활성");
}

#[test]
fn cascade_full_flow_simulation() {
    // 단일 책 시나리오 — T1(v0.4.1 적재 가정) → T2(mock 빌드) → 핫스왑.
    let app_data = tempfile::tempdir().unwrap();
    let db_path = app_data.path().join("cascade-full.sqlite");
    let mut conn = open_with_migrations(&db_path);
    seed_book(&conn, "b1");
    let ids = insert_chunks(&conn, "b1", 3);

    // T1 적재 시뮬: v0.4.1 흐름이라면 v041 indexer가 했을 일을 직접 흉내.
    // 본 PR은 T2만 책임이므로 T1은 chunks.embed_status_t1='done' UPDATE로 시뮬.
    for id in &ids {
        conn.execute(
            "UPDATE chunks SET embed_status_t1 = 'done' WHERE id = ?1",
            params![id],
        )
        .unwrap();
    }
    // manifest_t1.status='ready' 기록.
    let path_t1 = manifest_path(app_data.path(), "b1", IndexKind::V1Me5Small);
    let mut m_t1 = Manifest::new_building(IndexKind::V1Me5Small, 0, Some(3));
    m_t1.mark_ready(100, 3);
    write_manifest_atomic(&path_t1, &m_t1).unwrap();
    // active_index = v1 (디폴트라 안 써도 OK이지만 명시 기록).
    write_active_index_atomic(app_data.path(), "b1", IndexKind::V1Me5Small).unwrap();
    assert_eq!(
        read_active_index(app_data.path(), "b1").unwrap(),
        IndexKind::V1Me5Small
    );

    // T2 빌드 시작 — manifest_t2 building.
    let path_t2 = manifest_path(app_data.path(), "b1", IndexKind::V2BgeM3);
    let mut m_t2 = Manifest::new_building(IndexKind::V2BgeM3, 200, Some(3));
    write_manifest_atomic(&path_t2, &m_t2).unwrap();
    let job_id = create_t2_job(&conn, "b1", 3).unwrap();

    // T2 인덱싱 (mock).
    ensure_vec0_t2(&conn).unwrap();
    let embedder = MockT2::new();
    let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);
    let chunks: Vec<(i64, String)> = ids.iter().map(|id| (*id, format!("t-{id}"))).collect();
    let outcome = build_t2_for_chunks(&mut conn, job_id, &chunks, &embedder, &worker).unwrap();
    assert_eq!(outcome.embeddings_inserted, 3);

    // T2 완료 → manifest_t2 ready + active_index 핫스왑.
    m_t2.mark_ready(300, 3);
    write_manifest_atomic(&path_t2, &m_t2).unwrap();
    write_active_index_atomic(app_data.path(), "b1", IndexKind::V2BgeM3).unwrap();

    // 핫스왑 *후* — active = v2.
    assert_eq!(
        read_active_index(app_data.path(), "b1").unwrap(),
        IndexKind::V2BgeM3
    );
    // T1 manifest는 그대로 보존 (다운그레이드 시 폴백 가능).
    let m_t1_loaded = read_manifest(&path_t1).unwrap().unwrap();
    assert_eq!(m_t1_loaded.status, ManifestStatus::Ready);
    let m_t2_loaded = read_manifest(&path_t2).unwrap().unwrap();
    assert_eq!(m_t2_loaded.status, ManifestStatus::Ready);
    assert_eq!(m_t2_loaded.built_at, Some(300));

    // chunks.embed_status_t1='done' 3건, embed_status_t2='done' 3건.
    let t1_done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE embed_status_t1 = 'done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let t2_done: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE embed_status_t2 = 'done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(t1_done, 3);
    assert_eq!(t2_done, 3);

    // 폴더 layout 검증.
    let book = book_dir(app_data.path(), "b1");
    assert!(book.exists());
    assert!(book.join("indexes/v1_me5-small/manifest.json").exists());
    assert!(book.join("indexes/v2_bge-m3/manifest.json").exists());
    assert!(book.join("active_index.txt").exists());
}
