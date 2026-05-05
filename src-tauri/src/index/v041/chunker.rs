// v0.4.1 청커 — D-078 / D-079 / D-080 채택안 구현.
//
// 결정 근거:
//   * D-078 = `text-splitter` Rust crate (LangChain RecursiveCharacterTextSplitter 포팅).
//             800~1200 토큰 윈도우 + 100~150 토큰 overlap (architecture §4.3).
//   * D-079 = `icu_segmenter` SentenceSegmenter (ICU4X). 한국어 정확도 + binary 사이즈
//             양쪽에서 종결어미 정규식 대비 우월(D-079 측정 결과).
//   * D-080 = 토큰 카운팅 휴리스틱 = `(글자수 / 4) * 1.20` (안전 마진 20%, §4.7.3).
//             Claude / Gemini 토크나이저 정확 매치 어려움 → 보수적 휴리스틱이 합리.
//
// 부모 단위 (D-077):
//   * MD = 섹션 헤더 단위. parent = 그 섹션의 첫(=ord 가장 작은) 청크.
//   * PDF = 페이지 단위 폴백. parent = 그 페이지의 첫 청크.
//
// prev/next chunk_id (architecture §4.7.2 sentence window):
//   * 같은 부모 안에서 ord 인접한 청크끼리 연결.
//
// 본 함수는 *id 부여 X*. ChunkRecord.parent_ord / prev_ord / next_ord에 인덱스만
// 채우고, DB INSERT 후 호출 측(indexer.rs)이 ord → 실제 chunks.id로 변환.

#![allow(dead_code)]

use icu_segmenter::options::SentenceBreakInvariantOptions;
use icu_segmenter::SentenceSegmenter;
use text_splitter::{ChunkConfig, TextSplitter};

use crate::parsers::types::Section;

/// 청크 1개 — DB INSERT 직전 형태. id는 호출 측이 부여.
#[derive(Debug, Clone)]
pub struct ChunkRecord {
    /// 책 안 청크 순서 (0-base). DB의 `chunks.ord`.
    pub ord: usize,
    /// 청크 본문. icu_segmenter 문장 경계 보존(D-079).
    pub text: String,
    /// PDF 페이지 (1-base). MD/HTML은 None.
    pub page: Option<u32>,
    /// 원문 char offset 시작 (옵션). 본 PR은 청크 단위 정밀 매핑 X — None.
    pub span_start: Option<usize>,
    /// 원문 char offset 끝 (옵션). 본 PR은 None.
    pub span_end: Option<usize>,
    /// 부모 청크의 ord (auto-merging retrieval 용). 부모가 자기 자신이면 None.
    pub parent_ord: Option<usize>,
    /// 같은 부모 안 직전 청크의 ord. 처음이면 None.
    pub prev_ord: Option<usize>,
    /// 같은 부모 안 직후 청크의 ord. 마지막이면 None.
    pub next_ord: Option<usize>,
    /// 섹션 path (`Ch04/§State`) 또는 PDF 페이지 라벨(`p.42`).
    pub section_path: String,
    /// 토큰 카운트 휴리스틱 (D-080). `ceil(chars / 4 * 1.20)`.
    pub token_count: usize,
}

/// chunk_size·overlap 정책 — architecture §4.3 권고 800~1200 토큰 윈도우.
///
/// text-splitter는 *문자 수* 단위로 청크 길이를 제한한다. D-080 휴리스틱 역산:
///   token ≈ chars / 4 * 1.20 → chars ≈ token / 1.20 * 4
///   1000 token ≈ 3333 char.
/// 본 청커는 char 기준으로 1000±200 token = `~3000~4000` char 범위에서 자른다.
const CHUNK_CHAR_MIN: usize = 2400; // ~720 token
const CHUNK_CHAR_MAX: usize = 4000; // ~1200 token
const CHUNK_OVERLAP_CHAR: usize = 480; // ~144 token (overlap 100~150 token)

/// D-080 토큰 카운팅 휴리스틱.
///
/// `ceil((chars / 4) * 1.20)`. 글자 수 기반이라 한국어·영어 모두 같은 식.
/// LLM 토크나이저(Claude / Gemini)는 한국어가 더 토큰 많은 경향이라 휴리스틱이
/// *과대 추정*하는 방향으로 안전. 인덱싱 시점 패킹 토큰 예산 보수적이 좋다.
pub fn token_count_heuristic(text: &str) -> usize {
    // chars().count()는 grapheme cluster가 아닌 unicode scalar 단위.
    // 한국어 음절 1개 = 1 scalar이라 산문 텍스트엔 충분.
    let chars = text.chars().count();
    // (chars / 4) * 1.20 = chars * 0.30. 정수 산술로 ceil 반올림.
    chars.saturating_mul(30).div_ceil(100)
}

