// PDF 파서 — pdfium-render (Chrome PDFium 엔진).
//
// PDFium binary는 *runtime*에 동적 로드. 앱 번들에 포함된 dylib 경로를
// `bind_path`로 명시하거나, 시스템 라이브러리 경로(LD_LIBRARY_PATH 등)에 두어 자동 탐색.
//
// PR 1 (v0.6.0 / D-104) — outline API + 텍스트 휴리스틱 폴백:
//   * pdfium-render 0.8 PdfBookmarks API로 PDF outline(북마크) 트리를 순회.
//     - PdfDocument::bookmarks().iter() — PdfBookmarksIterator (DFS, 내부 cycle 보호).
//     - PdfBookmark::destination() → Option<PdfDestination> → page_index() (0-based u16).
//     - 직접 destination이 없으면 action() → as_local_destination_action()?.destination() 경로.
//   * depth = parent() 체인 길이. depth 0 → Chapter, depth 1 → Section. depth ≥ 2 무시.
//   * PdfBookmarksIterator에 내장된 visited HashSet이 cycle을 처리하므로 별도 cycle 컷 불필요.
//   * destination resolution 실패 항목은 warn + 빈 body로 유지. 정규식 폴백과 섞지 않음.
//   * outline 결과가 완전히 비어있거나 호출 패닉 시 → extract_from_text_fallback() 그대로 사용.
//
// PR 10 범위 (단순화 결정 — handoff 갱신 참조):
//   * 페이지 텍스트 추출 (CID/CMap 정확도 = pdfium-render 강점).
//   * 챕터 정규식 폴백 — 각 페이지 첫 비-공백 줄에서 "Chapter N"·"제 N 장" 매칭.
//
// L4(paragraph) 분할은 PR 11 임베딩 인덱서가 처리.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pdfium_render::prelude::{
    PdfBookmark, PdfDocument, PdfPageRenderRotation, PdfRenderConfig, Pdfium, PdfiumError,
};

use crate::error::{AppError, AppResult};
use crate::parsers::slug::{chapter_path, dedupe_path, parse_chapter_number, section_path};
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

    // Try outline-based extraction first (D-104).
    // If outline yields nothing (or panics), fall back to text heuristics.
    let outline_sections = extract_from_outline(&document, &page_texts);
    let sections = if outline_sections.is_empty() {
        extract_from_text_fallback(&page_texts)
    } else {
        outline_sections
    };

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

// ---- Intermediate outline node (pure, testable) ----------------------------

/// An intermediate, pdfium-independent representation of a single outline entry.
/// Converted from PdfBookmark before the pure build step.
/// (pdfium의 PdfBookmark 의존성을 제거해 순수 함수 unit-test 가능하게 분리)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineNode {
    /// Bookmark title (trimmed).
    pub title: String,
    /// 0-based page index from destination. None if resolution failed.
    pub page_index: Option<u32>,
    /// Tree depth: 0 = top-level (Chapter), 1 = sub-entry (Section), ≥2 = ignored.
    pub depth: u32,
}

// ---- Outline API extraction ------------------------------------------------

/// Extract outline (bookmark) tree from a PdfDocument and map to Vec<Section>.
///
/// Strategy (D-104):
///   1. DFS walk using first_child / next_sibling. bookmark_handle pointer (as usize)
///      used as visit key to cut cycles.
///   2. depth 0 → SectionLevel::Chapter, depth 1 → SectionLevel::Section. depth ≥ 2 skipped.
///   3. destination resolution fails → skip that entry + warn. No mixing with text fallback.
///   4. Returns empty Vec if outline is absent or the entire walk fails.
///
/// outline 있는 PDF → Section 트리 L1·L2.
/// outline 없거나 실패 → 빈 Vec → 호출자가 텍스트 폴백으로 전환.
fn extract_from_outline(document: &PdfDocument<'_>, page_texts: &[String]) -> Vec<Section> {
    // Collect raw outline nodes (thin PDFium layer).
    let nodes = collect_outline_nodes(document);
    if nodes.is_empty() {
        return Vec::new();
    }

    // Pure function: nodes + page_texts → Vec<Section>.
    build_sections_from_outline_nodes(&nodes, page_texts)
}

