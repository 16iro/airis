// v0.6.x PR (D-108) — Passage cleaning (검색 → 리랭크 사이 청크 정제).
//
// WeKnora의 "passage cleaning for rerank optimization"을 airis 제약(로컬·단일 바이너리·
// 외부 의존성 0)에 맞춰 *규칙 기반*으로 이식한 모듈. 검색으로 뽑힌 청크 본문에 섞인
// 파싱 잔해(반복 머리말/꼬리말, 페이지 번호, 깨진 공백, PDF 하이프네이션)를 제거해
//   (1) cross-encoder 리랭크 점수를 흐리는 노이즈를 줄이고,
//   (2) LLM 토큰 예산에서 잡음이 차지하는 자리를 본문에 돌려준다.
//
// 설계 원칙 (SUGGESTION 결정 — "보수적으로 시작"):
//   * *확실한 잡음만* 제거. 본문일 가능성이 조금이라도 있으면 남긴다.
//   * 메타데이터(page·section_path·token_count)는 *건드리지 않는다* — text만 정제.
//   * 모델 호출 X, 외부 crate 추가 X (순수 std + char 연산).
//
// 두 층위:
//   1. `clean_passage(text)`           — 단일 청크. 항상 안전한 정제만 (페이지번호 줄,
//                                         control 문자, 깨진 공백, 영어 하이프네이션).
//   2. `clean_passages(texts)`         — 후보 집합 전체. 1)에 더해 *여러 청크에 반복 등장*
//                                         하는 짧은 줄(=머리말/꼬리말 보일러플레이트)을
//                                         교차 검출해 제거. 반복은 보일러플레이트의 강한
//                                         증거이므로 본문 손실 위험이 낮다.

#![allow(dead_code)]

use std::collections::HashMap;

/// 보일러플레이트로 판정하기 위한 최소 등장 청크 수. 반복이 이보다 적으면 본문일 수
/// 있으므로 제거하지 않는다 (보수적 하한).
const BOILERPLATE_MIN_OCCURRENCES: usize = 3;

/// 보일러플레이트 후보 줄의 최대 길이(문자). 머리말/꼬리말은 보통 짧다 — 이보다 길면
/// 반복돼도 본문 단락일 가능성이 커 제거하지 않는다.
const BOILERPLATE_MAX_LINE_CHARS: usize = 60;

/// 단일 청크 정제 — 항상 안전한 규칙만 적용.
///
/// 적용 순서:
///   1. 줄 단위로 control 문자(form feed 등) 제거 + 내부 연속 공백 1칸으로 축약.
///   2. 페이지 번호만 있는 줄(`42`, `- 42 -`, `p. 42`, `42 페이지`)을 통째로 제거.
///   3. 연속 빈 줄을 최대 1줄로 축약.
///   4. 영어 줄바꿈 하이프네이션(`exam-\nple` → `example`) 결합.
///   5. 양 끝 공백 정리.
pub fn clean_passage(text: &str) -> String {
    let kept = filter_lines(text, None);
    finalize(kept)
}

/// 후보 집합 정제 — 단일 청크 정제 + 교차 보일러플레이트 제거.
///
/// 입력 순서를 보존해 같은 길이의 Vec를 반환한다. 호출 측은 결과를 원래 청크의 text에
/// 다시 매핑한다 (메타데이터는 그대로).
pub fn clean_passages(texts: &[String]) -> Vec<String> {
    if texts.is_empty() {
        return Vec::new();
    }
    let boilerplate = detect_boilerplate(texts);
    texts
        .iter()
        .map(|t| finalize(filter_lines(t, Some(&boilerplate))))
        .collect()
}

/// 줄 단위 1차 필터 — control 제거 + 공백 축약 + 페이지번호/보일러플레이트 줄 제거.
/// `boilerplate`가 Some이면 그 집합에 *정규화 형태*가 포함된 줄도 제거.
fn filter_lines(text: &str, boilerplate: Option<&std::collections::HashSet<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = collapse_inline_whitespace(&strip_control_chars(raw));
        let line = line.trim_end();
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        if is_page_number_line(line) {
            continue;
        }
        if let Some(bp) = boilerplate {
            if bp.contains(&normalize_for_match(line)) {
                continue;
            }
        }
        out.push(line.to_string());
    }
    out
}

