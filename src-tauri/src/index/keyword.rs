// 키워드 인덱서 — paragraphs INSERT (FTS5는 트리거가 자동 동기화).
//
// 흐름 (commands::book::start_indexing 호출):
//   1) PR 10 파서가 ParsedBook { metadata, sections } 반환.
//   2) 각 Section.body를 chunker로 분할.
//   3) paragraphs에 INSERT — book_id·section_path·section_label·chunk_index·content·page·char_offset.
//   4) FTS5 트리거가 paragraphs_fts에 자동 동기화.
//
// 트랜잭션: 한 책 인덱싱은 *한 트랜잭션*. 부분 INSERT 후 실패 시 전체 롤백 → 깨진 인덱스 회피.

use rusqlite::{params, Connection};

use crate::error::{AppError, AppResult};
use crate::index::chunker;
use crate::parsers::types::Section;

/// 한 책의 paragraphs를 *처음부터 다시* 작성. 기존 row는 트랜잭션 시작 시 삭제.
pub fn rebuild_book_paragraphs(
    conn: &mut Connection,
    book_id: &str,
    sections: &[Section],
) -> AppResult<u32> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM paragraphs WHERE book_id = ?1",
        params![book_id],
    )?;

    let mut total_chunks: u32 = 0;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO paragraphs (
                book_id, section_path, section_label, chunk_index, content, page, char_offset
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for section in sections {
            let chunks = chunker::chunk_section(&section.body);
            for (idx, chunk) in chunks.iter().enumerate() {
                stmt.execute(params![
                    book_id,
                    section.path,
                    section.display_label,
                    idx as i64,
                    chunk.content,
                    section.page,
                    chunk.char_offset as i64,
                ])
                .map_err(|e| AppError::Db {
                    message: format!("paragraph insert: {e}"),
                })?;
                total_chunks += 1;
            }
        }
    }
    tx.commit()?;
    Ok(total_chunks)
}

/// 책 삭제 시 자동 — books FK ON DELETE CASCADE가 paragraphs도 같이 지우므로
/// 명시 호출은 *재인덱싱 도중 취소* 등 특수 케이스에만.
pub fn purge_book_paragraphs(conn: &Connection, book_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM paragraphs WHERE book_id = ?1",
        params![book_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::parsers::types::SectionLevel;

    fn seed_book(db: &Db, book_id: &str) {
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (
                    id, study_slug, role, title, source_path, file_format,
                    file_size, file_hash, added_at
                 ) VALUES (?1,'s','main','Book','/tmp/x','md',0,'h',datetime('now'))",
                params![book_id],
            )
            .unwrap();
    }

    fn make_section(path: &str, body: &str) -> Section {
        Section {
            path: path.to_string(),
            display_label: path.replace('/', " "),
            level: SectionLevel::Chapter,
            parent_path: None,
            page: None,
            body: body.to_string(),
        }
    }

    #[test]
    fn rebuild_inserts_chunks_and_fts_indexes_them() {
        let mut db = Db::open_in_memory_for_test();
        seed_book(&db, "b1");
        let sections = vec![make_section(
            "Ch01",
            &"러스트의 소유권은 강력한 보장입니다. ".repeat(40),
        )];
        let count = rebuild_book_paragraphs(db.conn_mut(), "b1", &sections).unwrap();
        assert!(count >= 1);

        let in_db: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM paragraphs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(in_db, count as i64);

        // unicode61 tokenizer는 *공백 분리 단어* 단위 → 한국어 어미 흡수("소유권은")가 토큰.
        // 사용자 검색어와 매칭하려면 *prefix 와일드카드*가 필요 — search commands가 자동 부여.
        let matched: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM paragraphs_fts WHERE paragraphs_fts MATCH '소유권*'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            matched >= 1,
            "FTS prefix match should pick up Korean tokens"
        );
    }

    #[test]
    fn rebuild_replaces_previous_paragraphs() {
        let mut db = Db::open_in_memory_for_test();
        seed_book(&db, "b1");
        let s1 = vec![make_section("Ch01", "first version content here.")];
        rebuild_book_paragraphs(db.conn_mut(), "b1", &s1).unwrap();

        let s2 = vec![make_section(
            "Ch01",
            "second version content totally different.",
        )];
        rebuild_book_paragraphs(db.conn_mut(), "b1", &s2).unwrap();

        let total: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM paragraphs WHERE book_id='b1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // 두 번째 호출이 첫 번째를 *대체* → s2의 청크 수만 남음.
        assert!(total >= 1);

        let first_left: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM paragraphs WHERE content LIKE '%first version%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(first_left, 0, "old paragraphs must be deleted");
    }

    #[test]
    fn purge_removes_all_book_paragraphs() {
        let mut db = Db::open_in_memory_for_test();
        seed_book(&db, "b1");
        let sections = vec![make_section("Ch01", "some body")];
        rebuild_book_paragraphs(db.conn_mut(), "b1", &sections).unwrap();
        purge_book_paragraphs(db.conn(), "b1").unwrap();
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM paragraphs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
