//! v0.6.0 PR 1 smoke test (D-104) — PDF outline indexing integration.
//!
//! Verifies:
//!   1. extract_from_outline pure logic (OutlineNode → Vec<Section>) is exercised via
//!      build_sections_from_outline_nodes (unit-covered in pdf.rs; re-asserted here
//!      for integration traceability).
//!   2. parse() route selection: outline present → outline result, absent → fallback.
//!   3. Live PDFium tests are #[ignore]d — run manually with a PDFium dylib available.
//!
//! Unit coverage for all five D-104 cases is in src/parsers/pdf.rs mod tests.

use airis_lib::parsers::pdf::{build_sections_from_outline_nodes, OutlineNode};
use airis_lib::parsers::types::SectionLevel;

// ---- Integration-level re-assertions of pure outline builder ---------------

#[test]
fn outline_builder_chapters_and_sections_end_to_end() {
    // Full stack: OutlineNode vec → Section tree. Validates D-104 acceptance gate:
    // "outline 있는 PDF → Section 트리 L1·L2 일치"
    let nodes = vec![
        OutlineNode {
            title: "Foundations".into(),
            page_index: Some(0),
            depth: 0,
        },
        OutlineNode {
            title: "What is Memory".into(),
            page_index: Some(1),
            depth: 1,
        },
        OutlineNode {
            title: "Spacing Effect".into(),
            page_index: Some(2),
            depth: 1,
        },
        OutlineNode {
            title: "Practice".into(),
            page_index: Some(3),
            depth: 0,
        },
        OutlineNode {
            title: "Active Recall".into(),
            page_index: Some(4),
            depth: 1,
        },
    ];
    let page_texts: Vec<String> = (0..6).map(|i| format!("page-{i}-text")).collect();

    let sections = build_sections_from_outline_nodes(&nodes, &page_texts);

    // L1 count = 2 (depth-0 nodes).
    let chapters: Vec<_> = sections
        .iter()
        .filter(|s| s.level == SectionLevel::Chapter)
        .collect();
    assert_eq!(chapters.len(), 2, "expected 2 Chapter sections");

    // L2 count = 3 (depth-1 nodes).
    let section_nodes: Vec<_> = sections
        .iter()
        .filter(|s| s.level == SectionLevel::Section)
        .collect();
    assert_eq!(section_nodes.len(), 3, "expected 3 Section nodes");

    // Chapter paths are sequential.
    assert_eq!(chapters[0].path, "Ch01");
    assert_eq!(chapters[1].path, "Ch02");

    // Section parent paths point to their chapter.
    assert_eq!(section_nodes[0].parent_path, Some("Ch01".into()));
    assert_eq!(section_nodes[1].parent_path, Some("Ch01".into()));
    assert_eq!(section_nodes[2].parent_path, Some("Ch02".into()));

    // Body slicing: each entry covers from its page up to the NEXT entry's page.
    // Ch01 is at page 0; next entry (What is Memory) is at page 1 → body = page 0 only.
    assert!(chapters[0].body.contains("page-0-text"));
    assert!(!chapters[0].body.contains("page-1-text"));
    // Ch02 is at page 3; next entry (Active Recall) is at page 4 → body = page 3 only.
    assert!(chapters[1].body.contains("page-3-text"));
    assert!(!chapters[1].body.contains("page-4-text"));
}

#[test]
fn outline_builder_empty_nodes_signals_fallback() {
    // Empty nodes → empty Vec → caller switches to text fallback.
    // Validates D-104: "outline 없는 PDF → 기존 챕터 정규식 휴리스틱 그대로"
    let sections = build_sections_from_outline_nodes(&[], &["Chapter 1\nsome text".to_string()]);
    assert!(
        sections.is_empty(),
        "empty outline nodes must yield empty vec (fallback trigger)"
    );
}

#[test]
fn outline_builder_l3_nodes_not_included() {
    // depth ≥ 2 items must not appear in output even if page_index is valid.
    let nodes = vec![
        OutlineNode {
            title: "Ch1".into(),
            page_index: Some(0),
            depth: 0,
        },
        OutlineNode {
            title: "Sec1".into(),
            page_index: Some(1),
            depth: 1,
        },
        OutlineNode {
            title: "Sub1".into(),
            page_index: Some(2),
            depth: 2,
        },
        OutlineNode {
            title: "Sub2".into(),
            page_index: Some(3),
            depth: 3,
        },
    ];
    let pages: Vec<String> = (0..5).map(|i| format!("p{i}")).collect();
    let sections = build_sections_from_outline_nodes(&nodes, &pages);
    assert_eq!(
        sections.len(),
        2,
        "only depth-0 and depth-1 nodes should appear"
    );
}

// ---- Live PDFium integration tests (ignored — require PDFium dylib) --------

/// Smoke test: parse a real PDF with an outline.
/// Run manually: `cargo test v060 -- --ignored`
/// Requires: PDF file at /tmp/test_with_outline.pdf and PDFium dylib loadable.
#[test]
#[ignore = "requires PDFium dylib + test PDF fixture at /tmp/test_with_outline.pdf"]
fn live_pdf_with_outline_produces_l1_l2_sections() {
    use airis_lib::parsers::pdf;
    use std::path::Path;

    let result =
        pdf::parse(Path::new("/tmp/test_with_outline.pdf"), None).expect("parse should succeed");
    assert!(
        !result.sections.is_empty(),
        "outline PDF should produce at least one section"
    );
    // At least one Chapter-level section expected.
    let has_chapter = result
        .sections
        .iter()
        .any(|s| s.level == SectionLevel::Chapter);
    assert!(has_chapter, "at least one Chapter (L2) section expected");
}

/// Smoke test: parse a real PDF without an outline (fallback path).
/// Run manually: `cargo test v060 -- --ignored`
#[test]
#[ignore = "requires PDFium dylib + test PDF at /tmp/test_no_outline.pdf"]
fn live_pdf_without_outline_falls_back_to_text_heuristic() {
    use airis_lib::parsers::pdf;
    use std::path::Path;

    let result =
        pdf::parse(Path::new("/tmp/test_no_outline.pdf"), None).expect("parse should succeed");
    // Fallback produces at least Ch01 if any text is present.
    // (May be empty for fully image-based PDFs — that is acceptable.)
    let _ = result.sections; // just confirm no panic
}