/// 정제된 줄 Vec → 최종 문자열. 빈 줄 축약 + 하이프네이션 결합 + 양끝 trim.
fn finalize(lines: Vec<String>) -> String {
    let collapsed = collapse_blank_lines(&lines);
    let joined = join_english_hyphenation(&collapsed);
    joined.trim().to_string()
}

/// 탭을 제외한 control 문자(form feed `\u{C}`, vertical tab, NUL 등) 제거.
fn strip_control_chars(s: &str) -> String {
    s.chars()
        .filter(|c| *c == '\t' || !c.is_control())
        .collect()
}

/// 내부 연속 공백/탭을 1칸으로 축약. (줄바꿈은 호출 측이 줄 단위로 다루므로 여기엔 없음.)
fn collapse_inline_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        let is_ws = c == ' ' || c == '\t';
        if is_ws {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// 연속 빈 줄을 최대 1줄로 축약하고 줄을 `\n`으로 결합.
fn collapse_blank_lines(lines: &[String]) -> String {
    let mut out_lines: Vec<&str> = Vec::with_capacity(lines.len());
    let mut prev_blank = false;
    for line in lines {
        let blank = line.trim().is_empty();
        if blank {
            if prev_blank {
                continue;
            }
            prev_blank = true;
        } else {
            prev_blank = false;
        }
        out_lines.push(line.as_str());
    }
    out_lines.join("\n")
}

/// 영어 줄바꿈 하이프네이션 결합 — `[a-z]-\n[a-z]` 패턴에서 `-\n`을 제거해 단어를 잇는다.
///
/// 한국어엔 하이프네이션이 없고, 대문자/숫자가 끼면 진짜 하이픈(예: `UTF-8`, `2026-`)일
/// 수 있어 *소문자 사이*로만 한정 — 보수적.
fn join_english_hyphenation(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // 패턴: 소문자 + '-' + '\n' + (공백*) + 소문자.
        if c == '-' && i > 0 && chars[i - 1].is_ascii_lowercase() {
            // '-' 다음이 '\n' 인지 검사 (\n 앞뒤 공백 허용).
            let mut j = i + 1;
            // '-' 와 '\n' 사이 공백 skip.
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if j < chars.len() && chars[j] == '\n' {
                let mut k = j + 1;
                while k < chars.len() && chars[k] == ' ' {
                    k += 1;
                }
                if k < chars.len() && chars[k].is_ascii_lowercase() {
                    // 하이픈·줄바꿈·공백 제거하고 단어 결합.
                    i = k;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 페이지 번호만 있는 줄인지. 보수적 — 아래 중 하나에 정확히 맞을 때만 true:
///   * 순수 숫자 1~4자리 (`42`, `128`)
///   * 대시로 감싼 숫자 (`- 42 -`, `— 42 —`, `–42–`)
///   * 페이지 키워드 + 숫자 (`p. 42`, `p42`, `page 42`, `42 페이지`, `42쪽`, `- 12 페이지 -`)
fn is_page_number_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }

    // 1) 순수 숫자 1~4자리.
    if is_short_ascii_number(t) {
        return true;
    }

    // 2) 대시(-, —, –)로 감싼 형태 → 안쪽 토큰만 떼서 재검사.
    let dashes: &[char] = &['-', '—', '–', '·', '•'];
    let inner = t.trim_matches(|c: char| dashes.contains(&c) || c == ' ');
    if inner != t && !inner.is_empty() {
        // 안쪽이 순수 숫자거나 페이지 키워드+숫자면 페이지 줄.
        if is_short_ascii_number(inner) || is_page_keyword_number(inner) {
            return true;
        }
    }

    // 3) 페이지 키워드 + 숫자.
    is_page_keyword_number(t)
}

/// 1~4자리 ASCII 숫자만으로 구성됐는지.
fn is_short_ascii_number(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 4
        && s.chars().all(|c| c.is_ascii_digit())
}

/// 페이지 키워드(`p`, `p.`, `page`, `페이지`, `쪽`, `면`) + 숫자 조합인지.
fn is_page_keyword_number(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let lower = lower.trim();
    // 영어 prefix 형태: "page 42", "p. 42", "p42".
    for kw in ["page", "p.", "p"] {
        if let Some(rest) = lower.strip_prefix(kw) {
            let rest = rest.trim();
            if is_short_ascii_number(rest) {
                return true;
            }
        }
    }
    // 한국어 suffix 형태: "42 페이지", "42쪽", "42 면".
    for kw in ["페이지", "쪽", "면"] {
        if let Some(head) = s.trim().strip_suffix(kw) {
            if is_short_ascii_number(head.trim()) {
                return true;
            }
        }
        // prefix 형태 "페이지 42"도 허용.
        if let Some(tail) = s.trim().strip_prefix(kw) {
            if is_short_ascii_number(tail.trim()) {
                return true;
            }
        }
    }
    false
}

/// 줄 비교용 정규화 — 공백 축약 + 양끝 trim. (대소문자는 유지 — 머리말 대소문자 일관.)
fn normalize_for_match(line: &str) -> String {
    collapse_inline_whitespace(line).trim().to_string()
}

/// 여러 청크에 반복 등장하는 짧은 줄(=머리말/꼬리말)을 검출.
///
/// 규칙(보수적):
///   * 정규화 후 길이 ≤ BOILERPLATE_MAX_LINE_CHARS 인 줄만 후보.
///   * *서로 다른* 청크 ≥ BOILERPLATE_MIN_OCCURRENCES 개에 등장해야 함 (한 청크 내 반복은
///     카운트 1회 — 같은 청크 안 중복은 보일러플레이트 증거가 약하다).
///   * 문장 종결부호로 끝나는 줄은 본문일 확률이 높아 제외.
fn detect_boilerplate(texts: &[String]) -> std::collections::HashSet<String> {
    let mut doc_count: HashMap<String, usize> = HashMap::new();
    for text in texts {
        // 한 청크 안에서는 set으로 — 같은 줄 중복 등장은 1회만 카운트.
        let mut seen_in_chunk: std::collections::HashSet<String> = std::collections::HashSet::new();
        for raw in text.split('\n') {
            let norm = normalize_for_match(&collapse_inline_whitespace(&strip_control_chars(raw)));
            if norm.is_empty() || norm.chars().count() > BOILERPLATE_MAX_LINE_CHARS {
                continue;
            }
            if ends_with_terminator(&norm) {
                continue;
            }
            seen_in_chunk.insert(norm);
        }
        for line in seen_in_chunk {
            *doc_count.entry(line).or_insert(0) += 1;
        }
    }
    doc_count
        .into_iter()
        .filter(|(_, n)| *n >= BOILERPLATE_MIN_OCCURRENCES)
        .map(|(line, _)| line)
        .collect()
}

/// 문장 종결부호로 끝나는지 (보일러플레이트 후보 제외용 — 본문 보호).
fn ends_with_terminator(s: &str) -> bool {
    matches!(
        s.chars().last(),
        Some('.') | Some('!') | Some('?') | Some('。') | Some('！') | Some('？')
    ) || s.ends_with("다") // 한국어 종결어미 흔한 케이스 — 본문 보호.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_passage_collapses_inline_whitespace() {
        let input = "Rust   ownership    모델은\t\t안전합니다";
        assert_eq!(clean_passage(input), "Rust ownership 모델은 안전합니다");
    }

    #[test]
    fn clean_passage_collapses_blank_lines() {
        let input = "첫 문단\n\n\n\n둘째 문단";
        assert_eq!(clean_passage(input), "첫 문단\n\n둘째 문단");
    }

    #[test]
    fn clean_passage_drops_pure_page_number_line() {
        let input = "본문 시작\n42\n본문 끝";
        assert_eq!(clean_passage(input), "본문 시작\n본문 끝");
    }

    #[test]
    fn clean_passage_drops_dash_wrapped_page_number() {
        assert_eq!(clean_passage("앞\n- 128 -\n뒤"), "앞\n뒤");
        assert_eq!(clean_passage("앞\n— 12 —\n뒤"), "앞\n뒤");
    }

    #[test]
    fn clean_passage_drops_page_keyword_lines() {
        assert_eq!(clean_passage("a\np. 42\nb"), "a\nb");
        assert_eq!(clean_passage("a\nPage 7\nb"), "a\nb");
        assert_eq!(clean_passage("a\n42 페이지\nb"), "a\nb");
        assert_eq!(clean_passage("a\n128쪽\nb"), "a\nb");
    }

    #[test]
    fn clean_passage_keeps_numbered_list_marker() {
        // "1." 은 페이지번호로 오인하지 않는다 (목록 마커 보호).
        let input = "1. 첫째 항목\n2. 둘째 항목";
        assert_eq!(clean_passage(input), "1. 첫째 항목\n2. 둘째 항목");
    }

    #[test]
    fn clean_passage_keeps_number_inside_sentence() {
        // 본문 안 숫자는 줄 전체가 숫자가 아니므로 보존.
        let input = "이 장은 42페이지에서 시작합니다";
        assert_eq!(clean_passage(input), "이 장은 42페이지에서 시작합니다");
    }

    #[test]
    fn clean_passage_does_not_drop_large_numbers() {
        // 5자리 이상은 페이지 번호로 보지 않음 (연도·코드 가능성).
        let input = "본문\n20260608\n본문";
        assert_eq!(clean_passage(input), "본문\n20260608\n본문");
    }

    #[test]
    fn clean_passage_strips_form_feed() {
        let input = "앞\u{000C}뒤";
        assert_eq!(clean_passage(input), "앞뒤");
    }

    #[test]
    fn clean_passage_joins_english_hyphenation() {
        let input = "this is an exam-\nple of hyphenation";
        assert_eq!(clean_passage(input), "this is an example of hyphenation");
    }

    #[test]
    fn clean_passage_does_not_join_across_uppercase_or_digits() {
        // 진짜 하이픈(코드·식별자)은 보존.
        let input = "UTF-\n8 인코딩";
        // 대문자/숫자라 결합 안 함 → 줄바꿈은 그대로 유지.
        assert!(clean_passage(input).contains("UTF-\n8") || clean_passage(input).contains("UTF-"));
    }

    #[test]
    fn clean_passage_trims_outer_whitespace() {
        assert_eq!(clean_passage("\n\n  본문  \n\n"), "본문");
    }

    #[test]
    fn clean_passages_strips_repeated_header() {
        // 같은 머리말이 3개 청크에 반복 → 보일러플레이트로 제거.
        let texts = vec![
            "제3장 운영체제\n프로세스는 실행 단위입니다".to_string(),
            "제3장 운영체제\n스레드는 더 가벼운 단위입니다".to_string(),
            "제3장 운영체제\n스케줄러가 CPU를 배분합니다".to_string(),
        ];
        let out = clean_passages(&texts);
        assert_eq!(out.len(), 3);
        for o in &out {
            assert!(!o.contains("제3장 운영체제"), "반복 머리말 제거: {o}");
        }
        assert!(out[0].contains("프로세스는 실행 단위입니다"));
    }

    #[test]
    fn clean_passages_keeps_line_repeated_only_twice() {
        // 2개 청크에만 등장하면 하한(3) 미만 → 보존 (본문일 수 있음).
        let texts = vec![
            "공통 줄\n첫 청크 본문".to_string(),
            "공통 줄\n둘째 청크 본문".to_string(),
        ];
        let out = clean_passages(&texts);
        assert!(out[0].contains("공통 줄"), "2회 반복은 제거하지 않음");
    }

    #[test]
    fn clean_passages_keeps_repeated_sentence_like_line() {
        // 반복돼도 문장 종결로 끝나면 본문으로 보호.
        let texts = vec![
            "이것은 본문 문장입니다.\nA".to_string(),
            "이것은 본문 문장입니다.\nB".to_string(),
            "이것은 본문 문장입니다.\nC".to_string(),
        ];
        let out = clean_passages(&texts);
        assert!(
            out[0].contains("이것은 본문 문장입니다."),
            "문장 종결 줄은 반복돼도 보존"
        );
    }

    #[test]
    fn clean_passages_empty_input() {
        let out = clean_passages(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn clean_passages_applies_single_chunk_rules_too() {
        // 보일러플레이트가 없어도 단일 청크 규칙(페이지번호·공백)은 적용.
        let texts = vec!["본문   시작\n42\n본문 끝".to_string()];
        let out = clean_passages(&texts);
        assert_eq!(out[0], "본문 시작\n본문 끝");
    }

    #[test]
    fn page_number_detection_unit() {
        assert!(is_page_number_line("42"));
        assert!(is_page_number_line("- 42 -"));
        assert!(is_page_number_line("p. 42"));
        assert!(is_page_number_line("Page 7"));
        assert!(is_page_number_line("128쪽"));
        assert!(!is_page_number_line("1."));
        assert!(!is_page_number_line("20260608"));
        assert!(!is_page_number_line("본문 42 페이지 참조"));
    }
}
