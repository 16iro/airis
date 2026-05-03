// F12.1 Memory active 모순 검사 — 결정적, 이벤트 트리거.
//
// 정책 (PR 18, v0.2 시점):
//   * Memory body의 *active* 항목들을 읽어 *명백한 충돌*만 잡음.
//   * 휴리스틱: 같은 핵심어를 *서로 다른 섹션에서 부정 vs 권장*으로 표현 시 충돌.
//   * 거짓 양성·음성 모두 가능 — 결과는 *consistency_check_log*에 기록만, 사용자 alert는 v0.3+.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize)]
pub struct ConsistencyIssue {
    pub kind: &'static str,
    pub message: String,
    /// 충돌이 의심된 두 항목.
    pub a: String,
    pub b: String,
}

/// Memory body에서 active 항목 충돌 의심 검출.
pub fn detect_memory_conflicts(memory_body: &str) -> Vec<ConsistencyIssue> {
    let mut issues = Vec::new();
    let active = active_items(memory_body);

    // 매우 단순: Preferences active 항목 vs Corrections active 항목 사이에서 *공통 키워드*가 있으면
    // 의심. 같은 단어를 한쪽은 권장, 다른 쪽은 부정할 가능성.
    let prefs: Vec<&(String, String)> = active
        .iter()
        .filter(|(s, _)| s.contains("Preferences"))
        .collect();
    let corrs: Vec<&(String, String)> = active
        .iter()
        .filter(|(s, _)| s.contains("Corrections"))
        .collect();

    for (_, p) in &prefs {
        for (_, c) in &corrs {
            if let Some(common) = first_common_keyword(p, c) {
                issues.push(ConsistencyIssue {
                    kind: "preference_vs_correction_overlap",
                    message: format!(
                        "같은 키워드 '{common}'가 선호/교정에 동시 등장 — 의도 충돌 가능"
                    ),
                    a: p.clone(),
                    b: c.clone(),
                });
            }
        }
    }

    issues
}

/// (heading, item) 쌍의 active 항목들.
fn active_items(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current_heading = String::new();
    for line in body.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            current_heading = h.to_string();
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("- ") && trimmed.contains("(active") {
            out.push((current_heading.clone(), trimmed.to_string()));
        }
    }
    out
}

/// 두 항목에서 공통으로 나타나는 *길이 ≥ 2* 토큰 (영문/한글). 가장 먼저 발견된 것 반환.
fn first_common_keyword(a: &str, b: &str) -> Option<String> {
    let toks_a = significant_tokens(a);
    if toks_a.is_empty() {
        return None;
    }
    let toks_b = significant_tokens(b);
    for ta in &toks_a {
        if toks_b.iter().any(|tb| tb == ta) {
            return Some(ta.clone());
        }
    }
    None
}

fn significant_tokens(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || is_cjk(c) {
            buf.push(c);
        } else if !buf.is_empty() {
            if buf.chars().count() >= 2 && !is_status_word(&buf) {
                out.push(buf.clone());
            }
            buf.clear();
        }
    }
    if buf.chars().count() >= 2 && !is_status_word(&buf) {
        out.push(buf);
    }
    out
}

fn is_status_word(s: &str) -> bool {
    matches!(
        s,
        "active" | "deprecated" | "resolved" | "achieved" | "since" | "지적" | "회"
    )
}

fn is_cjk(c: char) -> bool {
    let n = c as u32;
    (0xAC00..=0xD7A3).contains(&n) || (0x4E00..=0x9FFF).contains(&n)
}

/// 결과를 consistency_check_log에 기록. issues가 비어있어도 *통과 사실*은 기록 (run history).
pub fn log_check(
    conn: &Connection,
    study_slug: &str,
    triggered_by: &str,
    issues: &[ConsistencyIssue],
) -> AppResult<()> {
    let issues_json = serde_json::to_string(issues).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO consistency_check_log
         (study_slug, check_type, triggered_by, issues_json, checked_at)
         VALUES (?1, 'memory_active_conflict', ?2, ?3, datetime('now'))",
        params![study_slug, triggered_by, issues_json],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_preference_correction_keyword_overlap() {
        let body = "## 1. 사용자 선호 (Preferences)\n\n\
            - (active) 영어 용어 그대로 사용\n\n\
            ## 2. 금지·교정 (Corrections)\n\n\
            - (active) 영어 용어를 한글로 강제 번역하지 마\n";
        let issues = detect_memory_conflicts(body);
        assert!(!issues.is_empty());
        assert_eq!(issues[0].kind, "preference_vs_correction_overlap");
    }

    #[test]
    fn no_issue_when_keywords_disjoint() {
        let body = "## 1. 사용자 선호 (Preferences)\n\n\
            - (active) 빠른 결과 우선\n\n\
            ## 2. 금지·교정 (Corrections)\n\n\
            - (active) 너무 길게 답하지 마\n";
        let issues = detect_memory_conflicts(body);
        assert!(issues.is_empty());
    }

    #[test]
    fn ignores_non_active_items() {
        let body = "## 1. 사용자 선호 (Preferences)\n\n\
            - (deprecated 2026-04-30) 영어 그대로\n\n\
            ## 2. 금지·교정 (Corrections)\n\n\
            - (active) 영어 강제 번역하지 마\n";
        let issues = detect_memory_conflicts(body);
        assert!(issues.is_empty(), "deprecated item should not match");
    }

    #[test]
    fn empty_memory_yields_no_issues() {
        assert!(detect_memory_conflicts("").is_empty());
        assert!(detect_memory_conflicts("# Memory\n\n(empty)\n").is_empty());
    }
}
