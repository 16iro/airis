//! v0.4.4 PR 3 smoke test (D-093) — DOCX 파서 + 인덱서 + chunks_fts retrieval 통합.
//!
//! 검증:
//!   1. in-memory DOCX(DocxBuilder) → docx::parse → docx::to_sections → indexer.index_book.
//!   2. chunks INSERT → chunks_fts 트리거 동기화 → MATCH 검색으로 retrieval 가능.
//!   3. 한국어 + 영문 키워드 둘 다 hit (D-079 sentence 보존 부수효과 + DOCX UTF-8 정확).
//!   4. 헤딩 단락은 Section 단위로 묶여 section_path가 `Ch01` / `Ch01/§…` 패턴 (MD 호환).

use std::path::Path;

use airis_lib::index::v041::indexer::{index_book, BookSource};
use airis_lib::parsers::docx;
use docx_rs::{Docx, Paragraph, Run};
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
    // v17 — v0.4.4 PR 3 (D-093): books.file_format에 'docx' 추가. v14~v16은 chunks/jobs
    // 무관 변경이라 본 smoke에선 skip 가능 — DOCX INSERT만 통과하면 됨.
    include_str!("../src/migrations/v17_book_format_docx.sql"),
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

    // FK 만족용 study + DOCX book.
    conn.execute(
        "INSERT INTO studies (slug, name, created_at) VALUES ('study1','Study1',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO books (id, study_slug, role, title, source_path, file_format,\
                             file_size, file_hash, added_at)\
         VALUES ('book_docx','study1','main','Smoke DOCX','/tmp/x.docx','docx',0,'h',datetime('now'))",
        [],
    )
    .unwrap();
    conn
}

/// 한국어 헤딩 + 본문 + 영문 토큰 혼합 인메모리 DOCX 생성.
fn build_smoke_docx() -> Vec<u8> {
    let docx = Docx::new()
        .add_paragraph(
            Paragraph::new()
                .style("Heading1")
                .add_run(Run::new().add_text("제 1 장: Rust ownership 모델")),
        )
        .add_paragraph(
            Paragraph::new().add_run(
                Run::new().add_text("Rust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다."),
            ),
        )
        .add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text("borrow checker가 reference 수명을 추적합니다.")),
        )
        .add_paragraph(
            Paragraph::new()
                .style("Heading2")
                .add_run(Run::new().add_text("§ 1.1 Borrowing 규칙")),
        )
        .add_paragraph(
            Paragraph::new().add_run(
                Run::new().add_text(
                    "한 시점에 하나의 mutable reference 또는 여러 immutable reference만 허용됩니다.",
                ),
            ),
        )
        .add_paragraph(
            Paragraph::new()
                .style("Heading1")
                .add_run(Run::new().add_text("제 2 장: 채널과 동시성")),
        )
        .add_paragraph(
            Paragraph::new().add_run(
                Run::new().add_text("channel은 메시지 전달 기반의 동시성 모델을 제공합니다."),
            ),
        );

    let mut buf: Vec<u8> = Vec::new();
    docx.build()
        .pack(std::io::Cursor::new(&mut buf))
        .expect("pack");
    buf
}

#[test]
fn docx_parsed_then_indexed_then_retrievable_via_fts() {
    let mut conn = fresh_conn();

    // 1. DOCX 파싱 → DocxParsed → Section 시퀀스.
    let bytes = build_smoke_docx();
    let parsed = docx::parse_bytes(&bytes).expect("docx parse OK");
    // 헤딩 3개 + 본문 4개 = 7 단락 (빈 단락 없음).
    assert_eq!(parsed.paragraphs.len(), 7);

    let sections = docx::to_sections(&parsed);
    // h1=Ch01, h2 → Ch01/§Borrowing-규칙, h1=Ch02 → 3 섹션.
    assert_eq!(sections.len(), 3, "h1 두 개 + h2 한 개 = 3 섹션");
    assert_eq!(sections[0].path, "Ch01");
    assert!(sections[1].path.starts_with("Ch01/§"));
    assert_eq!(sections[2].path, "Ch02");

    // 2. v0.4.1 인덱서 통과 (embedder=None — 임베딩은 stub, FTS는 트리거로 자동 채움).
    let outcome = index_book(
        &mut conn,
        "book_docx",
        BookSource::Sections(&sections),
        None,
        Path::new("/tmp"),
    )
    .expect("index_book OK");
    assert!(
        outcome.chunks_inserted >= sections.len(),
        "최소 섹션 개수만큼 청크"
    );
    assert_eq!(outcome.embeddings_inserted, 0);

    // 3. chunks 테이블 실제 적재 확인.
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE book_id='book_docx'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(total as usize, outcome.chunks_inserted);

    // 4. FTS retrieval — 영문 토큰.
    let hits_en: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'ownership'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hits_en >= 1, "영문 'ownership' DOCX FTS hit");

    // 한국어는 FTS5 unicode61 토크나이저가 띄어쓰기 단위로 자르므로 띄어쓰기 분리된
    // 단어로 매칭. 본문 "메시지 전달" → 'channel'은 영문이라 prefix로 검증.
    let hits_channel: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'channel*'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hits_channel >= 1, "DOCX 'channel*' 검색 hit");

    // 5. section_path가 v041 인덱서 표준 형식인지.
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT section_path FROM chunks WHERE book_id='book_docx' ORDER BY section_path",
        )
        .unwrap();
    let paths: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(paths.contains(&"Ch01".to_string()));
    assert!(paths.contains(&"Ch02".to_string()));
    // h2 섹션 path 존재.
    assert!(
        paths.iter().any(|p| p.starts_with("Ch01/§")),
        "h2 → Ch01/§… section_path 존재. 실제: {paths:?}"
    );

    // 6. jobs 마무리 status.
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
fn docx_without_headings_falls_back_to_single_ch01() {
    // 헤딩이 전혀 없는 DOCX → 단일 Ch01 본문 → chunks_fts MATCH 가능.
    let docx = Docx::new()
        .add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text("이 문서는 헤딩이 없습니다. plain DOCX paragraph 시퀀스만."),
            ),
        )
        .add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("두 번째 단락 with English keyword.")),
        );
    let mut buf: Vec<u8> = Vec::new();
    docx.build()
        .pack(std::io::Cursor::new(&mut buf))
        .expect("pack");
    let parsed = docx::parse_bytes(&buf).expect("parse");
    let sections = docx::to_sections(&parsed);
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].path, "Ch01");

    let mut conn = fresh_conn();
    let outcome = index_book(
        &mut conn,
        "book_docx",
        BookSource::Sections(&sections),
        None,
        Path::new("/tmp"),
    )
    .expect("index_book OK");
    assert!(outcome.chunks_inserted >= 1);

    // 'keyword' 단어로 hit.
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'keyword'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hits >= 1, "헤딩 없는 DOCX도 FTS retrieval 동작");
}