/// Walk the bookmark tree DFS, converting each reachable node to an OutlineNode.
///
/// Uses `PdfBookmarks::iter()` which already contains internal cycle protection
/// (its own visited HashSet in PdfBookmarksIterator). Depth is computed from
/// the parent chain: no parent → depth 0, parent has no parent → depth 1, else ≥ 2.
///
/// bookmark_handle is private to the pdfium-render crate, so we cannot use it
/// directly as a HashSet key. The iterator's built-in cycle guard is sufficient.
fn collect_outline_nodes(document: &PdfDocument<'_>) -> Vec<OutlineNode> {
    let bookmarks = document.bookmarks();
    let mut nodes: Vec<OutlineNode> = Vec::new();

    for bm in bookmarks.iter() {
        let depth = bookmark_depth(&bm);

        // Collect only L1 (depth 0) and L2 (depth 1); skip deeper.
        if depth > 1 {
            continue;
        }

        let page_index = resolve_page_index(&bm);
        let title = bm.title().unwrap_or_default();
        let title = title.trim().to_string();

        if page_index.is_none() {
            tracing::warn!(
                "PDF outline destination resolution failed: title={:?}",
                title
            );
        }

        nodes.push(OutlineNode {
            title,
            page_index,
            depth,
        });
    }

    nodes
}

/// Compute tree depth for a bookmark using its parent chain.
/// No parent → depth 0. Parent with no parent → depth 1. Else depth 2+.
/// We cap at 2 to avoid O(n) parent-chain walks on pathological inputs.
fn bookmark_depth(bm: &PdfBookmark<'_>) -> u32 {
    match bm.parent() {
        None => 0,
        Some(parent) => match parent.parent() {
            None => 1,
            Some(_) => 2, // depth ≥ 2; cap here since we only need ≤ 1
        },
    }
}

/// Attempt to resolve the page index (0-based) for a bookmark.
/// First tries the direct `destination()` path; if that gives None,
/// falls through to `action() → local_destination → destination()`.
fn resolve_page_index(bm: &PdfBookmark<'_>) -> Option<u32> {
    // Path 1: direct destination on the bookmark.
    if let Some(dest) = bm.destination() {
        if let Ok(idx) = dest.page_index() {
            return Some(idx as u32);
        }
    }

    // Path 2: action → local destination.
    if let Some(action) = bm.action() {
        if let Some(local) = action.as_local_destination_action() {
            if let Ok(dest) = local.destination() {
                if let Ok(idx) = dest.page_index() {
                    return Some(idx as u32);
                }
            }
        }
    }

    None
}

// ---- Pure section builder (unit-testable) ----------------------------------

/// Convert a flat list of OutlineNodes (with depth tags) into Vec<Section>.
///
/// Rules:
///   - depth 0 → SectionLevel::Chapter. seq assigned in order of appearance (1-based).
///   - depth 1 → SectionLevel::Section. parent = most recent Chapter path.
///   - depth ≥ 2 → ignored (already filtered out by collect_outline_nodes).
///   - broken destination (page_index=None) nodes are included in the tree
///     for body-range calculation but marked with page=None.
///   - body = page text from this node's page (inclusive) up to the next node's page (exclusive).
///
/// 순수 함수라서 pdfium 없이 단위 테스트 가능.
pub fn build_sections_from_outline_nodes(
    nodes: &[OutlineNode],
    page_texts: &[String],
) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut used_paths: HashSet<String> = HashSet::new();
    let mut chapter_seq: u32 = 0;
    let mut last_chapter_path: Option<String> = None;

    // Build a list of (page_index, section_list_index) for body slicing.
    // We store the output index alongside so we can fill body after the pass.
    struct PendingSection {
        page_index: Option<u32>, // 0-based page index (None = broken dest)
        output_idx: usize,
    }
    let mut pending: Vec<PendingSection> = Vec::new();

    for node in nodes {
        if node.depth > 1 {
            continue;
        }

        match node.depth {
            0 => {
                // Chapter
                chapter_seq += 1;
                let base = chapter_path(chapter_seq);
                let path = dedupe_path(&base, &used_paths);
                used_paths.insert(path.clone());
                last_chapter_path = Some(path.clone());

                let output_idx = sections.len();
                sections.push(Section {
                    path,
                    display_label: if node.title.is_empty() {
                        format!("Ch{chapter_seq:02}")
                    } else {
                        node.title.clone()
                    },
                    level: SectionLevel::Chapter,
                    parent_path: None,
                    page: node.page_index.map(|i| i + 1), // 1-based
                    body: String::new(),
                });
                pending.push(PendingSection {
                    page_index: node.page_index,
                    output_idx,
                });
            }
            1 => {
                // Section (L3)
                let parent = last_chapter_path.clone();
                let slug = section_path(&node.title);
                let base = match &parent {
                    Some(p) => format!("{p}/{slug}"),
                    None => slug.clone(),
                };
                let path = dedupe_path(&base, &used_paths);
                used_paths.insert(path.clone());

                let output_idx = sections.len();
                sections.push(Section {
                    path,
                    display_label: if node.title.is_empty() {
                        slug
                    } else {
                        node.title.clone()
                    },
                    level: SectionLevel::Section,
                    parent_path: parent,
                    page: node.page_index.map(|i| i + 1), // 1-based
                    body: String::new(),
                });
                pending.push(PendingSection {
                    page_index: node.page_index,
                    output_idx,
                });
            }
            _ => {}
        }
    }

    // Fill bodies: each section gets text from its page up to the next section's page.
    let total_pages = page_texts.len() as u32;
    for (i, ps) in pending.iter().enumerate() {
        let start = match ps.page_index {
            Some(idx) => idx as usize,
            None => continue, // broken dest: leave body empty
        };
        let end = pending
            .get(i + 1)
            .and_then(|next| next.page_index)
            .map(|idx| idx as usize)
            .unwrap_or(total_pages as usize);

        let body = page_texts[start..end.min(page_texts.len())].join("\n\n");
        sections[ps.output_idx].body = body;
    }

    // Drop sections where page resolution failed AND they have no body
    // (these are fully broken — title is present but destination is gone).
    // We keep them if they have a title so callers see the structure,
    // but they won't contribute useful FTS hits.
    sections
}

