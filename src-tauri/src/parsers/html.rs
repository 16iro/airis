// HTML 파서 — ammonia로 sanitize 후 scraper로 heading 추출.
//
// 보안 정책 (security.md):
//   * <script>·<iframe>·on* 이벤트 핸들러 모두 제거 (ammonia 기본).
//   * 검색·인덱싱 단계에서는 sanitized HTML만 본다. 뷰어(PR 12)도 같은 sanitized 본문 사용.
//
// 구조 정책:
//   * h1 = L2 챕터, h2~h6 = L3 섹션. (Markdown과 동일)
//   * h1 부재 시 첫 h2가 챕터로 승격.
//   * 각 섹션의 body는 *해당 heading element 다음 ~ 다음 같은-레벨-이상 heading 직전*의 텍스트.
//     테이블·리스트·코드 등은 plain text로 평탄화 (인덱싱·검색 용도).

use std::collections::HashSet;

use scraper::{Html, Selector};

use crate::parsers::slug::{chapter_path, dedupe_path, parse_chapter_number, section_path};
use crate::parsers::types::{Section, SectionLevel};

/// HTML 원본을 받아 sanitize + 섹션 추출. (sanitize 결과는 별도 반환 X — 본문에만 영향)
pub fn parse(source: &str) -> Vec<Section> {
    let cleaned = ammonia::clean(source);
    let doc = Html::parse_document(&cleaned);
    let headings = collect_headings(&doc);
    if headings.is_empty() {
        return Vec::new();
    }
    build_sections(&doc, &headings)
}

/// sanitize된 HTML을 그대로 돌려준다. 뷰어가 그대로 iframe/innerHTML에 띄울 때 사용.
pub fn sanitize(source: &str) -> String {
    ammonia::clean(source)
}

#[derive(Debug, Clone)]
struct RawHeading {
    /// 1=h1 ... 6=h6.
    level: u8,
    title: String,
    /// scraper의 NodeId 깊이를 1차원 indices로 — order는 DOM traversal 순서.
    order: usize,
}

fn collect_headings(doc: &Html) -> Vec<RawHeading> {
    let selector =
        Selector::parse("h1, h2, h3, h4, h5, h6").expect("static heading selector parses");
    let mut out = Vec::new();
    for (idx, element) in doc.select(&selector).enumerate() {
        let tag = element.value().name();
        let level = tag
            .strip_prefix('h')
            .and_then(|n| n.parse::<u8>().ok())
            .unwrap_or(6);
        let title: String = element.text().collect::<String>().trim().to_string();
        if !title.is_empty() {
            out.push(RawHeading {
                level,
                title,
                order: idx,
            });
        }
    }
    out
}

fn build_sections(doc: &Html, headings: &[RawHeading]) -> Vec<Section> {
    let chapter_threshold = if headings.iter().any(|h| h.level == 1) {
        1
    } else {
        2
    };

    // 섹션의 body — heading n의 element 직후 ~ heading n+1 element 직전의 *text*.
    // 단순 구현: 모든 heading element의 *컨텐츠 영역*을 따로 추출.
    // 재구현: 본문 전체 텍스트를 한 번에 뽑아 heading 위치로 분할.
    let bodies = compute_bodies(doc, headings);

    let mut sections = Vec::new();
    let mut used_paths: HashSet<String> = HashSet::new();
    let mut current_chapter_path: Option<String> = None;
    let mut chapter_counter: u32 = 0;

    for (h, body) in headings.iter().zip(bodies.iter()) {
        let level = if h.level <= chapter_threshold {
            SectionLevel::Chapter
        } else {
            SectionLevel::Section
        };

        let (path, parent_path) = match level {
            SectionLevel::Chapter => {
                chapter_counter += 1;
                let n = parse_chapter_number(&h.title).unwrap_or(chapter_counter);
                let base = chapter_path(n);
                let unique = dedupe_path(&base, &used_paths);
                current_chapter_path = Some(unique.clone());
                (unique, None)
            }
            SectionLevel::Section => {
                let token = section_path(&h.title);
                let prefixed = match &current_chapter_path {
                    Some(c) => format!("{c}/{token}"),
                    None => token,
                };
                let unique = dedupe_path(&prefixed, &used_paths);
                (unique, current_chapter_path.clone())
            }
        };
        used_paths.insert(path.clone());

        sections.push(Section {
            path,
            display_label: h.title.clone(),
            level,
            parent_path,
            page: None,
            body: body.clone(),
        });
    }
    sections
}

