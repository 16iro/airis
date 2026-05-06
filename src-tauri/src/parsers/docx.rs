// DOCX 파서 — docx-rs 기반 (v0.4.4 PR 3, D-093).
//
// 정책:
//   * docx-rs `read_docx`로 본문(`document.xml`)만 파싱. 헤더/푸터/주석 제외 (DOCX 구조상
//     별도 part로 분리되어 있어 자동 제외).
//   * 단락(paragraph) 단위로 텍스트 + 헤딩 레벨(Heading1~6 스타일) 추출.
//   * 빈 단락(텍스트가 모두 공백/0자) skip — 인덱싱·청킹 무의미.
//   * DOCX는 페이지 번호가 *없음* (viewer 렌더 시점에 결정). page=None.
//
// 결과 모델:
//   * 본 모듈은 `DocxParsed`(저수준)와 `to_sections`(MD 호환)를 모두 제공.
//   * `parse_for_v041` (commands/book.rs)에서 헤딩이 1개 이상 있으면 섹션 시퀀스로,
//     없으면 단일 Ch01 본문으로 반환 — chunk_md_sections에 그대로 투입 가능.
//
// Section.path 형식 (MD 패턴 호환):
//   * h1 = `Ch01` 챕터, h2~h6 = `Ch01/§Title` 섹션 (slug::section_path 사용).
//   * h1 부재 시 첫 h2를 챕터로 승격 (MD 파서와 동일 정책).
//   * 헤딩이 *전혀* 없는 DOCX는 단일 `Ch01`에 본문 전체.
//
// v0.4.1 인덱서 호환: chunk_md_sections는 Section.body 안에서 text-splitter로 잘라낸다.
// 단락 경계가 바로 청크 경계는 아니지만, `\n\n`을 단락 사이에 끼워 넣어
// text-splitter `\n\n` priority가 단락 경계를 우선 자르도록 한다.

#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;

use docx_rs::{read_docx, DocumentChild, ParagraphChild, RunChild};

use crate::error::{AppError, AppResult};
use crate::parsers::slug::{chapter_path, dedupe_path, parse_chapter_number, section_path};
use crate::parsers::types::{Section, SectionLevel};

/// DOCX 파싱 결과 — 단락 시퀀스(저수준).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocxParsed {
    pub paragraphs: Vec<DocxParagraph>,
}

/// DOCX 단락 1개 — 본문 텍스트 + 헤딩 레벨(있으면).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocxParagraph {
    /// 0-base 단락 순서 (빈 단락 제외 후).
    pub ord: i64,
    /// 단락 본문 텍스트 — runs concat. 한국어 UTF-8 안전.
    pub text: String,
    /// Heading1 ~ Heading6 스타일 → 1~6. 본문(스타일 없거나 비-헤딩)은 None.
    pub heading_level: Option<u8>,
}

/// DOCX 파일을 읽어 단락 시퀀스로 변환.
pub fn parse(path: &Path) -> AppResult<DocxParsed> {
    let bytes = std::fs::read(path).map_err(|e| AppError::Parser {
        message: format!("DOCX 파일 읽기 실패: {e}"),
    })?;
    parse_bytes(&bytes)
}

/// 바이트 배열에서 직접 파싱 — 테스트·in-memory 호출용.
pub fn parse_bytes(bytes: &[u8]) -> AppResult<DocxParsed> {
    let read = read_docx(bytes).map_err(|e| AppError::Parser {
        message: format!("DOCX 파싱 실패: {e:?}"),
    })?;

    let mut paragraphs: Vec<DocxParagraph> = Vec::new();
    for child in read.document.children {
        if let DocumentChild::Paragraph(p) = child {
            // 텍스트 = run 안 RunChild::Text concat.
            let text = collect_paragraph_text(&p);
            // 빈 단락 skip (검색·청킹 무의미).
            if text.trim().is_empty() {
                continue;
            }
            // 스타일 이름 → heading level. 보통 `Heading1`~`Heading6` (Word 기본 영문 스타일명).
            // 한국어 Word는 `제목 1`~`제목 6`도 가능 — 그 케이스도 매핑.
            let heading_level = p
                .property
                .style
                .as_ref()
                .and_then(|s| heading_level_from_style(&s.val));
            let ord = paragraphs.len() as i64;
            paragraphs.push(DocxParagraph {
                ord,
                text,
                heading_level,
            });
        }
        // 표·이미지 등 다른 children은 v0.4.4에선 skip — 본문 텍스트만.
    }

    Ok(DocxParsed { paragraphs })
}