/// D-079 한국어 sentence boundary — ICU4X SentenceSegmenter.
///
/// 입력 텍스트를 문장 경계 byte offset 리스트로 변환. 첫 byte(0)와 마지막 byte(len)
/// 포함. 인접 boundary 사이의 substring이 한 문장.
///
/// segmenter 자체는 stateless · 가벼움이지만 호출 횟수가 많으면 인스턴스 재사용을
/// 권장. 본 PR은 청크 1개당 1회만 사용해 별도 캐시 없이 매 호출 신규 생성.
fn sentence_boundaries(text: &str) -> Vec<usize> {
    let segmenter = SentenceSegmenter::new(SentenceBreakInvariantOptions::default());
    segmenter.segment_str(text).collect()
}

/// 입력 본문을 D-078 splitter + D-079 sentence boundary로 분할 → text vec.
///
/// text-splitter는 chunk_size 안에서 분할 우선순위(`\n\n` → `\n` → 문장 → 단어 → ...)를
/// 본인 휴리스틱으로 처리. 한국어 문장 경계는 text-splitter가 약하므로, 본 함수는
/// text-splitter 결과를 받은 뒤 **각 청크 *내부*가 문장 경계에서 끝나도록 보정**한다.
///
/// 보정 규칙:
///   1. text-splitter 청크의 마지막 문자가 한국어 산문 종결 부호(`다.` / `다!` / `다?` /
///      `요.` / `?` / `!` / `.` / `」` / `”`) 이거나 ICU 문장 경계와 일치하면 그대로.
///   2. 그렇지 않으면 *해당 청크 안에서* ICU 문장 경계 중 *마지막* 위치로 잘라내고,
///      잘려나간 꼬리는 다음 청크 앞에 붙이지 않는다(다음 청크가 어차피 overlap 포함).
///   3. 잘랐을 때 청크가 너무 짧아지면(<100자) 보정 안 함 — 정확도 < 컨텍스트.
fn split_text(body: &str) -> Vec<String> {
    if body.trim().is_empty() {
        return Vec::new();
    }

    // text-splitter는 chunk_size를 단일 값(=max)으로 받고 overlap을 별도. min은 갖지 않음.
    // RangeBound 형태로 min·max 둘 다 명시 가능.
    let config = ChunkConfig::new(CHUNK_CHAR_MIN..=CHUNK_CHAR_MAX)
        .with_overlap(CHUNK_OVERLAP_CHAR)
        .expect("overlap < chunk min");
    let splitter = TextSplitter::new(config);

    let raw_chunks: Vec<String> = splitter.chunks(body).map(|s| s.to_string()).collect();

    raw_chunks
        .into_iter()
        .map(|c| trim_to_sentence_boundary(&c))
        .filter(|c| !c.trim().is_empty())
        .collect()
}

/// 청크 내부에서 ICU 문장 경계 중 마지막에 해당하는 위치로 trailing 자르기.
///
/// 원문 끝이 이미 문장 종결이면 noop. 아니면 ICU boundary 리스트의 마지막 boundary로
/// 잘라낸다. 잘라서 100자 미만이 되면 보정 X(원본 유지) — 정밀도보다 본문 보존 우선.
fn trim_to_sentence_boundary(chunk: &str) -> String {
    let trimmed = chunk.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }

    if ends_with_sentence_terminator(trimmed) {
        return trimmed.to_string();
    }

    let bps = sentence_boundaries(trimmed);
    // bps는 [0, ..., len(trimmed)] 형태. 마지막 *내부* 경계 = 끝에서 두 번째.
    if bps.len() < 3 {
        // 문장 1개로 인식 — 자를 위치 없음.
        return trimmed.to_string();
    }
    let cut = bps[bps.len() - 2];
    if cut < 100 {
        // 너무 짧아짐 — 본문 유지.
        return trimmed.to_string();
    }
    trimmed[..cut].trim_end().to_string()
}

/// 한국어/영어 산문에서 흔한 문장 종결 표지로 끝나는지.
fn ends_with_sentence_terminator(s: &str) -> bool {
    const TERMS: &[&str] = &[
        "다.", "다!", "다?", "요.", "요!", "요?", "까.", "까!", "까?", "죠.", "죠!", "죠?",
    ];
    if TERMS.iter().any(|t| s.ends_with(t)) {
        return true;
    }
    // 단일 문자(영어 산문·CJK 전각 부호·인용 종결).
    matches!(
        s.chars().last(),
        Some('.') | Some('!') | Some('?') | Some('。') | Some('！') | Some('？') | Some('」') | Some('”')
    )
}