/// heading 사이 본문 텍스트 추출.
/// 알고리즘:
///   1) 전체 body의 텍스트(노드 순서대로 평탄화)를 뽑음.
///   2) heading 텍스트들을 *문서 등장 순서대로* 분할 키로 사용 — 같은 heading이 N번 나오면
///      n번째 등장이 n번째 섹션의 시작.
fn compute_bodies(doc: &Html, headings: &[RawHeading]) -> Vec<String> {
    let body_sel = Selector::parse("body").expect("body selector parses");
    let root = doc.select(&body_sel).next();

    // 전체 텍스트
    let full_text = match root {
        Some(b) => b.text().collect::<Vec<_>>().join("\n"),
        None => doc.root_element().text().collect::<Vec<_>>().join("\n"),
    };

    // heading 제목들을 등장 순서대로 분할.
    let mut bodies = Vec::with_capacity(headings.len());
    let mut cursor = 0usize;
    let normalized_text = full_text.as_str();

    for (i, h) in headings.iter().enumerate() {
        let heading_pos = match find_after(normalized_text, &h.title, cursor) {
            Some(p) => p,
            None => {
                bodies.push(String::new());
                continue;
            }
        };
        let body_start = heading_pos + h.title.len();

        // 다음 heading 위치 찾기
        let body_end = if let Some(next) = headings.get(i + 1) {
            find_after(normalized_text, &next.title, body_start).unwrap_or(normalized_text.len())
        } else {
            normalized_text.len()
        };

        let body = normalized_text
            .get(body_start..body_end)
            .unwrap_or("")
            .trim()
            .to_string();
        bodies.push(body);
        cursor = body_start;
    }

    bodies
}

/// `from` 이후 `needle`을 찾는다. 매치되면 byte 시작 offset, 없으면 None.
fn find_after(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..].find(needle).map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_h1_h2_hierarchy() {
        let html = r#"<!doctype html><html><body>
            <h1>Chapter 1: Intro</h1>
            <p>intro paragraph</p>
            <h2>Background</h2>
            <p>bg para</p>
            <h2>Motivation</h2>
            <p>mot para</p>
            <h1>Chapter 2: Deep</h1>
            <p>deep para</p>
        </body></html>"#;
        let sections = parse(html);
        assert_eq!(sections.len(), 4);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert_eq!(sections[1].path, "Ch01/§Background");
        assert_eq!(sections[1].parent_path.as_deref(), Some("Ch01"));
        assert_eq!(sections[3].path, "Ch02");
    }

    #[test]
    fn body_contains_paragraph_text() {
        let html = r#"<body><h1>Ch1</h1><p>본문입니다</p><h2>S</h2><p>섹션 본문</p></body>"#;
        let sections = parse(html);
        assert!(sections[0].body.contains("본문입니다"));
        assert!(sections[1].body.contains("섹션 본문"));
    }

    #[test]
    fn sanitize_removes_scripts_and_handlers() {
        let html =
            r#"<body><h1 onclick="alert(1)">Title</h1><script>evil()</script><p>safe</p></body>"#;
        let cleaned = sanitize(html);
        assert!(!cleaned.contains("script"));
        assert!(!cleaned.contains("onclick"));
        assert!(cleaned.contains("Title"));
        assert!(cleaned.contains("safe"));
    }

    #[test]
    fn sanitize_preserves_formatting_we_render() {
        let html = r#"<p><strong>굵게</strong> + <em>기울임</em> + <code>코드</code></p>"#;
        let cleaned = sanitize(html);
        assert!(cleaned.contains("<strong>"));
        assert!(cleaned.contains("<em>"));
        assert!(cleaned.contains("<code>"));
    }

    #[test]
    fn promotes_first_h2_when_no_h1() {
        let html = r#"<body><h2>First</h2><p>p1</p><h2>Second</h2><p>p2</p></body>"#;
        let sections = parse(html);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert_eq!(sections[1].level, SectionLevel::Chapter);
    }

    #[test]
    fn empty_or_no_headings_returns_empty() {
        assert!(parse("").is_empty());
        assert!(parse("<body><p>no headings</p></body>").is_empty());
    }
}