/// 단락 안 모든 run의 Text를 concat. RunChild::Tab은 `\t`, ::Break는 `\n`로 변환.
fn collect_paragraph_text(p: &docx_rs::Paragraph) -> String {
    let mut out = String::new();
    for child in &p.children {
        if let ParagraphChild::Run(r) = child {
            for rc in &r.children {
                match rc {
                    RunChild::Text(t) => out.push_str(&t.text),
                    RunChild::Tab(_) => out.push('\t'),
                    RunChild::Break(_) => out.push('\n'),
                    _ => {}
                }
            }
        }
        // ParagraphChild의 다른 변형(Hyperlink·Insert 등)은 v0.4.4에선 skip.
        // 사용자 시나리오상 본문 텍스트가 우선. 추후 PR에서 Hyperlink 텍스트 추출도 가능.
    }
    out
}

/// 스타일명 → 헤딩 레벨 매핑.
///
/// 표준 Word 영문 스타일: `Heading1`~`Heading6` (공백 없음, 정확 매치).
/// 한국어 Word: `제목 1`~`제목 6` (공백 1개 + 숫자).
/// 일부 변형: `heading 1`, `Heading 1` (대소문자·공백 변형).
fn heading_level_from_style(name: &str) -> Option<u8> {
    let normalized = name.trim().to_lowercase().replace(' ', "");
    // 영문 표준
    if let Some(rest) = normalized.strip_prefix("heading") {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=6).contains(&n) {
                return Some(n);
            }
        }
    }
    // 한국어 Word ("제목 N") — 정규화 후 "제목N"
    if let Some(rest) = normalized.strip_prefix("제목") {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=6).contains(&n) {
                return Some(n);
            }
        }
    }
    None
}

/// DOCX 파싱 결과를 v0.4.1 인덱서가 받는 Section 시퀀스로 변환.
///
/// 정책 (MD 파서 패턴 호환):
///   * 헤딩이 1개 이상 → 헤딩 단위 섹션 분할. h1=Chapter, h2~h6=Section.
///     h1 부재 시 첫 h2를 챕터로 승격.
///   * 헤딩 0개 → 단일 `Ch01` 챕터에 본문 전체 (PDF 폴백 패턴).
///   * 본문 단락은 직전 헤딩의 body에 누적, 단락 사이는 `\n\n`로 구분 → text-splitter
///     `\n\n` priority가 단락 경계를 우선 자르게 됨.
///   * DOCX는 page 번호 없음 → Section.page = None.
pub fn to_sections(parsed: &DocxParsed) -> Vec<Section> {
    let paragraphs = &parsed.paragraphs;
    if paragraphs.is_empty() {
        return Vec::new();
    }

    // 헤딩 존재 여부 + chapter_threshold 결정.
    let has_any_heading = paragraphs.iter().any(|p| p.heading_level.is_some());
    if !has_any_heading {
        // 단일 Ch01에 모든 본문 — body는 단락 `\n\n` join.
        let body = paragraphs
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        if body.trim().is_empty() {
            return Vec::new();
        }
        return vec![Section {
            path: "Ch01".to_string(),
            display_label: "Ch01".to_string(),
            level: SectionLevel::Chapter,
            parent_path: None,
            page: None,
            body,
        }];
    }

    let chapter_threshold = if paragraphs.iter().any(|p| p.heading_level == Some(1)) {
        1
    } else {
        2
    };

    // 헤딩 단위 섹션 누적 — `current_section` 빌더가 다음 헤딩(또는 EOF)에서 push.
    let mut sections: Vec<Section> = Vec::new();
    let mut used_paths: HashSet<String> = HashSet::new();
    let mut current_chapter_path: Option<String> = None;
    let mut chapter_counter: u32 = 0;
    let mut current_section: Option<SectionBuilder> = None;
    let mut prelude_body_lines: Vec<String> = Vec::new();

    for p in paragraphs {
        if let Some(level) = p.heading_level {
            // 새 헤딩 — 직전 섹션 마무리.
            if let Some(builder) = current_section.take() {
                sections.push(builder.finish());
            } else if !prelude_body_lines.is_empty() {
                // 첫 헤딩 *이전* 본문이 있으면 별도 `Ch01-Preamble` 섹션.
                let body = prelude_body_lines.join("\n\n");
                if !body.trim().is_empty() {
                    let path = "Ch01-Preamble".to_string();
                    sections.push(Section {
                        path: path.clone(),
                        display_label: path,
                        level: SectionLevel::Chapter,
                        parent_path: None,
                        page: None,
                        body,
                    });
                }
                prelude_body_lines.clear();
            }

            let title = p.text.clone();
            let section_level = if level <= chapter_threshold {
                SectionLevel::Chapter
            } else {
                SectionLevel::Section
            };
            let (path, parent_path) = match section_level {
                SectionLevel::Chapter => {
                    chapter_counter += 1;
                    let n = parse_chapter_number(&title).unwrap_or(chapter_counter);
                    let base = chapter_path(n);
                    let unique = dedupe_path(&base, &used_paths);
                    current_chapter_path = Some(unique.clone());
                    (unique, None)
                }
                SectionLevel::Section => {
                    let base_token = section_path(&title);
                    let prefixed = match &current_chapter_path {
                        Some(c) => format!("{c}/{base_token}"),
                        None => base_token.clone(),
                    };
                    let unique = dedupe_path(&prefixed, &used_paths);
                    (unique, current_chapter_path.clone())
                }
            };
            used_paths.insert(path.clone());

            current_section = Some(SectionBuilder {
                path,
                display_label: display_label(&title),
                level: section_level,
                parent_path,
                body_lines: Vec::new(),
            });
        } else if let Some(builder) = current_section.as_mut() {
            builder.body_lines.push(p.text.clone());
        } else {
            // 첫 헤딩 이전 본문.
            prelude_body_lines.push(p.text.clone());
        }
    }
    if let Some(builder) = current_section.take() {
        sections.push(builder.finish());
    } else if !prelude_body_lines.is_empty() {
        // 헤딩이 *전혀* 등장하지 않았으나(이미 위에서 처리됨), 안전망.
        let body = prelude_body_lines.join("\n\n");
        if !body.trim().is_empty() {
            sections.push(Section {
                path: "Ch01".to_string(),
                display_label: "Ch01".to_string(),
                level: SectionLevel::Chapter,
                parent_path: None,
                page: None,
                body,
            });
        }
    }

    sections
}

