//! v0.4.3 PR 2 (D-088) smoke — sentence window 확장 + auto-merging + MMR 통합.
//!
//! 검증:
//!   1. 작은 MD → markdown::parse → indexer.index_book → chunks 적재 (parent/prev/next 채움).
//!   2. fts_only_search로 retrieval 결과 (점수 내림차순, prev/next/parent 메타 포함).
//!   3. expand_sentence_window 결과의 합본 텍스트 길이 ≥ 원본 청크 텍스트 길이.
//!   4. 같은 parent의 자식 청크 2개를 retrieved에 넣으면 merge_parents가 부모로 치환.
//!   5. mmr_dedupe → context::build_context_from_merged 패킹까지 무파괴 동작.
//!
//! fastembed 모델 다운로드 X — 실제 임베딩 KNN 없이 fts_only로 retrieved를 만든다.
//! (그래서 본 smoke는 v0.4.3 알고리즘 동작 검증에 집중. embedding KNN e2e는 별도.)

use std::path::Path;

use airis_lib::index::v041::context::build_context_from_merged;
use airis_lib::index::v041::indexer::{index_book, BookSource};
use airis_lib::index::v041::retrieval::fts_only_search;
use airis_lib::index::v043::post_retrieval::{
    expand_sentence_window, merge_parents, mmr_dedupe, AUTO_MERGE_TOKEN_LIMIT, MMR_LAMBDA_DEFAULT,
};
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
        "INSERT INTO studies (slug, name, created_at) VALUES ('s1','S1',datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO books (id, study_slug, role, title, source_path, file_format,\
                             file_size, file_hash, added_at)\
         VALUES ('book1','s1','main','Smoke','/tmp/x.md','md',0,'h',datetime('now'))",
        [],
    )
    .unwrap();
    conn
}

#[test]
fn expand_sentence_window_against_indexed_md_grows_text_with_neighbors() {
    let mut conn = fresh_conn();
    // 여러 청크가 생기도록 충분히 긴 한국어 본문.
    let mut md = String::from("# 챕터\n\n");
    for i in 0..40 {
        md.push_str(&format!(
            "{i}번 문장입니다. 한국어 학습 도우미가 검색할 수 있는 본문이고, ownership 모델이 \
             조금씩 등장합니다. 길게 풀어 써서 chunker가 여러 청크로 자르도록 합니다.\n\n"
        ));
    }
    let sections = markdown::parse(&md);
    let outcome = index_book(
        &mut conn,
        "book1",
        BookSource::Sections(&sections),
        None,
        Path::new("/tmp"),
    )
    .expect("index_book OK");
    assert!(outcome.chunks_inserted >= 2, "chunker가 다중 청크로 자른다");

    // 중간 청크 1개를 retrieved처럼 직접 SELECT해서 expand_sentence_window 입력으로.
    let retrieved = fts_only_search(&conn, "book1", "ownership", 5).expect("fts_only_search");
    assert!(!retrieved.is_empty(), "fts hit");
    // 적어도 한 청크는 prev_chunk_id가 Some이어야 sentence window 확장 효과를 검증할 수 있다.
    let with_prev = retrieved.iter().any(|c| c.prev_chunk_id.is_some());
    let with_next = retrieved.iter().any(|c| c.next_chunk_id.is_some());
    assert!(
        with_prev || with_next,
        "최소 한 청크는 prev/next 이웃이 있어야 함"
    );

    let expanded = expand_sentence_window(&conn, &retrieved).expect("expand OK");
    assert_eq!(expanded.len(), retrieved.len());
    // 적어도 한 항목은 합본 텍스트가 *원본보다 길다*.
    let any_grown = expanded
        .iter()
        .any(|e| e.expanded_text.chars().count() > e.core.text.chars().count());
    assert!(any_grown, "sentence window 확장으로 합본 길이 증가");
}

