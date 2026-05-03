// 섹션 제목 → path 슬러그 변환.
//
// 정책:
//   * "Chapter 4" → "Ch04"   (영문 챕터 표기)
//   * "제 4 장"   → "Ch04"   (한글 장)
//   * 일반 제목  → "§{title}" (앞에 § + 제목 sanitize)
//   * sanitize: 공백·특수문자 → 하이픈, 연속 하이픈 압축, trim
//
// 이렇게 하면 인용 라벨도 자연스럽고 (`Ch04 §State`) 충돌 가능성 낮다.

/// 챕터 표기에서 챕터 번호 추출. 매치 실패 시 None.
///
/// 인식하는 패턴(앞쪽부터):
///   * "Chapter 12" / "chapter 12" / "Ch. 12" / "Ch12"
///   * "제 12 장" / "제12장" / "12장"
///   * 단순 숫자만 ("12") 도 챕터 번호로 인정
pub fn parse_chapter_number(title: &str) -> Option<u32> {
    let trimmed = title.trim();
    let lower = trimmed.to_lowercase();

    // 영문 변형
    for prefix in ["chapter ", "ch. ", "ch.", "ch "] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if let Some(n) = leading_u32(rest.trim()) {
                return Some(n);
            }
        }
    }
    if let Some(rest) = lower.strip_prefix("ch") {
        if let Some(n) = leading_u32(rest) {
            return Some(n);
        }
    }

    // 한글 변형 — "제 N 장" / "제N장" / "N장"
    let zero_width_stripped = trimmed.replace('\u{200b}', "");
    if let Some(rest) = zero_width_stripped.strip_prefix('제') {
        let inner = rest.trim_start();
        if let Some(n) = leading_u32(inner) {
            return Some(n);
        }
    }
    if let Some(n) = leading_u32(&zero_width_stripped) {
        // 끝이 "장"으로 끝나거나 그냥 숫자만이면 인정
        let after = &zero_width_stripped[n.to_string().len()..];
        let after_t = after.trim_start();
        if after_t.is_empty() || after_t.starts_with('장') {
            return Some(n);
        }
    }

    None
}

fn leading_u32(s: &str) -> Option<u32> {
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            break;
        }
    }
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

/// 챕터 번호를 표준 path token으로. 4 → "Ch04".
pub fn chapter_path(number: u32) -> String {
    format!("Ch{number:02}")
}

/// 섹션 제목을 § path token으로. 공백·특수문자 정리.
pub fn section_path(title: &str) -> String {
    let cleaned = sanitize_title(title);
    if cleaned.is_empty() {
        "§untitled".to_string()
    } else {
        format!("§{cleaned}")
    }
}

/// 디스플레이용 라벨. `Ch04` + ` ` + `§State` 식으로 호출자가 조합.
pub fn display_label_from_path(path: &str) -> String {
    // path = "Ch04/§State" → "Ch04 §State"
    path.replace('/', " ")
}

/// 동일 path 충돌 시 -2, -3 ... suffix를 붙인다.
/// `existing`엔 이미 사용 중인 path들이 들어가 있어야 한다.
pub fn dedupe_path(base: &str, existing: &std::collections::HashSet<String>) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// 섹션 제목 → path-safe 토큰. 한글·영문·숫자 그대로 두고
/// 공백·구두점·기타 비-단어 문자는 모두 하이픈으로 압축. URL-safe보단 *디스플레이 호환* 우선.
fn sanitize_title(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.trim().chars() {
        if c.is_alphanumeric() || is_cjk(c) {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn is_cjk(c: char) -> bool {
    let n = c as u32;
    // 한글 음절 (가-힣) + 한자 기본 + 가나
    (0xAC00..=0xD7A3).contains(&n)
        || (0x4E00..=0x9FFF).contains(&n)
        || (0x3040..=0x30FF).contains(&n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_english_chapter_variants() {
        assert_eq!(parse_chapter_number("Chapter 4"), Some(4));
        assert_eq!(parse_chapter_number("Chapter 12: State"), Some(12));
        assert_eq!(parse_chapter_number("Ch. 7"), Some(7));
        assert_eq!(parse_chapter_number("Ch7"), Some(7));
        assert_eq!(parse_chapter_number("ch 9"), Some(9));
    }

    #[test]
    fn parses_korean_chapter_variants() {
        assert_eq!(parse_chapter_number("제 4 장"), Some(4));
        assert_eq!(parse_chapter_number("제4장"), Some(4));
        assert_eq!(parse_chapter_number("4장"), Some(4));
        assert_eq!(parse_chapter_number("12장 — 상태 관리"), Some(12));
    }

    #[test]
    fn rejects_non_chapter_titles() {
        assert_eq!(parse_chapter_number("Introduction"), None);
        assert_eq!(parse_chapter_number("머리말"), None);
        assert_eq!(parse_chapter_number(""), None);
    }

    #[test]
    fn chapter_path_pads_to_two_digits() {
        assert_eq!(chapter_path(4), "Ch04");
        assert_eq!(chapter_path(12), "Ch12");
        assert_eq!(chapter_path(100), "Ch100");
    }

    #[test]
    fn section_path_handles_korean_and_special() {
        assert_eq!(section_path("State 관리"), "§State-관리");
        assert_eq!(section_path("§hello"), "§hello");
        assert_eq!(section_path("4.1 The Machine"), "§4-1-The-Machine");
        assert_eq!(section_path(""), "§untitled");
    }

    #[test]
    fn section_path_preserves_unicode() {
        assert_eq!(section_path("소유권과 차용"), "§소유권과-차용");
    }

    #[test]
    fn dedupe_path_appends_suffix_on_collision() {
        let mut existing = std::collections::HashSet::new();
        existing.insert("§Summary".to_string());
        existing.insert("§Summary-2".to_string());
        assert_eq!(dedupe_path("§Summary", &existing), "§Summary-3");
        assert_eq!(dedupe_path("§Other", &existing), "§Other");
    }

    #[test]
    fn display_label_replaces_slashes() {
        assert_eq!(display_label_from_path("Ch04/§State"), "Ch04 §State");
        assert_eq!(display_label_from_path("Ch01"), "Ch01");
    }
}
