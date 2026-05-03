// Markdown 파서 — pulldown-cmark 기반 heading 추적.
//
// 정책:
//   * h1 = L2 챕터, h2~h6 = L3 섹션. 사용자가 h1 없이 h2부터 시작하면 첫 h2가 챕터로 승격.
//   * 각 섹션의 body는 *해당 heading 직후 ~ 다음 같은-레벨-이상 heading 직전*의 raw 마크다운.
//   * 코드 블록·인용 등 중첩 구조는 본문에 그대로 유지 (검색·임베딩 단계에서 정규화).

use std::collections::HashSet;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::parsers::slug::{chapter_path, dedupe_path, parse_chapter_number, section_path};
use crate::parsers::types::{Section, SectionLevel};

pub fn parse(source: &str) -> Vec<Section> {
    let headings = collect_headings(source);
    if headings.is_empty() {
        return Vec::new();
    }

    let body_ranges = compute_body_ranges(&headings, source.len());
    build_sections(source, &headings, &body_ranges)
}

#[derive(Debug, Clone)]
struct RawHeading {
    /// 1=h1 ... 6=h6.
    level: u8,
    title: String,
    /// 원본 source의 byte offset — heading의 *시작* 위치.
    start: usize,
}

fn collect_headings(source: &str) -> Vec<RawHeading> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(source, options).into_offset_iter();
    let mut headings = Vec::new();
    let mut current: Option<(u8, String, usize)> = None;

    for (event, range) in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let n = heading_level_to_u8(level);
                current = Some((n, String::new(), range.start));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, title, start)) = current.take() {
                    let title_t = title.trim().to_string();
                    if !title_t.is_empty() {
                        headings.push(RawHeading {
                            level,
                            title: title_t,
                            start,
                        });
                    }
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, ref mut title, _)) = current {
                    title.push_str(&text);
                }
            }
            _ => {}
        }
    }
    headings
}

fn heading_level_to_u8(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// 각 heading의 본문 범위 = (해당 heading 다음 byte) ~ (다음 heading 시작 byte 또는 EOF).
fn compute_body_ranges(headings: &[RawHeading], total_len: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::with_capacity(headings.len());
    for (i, h) in headings.iter().enumerate() {
        let end = headings.get(i + 1).map(|n| n.start).unwrap_or(total_len);
        ranges.push((h.start, end));
    }
    ranges
}

fn build_sections(
    source: &str,
    headings: &[RawHeading],
    ranges: &[(usize, usize)],
) -> Vec<Section> {
    // h1 부재 시 첫 h2를 챕터로 승격.
    let chapter_threshold = if headings.iter().any(|h| h.level == 1) {
        1
    } else {
        2
    };

    let mut sections = Vec::new();
    let mut used_paths: HashSet<String> = HashSet::new();
    let mut current_chapter_path: Option<String> = None;
    let mut chapter_counter: u32 = 0;

    for (h, (start, end)) in headings.iter().zip(ranges.iter()) {
        let body = source.get(*start..*end).unwrap_or("").trim().to_string();
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
                let base_token = section_path(&h.title);
                let prefixed = match &current_chapter_path {
                    Some(c) => format!("{c}/{base_token}"),
                    None => base_token.clone(),
                };
                let unique = dedupe_path(&prefixed, &used_paths);
                (unique, current_chapter_path.clone())
            }
        };
        used_paths.insert(path.clone());

        let display_label = display_label(&h.title, &path);
        sections.push(Section {
            path,
            display_label,
            level,
            parent_path,
            page: None,
            body,
        });
    }
    sections
}

/// 디스플레이 라벨 — heading 원문이 우선. path는 fallback.
fn display_label(title: &str, path: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        path.replace('/', " ")
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_h1_chapters_and_h2_sections() {
        let md = "\
# Chapter 1: Intro
intro body line 1
intro body line 2

## Background
bg body

## Motivation
mot body

# Chapter 2: Deep Dive
deep dive body

## Setup
setup body
";
        let sections = parse(md);
        assert_eq!(sections.len(), 5);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert_eq!(sections[1].path, "Ch01/§Background");
        assert_eq!(sections[1].level, SectionLevel::Section);
        assert_eq!(sections[1].parent_path.as_deref(), Some("Ch01"));
        assert_eq!(sections[2].path, "Ch01/§Motivation");
        assert_eq!(sections[3].path, "Ch02");
        assert_eq!(sections[4].path, "Ch02/§Setup");
        assert_eq!(sections[4].parent_path.as_deref(), Some("Ch02"));
    }

    #[test]
    fn handles_korean_chapter_titles() {
        let md = "\
# 제 4 장 — 상태 관리

## 4.1 변경 가능성

본문

## 4.2 불변성

본문
";
        let sections = parse(md);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].path, "Ch04");
        assert!(sections[0].display_label.contains("상태 관리"));
        assert!(sections[1].path.starts_with("Ch04/§"));
    }

    #[test]
    fn promotes_first_h2_to_chapter_when_no_h1() {
        let md = "\
## Background
text
## Discussion
text
";
        let sections = parse(md);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert_eq!(sections[1].level, SectionLevel::Chapter);
    }

    #[test]
    fn deduplicates_repeated_section_titles() {
        let md = "\
# Ch1

## Summary
a

## Summary
b
";
        let sections = parse(md);
        // Summary 두 개 → 두 번째에 -2 suffix
        assert_eq!(sections[1].path, "Ch01/§Summary");
        assert_eq!(sections[2].path, "Ch01/§Summary-2");
    }

    #[test]
    fn captures_heading_body_to_next_heading() {
        let md = "\
# Ch1

본문 줄 1
본문 줄 2

## Sub
sub body
";
        let sections = parse(md);
        assert!(sections[0].body.contains("본문 줄 1"));
        assert!(sections[0].body.contains("본문 줄 2"));
        // 챕터 본문에 다음 heading은 포함 X
        assert!(!sections[0].body.contains("sub body"));
        assert!(sections[1].body.contains("sub body"));
    }

    #[test]
    fn empty_source_returns_empty_sections() {
        assert!(parse("").is_empty());
        assert!(parse("just plain prose with no headings").is_empty());
    }
}
