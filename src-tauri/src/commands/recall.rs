// F7.7 회상 챌린지 — 사용자가 챕터 핵심을 적으면 *키워드 누락률* 평가.
//
// PR 22 정책:
//   * 평가 = *결정적 코어*가 *기준 키워드*를 paragraphs에서 추출
//   * 키워드 = 챕터 본문에서 *빈도 높은 명사구* 단순 휴리스틱 (token 빈도 top-N).
//     LLM 평가는 v0.3+ (active provider 호출 + 비용)
//   * 사용자 입력 vs 기준 키워드 비교 → present/missing 분류
//   * passed = (present.len() / expected.len()) >= 0.6
//   * 통과 시 *자동 SRS 카드 생성*: front=chapter_ref, back=missing/expected 요약
//
// 결과는 recall_challenges에 영속 + 통과 시 SRS 카드 1개 자동 추가.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::AppState;

const PASS_THRESHOLD: f64 = 0.6;
const TOP_KEYWORDS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    pub id: i64,
    pub study_slug: String,
    pub chapter_ref: String,
    pub keywords_expected: Vec<String>,
    pub keywords_present: Vec<String>,
    pub keywords_missing: Vec<String>,
    pub passed: bool,
    pub challenged_at: String,
}

#[tauri::command]
pub fn recall_evaluate(
    state: State<'_, AppState>,
    study_slug: String,
    chapter_ref: String,
    user_input: String,
) -> AppResult<RecallResult> {
    if study_slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 슬러그가 비어 있습니다".into(),
        });
    }
    if chapter_ref.trim().is_empty() || user_input.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "챕터·사용자 입력은 필수입니다".into(),
        });
    }

    let chapter_body = fetch_chapter_body(&state, &study_slug, &chapter_ref)?;
    if chapter_body.trim().is_empty() {
        return Err(AppError::NotFound {
            message: format!("챕터 '{chapter_ref}' 본문이 비어 있습니다 (인덱싱 필요)"),
        });
    }

    let expected = extract_top_keywords(&chapter_body, TOP_KEYWORDS);
    let user_norm = normalize(&user_input);
    let present: Vec<String> = expected
        .iter()
        .filter(|k| user_norm.contains(&k.to_lowercase()))
        .cloned()
        .collect();
    let missing: Vec<String> = expected
        .iter()
        .filter(|k| !present.contains(k))
        .cloned()
        .collect();
    let ratio = if expected.is_empty() {
        0.0
    } else {
        present.len() as f64 / expected.len() as f64
    };
    let passed = ratio >= PASS_THRESHOLD;

    let db = state.db.lock().expect("db mutex");
    let id = persist_challenge(
        db.conn(),
        &study_slug,
        &chapter_ref,
        &user_input,
        &expected,
        &present,
        &missing,
        passed,
    )?;

    // F8.2 자동 카드 생성 — 통과 시.
    if passed {
        let summary_back = if missing.is_empty() {
            format!("키워드: {}", expected.join(", "))
        } else {
            format!(
                "키워드: {}\n\n놓친 항목: {}",
                expected.join(", "),
                missing.join(", ")
            )
        };
        let today = today_iso();
        if let Err(e) = db.conn().execute(
            "INSERT INTO srs_cards
             (study_slug, front, back, section_ref, e_factor, interval_days, repetitions, due_date, created_at)
             VALUES (?1, ?2, ?3, ?4, 2.5, 0, 0, ?5, datetime('now'))",
            params![
                study_slug,
                format!("{chapter_ref} 핵심을 떠올려 적어보세요"),
                summary_back,
                chapter_ref,
                today,
            ],
        ) {
            tracing::warn!(target: "recall", error = %e, "auto SRS card insert failed");
        }
    }

    info!(
        target: "recall",
        slug = %study_slug,
        chapter = %chapter_ref,
        passed,
        ratio,
        "recall_evaluate"
    );

    Ok(RecallResult {
        id,
        study_slug,
        chapter_ref,
        keywords_expected: expected,
        keywords_present: present,
        keywords_missing: missing,
        passed,
        challenged_at: now_iso(),
    })
}