/// MD/HTML 파서가 만든 `Vec<Section>`을 청크 시퀀스로 변환.
///
/// 부모 단위 = 섹션. 각 섹션 본문 → split_text → 인접 청크끼리 prev/next 링크.
/// section_path = Section.path 그대로 (`Ch04/§State`).
pub fn chunk_md_sections(sections: &[Section]) -> Vec<ChunkRecord> {
    let mut out: Vec<ChunkRecord> = Vec::new();

    for section in sections {
        let chunks = split_text(&section.body);
        if chunks.is_empty() {
            continue;
        }

        // 부모 = 이 섹션의 첫 청크 = 곧 만들 ord 값.
        let parent_ord = out.len();
        let in_section_count = chunks.len();

        for (i, text) in chunks.into_iter().enumerate() {
            let ord = out.len();
            let token_count = token_count_heuristic(&text);

            // 부모: 자기 자신이면 None (parent_id 자기 참조 금지).
            let parent = if ord == parent_ord { None } else { Some(parent_ord) };
            // 같은 부모(=섹션) 안 인접 청크 연결.
            let prev = if i == 0 { None } else { Some(ord - 1) };
            let next = if i + 1 == in_section_count {
                None
            } else {
                Some(ord + 1)
            };

            out.push(ChunkRecord {
                ord,
                text,
                page: section.page,
                span_start: None,
                span_end: None,
                parent_ord: parent,
                prev_ord: prev,
                next_ord: next,
                section_path: section.path.clone(),
                token_count,
            });
        }
    }

    out
}