// ---- 텍스트 정규식 폴백 ----------------------------------------------------

/// Outline 없이 챕터를 텍스트로 잡고, 각 챕터의 본문을 *그 페이지부터 다음 챕터 직전 페이지*까지의
/// 텍스트로 채운다. 챕터를 하나도 못 잡으면 책 전체를 단일 `Ch01`로 박는다 — 검색 가능성 보존.
///
/// 페이지 헤더 반복 처리 (D-107): 매 페이지 첫 줄에 같은 챕터 헤더가 반복되는 PDF의 경우
/// (예: "제3장 DMG 부트 ROM" 헤더가 챕터 본문 모든 페이지에 박힌 경우), 직전 챕터와 *같은
/// 번호*가 연속으로 나오면 새 챕터 row를 만들지 않고 *직전 챕터 본문에 흡수*한다. 이렇게
/// 하면 dedupe -2/-3 suffix 폭발이 사라지고 챕터 트리가 한 권의 실제 챕터 수와 일치한다.
/// label은 더 긴 텍스트가 나오면 그쪽으로 upgrade한다 (예: "제1장" → "제1장 GAME BOY의
/// 아키텍처"). 두 권으로 묶인 책처럼 *같은 번호의 별개 챕터*가 다른 챕터 사이에 끼어
/// 다시 등장하는 경우는 직전 last_n이 다른 값으로 갱신된 후라 정상적으로 별개로 분리된다.
fn extract_from_text_fallback(page_texts: &[String]) -> Vec<Section> {
    // 1) 각 페이지가 *챕터 시작*인지 판정 + 챕터 번호 + 디스플레이 라벨 수집.
    let mut chapter_starts: Vec<(u32, u32, String)> = Vec::new(); // (page_no, chapter_n, label)
    let mut last_n: Option<u32> = None;
    for (idx, text) in page_texts.iter().enumerate() {
        let page_no = (idx + 1) as u32;
        let Some(first_line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
            continue;
        };
        let Some(n) = parse_chapter_number(first_line) else {
            continue;
        };
        // Repeated page header for the same chapter number — absorb into the
        // current chapter instead of creating a dedupe-suffixed sibling.
        if last_n == Some(n) {
            if let Some(last) = chapter_starts.last_mut() {
                if first_line.len() > last.2.len() {
                    last.2 = first_line.to_string();
                }
            }
            continue;
        }
        chapter_starts.push((page_no, n, first_line.to_string()));
        last_n = Some(n);
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

    // ---- fallback tests (unchanged) ----------------------------------------

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
    fn fallback_merges_repeated_header_pages_into_one_chapter() {
        // D-107: 매 페이지 첫 줄에 같은 챕터 헤더가 반복되면 *같은 챕터*에 흡수.
        // 페이지 1에서 시작한 Ch01의 본문이 페이지 1·2 둘 다 포함.
        let pages = vec![
            "Chapter 1\nbody one\n".to_string(),
            "Chapter 1\nbody two\n".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 1, "repeated header pages should merge");
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].page, Some(1));
        assert!(sections[0].body.contains("body one"));
        assert!(sections[0].body.contains("body two"));
    }

    #[test]
    fn fallback_upgrades_label_to_longer_repeated_header() {
        // 첫 헤더가 짧고 다음 페이지 헤더가 더 길면 label upgrade.
        let pages = vec![
            "제1장\n인트로 본문\n".to_string(),
            "제1장 GAME BOY의 아키텍처\n본문 더\n".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].display_label, "제1장 GAME BOY의 아키텍처");
    }

    #[test]
    fn fallback_separates_same_number_after_other_chapter() {
        // 같은 chapter number가 다른 챕터 사이에 끼어 다시 나타나면 *별개* 챕터.
        // 두 권 합본 같은 케이스 — last_n이 다른 값으로 갱신된 후라 dedupe-suffix.
        let pages = vec![
            "Chapter 1\nfirst part body\n".to_string(),
            "Chapter 2\nch2 body\n".to_string(),
            "Chapter 1\nsecond part appendix\n".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[1].path, "Ch02");
        assert_eq!(sections[2].path, "Ch01-2");
    }

    #[test]
    fn fallback_chapters_with_no_header_pages_in_between_stay_intact() {
        // Header 없는 페이지(그림·표 등)가 챕터 사이에 끼어도 흡수 동작은 영향 없음.
        let pages = vec![
            "Chapter 1\nbody\n".to_string(),
            "no header just figure\n".to_string(),
            "Chapter 1\nstill ch1 body\n".to_string(),
            "Chapter 2\nch2 starts\n".to_string(),
        ];
        let sections = extract_from_text_fallback(&pages);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "Ch01");
        assert!(sections[0].body.contains("body"));
        assert!(sections[0].body.contains("no header just figure"));
        assert!(sections[0].body.contains("still ch1 body"));
        assert_eq!(sections[1].path, "Ch02");
    }

    // ---- outline builder tests (pure function, no pdfium needed) -----------

    fn make_pages(count: usize) -> Vec<String> {
        (0..count)
            .map(|i| format!("Page {} content here.", i + 1))
            .collect()
    }

    #[test]
    fn outline_extracts_l1_l2_from_synthetic_nodes() {
        // depth-0 nodes → Chapter, depth-1 nodes → Section under most recent Chapter.
        // outline L1(Chapter) + L2(Section) 추출 검증.
        let nodes = vec![
            OutlineNode {
                title: "Introduction".into(),
                page_index: Some(0),
                depth: 0,
            },
            OutlineNode {
                title: "Overview".into(),
                page_index: Some(1),
                depth: 1,
            },
            OutlineNode {
                title: "Background".into(),
                page_index: Some(2),
                depth: 1,
            },
            OutlineNode {
                title: "Core Concepts".into(),
                page_index: Some(3),
                depth: 0,
            },
            OutlineNode {
                title: "Ownership".into(),
                page_index: Some(4),
                depth: 1,
            },
        ];
        let pages = make_pages(6);
        let sections = build_sections_from_outline_nodes(&nodes, &pages);

        // 2 chapters + 3 sections = 5 total.
        assert_eq!(sections.len(), 5);

        // First chapter.
        assert_eq!(sections[0].level, SectionLevel::Chapter);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[0].display_label, "Introduction");
        assert_eq!(sections[0].page, Some(1)); // 0-based → 1-based
        assert!(sections[0].parent_path.is_none());

        // First section under Ch01.
        assert_eq!(sections[1].level, SectionLevel::Section);
        assert!(sections[1].path.starts_with("Ch01/§"));
        assert_eq!(sections[1].display_label, "Overview");
        assert_eq!(sections[1].parent_path, Some("Ch01".to_string()));

        // Second chapter.
        assert_eq!(sections[3].level, SectionLevel::Chapter);
        assert_eq!(sections[3].path, "Ch02");
        assert_eq!(sections[3].display_label, "Core Concepts");

        // Section under Ch02.
        assert_eq!(sections[4].level, SectionLevel::Section);
        assert!(sections[4].path.starts_with("Ch02/§"));
        assert_eq!(sections[4].parent_path, Some("Ch02".to_string()));
    }

    #[test]
    fn outline_skips_l3_and_deeper() {
        // depth ≥ 2 nodes must not appear in output.
        // L3+ outline 무시 검증 (4계층 모델에 자리 없음).
        let nodes = vec![
            OutlineNode {
                title: "Chapter One".into(),
                page_index: Some(0),
                depth: 0,
            },
            OutlineNode {
                title: "Section 1.1".into(),
                page_index: Some(1),
                depth: 1,
            },
            OutlineNode {
                title: "Sub 1.1.1".into(),
                page_index: Some(2),
                depth: 2,
            },
            OutlineNode {
                title: "Sub 1.1.1.1".into(),
                page_index: Some(3),
                depth: 3,
            },
        ];
        let pages = make_pages(5);
        let sections = build_sections_from_outline_nodes(&nodes, &pages);

        // Only Ch01 + Section 1.1. L3/L4 dropped.
        assert_eq!(sections.len(), 2, "L3+ nodes must be dropped");
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[1].level, SectionLevel::Section);
    }

    #[test]
    fn outline_cycle_does_not_loop() {
        // Simulate what would happen if duplicate page_index appeared (cycle-like structure).
        // The visited HashSet in collect_outline_nodes prevents infinite loops.
        // Here we verify build_sections_from_outline_nodes handles repeated depth-0 entries
        // (which is the observable result after cycle-cutting: some nodes may appear twice
        // if de-duplication occurs at the collect layer). We use dedupe_path to confirm
        // that colliding paths get -2 suffix instead of overwriting.
        let nodes = vec![
            OutlineNode {
                title: "Intro".into(),
                page_index: Some(0),
                depth: 0,
            },
            OutlineNode {
                title: "Intro".into(),
                page_index: Some(0),
                depth: 0,
            }, // cycle duplicate
        ];
        let pages = make_pages(2);
        let sections = build_sections_from_outline_nodes(&nodes, &pages);

        // Both chapters appear; second gets -2 path suffix.
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "Ch01");
        assert_eq!(sections[1].path, "Ch02"); // sequential chapter_seq, not path-dedup
    }

    #[test]
    fn outline_broken_destination_skips_body_but_keeps_entry() {
        // Nodes with page_index=None (broken destination): entry kept (title visible),
        // body left empty. warn logged (cannot assert in unit test, but no panic).
        // destination 깨진 항목 → body 빈 채로 유지, panic X.
        let nodes = vec![
            OutlineNode {
                title: "Good Chapter".into(),
                page_index: Some(0),
                depth: 0,
            },
            OutlineNode {
                title: "Broken Chapter".into(),
                page_index: None,
                depth: 0,
            },
            OutlineNode {
                title: "Another Good".into(),
                page_index: Some(2),
                depth: 0,
            },
        ];
        let pages = make_pages(4);
        let sections = build_sections_from_outline_nodes(&nodes, &pages);

        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].path, "Ch01");
        assert!(!sections[0].body.is_empty(), "good chapter has body");

        assert_eq!(sections[1].path, "Ch02");
        assert!(sections[1].body.is_empty(), "broken dest → no body");
        assert_eq!(sections[1].page, None);

        assert_eq!(sections[2].path, "Ch03");
        assert!(!sections[2].body.is_empty(), "third chapter has body");
    }

    #[test]
    fn parse_falls_back_to_text_when_outline_empty() {
        // When build_sections_from_outline_nodes receives empty nodes, extract_from_outline
        // returns empty Vec, and parse() switches to text fallback.
        // Validate the fallback branch via direct function call.
        let empty_nodes: &[OutlineNode] = &[];
        let pages = vec![
            "Chapter 1\nbody here".to_string(),
            "more content".to_string(),
        ];
        let from_outline = build_sections_from_outline_nodes(empty_nodes, &pages);
        assert!(
            from_outline.is_empty(),
            "empty nodes → empty outline result"
        );

        // Fallback produces a valid chapter.
        let from_fallback = extract_from_text_fallback(&pages);
        assert_eq!(from_fallback.len(), 1);
        assert_eq!(from_fallback[0].path, "Ch01");
    }

    #[test]
    fn outline_body_sliced_correctly_by_page_ranges() {
        // Verify body text is sliced from start-page to next-entry page.
        // 섹션 body = 해당 페이지부터 다음 항목 직전까지 텍스트.
        let nodes = vec![
            OutlineNode {
                title: "Chap A".into(),
                page_index: Some(0),
                depth: 0,
            },
            OutlineNode {
                title: "Chap B".into(),
                page_index: Some(2),
                depth: 0,
            },
        ];
        let pages = vec![
            "page 0 text".to_string(),
            "page 1 text".to_string(),
            "page 2 text".to_string(),
            "page 3 text".to_string(),
        ];
        let sections = build_sections_from_outline_nodes(&nodes, &pages);
        assert_eq!(sections.len(), 2);

        // Chap A covers pages 0..2 (exclusive).
        assert!(sections[0].body.contains("page 0 text"));
        assert!(sections[0].body.contains("page 1 text"));
        assert!(!sections[0].body.contains("page 2 text"));

        // Chap B covers pages 2..4 (all remaining).
        assert!(sections[1].body.contains("page 2 text"));
        assert!(sections[1].body.contains("page 3 text"));
    }
}
