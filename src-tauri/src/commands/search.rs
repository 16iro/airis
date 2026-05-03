// F5 — 검색.
//
// PR 11 단순화 (D-064): SQLite FTS5 키워드 검색만. 임베딩·하이브리드는 v0.3+.
//
// 검색어 정규화:
//   * 사용자가 입력한 단어들을 unicode61 토큰화한 *각 토큰*에 prefix `*`를 붙여 OR 결합.
//   * 예: "소유권 차용"  →  `소유권* OR 차용*`
//   * 한국어 어미 흡수("소유권은") 토큰을 잡으려면 prefix 와일드카드가 필요.
//   * 영어는 stem 효과 (예: "rust" → "rusty"·"rusting" 잡음).
//
// bm25 점수: SQLite FTS5 내장. 음수일수록 더 관련 (관행). 호출자엔 *양수 score*로 변환.

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;

const DEFAULT_LIMIT: u32 = 5;
const HARD_MAX_LIMIT: u32 = 50;

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub book_id: String,
    pub book_title: String,
    pub section_path: String,
    pub section_label: String,
    pub page: Option<i64>,
    pub snippet: String,
    pub score: f64,
}

#[tauri::command]
pub fn search_sections(
    state: State<'_, AppState>,
    study_slug: String,
    query: String,
    limit: Option<u32>,
) -> AppResult<Vec<SearchHit>> {
    let lim = limit.unwrap_or(DEFAULT_LIMIT).min(HARD_MAX_LIMIT) as i64;
    let normalized = normalize_query(&query)?;

    let db = state.db.lock().expect("db mutex");
    fts_search(db.conn(), &study_slug, &normalized, lim)
}

/// 사용자 입력 → FTS5 MATCH 표현식.
/// 빈 토큰만 남는 경우 InvalidInput.
pub fn normalize_query(query: &str) -> AppResult<String> {
    // 토큰화: 영문/한글/숫자만 남기고 나머지는 공백으로.
    let cleaned: String = query
        .chars()
        .map(|c| if is_token_char(c) { c } else { ' ' })
        .collect();
    let tokens: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err(AppError::InvalidInput {
            message: "검색어가 비어 있습니다".into(),
        });
    }
    // 각 토큰에 prefix `*` 부여 후 OR 결합.
    let expr = tokens
        .iter()
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    Ok(expr)
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric()
        || ('\u{AC00}'..='\u{D7A3}').contains(&c) // 한글
        || ('\u{4E00}'..='\u{9FFF}').contains(&c) // 한자
}

pub fn fts_search(
    conn: &Connection,
    study_slug: &str,
    match_expr: &str,
    limit: i64,
) -> AppResult<Vec<SearchHit>> {
    let mut stmt = conn.prepare(
        "SELECT
            p.book_id,
            b.title,
            p.section_path,
            p.section_label,
            p.page,
            snippet(paragraphs_fts, 0, '<<', '>>', '…', 12) AS snip,
            bm25(paragraphs_fts) AS score
         FROM paragraphs_fts
         JOIN paragraphs p ON p.id = paragraphs_fts.rowid
         JOIN books b ON b.id = p.book_id
         WHERE paragraphs_fts MATCH ?1
           AND b.study_slug = ?2
         ORDER BY score
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![match_expr, study_slug, limit], |r| {
        let bm25: f64 = r.get(6)?;
        Ok(SearchHit {
            book_id: r.get(0)?,
            book_title: r.get(1)?,
            section_path: r.get(2)?,
            section_label: r.get(3)?,
            page: r.get(4)?,
            snippet: r.get(5)?,
            // bm25는 음수가 더 관련 → 부호 뒤집어 양수 score로 노출.
            score: -bm25,
        })
    })?;
    let mut hits = Vec::new();
    for h in rows {
        hits.push(h?);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::index::keyword;
    use crate::parsers::types::{Section, SectionLevel};

    fn seed(db: &mut Db, study: &str, book_id: &str, section: &str, body: &str) {
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO studies (slug, name, created_at) VALUES (?1, ?1, datetime('now'))",
                params![study],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (id, study_slug, role, title, source_path, file_format, file_size, file_hash, added_at)
                 VALUES (?1, ?2, 'main', 'Book', '/tmp/x', 'md', 0, 'h', datetime('now'))",
                params![book_id, study],
            )
            .unwrap();
        let s = Section {
            path: section.to_string(),
            display_label: section.replace('/', " "),
            level: SectionLevel::Chapter,
            parent_path: None,
            page: Some(1),
            body: body.to_string(),
        };
        keyword::rebuild_book_paragraphs(db.conn_mut(), book_id, &[s]).unwrap();
    }

    #[test]
    fn normalize_query_emits_or_of_prefixes() {
        let expr = normalize_query("소유권 차용").unwrap();
        assert!(expr.contains("\"소유권\"*"));
        assert!(expr.contains("\"차용\"*"));
        assert!(expr.contains(" OR "));
    }

    #[test]
    fn normalize_query_rejects_punctuation_only() {
        assert!(normalize_query("...!?").is_err());
        assert!(normalize_query("   ").is_err());
    }

    #[test]
    fn fts_finds_korean_token_with_prefix() {
        let mut db = Db::open_in_memory_for_test();
        seed(
            &mut db,
            "s",
            "b1",
            "Ch01",
            "러스트의 소유권은 컴파일러가 검사합니다.",
        );
        let expr = normalize_query("소유권").unwrap();
        let hits = fts_search(db.conn(), "s", &expr, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].book_id, "b1");
        assert!(
            hits[0].snippet.contains("<<"),
            "snippet should highlight match"
        );
    }

    #[test]
    fn fts_excludes_other_studies() {
        let mut db = Db::open_in_memory_for_test();
        seed(&mut db, "s1", "b1", "Ch01", "러스트 소유권 시스템.");
        seed(&mut db, "s2", "b2", "Ch01", "러스트 소유권 시스템.");
        let expr = normalize_query("소유권").unwrap();
        let hits = fts_search(db.conn(), "s1", &expr, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].book_id, "b1");
    }

    #[test]
    fn fts_returns_empty_when_no_match() {
        let mut db = Db::open_in_memory_for_test();
        seed(&mut db, "s", "b1", "Ch01", "한글 본문.");
        let expr = normalize_query("zzzz").unwrap();
        let hits = fts_search(db.conn(), "s", &expr, 5).unwrap();
        assert!(hits.is_empty());
    }
}