#[test]
fn merge_parents_replaces_when_two_children_match_under_token_limit() {
    let conn = fresh_conn();
    // chunks 직접 적재 — chunker 거치지 않고 parent/child 관계를 명시.
    // parent: small 부모 (50 토큰) + 자식 2개 (각 60·70 토큰, 합 130 < 800).
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, section_path, token_count) \
         VALUES ('book1', 0, '부모 본문', 'Ch01', 50)",
        [],
    )
    .unwrap();
    let parent_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, parent_id, token_count) \
         VALUES ('book1', 1, '자식 A 본문', ?1, 60)",
        params![parent_id],
    )
    .unwrap();
    let c1 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, parent_id, prev_chunk_id, token_count) \
         VALUES ('book1', 2, '자식 B 본문', ?1, ?2, 70)",
        params![parent_id, c1],
    )
    .unwrap();
    let c2 = conn.last_insert_rowid();

    // retrieved = [c1, c2]. expand → merge → 부모 1개로 치환.
    let retrieved = vec![
        airis_lib::index::v041::retrieval::RetrievedChunk {
            id: c1,
            text: "자식 A 본문".into(),
            page: None,
            section_path: None,
            parent_id: Some(parent_id),
            prev_chunk_id: None,
            next_chunk_id: Some(c2),
            token_count: Some(60),
            score: 0.9,
        },
        airis_lib::index::v041::retrieval::RetrievedChunk {
            id: c2,
            text: "자식 B 본문".into(),
            page: None,
            section_path: None,
            parent_id: Some(parent_id),
            prev_chunk_id: Some(c1),
            next_chunk_id: None,
            token_count: Some(70),
            score: 0.7,
        },
    ];
    let expanded = expand_sentence_window(&conn, &retrieved).expect("expand");
    let merged = merge_parents(&conn, &expanded).expect("merge");
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].id, parent_id);
    assert_eq!(merged[0].text, "부모 본문");
    assert_eq!(merged[0].source_chunks.len(), 2);
}

#[test]
fn mmr_dedupe_with_score_only_returns_top_n_when_embeddings_missing() {
    // smoke — embeddings 없이 (vectors_t1 미적재) MMR이 score 폴백으로 동작.
    let merged = vec![
        airis_lib::index::v043::post_retrieval::MergedChunk {
            id: 1,
            text: "A".into(),
            score: 0.9,
            page: None,
            section_path: None,
            token_count: 10,
            source_chunks: vec![1],
        },
        airis_lib::index::v043::post_retrieval::MergedChunk {
            id: 2,
            text: "B".into(),
            score: 0.5,
            page: None,
            section_path: None,
            token_count: 10,
            source_chunks: vec![2],
        },
        airis_lib::index::v043::post_retrieval::MergedChunk {
            id: 3,
            text: "C".into(),
            score: 0.1,
            page: None,
            section_path: None,
            token_count: 10,
            source_chunks: vec![3],
        },
    ];
    let emb = std::collections::HashMap::new();
    let out = mmr_dedupe(&[], &merged, &emb, MMR_LAMBDA_DEFAULT, 2);
    assert_eq!(out.len(), 2);
    // score 폴백으로 0.9, 0.5 순.
    assert_eq!(out[0].id, 1);
    assert_eq!(out[1].id, 2);
}