struct SectionBuilder {
    path: String,
    display_label: String,
    level: SectionLevel,
    parent_path: Option<String>,
    body_lines: Vec<String>,
}

impl SectionBuilder {
    fn finish(self) -> Section {
        // body는 헤딩 제목 + (단락 \n\n join). 청커가 \n\n priority로 단락 경계 우선 분할.
        // 다만 chunk_md_sections는 section.body의 *모든* 텍스트를 본문으로 봄 — heading text를
        // 포함하든 빼든 검색 정확도엔 큰 차이 없음. MD 파서는 heading raw도 포함하므로 동등.
        let body = if self.body_lines.is_empty() {
            self.display_label.clone()
        } else {
            // 헤딩 텍스트 + 본문 단락들 (검색 시 헤딩어로도 hit 가능).
            std::iter::once(self.display_label.clone())
                .chain(self.body_lines)
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        Section {
            path: self.path,
            display_label: self.display_label,
            level: self.level,
            parent_path: self.parent_path,
            page: None,
            body,
        }
    }
}

fn display_label(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        "(제목 없음)".to_string()
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use docx_rs::{Docx, Paragraph, Run};

    fn build_test_docx() -> Vec<u8> {
        // 한국어 헤딩 + 본문 + 빈 단락 + h2/h3 mix.
        let docx = Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("제 1 장 한국어 헤딩")),
            )
            .add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text("이것은 한국어 본문 단락입니다.")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading2")
                    .add_run(Run::new().add_text("§ 1.1 부절 제목")),
            )
            .add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text("두 번째 본문 단락. English mixed.")),
            )
            .add_paragraph(Paragraph::new()) // 빈 단락 → skip
            .add_paragraph(
                Paragraph::new()
                    .style("Heading3")
                    .add_run(Run::new().add_text("h3 sub")),
            )
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("마지막 본문.")),
            );
        let mut buf: Vec<u8> = Vec::new();
        docx.build()
            .pack(std::io::Cursor::new(&mut buf))
            .expect("pack");
        buf
    }

    #[test]
    fn parses_paragraphs_and_skips_empty() {
        let bytes = build_test_docx();
        let parsed = parse_bytes(&bytes).expect("parse ok");
        // 빈 단락 1개 skip → 6개.
        assert_eq!(parsed.paragraphs.len(), 6);
        // ord는 0..n 빈틈 없이.
        for (i, p) in parsed.paragraphs.iter().enumerate() {
            assert_eq!(p.ord, i as i64);
        }
        // 한국어 텍스트 정확 추출.
        assert_eq!(parsed.paragraphs[0].text, "제 1 장 한국어 헤딩");
        assert_eq!(parsed.paragraphs[1].text, "이것은 한국어 본문 단락입니다.");
    }

    #[test]
    fn detects_heading_levels() {
        let bytes = build_test_docx();
        let parsed = parse_bytes(&bytes).expect("parse ok");
        assert_eq!(parsed.paragraphs[0].heading_level, Some(1));
        assert_eq!(parsed.paragraphs[1].heading_level, None);
        assert_eq!(parsed.paragraphs[2].heading_level, Some(2));
        assert_eq!(parsed.paragraphs[3].heading_level, None);
        assert_eq!(parsed.paragraphs[4].heading_level, Some(3)); // 빈 단락 skip 후
        assert_eq!(parsed.paragraphs[5].heading_level, None);
    }

    #[test]
    fn heading_style_matcher_handles_variants() {
        assert_eq!(heading_level_from_style("Heading1"), Some(1));
        assert_eq!(heading_level_from_style("heading 2"), Some(2));
        assert_eq!(heading_level_from_style("Heading 3"), Some(3));
        assert_eq!(heading_level_from_style("HEADING6"), Some(6));
        assert_eq!(heading_level_from_style("제목 1"), Some(1));
        assert_eq!(heading_level_from_style("제목3"), Some(3));
        // 비-헤딩 스타일은 None.
        assert_eq!(heading_level_from_style("Normal"), None);
        assert_eq!(heading_level_from_style("Title"), None); // 의도적: Title은 doc 표지, 헤딩 X
        assert_eq!(heading_level_from_style("Heading7"), None); // h7+ 무시
    }

    #[test]
    fn to_sections_groups_by_heading() {
        let bytes = build_test_docx();
        let parsed = parse_bytes(&bytes).expect("parse ok");
        let sections = to_sections(&parsed);
        // h1=Ch01 (chapter), h2=Ch01/§…, h3=Ch01/§…/§… 또는 same chapter sub.
        // 본 케이스는 h1 1 + h2 1 + h3 1 → 3 섹션.
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert!(sections[0].body.contains("이것은 한국어 본문 단락입니다."));
        assert!(sections[1].path.starts_with("Ch01/§"));
        assert_eq!(sections[1].level, SectionLevel::Section);
        assert_eq!(sections[1].parent_path.as_deref(), Some("Ch01"));
        assert!(sections[1].body.contains("두 번째 본문 단락"));
        assert!(sections[2].path.starts_with("Ch01/§"));
        assert!(sections[2].body.contains("마지막 본문"));
        // page 번호는 모두 None (DOCX는 페이지 X).
        for s in &sections {
            assert!(s.page.is_none());
        }
    }

    #[test]
    fn to_sections_no_heading_yields_single_ch01() {
        let docx = Docx::new()
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("첫 번째 본문 단락.")),
            )
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("두 번째 본문 단락.")),
            );
        let mut buf: Vec<u8> = Vec::new();
        docx.build()
            .pack(std::io::Cursor::new(&mut buf))
            .expect("pack");
        let parsed = parse_bytes(&buf).expect("parse ok");
        let sections = to_sections(&parsed);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert!(sections[0].body.contains("첫 번째"));
        assert!(sections[0].body.contains("두 번째"));
    }

    #[test]
    fn to_sections_promotes_h2_when_no_h1() {
        let docx = Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Heading2")
                    .add_run(Run::new().add_text("Background")),
            )
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("bg body")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading2")
                    .add_run(Run::new().add_text("Discussion")),
            )
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("disc body")),
            );
        let mut buf: Vec<u8> = Vec::new();
        docx.build()
            .pack(std::io::Cursor::new(&mut buf))
            .expect("pack");
        let parsed = parse_bytes(&buf).expect("parse ok");
        let sections = to_sections(&parsed);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert_eq!(sections[1].level, SectionLevel::Chapter);
    }

    #[test]
    fn to_sections_keeps_prelude_body_before_first_heading() {
        let docx = Docx::new()
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("머리말 본문")),
            )
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("제 1 장")),
            )
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("본문")),
            );
        let mut buf: Vec<u8> = Vec::new();
        docx.build()
            .pack(std::io::Cursor::new(&mut buf))
            .expect("pack");
        let parsed = parse_bytes(&buf).expect("parse ok");
        let sections = to_sections(&parsed);
        // 머리말 → Ch01-Preamble, 제 1 장 → Ch01.
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "Ch01-Preamble");
        assert!(sections[0].body.contains("머리말 본문"));
        assert_eq!(sections[1].path, "Ch01");
    }

    #[test]
    fn to_sections_empty_returns_empty() {
        let parsed = DocxParsed { paragraphs: Vec::new() };
        assert!(to_sections(&parsed).is_empty());
    }
}
