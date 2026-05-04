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

use pdfium_render::prelude::{
    PdfPageRenderRotation, PdfRenderConfig, Pdfium, PdfiumError,
};

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

/// Outline 없이 챕터를 텍스트로 잡고, 각 챕터의 본문을 *그 페이지부터 다음 챕터 직전 페이지*까지의
/// 텍스트로 채운다. 챕터를 하나도 못 잡으면 책 전체를 단일 `Ch01`로 박는다 — 검색 가능성 보존.
fn extract_from_text_fallback(page_texts: &[String]) -> Vec<Section> {
    // 1) 각 페이지가 *챕터 시작*인지 판정 + 챕터 번호 + 디스플레이 라벨 수집.
    let mut chapter_starts: Vec<(u32, u32, String)> = Vec::new(); // (page_no, chapter_n, label)
    for (idx, text) in page_texts.iter().enumerate() {
        let page_no = (idx + 1) as u32;
        let Some(first_line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
            continue;
        };
        let Some(n) = parse_chapter_number(first_line) else {
            continue;
        };
        chapter_starts.push((page_no, n, first_line.to_string()));
    }

    // 2) 챕터가 하나도 없다면 단일 Ch01에 책 전체 본문.
    if chapter_starts.is_empty() {
        let body = page_texts.join("\n\n");
        if body.trim().is_empty() {
            return Vec::new();
        }
        return vec![Section {
            path: "Ch01".to_string(),
            display_label: "Ch01".to_string(),
            level: SectionLevel::Chapter,
            parent_path: None,
            page: Some(1),
            body,
        }];
    }

    // 3) 챕터별 본문 = 시작 페이지 ~ 다음 챕터 시작 직전 페이지의 text concat.
    let mut sections = Vec::new();
    let mut used_paths: HashSet<String> = HashSet::new();
    for (i, &(page_no, n, ref label)) in chapter_starts.iter().enumerate() {
        let next_page = chapter_starts
            .get(i + 1)
            .map(|next| next.0)
            .unwrap_or((page_texts.len() as u32) + 1);
        let from = (page_no - 1) as usize;
        let to = (next_page - 1) as usize;
        let body = page_texts[from..to.min(page_texts.len())].join("\n\n");

        let base = chapter_path(n);
        let unique = dedupe_path(&base, &used_paths);
        used_paths.insert(unique.clone());

        sections.push(Section {
            path: unique,
            display_label: label.clone(),
            level: SectionLevel::Chapter,
            parent_path: None,
            page: Some(page_no),
            body,
        });
    }
    sections
}

/// PDF 첫 페이지를 PNG로 렌더해 dest_path에 저장 (PR 60 — 책 썸네일).
/// thumbnail_px = 가장 긴 변 기준 픽셀. 가로/세로 비율 보존.
pub fn render_first_page_png(
    src: &Path,
    lib_dir: Option<&Path>,
    dest: &Path,
    thumbnail_px: u32,
) -> AppResult<()> {
    let pdfium = open_pdfium(lib_dir)?;
    let document = pdfium
        .load_pdf_from_file(src, None)
        .map_err(map_pdfium_error)?;
    let pages = document.pages();
    let page = pages.first().map_err(map_pdfium_error)?;

    let px = thumbnail_px.try_into().unwrap_or(i32::MAX);
    let config = PdfRenderConfig::new()
        .set_target_width(px)
        .set_maximum_height(px)
        .rotate_if_landscape(PdfPageRenderRotation::None, false);
    let bitmap = page.render_with_config(&config).map_err(map_pdfium_error)?;
    let dyn_img = bitmap.as_image();

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::Internal {
            message: format!("썸네일 디렉토리 생성 실패: {e}"),
        })?;
    }
    dyn_img.save(dest).map_err(|e| AppError::Internal {
        message: format!("썸네일 PNG 저장 실패: {e}"),
    })?;
    Ok(())
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
    fn fallback_picks_chapters_from_first_lines_with_bodies() {
        let pages = vec![
            "Cover\nblah\n".to_string(),
            "Chapter 1\nIntro paragraph\n".to_string(),
            "more body of ch1\n".to_string(),
            "제 2 장\n본문\n".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].page, Some(2));
        // Ch01의 body = 페이지 2~3 (다음 챕터가 페이지 4에서 시작)
        assert!(sections[0].body.contains("Intro paragraph"));
        assert!(sections[0].body.contains("more body of ch1"));
        assert_eq!(sections[1].path, "Ch02");
        assert_eq!(sections[1].page, Some(4));
        assert!(sections[1].body.contains("본문"));
    }

    #[test]
    fn fallback_falls_back_to_single_ch01_when_no_chapter_markers() {
        // 챕터 없는 PDF → 책 전체를 Ch01 단일 섹션에 박음 (검색 가능성 보존).
        let pages = vec![
            "just regular text".to_string(),
            "no chapters here".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].path, "Ch01");
        assert!(sections[0].body.contains("just regular"));
        assert!(sections[0].body.contains("no chapters"));
    }

    #[test]
    fn fallback_returns_empty_for_empty_input() {
        assert!(extract_from_text_fallback(&[]).is_empty());
        let only_blank: Vec<String> = vec!["   ".into(), "".into()];
        assert!(extract_from_text_fallback(&only_blank).is_empty());
    }

    #[test]
    fn fallback_dedupes_repeated_chapter_numbers() {
        // 같은 챕터 번호가 두 페이지에 나오면 -2 suffix.
        let pages = vec![
            "Chapter 1\nbody one\n".to_string(),
            "Chapter 1\nbody two\n".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[1].path, "Ch01-2");
        assert!(sections[1].body.contains("body two"));
    }
}