#[test]
fn merge_parents_skips_when_token_sum_exceeds_limit() {
    let conn = fresh_conn();
    // 부모 + 자식 2개. 자식 토큰 합 = 900 ≥ AUTO_MERGE_TOKEN_LIMIT.
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, token_count) \
         VALUES ('book1', 0, 'huge parent', 900)",
        [],
    )
    .unwrap();
    let parent_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, parent_id, token_count) \
         VALUES ('book1', 1, 'A', ?1, 450)",
        params![parent_id],
    )
    .unwrap();
    let c1 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, parent_id, token_count) \
         VALUES ('book1', 2, 'B', ?1, 450)",
        params![parent_id],
    )
    .unwrap();
    let c2 = conn.last_insert_rowid();

    // 토큰 한도 초과 시나리오 가드 — 상수 비교라 컴파일 타임에 트루.
    const _: () = assert!(450 + 450 >= AUTO_MERGE_TOKEN_LIMIT);
    let retrieved = vec![
        airis_lib::index::v041::retrieval::RetrievedChunk {
            id: c1,
            text: "A".into(),
            page: None,
            section_path: None,
            parent_id: Some(parent_id),
            prev_chunk_id: None,
            next_chunk_id: None,
            token_count: Some(450),
            score: 0.9,
        },
        airis_lib::index::v041::retrieval::RetrievedChunk {
            id: c2,
            text: "B".into(),
            page: None,
            section_path: None,
            parent_id: Some(parent_id),
            prev_chunk_id: None,
            next_chunk_id: None,
            token_count: Some(450),
            score: 0.7,
        },
    ];
    let expanded = expand_sentence_window(&conn, &retrieved).expect("expand");
    let merged = merge_parents(&conn, &expanded).expect("merge");
    assert_eq!(
        merged.len(),
        2,
        "토큰 한도 초과 → merge skip, sentence window만"
    );
}

#[test]
fn end_to_end_post_retrieval_to_context_pack_yields_valid_bundle() {
    let conn = fresh_conn();
    // 부모 + 자식 2개 + 단독 청크 1개.
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, section_path, page, token_count) \
         VALUES ('book1', 0, '부모 본문 (작은 섹션)', 'Ch01', 10, 50)",
        [],
    )
    .unwrap();
    let parent_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, parent_id, token_count) \
         VALUES ('book1', 1, '자식 A', ?1, 40)",
        params![parent_id],
    )
    .unwrap();
    let c1 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, parent_id, token_count) \
         VALUES ('book1', 2, '자식 B', ?1, 40)",
        params![parent_id],
    )
    .unwrap();
    let c2 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chunks (book_id, ord, text, section_path, token_count) \
         VALUES ('book1', 3, '단독 청크 본문', 'Ch02', 30)",
        [],
    )
    .unwrap();
    let solo = conn.last_insert_rowid();

    let retrieved = vec![
        airis_lib::index::v041::retrieval::RetrievedChunk {
            id: c1,
            text: "자식 A".into(),
            page: None,
            section_path: None,
            parent_id: Some(parent_id),
            prev_chunk_id: None,
            next_chunk_id: None,
            token_count: Some(40),
            score: 0.9,
        },
        airis_lib::index::v041::retrieval::RetrievedChunk {
            id: c2,
            text: "자식 B".into(),
            page: None,
            section_path: None,
            parent_id: Some(parent_id),
            prev_chunk_id: None,
            next_chunk_id: None,
            token_count: Some(40),
            score: 0.7,
        },
        airis_lib::index::v041::retrieval::RetrievedChunk {
            id: solo,
            text: "단독 청크 본문".into(),
            page: None,
            section_path: Some("Ch02".into()),
            parent_id: None,
            prev_chunk_id: None,
            next_chunk_id: None,
            token_count: Some(30),
            score: 0.5,
        },
    ];
    let expanded = expand_sentence_window(&conn, &retrieved).expect("expand");
    let merged = merge_parents(&conn, &expanded).expect("merge");
    assert_eq!(merged.len(), 2, "[parent, solo]");

    let emb = std::collections::HashMap::new();
    let top = mmr_dedupe(&[], &merged, &emb, MMR_LAMBDA_DEFAULT, 6);
    assert_eq!(top.len(), 2);

    let bundle = build_context_from_merged(&top, "Smoke Book", 4_000);
    assert_eq!(bundle.citation_index_map.len(), 2);
    // 병합된 부모 entry는 헤더에 "2 청크 병합" 표기.
    assert!(bundle.sources_block.contains("2 청크 병합"));
    // chunk_id가 실제 row를 가리켜야 함.
    for entry in &bundle.citation_index_map {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE id = ?1",
                params![entry.chunk_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1);
    }
}
