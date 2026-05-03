// F10.3 발화 트리거 감지.
//
// 사용자 발화에서 *Memory.md에 추가할 후보*를 잡아내는 정규식 매처.
// 결정 (PR 15): 트리거 패턴은 *코드 박음* (Rust static). 사용자 커스터마이징(triggers.toml)은 v0.3+.
//
// 분류 (5섹션 매핑):
//   * preference  → "이제부터~", "X 스타일로 가자", "X 우선" 류
//   * correction  → "그러지 마", "X 하지 말아줘", "X 빼줘" 류
//   * goal        → "X까지 끝내고 싶어", "X가 목표야" 류
//
// chat_send 후 *사용자 메시지*에 대해 1회 호출. 결과는 memory:trigger event로 emit.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerKind {
    Preference,
    Correction,
    Goal,
}

impl TriggerKind {
    pub fn section_heading(&self) -> &'static str {
        match self {
            Self::Preference => "## 1. 사용자 선호 (Preferences)",
            Self::Correction => "## 2. 금지·교정 (Corrections)",
            Self::Goal => "## 5. 학습 목표 (Goals)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerHit {
    pub kind: TriggerKind,
    /// 매치된 사용자 발화 원문 (전체 메시지가 아닌 매치 *주변*).
    pub matched_text: String,
    /// Memory에 append할 항목 본문 (status tag 제외 — 호출자가 `(active, since 시각)` prefix).
    pub suggested_entry: String,
}

struct PatternEntry {
    regex_src: &'static str,
    kind: TriggerKind,
}

/// 한국어·영어 혼합 패턴. 부정확 매치 줄이려고 *어구 시작 또는 공백 뒤*에 anchor.
const PATTERNS: &[PatternEntry] = &[
    // Preferences
    PatternEntry {
        regex_src: r"(?m)(?:^|\s)이제부터\s+(.+?)(?:[.,!?]|$)",
        kind: TriggerKind::Preference,
    },
    PatternEntry {
        regex_src: r"(?m)(?:^|\s)앞으로(?:는)?\s+(.+?)(?:[.,!?]|$)",
        kind: TriggerKind::Preference,
    },
    PatternEntry {
        regex_src: r"(?m)(.+?)\s*스타일로\s+(?:가자|해줘|부탁)",
        kind: TriggerKind::Preference,
    },
    // Corrections
    PatternEntry {
        regex_src: r"(?m)(.+?)\s*하지\s*말아?\s*줘",
        kind: TriggerKind::Correction,
    },
    PatternEntry {
        regex_src: r"(?m)그러지\s*마",
        kind: TriggerKind::Correction,
    },
    PatternEntry {
        regex_src: r"(?m)(.+?)\s*빼줘",
        kind: TriggerKind::Correction,
    },
    // Goals
    PatternEntry {
        regex_src: r"(?m)(.+?)(?:까지|을|를)\s*(?:끝내고\s*싶|목표야|마치고\s*싶)",
        kind: TriggerKind::Goal,
    },
];

fn compiled() -> &'static [(Regex, TriggerKind)] {
    static CACHE: OnceLock<Vec<(Regex, TriggerKind)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        PATTERNS
            .iter()
            .filter_map(|p| Regex::new(p.regex_src).ok().map(|r| (r, p.kind)))
            .collect()
    })
}

/// 사용자 발화에서 트리거 감지 — 매치된 모든 hit 반환.
/// 같은 위치 중복 hit는 제거 (가장 먼저 매치된 패턴 우선).
pub fn detect(text: &str) -> Vec<TriggerHit> {
    let mut hits: Vec<TriggerHit> = Vec::new();
    let mut covered: Vec<(usize, usize)> = Vec::new();

    for (re, kind) in compiled() {
        for m in re.captures_iter(text) {
            let full = m.get(0).expect("group 0 always exists");
            let span = (full.start(), full.end());
            if covered.iter().any(|(s, e)| !(span.1 <= *s || span.0 >= *e)) {
                continue;
            }
            covered.push(span);

            let matched_text = full.as_str().trim().to_string();
            // 캡처 그룹 1이 있으면 그걸 *suggested entry*로. 없으면 전체.
            let suggested_entry = m
                .get(1)
                .map(|c| c.as_str().trim().to_string())
                .unwrap_or_else(|| matched_text.clone());
            if suggested_entry.is_empty() {
                continue;
            }
            hits.push(TriggerHit {
                kind: *kind,
                matched_text,
                suggested_entry,
            });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_preference_from_imeobuteo() {
        let hits = detect("이제부터 코드 예제를 컴파일 가능한 형태로 줘.");
        assert!(!hits.is_empty(), "should detect 이제부터 trigger");
        assert!(hits.iter().any(|h| h.kind == TriggerKind::Preference));
        assert!(hits[0].suggested_entry.contains("코드 예제"));
    }

    #[test]
    fn detects_correction_from_haji_mara() {
        let hits = detect("영어 용어를 한글로 강제 번역하지 말아줘");
        assert!(hits.iter().any(|h| h.kind == TriggerKind::Correction));
    }

    #[test]
    fn detects_geureoji_ma() {
        let hits = detect("그러지 마. 그냥 영어 그대로 둬");
        assert!(hits.iter().any(|h| h.kind == TriggerKind::Correction));
    }

    #[test]
    fn detects_goal_from_kkaji_finish() {
        let hits = detect("Ch09까지 끝내고 싶어");
        assert!(hits.iter().any(|h| h.kind == TriggerKind::Goal));
    }

    #[test]
    fn no_false_positives_on_neutral_text() {
        let hits = detect("Rust의 소유권에 대해 설명해줘.");
        assert!(hits.is_empty(), "neutral query should not trigger");
    }

    #[test]
    fn deduplicates_overlapping_hits() {
        // 한 어구가 여러 패턴에 동시 매치돼도 첫 패턴만.
        let hits = detect("이제부터 짧게 답해줘");
        // 너무 많은 hit가 나오면 noise. 최대 1~2개.
        assert!(hits.len() <= 2, "got {} hits", hits.len());
    }

    #[test]
    fn section_heading_maps_to_5section_constants() {
        assert!(TriggerKind::Preference
            .section_heading()
            .contains("Preferences"));
        assert!(TriggerKind::Correction
            .section_heading()
            .contains("Corrections"));
        assert!(TriggerKind::Goal.section_heading().contains("Goals"));
    }
}
