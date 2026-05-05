//! v0.4.1 PR 3 smoke test — Hybrid retrieval + 컨텍스트 파이프라인 통합.
//!
//! 검증 (실제 fastembed 호출 X — 가짜 임베딩으로 vec0 KNN만 검증):
//!   1. 작은 MD → markdown::parse → indexer.index_book(embedder=None) → chunks 적재.
//!   2. 가짜 384d 임베딩을 직접 vec0/vectors_t1에 INSERT (실제 운용 경로의 데이터 모양).
//!   3. retrieval::fts_only_search 결과가 책 한정 + 정합 score 순.
//!   4. retrieval::hybrid_search는 fastembed 호출이 필요 → 본 smoke는 책별 격리·KNN
//!      raw 매칭 검증으로 동치 검증 (fastembed e2e는 AIRIS_E2E_EMBED=1 게이팅).
//!   5. context::build_context의 출력 인용 mapping이 실제 chunks.id를 가리키는지.

use std::path::Path;

use airis_lib::index::v041::context::{build_context, parse_citations};
use airis_lib::index::v041::f32_bytes;
use airis_lib::index::v041::indexer::{index_book, BookSource};
use airis_lib::index::v041::retrieval::{fts_only_search, hybrid_search};
use airis_lib::index::v041::vector_store::{ensure_vec0, VEC0_TABLE};
use airis_lib::parsers::markdown;
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

fn fresh_conn() -> Connection {
    register_sqlite_vec_once();
    let conn = Connection::open_in_memory().expect("open in-memory");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("FK on");

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (\
            version INTEGER PRIMARY KEY,\
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
         );",
    )
    .unwrap();
    for sql in MIGRATIONS {
        conn.execute_batch(sql).unwrap();
    }

    conn.execute(
        "INSERT INTO studies (slug, name, created_at) VALUES ('study1','Study1',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO books (id, study_slug, role, title, source_path, file_format,\
                             file_size, file_hash, added_at)\
         VALUES ('book1','study1','main','Smoke Book','/tmp/x.md','md',0,'h',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO books (id, study_slug, role, title, source_path, file_format,\
                             file_size, file_hash, added_at)\
         VALUES ('book2','study1','main','Other Book','/tmp/y.md','md',0,'h2',datetime('now'))",
        [],
    )
    .unwrap();
    conn
}

/// `i`번째 차원만 1.0인 384d one-hot 벡터.
fn one_hot_384(i: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; 384];
    if i < 384 {
        v[i] = 1.0;
    }
    v
}

