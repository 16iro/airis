//! v0.6.0 PR 3 — PDFium performance bench harness (D-106).
//!
//! Measures wall-clock time for the four main PDF processing stages:
//!   1. load_document      — `Pdfium::bind + load_pdf_from_file`
//!   2. collect_texts      — page-by-page text extraction (main indexing cost)
//!   3. build_outline_post — `build_sections_from_outline_nodes` post-processing.
//!      Inline outline walk benchmark; the PDFium bookmark walk is crate-private
//!      (see D-106 doc §1 for rationale).
//!   4. render_thumbnail   — `render_first_page_png` (PNG encode included)
//!
//! Usage (manual, env vars required):
//!   export AIRIS_BENCH_PDF_S=/path/to/small.pdf   # ≤50 pages
//!   export AIRIS_BENCH_PDF_M=/path/to/medium.pdf  # ~100 pages
//!   export AIRIS_BENCH_PDF_L=/path/to/large.pdf   # ≥200 pages
//!   cargo test --test v060_pdfium_perf -- --ignored --nocapture
//!
//! If a variable is absent the corresponding sample is skipped gracefully.
//! All three absent → test exits immediately with skip messages.
//!
//! Metrics: 5 runs (cold = first run, then 4 warm). Reports cold / avg / median / min / max.