/// paragraphs에서 chapter_ref의 본문을 모두 concat. chapter_ref는
/// `book_id/Ch04` 또는 `Ch04` 형식. 후자는 active 스터디 모든 책에서 매치.
fn fetch_chapter_body(state: &AppState, study_slug: &str, chapter_ref: &str) -> AppResult<String> {
    let db = state.db.lock().expect("db mutex");
    // chapter_ref이 "book_id/path" 형태면 book_id 분리, 아니면 study 안 모든 books에서 path 매치.
    let (book_id, path) = match chapter_ref.split_once('/') {
        Some((b, p)) if !p.is_empty() => (Some(b.to_string()), p.to_string()),
        _ => (None, chapter_ref.to_string()),
    };
    let mut stmt = if book_id.is_some() {
        db.conn().prepare(
            "SELECT p.content FROM paragraphs p
             JOIN books b ON b.id = p.book_id
             WHERE b.study_slug = ?1 AND p.book_id = ?2 AND p.section_path = ?3
             ORDER BY p.chunk_index ASC",
        )?
    } else {
        db.conn().prepare(
            "SELECT p.content FROM paragraphs p
             JOIN books b ON b.id = p.book_id
             WHERE b.study_slug = ?1 AND p.section_path = ?2
             ORDER BY p.book_id, p.chunk_index ASC",
        )?
    };
    let rows: Vec<String> = if let Some(bid) = book_id {
        stmt.query_map(params![study_slug, bid, path], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?
    } else {
        stmt.query_map(params![study_slug, path], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?
    };
    Ok(rows.join("\n\n"))
}

/// 빈도 top-N 토큰 (영문/한글, 길이 ≥ 2). 매우 단순 휴리스틱 — 본문이 작으면 결과도 적음.
pub fn extract_top_keywords(body: &str, n: usize) -> Vec<String> {
    let stop_words: &[&str] = &[
        "the",
        "and",
        "for",
        "with",
        "that",
        "this",
        "from",
        "into",
        "have",
        "has",
        "was",
        "were",
        "are",
        "is",
        "or",
        "but",
        "not",
        "you",
        "your",
        "they",
        "them",
        "이것",
        "그것",
        "저것",
        "그리고",
        "또는",
        "하지만",
        "그러나",
    ];
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut buf = String::new();
    for c in body.chars() {
        if c.is_alphanumeric() || is_cjk(c) {
            buf.push(c.to_ascii_lowercase());
        } else if !buf.is_empty() {
            let token = std::mem::take(&mut buf);
            if token.chars().count() >= 2 && !stop_words.contains(&token.as_str()) {
                *counts.entry(token).or_insert(0) += 1;
            }
        }
    }
    if !buf.is_empty() && buf.chars().count() >= 2 {
        *counts.entry(buf).or_insert(0) += 1;
    }
    let mut sorted: Vec<(String, u32)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted.into_iter().take(n).map(|(k, _)| k).collect()
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || is_cjk(*c) || c.is_whitespace())
        .collect()
}

fn is_cjk(c: char) -> bool {
    let n = c as u32;
    (0xAC00..=0xD7A3).contains(&n) || (0x4E00..=0x9FFF).contains(&n)
}

#[allow(clippy::too_many_arguments)]
fn persist_challenge(
    conn: &Connection,
    study_slug: &str,
    chapter_ref: &str,
    user_input: &str,
    expected: &[String],
    present: &[String],
    missing: &[String],
    passed: bool,
) -> AppResult<i64> {
    let exp_json = serde_json::to_string(expected).unwrap_or_else(|_| "[]".into());
    let pres_json = serde_json::to_string(present).unwrap_or_else(|_| "[]".into());
    let miss_json = serde_json::to_string(missing).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO recall_challenges
         (study_slug, chapter_ref, user_input, keywords_expected_json, keywords_present_json,
          keywords_missing_json, passed, challenged_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
        params![
            study_slug,
            chapter_ref,
            user_input,
            exp_json,
            pres_json,
            miss_json,
            if passed { 1 } else { 0 }
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn today_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs / 86400;
    let (y, m, d) = crate::commands::pomodoro::days_to_ymd_pub(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let (y, m, d) = crate::commands::pomodoro::days_to_ymd_pub(days);
    let in_day = secs % 86400;
    let h = in_day / 3600;
    let mm = (in_day % 3600) / 60;
    let s = in_day % 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mm:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_top_keywords_picks_frequent_tokens() {
        let body = "Rust ownership ownership ownership lifetime lifetime borrow checker checker";
        let kws = extract_top_keywords(body, 3);
        assert!(kws.contains(&"ownership".to_string()));
        assert!(kws.contains(&"checker".to_string()) || kws.contains(&"lifetime".to_string()));
    }

    #[test]
    fn extract_top_keywords_filters_stopwords() {
        let body = "the the the rust ownership ownership";
        let kws = extract_top_keywords(body, 5);
        assert!(!kws.contains(&"the".to_string()));
        assert!(kws.contains(&"ownership".to_string()));
    }

    #[test]
    fn extract_top_keywords_handles_korean() {
        let body = "소유권 소유권 라이프타임 라이프타임 라이프타임 컴파일러";
        let kws = extract_top_keywords(body, 3);
        assert_eq!(kws[0], "라이프타임");
        assert!(kws.contains(&"소유권".to_string()));
    }

    #[test]
    fn normalize_strips_punctuation() {
        let n = normalize("Rust의 ownership!");
        assert!(n.contains("rust"));
        assert!(n.contains("ownership"));
        assert!(!n.contains('!'));
    }
}