/// PDF 페이지 텍스트 배열을 청크 시퀀스로 변환 (D-077 페이지 폴백).
///
/// 부모 단위 = 페이지. section_path = `p.{page_no}`.
/// page_texts[i]는 (i+1) 페이지의 텍스트.
pub fn chunk_pdf_pages(page_texts: &[String]) -> Vec<ChunkRecord> {
    let mut out: Vec<ChunkRecord> = Vec::new();

    for (idx, body) in page_texts.iter().enumerate() {
        let page_no = (idx + 1) as u32;
        let chunks = split_text(body);
        if chunks.is_empty() {
            continue;
        }

        let parent_ord = out.len();
        let in_page_count = chunks.len();
        let section_path = format!("p.{page_no}");

        for (i, text) in chunks.into_iter().enumerate() {
            let ord = out.len();
            let token_count = token_count_heuristic(&text);
            let parent = if ord == parent_ord { None } else { Some(parent_ord) };
            let prev = if i == 0 { None } else { Some(ord - 1) };
            let next = if i + 1 == in_page_count { None } else { Some(ord + 1) };

            out.push(ChunkRecord {
                ord,
                text,
                page: Some(page_no),
                span_start: None,
                span_end: None,
                parent_ord: parent,
                prev_ord: prev,
                next_ord: next,
                section_path: section_path.clone(),
                token_count,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::types::SectionLevel;

    fn mk_section(path: &str, body: &str) -> Section {
        Section {
            path: path.to_string(),
            display_label: path.to_string(),
            level: SectionLevel::Section,
            parent_path: None,
            page: None,
            body: body.to_string(),
        }
    }

    #[test]
    fn token_count_heuristic_matches_d080_formula() {
        // 4 chars → 4 / 4 * 1.20 = 1.2 → ceil = 2 (정수 산술 ceil).
        assert_eq!(token_count_heuristic("abcd"), 2);
        // 100 chars → 100 / 4 * 1.20 = 30 (정확).
        assert_eq!(token_count_heuristic(&"a".repeat(100)), 30);
        // 1000 chars → 300.
        assert_eq!(token_count_heuristic(&"a".repeat(1000)), 300);
        // 한국어도 같은 식 (음절 = scalar 1).
        assert_eq!(token_count_heuristic(&"가".repeat(100)), 30);
    }

    #[test]
    fn small_section_yields_single_chunk() {
        // 본문이 chunk min 미만이면 1개 청크.
        let body = "한국어 문장입니다. 두 번째 문장입니다.";
        let sections = vec![mk_section("Ch01/§Intro", body)];
        let chunks = chunk_md_sections(&sections);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].ord, 0);
        assert_eq!(chunks[0].section_path, "Ch01/§Intro");
        assert!(chunks[0].parent_ord.is_none(), "단일 청크는 자기 자신이 부모 — None");
        assert!(chunks[0].prev_ord.is_none());
        assert!(chunks[0].next_ord.is_none());
        assert!(chunks[0].token_count > 0);
    }

    #[test]
    fn large_section_splits_with_links() {
        // chunk min 약 2400자. 8000자 본문 → 최소 2개 청크.
        let body: String = (0..50)
            .map(|i| format!("이것은 문장 {i}번입니다. 한국어 산문 테스트 본문입니다. "))
            .collect::<String>()
            .repeat(8);
        let sections = vec![mk_section("Ch01/§Big", &body)];
        let chunks = chunk_md_sections(&sections);
        assert!(
            chunks.len() >= 2,
            "8000+자는 최소 2 청크, 실제 = {}",
            chunks.len()
        );

        // 첫 청크 = 부모. parent_ord None.
        assert!(chunks[0].parent_ord.is_none());
        // 나머지는 부모(=ord 0) 가리킴.
        for c in &chunks[1..] {
            assert_eq!(c.parent_ord, Some(0));
        }
        // prev/next 링크 정합성 — 첫 prev는 None, 마지막 next는 None.
        assert!(chunks[0].prev_ord.is_none());
        assert!(chunks[chunks.len() - 1].next_ord.is_none());
        for w in chunks.windows(2) {
            assert_eq!(w[0].next_ord, Some(w[1].ord));
            assert_eq!(w[1].prev_ord, Some(w[0].ord));
        }
        // 모두 같은 section_path.
        for c in &chunks {
            assert_eq!(c.section_path, "Ch01/§Big");
        }
    }

    #[test]
    fn separate_sections_have_separate_parents() {
        // 두 섹션은 각자의 첫 청크가 부모. 섹션 간 prev/next 연결 없음.
        let body_a: String = "가나다 ".repeat(2000);
        let body_b: String = "라마바 ".repeat(2000);
        let sections = vec![
            mk_section("Ch01/§A", &body_a),
            mk_section("Ch01/§B", &body_b),
        ];
        let chunks = chunk_md_sections(&sections);

        // §A·§B 둘 다 ≥ 1 청크.
        let path_counts = chunks.iter().fold((0usize, 0usize), |(a, b), c| {
            if c.section_path == "Ch01/§A" {
                (a + 1, b)
            } else {
                (a, b + 1)
            }
        });
        assert!(path_counts.0 >= 1);
        assert!(path_counts.1 >= 1);

        // §B의 첫 청크는 *§B의 첫 ord*가 부모(=자기 자신)라 None.
        let first_b_idx = chunks
            .iter()
            .position(|c| c.section_path == "Ch01/§B")
            .unwrap();
        assert!(chunks[first_b_idx].parent_ord.is_none(), "섹션 간 부모 누수 X");
        // §B의 마지막 prev는 §A로 넘어가지 않음 — 첫 청크는 prev None.
        assert!(chunks[first_b_idx].prev_ord.is_none());
    }

    #[test]
    fn pdf_pages_use_page_path_and_per_page_parent() {
        let pages = vec![
            "첫 페이지 짧은 본문입니다.".to_string(),
            "두 번째 페이지 본문입니다.".to_string(),
        ];
        let chunks = chunk_pdf_pages(&pages);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].section_path, "p.1");
        assert_eq!(chunks[0].page, Some(1));
        assert_eq!(chunks[1].section_path, "p.2");
        assert_eq!(chunks[1].page, Some(2));
        // 페이지 별로 부모 없음(각 페이지의 첫 청크는 자기 자신).
        assert!(chunks[0].parent_ord.is_none());
        assert!(chunks[1].parent_ord.is_none());
        // 페이지 사이 prev/next 연결 X.
        assert!(chunks[0].next_ord.is_none());
        assert!(chunks[1].prev_ord.is_none());
    }

    #[test]
    fn empty_body_yields_no_chunks() {
        let sections = vec![mk_section("Ch01/§Empty", "   \n\t\n  ")];
        let chunks = chunk_md_sections(&sections);
        assert!(chunks.is_empty());
    }

    #[test]
    fn ord_values_are_dense_and_zero_based() {
        // ord는 0부터 빈틈 없이 증가. parent/prev/next도 같은 인덱스 공간.
        let body_a: String = "한 ".repeat(2000);
        let body_b: String = "두 ".repeat(2000);
        let sections = vec![
            mk_section("Ch01/§A", &body_a),
            mk_section("Ch01/§B", &body_b),
        ];
        let chunks = chunk_md_sections(&sections);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.ord, i, "ord는 dense 0-base");
        }
    }

    #[test]
    fn sentence_terminator_recognized_korean_and_punctuation() {
        assert!(ends_with_sentence_terminator("이것은 문장입니다."));
        assert!(ends_with_sentence_terminator("정말 좋네요!"));
        assert!(ends_with_sentence_terminator("Hello world."));
        assert!(ends_with_sentence_terminator("「인용」"));
        assert!(!ends_with_sentence_terminator("한국어 본문 중간"));
        assert!(!ends_with_sentence_terminator("English mid"));
    }
}
