//! v0.4.1 PR 1 smoke test — DB v13 + sqlite-vec auto_extension.
//!
//! 검증:
//!   1. 마이그레이션 v1~v13 일괄 적용 (db.rs MIGRATIONS 배열과 같은 순서·내용).
//!   2. v13 신설 객체 4종 (chunks / chunks_fts / vectors_t1 / indexing_jobs) 존재.
//!   3. chunks INSERT → 트리거가 chunks_fts 동기화 (MATCH 검색 가능).
//!   4. sqlite-vec auto_extension 동작 — vec_version() 호출 + vec0 가상 테이블 생성.
//!
//! 통합 테스트라 `airis_lib::db` 사적 모듈을 거치지 않고, 같은 SQL 파일을
//! `include_str!`로 직접 읽어 적용한다. 마이그레이션 파일 자체의 forward-only 정상성을
//! cross-check하는 효과.

use rusqlite::{params, Connection};

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
];

fn register_sqlite_vec() {
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

fn fresh_conn() -> Connection {
    register_sqlite_vec();
    let conn = Connection::open_in_memory().expect("open in-memory");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("foreign_keys ON");

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (\
            version INTEGER PRIMARY KEY,\
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
         );",
    )
    .expect("schema_version bootstrap");

    for (idx, sql) in MIGRATIONS.iter().enumerate() {
        let version = (idx + 1) as i64;
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("migration v{version} 실행 실패: {e}"));
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![version],
        )
        .expect("schema_version insert");
    }
    conn
}

fn count_object(conn: &Connection, name: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
        params![name],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn v13_new_objects_exist() {
    let conn = fresh_conn();
    // 4종 모두 sqlite_master에 등록 (FTS5 vtable·일반 테이블 모두 잡힘).
    assert!(count_object(&conn, "chunks") >= 1, "chunks 테이블 누락");
    assert!(count_object(&conn, "chunks_fts") >= 1, "chunks_fts 누락");
    assert!(count_object(&conn, "vectors_t1") >= 1, "vectors_t1 누락");
    assert!(count_object(&conn, "indexing_jobs") >= 1, "indexing_jobs 누락");
}

#[test]
fn chunks_insert_propagates_to_fts() {
    let conn = fresh_conn();
    // FK 만족 — study + book 미리 만든다 (v0.3.2 paragraphs 테스트와 같은 패턴).
    conn.execute(
        "INSERT INTO studies (slug, name, created_at) VALUES ('s1','S1',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO books (id, study_slug, role, title, source_path, file_format,\
                            file_size, file_hash, added_at)\
         VALUES ('b1','s1','main','Book','/tmp/x','md',0,'h',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, section_path)\
         VALUES ('b1', 0, 'Rust ownership and borrowing', 'Ch01')",
        [],
    )
    .unwrap();

    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'ownership'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1, "FTS 트리거가 chunks INSERT를 chunks_fts에 반영해야 한다");

    // DELETE 트리거도 점검 — books CASCADE로 chunks가 지워질 때 FTS도 클린.
    conn.execute("DELETE FROM books WHERE id='b1'", []).unwrap();
    let after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'ownership'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, 0, "books 삭제 → chunks CASCADE → FTS DELETE 트리거 동작");
}

#[test]
fn sqlite_vec_extension_loaded() {
    let conn = fresh_conn();
    let version: String = conn
        .query_row("SELECT vec_version()", [], |r| r.get(0))
        .expect("vec_version()는 auto_extension 등록 직후 호출 가능해야 한다");
    // sqlite-vec 0.1.9 → "v0.1.9"
    assert!(
        version.starts_with("v0.1."),
        "vec_version() 결과가 예상 prefix와 다름: {version}"
    );
}

#[test]
fn vec0_virtual_table_creatable_with_384_dim() {
    let conn = fresh_conn();
    // mE5-small 384d — vec0 가상 테이블이 정상 생성되고 INSERT/KNN까지 동작하는지.
    conn.execute(
        "CREATE VIRTUAL TABLE smoke_vec USING vec0(embedding FLOAT[384])",
        [],
    )
    .expect("vec0 가상 테이블 생성 가능해야 한다");

    // 차원 strict 검증 — 4 byte * 384 = 1536 byte 임베딩만 받는다.
    let v: Vec<f32> = (0..384).map(|i| i as f32 * 0.001).collect();
    let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
    conn.execute(
        "INSERT INTO smoke_vec(rowid, embedding) VALUES (1, ?1)",
        params![bytes],
    )
    .expect("384d 임베딩 INSERT 가능");

    let hit: i64 = conn
        .query_row(
            "SELECT rowid FROM smoke_vec WHERE embedding MATCH ?1 ORDER BY distance LIMIT 1",
            params![bytes],
            |r| r.get(0),
        )
        .expect("KNN 쿼리 결과");
    assert_eq!(hit, 1, "동일 벡터 검색 시 자기 자신이 top-1");
}
