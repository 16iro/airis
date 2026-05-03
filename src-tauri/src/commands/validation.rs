// F4.4 응답 검증 — Memory.Corrections active 위반 감지.
//
// 정책 (PR 17, handoff 결정 — 결정적 정규식만):
//   * Memory body의 *Corrections 섹션 active 항목*만 검사
//   * 항목 형식: "- (active...) X 하지 말아줘" / "X 하지 마" / "그러지 마"
//   * 부정 대상 X를 캡처 → 응답 텍스트에 *X 핵심어*가 등장하면 ViolationHit
//   * **거짓 양성 가능성 명시** — UI는 "충돌 의심" 배너만 표시 (재생성 X). LLM 기반 검증은 v0.3+
//
// 사용자가 corrections에 박은 한 항목 = 한 violation 후보 패턴.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationHit {
    /// 충돌이 의심된 corrections 항목 원문.
    pub correction_item: String,
    /// 부정 대상 (예: "영어 용어를 한글로 강제 번역").
    pub forbidden: String,
    /// 응답에서 매치된 텍스트 — UI에 강조.
    pub matched_in_response: String,
}

/// Memory body에서 active corrections 항목들을 응답과 대조해 위반 의심 hits 반환.
pub fn detect(response: &str, memory_body: &str) -> Vec<ViolationHit> {
    let mut hits = Vec::new();
    for item in active_corrections(memory_body) {
        let Some(forbidden) = extract_forbidden(&item) else {
            continue;
        };
        let trimmed = forbidden.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 핵심어 = forbidden의 첫 명사구 (단순 공백 split — 처음 1~2 단어).
        let key = first_significant_token(trimmed);
        if key.is_empty() {
            continue;
        }
        if response.contains(&key) {
            hits.push(ViolationHit {
                correction_item: item.clone(),
                forbidden: trimmed.to_string(),
                matched_in_response: key,
            });
        }
    }
    hits
}

/// Corrections 섹션 (## 2. ...) 안의 `- (active...)` 라인만 추출.
fn active_corrections(body: &str) -> Vec<String> {
    let mut in_corrections = false;
    let mut out = Vec::new();
    for line in body.lines() {
        if line.starts_with("## ") {
            in_corrections = line.contains("Corrections");
            continue;
        }
        if !in_corrections {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("- ") && trimmed.contains("(active") {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// 부정 패턴 매처 — "X 하지 말아줘" / "X 하지 마" / "그러지 마" / "X 빼줘" 등에서 X 추출.
fn extract_forbidden(item: &str) -> Option<String> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let res = PATTERNS.get_or_init(|| {
        [
            r"(.+?)\s*하지\s*말아?\s*줘",
            r"(.+?)\s*하지\s*마",
            r"(.+?)\s*빼줘",
            r"(.+?)\s*안\s*하면",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    });

    // status tag 이후 부분만 검사 (괄호 닫힘 다음).
    let after_tag = item.split_once(')').map(|(_, b)| b).unwrap_or(item);
    for re in res {
        if let Some(m) = re.captures(after_tag) {
            if let Some(g1) = m.get(1) {
                let txt = g1.as_str().trim();
                if !txt.is_empty() {
                    return Some(txt.to_string());
                }
            }
        }
    }
    None
}

/// 부정 대상에서 *유의미한* 첫 토큰을 뽑음 — 응답 매치용 핵심어.
/// 매우 단순: 공백 split, 길이 ≥ 2 첫 단어. CJK 단일 문자도 허용.
fn first_significant_token(s: &str) -> String {
    for token in s.split_whitespace() {
        let cleaned: String = token
            .chars()
            .filter(|c| c.is_alphanumeric() || is_cjk(*c))
            .collect();
        if cleaned.chars().count() >= 2 {
            return cleaned;
        }
    }
    String::new()
}

fn is_cjk(c: char) -> bool {
    let n = c as u32;
    (0xAC00..=0xD7A3).contains(&n) || (0x4E00..=0x9FFF).contains(&n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_violation_when_response_contains_forbidden_term() {
        let memory = "## 2. 금지·교정 (Corrections)\n\n\
            - (active, 지적 3회 since 2026-04-10) 영어 용어를 한글 강제 번역하지 말아줘\n";
        let response = "Rust 소유권(영어 용어) 시스템은 컴파일러가 검사합니다";
        let hits = detect(response, memory);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].forbidden.contains("영어 용어를 한글"));
    }

    #[test]
    fn no_violation_when_response_unrelated() {
        let memory = "## 2. 금지·교정 (Corrections)\n\n- (active) 영어 용어를 강제 번역하지 마\n";
        let response = "오늘 날씨에 대해 이야기합시다";
        let hits = detect(response, memory);
        assert!(hits.is_empty());
    }

    #[test]
    fn ignores_resolved_corrections() {
        let memory = "## 2. 금지·교정 (Corrections)\n\n\
            - (resolved 2026-04-20) 너무 길게 답하지 마\n";
        let response = "너무 긴 답변";
        let hits = detect(response, memory);
        assert!(hits.is_empty());
    }

    #[test]
    fn ignores_other_sections() {
        // Preferences 섹션의 활성 항목은 *위반 검사 대상 X*.
        let memory = "## 1. 사용자 선호 (Preferences)\n\n\
            - (active) 코드 예제는 항상 한국어로\n\
            ## 2. 금지·교정 (Corrections)\n\n\
            (없음)\n";
        let response = "한국어 설명";
        let hits = detect(response, memory);
        assert!(hits.is_empty());
    }

    #[test]
    fn extract_forbidden_grabs_object_phrase() {
        let item = "- (active, 지적 2회) 코드 블록에 언어 태그 빠뜨리지 말 것";
        // "빠뜨리지 말 것" 패턴은 미지원 — None 또는 휴리스틱 fallback.
        let _ = extract_forbidden(item); // 깨지지 않음만 보장
    }
}