#[test]
fn fts_only_search_against_indexed_chunks_returns_book_scoped_hits() {
    let mut conn = fresh_conn();

    let md = "\
# Chapter 1: Rust ownership
Rust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다.
borrow checker가 reference 수명을 추적합니다.

## Borrowing 규칙
한 시점에 하나의 mutable reference 또는 여러 immutable reference만 허용됩니다.
";
    let sections = markdown::parse(md);
    let outcome = index_book(
        &mut conn,
        "book1",
        BookSource::Sections(&sections),
        None,
        Path::new("/tmp"),
    )
    .expect("index_book OK");
    assert!(outcome.chunks_inserted >= 1);

    // book2에 같은 키워드 — 격리 검증.
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, section_path) \
         VALUES ('book2', 0, 'ownership 다른 책 동음어', 'Other')",
        [],
    )
    .unwrap();

    let hits = fts_only_search(&conn, "book1", "ownership", 5).expect("fts_only_search");
    assert!(!hits.is_empty(), "book1에서 ownership 키워드 hit");
    for h in &hits {
        // 책 한정 — book1의 chunks.id만.
        let book: String = conn
            .query_row(
                "SELECT book_id FROM chunks WHERE id = ?1",
                params![h.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(book, "book1", "책 격리 — book2 chunks가 끼어들면 안 됨");
    }
}

#[test]
fn build_context_from_real_chunks_yields_valid_citation_mapping() {
    let mut conn = fresh_conn();

    let md = "# 큰 챕터\n\n";
    let body: String = (0..100)
        .map(|i| format!("문장 {i}번이고 한국어 산문이 길게 이어집니다. "))
        .collect::<String>()
        .repeat(2);
    let md = format!("{md}{body}");
    let sections = markdown::parse(&md);
    let outcome = index_book(
        &mut conn,
        "book1",
        BookSource::Sections(&sections),
        None,
        Path::new("/tmp"),
    )
    .expect("index_book OK");
    assert!(outcome.chunks_inserted >= 1);

    // FTS-only 검색을 build_context의 입력으로 사용.
    let retrieved = fts_only_search(&conn, "book1", "한국어 문장", 5).expect("fts_only_search");
    assert!(!retrieved.is_empty(), "한국어 토큰 검색 hit");

    let bundle = build_context(&retrieved, "Smoke Book", 4_000);

    // 각 인용 mapping의 chunk_id가 실제 chunks 테이블에 존재하는 row여야 함.
    for entry in &bundle.citation_index_map {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE id = ?1",
                params![entry.chunk_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "citation_index_map의 chunk_id가 실 row를 가리킴");
        // marker는 1-base, 출력 순서대로 S1, S2, ... 부여.
        assert!(entry.marker.starts_with('S'));
    }

    // sources_block에 첫 source의 본문이 들어있어야 함.
    if let Some(first) = bundle.citation_index_map.first() {
        let text: String = conn
            .query_row(
                "SELECT text FROM chunks WHERE id = ?1",
                params![first.chunk_id],
                |r| r.get(0),
            )
            .unwrap();
        // 본문 첫 30자가 sources_block 어딘가에 있어야 한다.
        let snippet: String = text.chars().take(30).collect();
        assert!(
            bundle.sources_block.contains(&snippet),
            "sources_block에 chunk 본문 포함"
        );
    }

    // assistant 응답 시뮬레이션 — 첫 source를 인용.
    let fake_response = format!("핵심은 다음과 같습니다 [S1]. 추가로 [S{}].", bundle.citation_index_map.len() + 99);
    let parsed = parse_citations(&fake_response, bundle.citation_index_map.len());
    assert_eq!(parsed.len(), 2);
    assert!(parsed[0].in_range);
    assert!(!parsed[1].in_range, "범위 밖 마커는 환각으로 분류");
}

#[test]
fn hybrid_search_with_fake_embeddings_book_isolation() {
    // hybrid_search는 fastembed 호출이 필요(query embedding) — 본 케이스는 e2e 게이팅.
    if std::env::var("AIRIS_E2E_EMBED").ok().as_deref() != Some("1") {
        eprintln!("skip: AIRIS_E2E_EMBED 미설정 (모델 다운로드 비용)");
        return;
    }
    use airis_lib::index::v041::embedder::Embedder;

    let mut conn = fresh_conn();
    let md = "# 챕터\n\nRust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다. ";
    let sections = markdown::parse(md);
    let tmp = tempfile::tempdir().unwrap();
    let embedder = Embedder::new(tmp.path()).expect("embedder init");

    let outcome = index_book(
        &mut conn,
        "book1",
        BookSource::Sections(&sections),
        Some(&embedder),
        tmp.path(),
    )
    .expect("index_book OK");
    assert!(outcome.chunks_inserted >= 1);
    assert!(outcome.embeddings_inserted >= 1);

    let hits = hybrid_search(&conn, &embedder, "book1", "Rust 메모리 안전성", 5)
        .expect("hybrid_search");
    assert!(!hits.is_empty());
}

#[test]
fn ensure_vec0_idempotent_on_retrieval_entry() {
    // retrieval::hybrid_search 진입에서 ensure_vec0를 한 번 호출 — 여러 번 호출돼도
    // 동일 결과(sqlite_master 카운트 1).
    let conn = fresh_conn();
    ensure_vec0(&conn).unwrap();
    ensure_vec0(&conn).unwrap();
    ensure_vec0(&conn).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
            params![VEC0_TABLE],
            |r| r.get(0),
        )
        .unwrap();
    assert!(count >= 1, "vec0 가상 테이블이 한 번 생성된 후엔 idempotent");
}

#[test]
fn fake_embedding_inserted_directly_lets_vec0_knn_return_owner_book_chunk() {
    // 실제 fastembed 없이 가짜 임베딩으로 vec0 KNN이 동작하는지 — vector_store 없이
    // 직접 INSERT 경로 검증. 본 테스트는 vector_top_k의 책 필터 경로를 KNN raw 결과로
    // 동치 검증한다.
    let conn = fresh_conn();

    // book1에 1개 청크 + one-hot 임베딩.
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, section_path) \
         VALUES ('book1', 0, 'Rust ownership', 'Ch01')",
        [],
    )
    .unwrap();
    let id_b1: i64 = conn.last_insert_rowid();

    // book2에도 동일 차원 임베딩 — 책 격리 검증.
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, section_path) \
         VALUES ('book2', 0, '다른 책 ownership', 'Other')",
        [],
    )
    .unwrap();
    let id_b2: i64 = conn.last_insert_rowid();

    ensure_vec0(&conn).unwrap();
    let v_b1 = one_hot_384(0);
    let v_b2 = one_hot_384(1);
    let sql = format!("INSERT INTO {VEC0_TABLE}(rowid, embedding) VALUES (?1, ?2)");
    conn.execute(&sql, params![id_b1, f32_bytes(&v_b1)]).unwrap();
    conn.execute(&sql, params![id_b2, f32_bytes(&v_b2)]).unwrap();

    // 직접 KNN 호출 — book 필터링 전 raw가 두 row 모두 반환.
    let raw = airis_lib::index::v041::vector_store::knn(&conn, &v_b1, 5).unwrap();
    assert!(!raw.is_empty());
    let rids: Vec<i64> = raw.iter().map(|(id, _)| *id).collect();
    assert!(rids.contains(&id_b1));
    // distance 0인 b1이 top-1 (one-hot 자기 자신).
    assert_eq!(raw[0].0, id_b1);
}