#[test]
#[ignore = "manual perf bench — set AIRIS_BENCH_PDF_S/M/L env vars + run with --ignored --nocapture"]
fn pdfium_four_stage_wall_clock() {
    use airis_lib::parsers::pdf::{build_sections_from_outline_nodes, OutlineNode};
    use pdfium_render::prelude::Pdfium;
    use std::path::PathBuf;
    use std::time::Instant;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Resolve a PDF sample path from an env var; skip gracefully if absent/nonexistent.
    fn sample_path(env_key: &str, size_label: &str) -> Option<PathBuf> {
        match std::env::var(env_key) {
            Ok(val) => {
                let p = PathBuf::from(&val);
                if p.exists() {
                    Some(p)
                } else {
                    eprintln!(
                        "[v060_pdfium_perf] skipping {} — path '{}' does not exist ({})",
                        size_label, val, env_key
                    );
                    None
                }
            }
            Err(_) => {
                eprintln!(
                    "[v060_pdfium_perf] skipping {} — set {} to enable",
                    size_label, env_key
                );
                None
            }
        }
    }

    /// Per-stage timing statistics.
    #[derive(Debug, Clone)]
    struct StageStats {
        cold_ms: f64,
        avg_ms: f64,
        median_ms: f64,
        min_ms: f64,
        max_ms: f64,
    }

    /// Run `f` `runs` times. First run = cold. Returns StageStats over all runs.
    fn measure<F: FnMut()>(mut f: F, runs: usize) -> StageStats {
        assert!(runs >= 1, "runs must be >= 1");
        let mut samples: Vec<f64> = Vec::with_capacity(runs);
        for _ in 0..runs {
            let t = Instant::now();
            f();
            samples.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let cold_ms = samples[0];
        let min_ms = samples.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_ms = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let avg_ms = samples.iter().sum::<f64>() / samples.len() as f64;
        let mut sorted = samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_ms = if sorted.len() % 2 == 0 {
            (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
        } else {
            sorted[sorted.len() / 2]
        };
        StageStats {
            cold_ms,
            avg_ms,
            median_ms,
            min_ms,
            max_ms,
        }
    }

    /// Print a results table for one PDF sample.
    fn print_table(
        label: &str,
        path: &std::path::Path,
        page_count: u32,
        results: &[(&str, StageStats)],
    ) {
        let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
        println!();
        println!(
            "=== PDF: {} ({}, {}p, {:.1}MB) ===",
            label,
            path.display(),
            page_count,
            size_mb
        );
        println!(
            "{:<24}  {:>10}  {:>10}  {:>12}  {:>10}  {:>10}",
            "stage", "cold_ms", "avg_ms", "median_ms", "min_ms", "max_ms"
        );
        println!("{}", "-".repeat(82));
        for (name, s) in results {
            println!(
                "{:<24}  {:>10.2}  {:>10.2}  {:>12.2}  {:>10.2}  {:>10.2}",
                name, s.cold_ms, s.avg_ms, s.median_ms, s.min_ms, s.max_ms
            );
        }
    }

    // -------------------------------------------------------------------------
    // Resolve sample paths
    // -------------------------------------------------------------------------

    let small = sample_path("AIRIS_BENCH_PDF_S", "small (≤50p)");
    let medium = sample_path("AIRIS_BENCH_PDF_M", "medium (~100p)");
    let large = sample_path("AIRIS_BENCH_PDF_L", "large (≥200p)");

    if small.is_none() && medium.is_none() && large.is_none() {
        eprintln!(
            "[v060_pdfium_perf] All samples absent. Set at least one of \
                   AIRIS_BENCH_PDF_S / AIRIS_BENCH_PDF_M / AIRIS_BENCH_PDF_L and rerun."
        );
        return; // graceful skip — test exits 0
    }

    // -------------------------------------------------------------------------
    // PDFium binding (system library path; PDFIUM_DYNAMIC_LIB env override opt.)
    // -------------------------------------------------------------------------

    // Attempt to bind PDFium. Try env var PDFIUM_LIB_DIR first; fall back to system.
    let pdfium = {
        let bindings = match std::env::var("PDFIUM_LIB_DIR") {
            Ok(dir) => {
                let lib_name = Pdfium::pdfium_platform_library_name_at_path(&dir);
                Pdfium::bind_to_library(lib_name)
            }
            Err(_) => Pdfium::bind_to_system_library(),
        };
        match bindings {
            Ok(b) => Pdfium::new(b),
            Err(e) => {
                eprintln!(
                    "[v060_pdfium_perf] PDFium bind failed: {}. \
                     Set PDFIUM_LIB_DIR to directory containing the PDFium dylib and retry.",
                    e
                );
                return;
            }
        }
    };

    const RUNS: usize = 5;
    const THUMBNAIL_PX: u32 = 480;

    // -------------------------------------------------------------------------
    // Per-sample measurement
    // -------------------------------------------------------------------------

    let samples: &[(&str, Option<PathBuf>)] =
        &[("small", small), ("medium", medium), ("large", large)];

    for (label, maybe_path) in samples {
        let pdf_path = match maybe_path {
            Some(p) => p,
            None => continue,
        };

        // Pre-load to determine page_count (not counted as bench time).
        let doc_probe = match pdfium.load_pdf_from_file(pdf_path, None) {
            Ok(d) => d,
            Err(e) => {
                eprintln!(
                    "[v060_pdfium_perf] {} — load probe failed: {}. Skipping.",
                    label, e
                );
                continue;
            }
        };
        let page_count = doc_probe.pages().len() as u32;
        drop(doc_probe);

        // ---- Stage 1: load_document ----------------------------------------
        // Measures PDFium bind + file load cost per call. Note: open_pdfium()
        // re-binds the dylib each time (Pdfium is not Send/Sync), but the OS
        // caches the dylib so re-bind overhead is small.
        let stat_load = {
            let path = pdf_path.clone();
            measure(
                || {
                    let _doc = pdfium
                        .load_pdf_from_file(&path, None)
                        .expect("load_pdf_from_file should succeed");
                },
                RUNS,
            )
        };

        // ---- Stage 2: collect_texts ----------------------------------------
        // Inline replica of parsers::pdf::collect_page_texts (crate-private fn).
        // Rationale: the function is a simple per-page loop; inlining avoids
        // pub(crate) exposure while measuring the same code path.
        let stat_texts = {
            let path = pdf_path.clone();
            measure(
                || {
                    let doc = pdfium
                        .load_pdf_from_file(&path, None)
                        .expect("load for collect_texts");
                    let pages = doc.pages();
                    let n = pages.len() as u32;
                    let mut out: Vec<String> = Vec::with_capacity(n as usize);
                    for idx in 0..n {
                        let page_idx = idx as u16;
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
                    // Keep out alive so the compiler doesn't elide the work.
                    let _ = out;
                },
                RUNS,
            )
        };

        // ---- Stage 3: build_outline_post -----------------------------------
        // Measures the pure post-processing step (OutlineNode list → Vec<Section>).
        // The upstream PDFium bookmark walk (collect_outline_nodes) is crate-private.
        // We generate synthetic OutlineNodes proportional to page_count to stress
        // the O(n) body-slicing loop. The benchmark covers data-structure cost;
        // actual PDFium API walk cost is noted as unmeasured in D-106 doc §1.
        let stat_outline_post = {
            // Build synthetic nodes: one Chapter per 10 pages, one Section per Chapter.
            let chapter_count = (page_count / 10).max(1);
            let nodes: Vec<OutlineNode> = (0..chapter_count)
                .flat_map(|ci| {
                    let chapter_page = ci * 10;
                    let section_page = (chapter_page + 5).min(page_count.saturating_sub(1));
                    [
                        OutlineNode {
                            title: format!("Chapter {}", ci + 1),
                            page_index: Some(chapter_page),
                            depth: 0,
                        },
                        OutlineNode {
                            title: format!("Section {}.1", ci + 1),
                            page_index: Some(section_page),
                            depth: 1,
                        },
                    ]
                    .into_iter()
                })
                .collect();

            // Build synthetic page texts (cheap placeholder).
            let page_texts: Vec<String> = (0..page_count)
                .map(|i| format!("page {} content placeholder", i + 1))
                .collect();

            measure(
                || {
                    let _ = build_sections_from_outline_nodes(&nodes, &page_texts);
                },
                RUNS,
            )
        };

        // ---- Stage 4: render_thumbnail -------------------------------------
        // Directly calls the pub render_first_page_png. Uses a temp file for dest.
        let stat_thumb = {
            let path = pdf_path.clone();
            let dest = std::env::temp_dir().join(format!("airis_bench_thumb_{}.png", label));
            measure(
                || {
                    airis_lib::parsers::pdf::render_first_page_png(
                        &path,
                        None, // system library
                        &dest,
                        THUMBNAIL_PX,
                    )
                    .expect("render_first_page_png should succeed");
                },
                RUNS,
            )
        };

        // ---- Print results --------------------------------------------------
        let results: &[(&str, StageStats)] = &[
            ("load_document", stat_load),
            ("collect_texts", stat_texts),
            ("build_outline_post", stat_outline_post),
            ("render_thumbnail", stat_thumb),
        ];
        print_table(label, pdf_path, page_count, results);
    }

    println!();
    println!("Notes:");
    println!("  build_outline_post uses synthetic OutlineNodes (crate-private bookmark walk not measured here).");
    println!("  See D-106 doc §3 for spawn_blocking isolation analysis.");
}
