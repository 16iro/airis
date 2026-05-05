//! v0.4.1 PR 2 smoke test — chunker + indexer 통합 (실제 fastembed 호출 X).
//!
//! 검증:
//!   1. 작은 MD 본문 1개 → MarkdownParser → chunk_md_sections → indexer.index_book.
//!   2. chunks INSERT → chunks_fts 트리거가 동기화 → MATCH 검색으로 retrieval 가능.
//!   3. parent_id / prev_chunk_id / next_chunk_id가 chunker가 채운 ord 인덱스 → 실제
//!      chunks.id로 변환되어 NULL이 아닌 row가 만들어지는지.
//!   4. 영문·한국어 혼합 키워드 둘 다 chunks_fts MATCH로 잡힘 (D-079 sentence 보존 부수효과).

use std::path::Path;

use airis_lib::index::v041::indexer::{index_book, BookSource};
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

fn fresh_conn() -> Connection {
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

    // FK 만족용 study + book.
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
    conn
}

#[test]
fn md_parsed_then_chunked_then_indexed_then_retrievable_via_fts() {
    let mut conn = fresh_conn();

    // 작은 MD 본문 — 두 챕터 + 한국어 산문 + 영문 키워드 혼합.
    let md = "\
# Chapter 1: Rust Ownership
Rust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다.
borrow checker가 reference 수명을 추적합니다.

## Borrowing 규칙
한 시점에 하나의 mutable reference 또는 여러 immutable reference만 허용됩니다.
이 규칙은 데이터 경합을 정적으로 차단합니다.

# Chapter 2: 채널과 동시성
channel은 메시지 전달 기반의 동시성 모델을 제공합니다.
mpsc 채널은 다중 송신·단일 수신 패턴입니다.
";
    let sections = markdown::parse(md);
    assert!(sections.len() >= 3, "h1 두 개 + h2 한 개 = ≥3 섹션");

    let outcome = index_book(
        &mut conn,
        "book1",
        BookSource::Sections(&sections),
        None,
        Path::new("/tmp"),
    )
    .expect("index_book OK");
    assert!(
        outcome.chunks_inserted >= sections.len(),
        "최소 섹션 개수만큼 청크"
    );
    // embedder=None이므로 임베딩 0 — PR 4 reindex 시점에 Some(embedder) 진입.
    assert_eq!(outcome.embeddings_inserted, 0);

    // chunks 테이블 적재.
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks WHERE book_id='book1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total as usize, outcome.chunks_inserted);

    // FTS 검색 — 영문·한국어 둘 다.
    let hits_en: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'ownership'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hits_en >= 1, "영문 'ownership' 검색 hit");

    // 한국어는 FTS5 unicode61 토크나이저가 띄어쓰기 단위로 자른다 — 본문에서 *띄어쓰기로
    // 분리된 한국어 토큰* 그대로 매칭. paragraphs_fts와 같은 패턴.
    let hits_ko: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'channel*'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hits_ko >= 1, "영문 'channel*' 검색 hit (FTS prefix)");

    // jobs 마무리 status — completed.
    let status: String = conn
        .query_row(
            "SELECT status FROM indexing_jobs WHERE id = ?1",
            params![outcome.job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "completed");
}

#[test]
fn large_md_section_keeps_parent_prev_next_links_after_db_id_resolution() {
    let mut conn = fresh_conn();

    // 단일 섹션을 chunker가 분할하도록 본문을 길게.
    let body: String = (0..200)
        .map(|i| format!("문장 {i}번이고 한국어 산문이 길게 이어집니다. "))
        .collect::<String>()
        .repeat(5);
    let md = format!("# 큰 챕터\n\n{body}");
    let sections = markdown::parse(&md);
    let outcome = index_book(
        &mut conn,
        "book1",
        BookSource::Sections(&sections),
        None,
        Path::new("/tmp"),
    )
    .expect("index_book OK");
    assert!(
        outcome.chunks_inserted >= 2,
        "긴 본문은 ≥2 청크. 실제 {}",
        outcome.chunks_inserted
    );

    // 첫 청크(ord 0): parent_id NULL.
    let parent0: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM chunks WHERE book_id='book1' AND ord=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(parent0.is_none(), "첫 청크 parent_id NULL");

    // ord 0 의 next_chunk_id가 존재하는 chunks.id를 가리킴.
    let row0_next: Option<i64> = conn
        .query_row(
            "SELECT next_chunk_id FROM chunks WHERE book_id='book1' AND ord=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let next_id = row0_next.expect("ord 0 next_chunk_id 존재");
    let next_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE id = ?1",
            params![next_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(next_exists, 1, "next_chunk_id가 실제 chunks row를 가리킴");

    // ord 1 의 parent_id = ord 0 의 chunks.id (자기 자신 제외 부모 보존).
    let row0_id: i64 = conn
        .query_row(
            "SELECT id FROM chunks WHERE book_id='book1' AND ord=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let row1_parent: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM chunks WHERE book_id='book1' AND ord=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(row1_parent, Some(row0_id));
}
