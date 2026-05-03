// PDF 파서 — pdfium-render (Chrome PDFium 엔진).
//
// PDFium binary는 *runtime*에 동적 로드. 앱 번들에 포함된 dylib 경로를
// `bind_path`로 명시하거나, 시스템 라이브러리 경로(LD_LIBRARY_PATH 등)에 두어 자동 탐색.
//
// PR 10 범위 (단순화 결정 — handoff 갱신 참조):
//   * 페이지 텍스트 추출 (CID/CMap 정확도 = pdfium-render 강점).
//   * 챕터 정규식 폴백 — 각 페이지 첫 비-공백 줄에서 "Chapter N"·"제 N 장" 매칭.
//   * Outline(북마크) 기반 L1 추출은 PR 19로 이연 (pdfium-render 0.8 API 추가 검토).
//
// L4(paragraph) 분할은 PR 11 임베딩 인덱서가 처리.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pdfium_render::prelude::{Pdfium, PdfiumError};

use crate::error::{AppError, AppResult};
use crate::parsers::slug::{chapter_path, dedupe_path, parse_chapter_number};
use crate::parsers::types::{Section, SectionLevel};

/// PDF 파싱 결과 — 섹션 + 메타 (페이지 수).
pub struct PdfParseResult {
    pub sections: Vec<Section>,
    pub page_count: u32,
}

pub fn parse(path: &Path, lib_dir: Option<&Path>) -> AppResult<PdfParseResult> {
    let pdfium = open_pdfium(lib_dir)?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(map_pdfium_error)?;

    let pages = document.pages();
    let page_count = pages.len() as u32;
    let page_texts = collect_page_texts(&document, page_count);
    let sections = extract_from_text_fallback(&page_texts);

    Ok(PdfParseResult {
        sections,
        page_count,
    })
}

/// PDFium binding을 매 호출마다 새로 — Pdfium 자체가 Send/Sync가 아니라
/// 전역 OnceLock 사용 불가. 시스템 라이브러리는 OS가 캐시하므로 재바인딩 비용은 작다.
/// 호출자(인덱서)는 일반적으로 `tokio::task::spawn_blocking`으로 격리해 사용.
fn open_pdfium(lib_dir: Option<&Path>) -> AppResult<Pdfium> {
    let bindings = match lib_dir {
        Some(dir) => Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(dir)),
        None => Pdfium::bind_to_system_library(),
    }
    .map_err(|e| AppError::Parser {
        message: format!("PDFium 라이브러리를 로드할 수 없습니다: {e}"),
    })?;
    Ok(Pdfium::new(bindings))
}

fn map_pdfium_error(e: PdfiumError) -> AppError {
    AppError::Parser {
        message: format!("PDF 파싱 실패: {e}"),
    }
}

fn collect_page_texts(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_count: u32,
) -> Vec<String> {
    let mut out = Vec::with_capacity(page_count as usize);
    for idx in 0..page_count {
        let page_idx = idx as u16;
        let pages = document.pages();
        let Ok(page) = pages.get(page_idx) else {
            out.push(String::new());
            continue;
        };
        let text = match page.text() {
            Ok(t) => t.all(),
            Err(_) => String::new(),
        };
        out.push(text);
    }
    out
}

// ---- 텍스트 정규식 폴백 ----------------------------------------------------

/// Outline 없이 챕터를 텍스트로 잡는다.
/// 매칭은 *각 페이지의 첫 비-공백 줄* 기준 — 본문 중간의 "Chapter 4" 언급에 흔들리지 않음.
fn extract_from_text_fallback(page_texts: &[String]) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut used_paths: HashSet<String> = HashSet::new();

    for (idx, text) in page_texts.iter().enumerate() {
        let page_no = (idx + 1) as u32;
        let Some(first_line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
            continue;
        };
        let Some(n) = parse_chapter_number(first_line) else {
            continue;
        };
        let base = chapter_path(n);
        let unique = dedupe_path(&base, &used_paths);
        used_paths.insert(unique.clone());

        sections.push(Section {
            path: unique,
            display_label: first_line.to_string(),
            level: SectionLevel::Chapter,
            parent_path: None,
            page: Some(page_no),
            body: String::new(),
        });
    }
    sections
}

/// 앱 번들 내 PDFium 라이브러리 경로 — Tauri resource_dir에서 호출자가 결정.
/// PR 12(뷰어 통합) 시점에 release-pipeline.md 갱신과 함께 정식화.
pub fn bundled_library_dir(resource_dir: &Path) -> PathBuf {
    resource_dir.join("pdfium")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_picks_chapters_from_first_lines() {
        let pages = vec![
            "Cover\nblah\n".to_string(),
            "Chapter 1\nIntro paragraph\n".to_string(),
            "more body\n".to_string(),
            "제 2 장\n본문\n".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].page, Some(2));
        assert_eq!(sections[1].path, "Ch02");
        assert_eq!(sections[1].page, Some(4));
    }

    #[test]
    fn fallback_returns_empty_when_no_chapter_lines() {
        let pages = vec![
            "just regular text".to_string(),
            "no chapters here".to_string(),
        ];
        assert!(extract_from_text_fallback(&pages).is_empty());
    }

    #[test]
    fn fallback_dedupes_repeated_chapter_numbers() {
        // 같은 챕터 번호가 두 페이지에 나오면 -2 suffix.
        let pages = vec!["Chapter 1\n".to_string(), "Chapter 1\n".to_string()];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[1].path, "Ch01-2");
    }
}
