// v0.4.1 context — 컨텍스트 파이프라인 (검색 결과 → LLM 입력 조립).
//
// architecture §4.7·§4.9 그대로:
//   * 시스템 프롬프트 = 환각 방지 + 한국어 few-shot 2건 (PoC d4_citation_marker.rs 이식).
//   * 메타데이터 블록 = `[S1] (책: {title}, p.{page}, §{section_path})\n{text}` 형식.
//   * 토큰 예산 패킹 = 점수 *오름차순* 배치 (Lost in the Middle 회피, §4.7.3).
//     낮은 점수가 위, 높은 점수가 아래 = 질문 직전. 토큰 합계가 budget을 넘기 직전까지.
//   * 인용 마커 [Sx] 스트리밍 파싱 = `[S\d+]` 정규식, source 인덱스 범위 검증.
//
// 토큰 카운팅은 chunker::token_count_heuristic 재사용 (D-080 일관성).

#![allow(dead_code)]

use std::sync::OnceLock;

use regex::Regex;

use crate::index::v041::chunker::token_count_heuristic;
use crate::index::v041::retrieval::RetrievedChunk;

/// 환각 방지 + 한국어 few-shot 시스템 프롬프트.
///
/// PoC `experiments/v040-poc/src/bin/d4_citation_marker.rs::SYSTEM_PROMPT`를 *학습 보조*
/// 톤으로 다듬은 버전. 핵심 변경:
///   * "학습 보조자" 톤 유지(원본 그대로).
///   * 답변 길이 가이드: 2~4 문장 → 2~6 문장 (학습 챗 답변이 더 길어질 수 있음).
///   * 자료에 없으면 "제공된 자료에는 해당 정보가 없습니다"라고 답하라는 원칙 명시
///     (§4.7.5 NotebookLM 차별점 핵심).
///   * 자료가 모순될 경우 양쪽을 모두 제시.
///   * few-shot 2건은 PoC 그대로 (의학 + 게임보이) — 사용자 검증된 프롬프트 라인.
pub const SYSTEM_PROMPT: &str = r#"당신은 사용자가 제공한 자료를 바탕으로 답하는 한국어 학습 보조자입니다.

규칙:
1. 답변은 [SOURCES] 섹션에 제공된 자료에만 근거합니다.
   자료에 없는 정보는 "제공된 자료에는 해당 정보가 없습니다"라고 답합니다.
2. 모든 사실 진술 끝에 [S1], [S2] 형식으로 출처를 표시합니다.
   번호는 반드시 제공된 자료의 범위 안에서만 사용합니다.
3. 추측하지 않습니다. 자료가 모순될 경우 양쪽을 모두 제시합니다.
4. 답변은 한국어로 작성하고 2~6 문장으로 간결하게 유지합니다.

다음은 예시입니다.

# 예시 1
자료:
[S1] 라스트핀치는 갑상선 호르몬을 분비한다.
[S2] 갑상선 자극 호르몬은 뇌하수체 전엽에서 만들어진다.

질문: 갑상선 자극 호르몬은 어디서 만들어지나?
답: 갑상선 자극 호르몬은 뇌하수체 전엽에서 만들어집니다 [S2].

# 예시 2
자료:
[S1] CPU는 프로그램 카운터로 다음 명령 주소를 알 수 있다.
[S2] 게임보이의 CPU는 8비트 레지스터 8개와 16비트 레지스터 2개를 갖는다.
[S3] 스택 포인터는 스택의 가장 위 주소를 보관한다.

질문: 게임보이 CPU의 레지스터 구성과 PC, SP 역할을 짧게 설명해.
답: 게임보이 CPU는 8비트 8개와 16비트 2개의 레지스터를 갖습니다 [S2]. 프로그램 카운터는 다음 명령 주소를 보관하고 [S1], 스택 포인터는 스택의 가장 위 주소를 가리킵니다 [S3]."#;

/// 인용 마커 정규식 — `[S` 다음 한 자리 이상 숫자 + `]`. 캡처 그룹 1 = 숫자.
///
/// `OnceLock` 으로 1회 컴파일. 스트리밍 파싱·후검증 양쪽 공유.
fn citation_re() -> &'static Regex {
    static CACHE: OnceLock<Regex> = OnceLock::new();
    CACHE.get_or_init(|| Regex::new(r"\[S(\d+)\]").expect("citation regex"))
}

/// 시스템 프롬프트의 source 인덱스 (`Sx`)와 실제 chunk_id 사이 매핑.
///
/// `marker = "S1"`일 때 chunk_id는 build_context가 만든 메타 블록의 1번째 source.
/// 1-base.
#[derive(Debug, Clone, PartialEq)]
pub struct CitationEntry {
    /// "S1", "S2", ... 형식 마커 (대괄호 제외).
    pub marker: String,
    /// chunks.id.
    pub chunk_id: i64,
    /// 해당 source의 원래 책 페이지 (1-base, MD/HTML은 None).
    pub page: Option<i64>,
    /// 해당 source의 section_path (`Ch04/§State` 또는 `p.42`).
    pub section_path: Option<String>,
}

/// 토큰 예산 안에 패킹되지 못해 잘려나간 source의 요약.
///
/// UI에서 "추가 자료 N건 생략됨" 식으로 노출하기 위한 정보.
#[derive(Debug, Clone, PartialEq)]
pub struct TruncationInfo {
    /// 잘려나간 source 개수.
    pub dropped: usize,
    /// 패킹된 source 개수.
    pub kept: usize,
    /// 적용된 토큰 예산.
    pub budget: usize,
    /// 패킹된 source의 토큰 합계 (휴리스틱).
    pub used_tokens: usize,
}

/// build_context 결과 번들 — 시스템 프롬프트 + 메타블록 + 인용 인덱스 + truncation 메타.
#[derive(Debug, Clone)]
pub struct ContextBundle {
    /// 시스템 프롬프트 (SYSTEM_PROMPT 그대로 — 호출 측이 수정 가능).
    pub system_prompt: String,
    /// `[S1] (책: ..., p.42, §...)\n{text}\n\n[S2] ...` 형식의 source 블록.
    /// 빈 문자열이면 검색 결과 0건.
    pub sources_block: String,
    /// citation_index_map — 마커 → chunk_id + 메타. 1-base 인덱스.
    pub citation_index_map: Vec<CitationEntry>,
    /// truncation 메타 — 토큰 예산 패킹 결과.
    pub truncation: TruncationInfo,
}

/// 단일 source 메타 블록 직렬화 — `[Sx] (책: {title}, p.{page}, §{section_path})\n{text}`.
///
/// page / section_path가 None이면 해당 항목을 생략. 책 제목이 빈 문자열이면 책 표기 자체
/// 생략(테스트 편의).
fn format_source_block(
    marker_index: usize,
    chunk: &RetrievedChunk,
    book_title: &str,
) -> String {
    let mut header_parts: Vec<String> = Vec::with_capacity(3);
    if !book_title.is_empty() {
        header_parts.push(format!("책: {book_title}"));
    }
    if let Some(p) = chunk.page {
        header_parts.push(format!("p.{p}"));
    }
    if let Some(sp) = chunk.section_path.as_ref().filter(|s| !s.is_empty()) {
        header_parts.push(format!("§{sp}"));
    }
    let header = if header_parts.is_empty() {
        format!("[S{marker_index}]")
    } else {
        format!("[S{marker_index}] ({})", header_parts.join(", "))
    };
    format!("{header}\n{}", chunk.text)
}

/// 토큰 예산 패킹 결과.
struct PackResult {
    /// 패킹에 포함된 청크들 (입력 순서 그대로 — 점수 오름차순으로 진입한 호출 측이 보장).
    kept: Vec<RetrievedChunk>,
    /// 패킹된 source 들의 토큰 합계 휴리스틱.
    used_tokens: usize,
    /// 패킹에서 잘려나간 source 개수.
    dropped: usize,
}

/// 토큰 예산 안에서 source 채우기 — 점수 오름차순으로 받아 *낮은 점수가 위, 높은 점수가
/// 아래(=질문 가까이)*가 되도록 한다 (Lost in the Middle 회피, §4.7.3).
///
/// 입력 `retrieved`는 hybrid_search 결과(점수 내림차순). 본 함수는 토큰 budget 안에 들어가는
/// *상위 점수* 청크를 우선 선택한 뒤, 출력 순서를 점수 *오름차순*으로 뒤집는다.
fn pack_within_budget(retrieved: &[RetrievedChunk], budget: usize) -> PackResult {
    if retrieved.is_empty() || budget == 0 {
        return PackResult {
            kept: Vec::new(),
            used_tokens: 0,
            dropped: retrieved.len(),
        };
    }

    let mut used = 0_usize;
    // 메타 헤더 자체도 토큰을 잡아먹지만 휴리스틱으로 source당 +20 토큰 마진.
    const PER_SOURCE_HEADER_TOKENS: usize = 20;
    let mut kept_desc: Vec<RetrievedChunk> = Vec::new();

    for c in retrieved.iter() {
        let body_tokens = c
            .token_count
            .map(|t| t.max(0) as usize)
            .unwrap_or_else(|| token_count_heuristic(&c.text));
        let need = body_tokens.saturating_add(PER_SOURCE_HEADER_TOKENS);
        if used.saturating_add(need) > budget {
            // budget 초과 — 더 이상 추가 X. 첫 청크 1개도 못 들어가면 그대로 버린다
            // (호출 측이 budget을 너무 작게 잡은 경우의 안전 분기).
            break;
        }
        used = used.saturating_add(need);
        kept_desc.push(c.clone());
    }

    // 출력 순서 = 점수 오름차순. retrieved가 점수 내림차순이라 reverse하면 됨.
    let mut kept = kept_desc;
    kept.reverse();

    let dropped = retrieved.len().saturating_sub(kept.len());
    PackResult {
        kept,
        used_tokens: used,
        dropped,
    }
}

/// retrieval 결과 + 책 제목 + 토큰 예산 → `ContextBundle`.
///
/// 입력 `retrieved`는 `hybrid_search` 결과 그대로 (점수 내림차순). 본 함수는 budget 안에
/// 들어가는 청크만 선택한 뒤, 출력 순서는 점수 오름차순(낮은 점수 → 높은 점수)으로 배치
/// 한다 (Lost in the Middle 회피).
///
/// citation_index_map의 마커 번호는 1-base. 출력 메타 블록 첫 source = `[S1]`.
pub fn build_context(
    retrieved: &[RetrievedChunk],
    book_title: &str,
    token_budget: usize,
) -> ContextBundle {
    let pack = pack_within_budget(retrieved, token_budget);

    let mut sources_block = String::new();
    let mut citation_index_map: Vec<CitationEntry> = Vec::with_capacity(pack.kept.len());

    for (i, chunk) in pack.kept.iter().enumerate() {
        let marker_index = i + 1; // 1-base
        let block = format_source_block(marker_index, chunk, book_title);
        if !sources_block.is_empty() {
            sources_block.push_str("\n\n");
        }
        sources_block.push_str(&block);

        citation_index_map.push(CitationEntry {
            marker: format!("S{marker_index}"),
            chunk_id: chunk.id,
            page: chunk.page,
            section_path: chunk.section_path.clone(),
        });
    }

    ContextBundle {
        system_prompt: SYSTEM_PROMPT.to_string(),
        sources_block,
        citation_index_map,
        truncation: TruncationInfo {
            dropped: pack.dropped,
            kept: pack.kept.len(),
            budget: token_budget,
            used_tokens: pack.used_tokens,
        },
    }
}

/// 인용 마커 파싱 결과 — 한 응답에서 발견된 모든 [Sx] 마커.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCitation {
    /// 마커 ("S1", "S2", ...).
    pub marker: String,
    /// 본문 안 byte 위치 (start, end). [Sx] 전체 범위.
    pub span: (usize, usize),
    /// 마커 번호가 source 인덱스 범위(1..=kept) 안인지.
    /// false면 환각·오타 마커 (UI에 ⚠️ 표시 권장).
    pub in_range: bool,
}

/// 응답 텍스트에서 [Sx] 모두 추출. source_count = citation_index_map.len() (1-base 범위).
///
/// 스트리밍 적용 시 호출 측은 누적 buffer를 가지고 본 함수를 부분 입력에 호출하면 된다.
/// 닫는 `]`까지 들어와야 매칭되므로 미완성 `[S1` 만 들어온 상태에선 결과가 비어 있다.
pub fn parse_citations(response: &str, source_count: usize) -> Vec<ParsedCitation> {
    let mut out = Vec::new();
    for caps in citation_re().captures_iter(response) {
        let m = caps.get(0).expect("group 0");
        let num: usize = caps[1].parse().unwrap_or(0);
        let in_range = num >= 1 && num <= source_count;
        out.push(ParsedCitation {
            marker: format!("S{num}"),
            span: (m.start(), m.end()),
            in_range,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::v041::retrieval::RetrievedChunk;

    fn rc(id: i64, score: f64, text: &str, page: Option<i64>, sp: Option<&str>) -> RetrievedChunk {
        RetrievedChunk {
            id,
            text: text.to_string(),
            page,
            section_path: sp.map(|s| s.to_string()),
            parent_id: None,
            prev_chunk_id: None,
            next_chunk_id: None,
            token_count: None,
            score,
        }
    }

    #[test]
    fn system_prompt_mentions_sources_and_citation_format() {
        // 핵심 키워드가 빠지면 ChatBridge 측 회귀 — 기능 변경 시도가 무심코 환각 가드를
        // 깎지 못하게 sentinel 검사.
        assert!(SYSTEM_PROMPT.contains("제공된 자료에는 해당 정보가 없습니다"));
        assert!(SYSTEM_PROMPT.contains("[S1], [S2]"));
        assert!(SYSTEM_PROMPT.contains("한국어"));
        // few-shot 2건 — 의학 / 게임보이.
        assert!(SYSTEM_PROMPT.contains("갑상선"));
        assert!(SYSTEM_PROMPT.contains("게임보이"));
    }

    #[test]
    fn format_source_block_includes_title_page_section() {
        let chunk = rc(7, 0.5, "본문 내용", Some(42), Some("Ch04/§State"));
        let s = format_source_block(1, &chunk, "Rust 책");
        // 모든 메타가 한 줄 헤더에 들어가고 본문은 다음 줄.
        assert!(s.starts_with("[S1] (책: Rust 책, p.42, §Ch04/§State)\n"));
        assert!(s.ends_with("본문 내용"));
    }

    #[test]
    fn format_source_block_omits_missing_meta() {
        let chunk = rc(7, 0.5, "본문", None, None);
        let s = format_source_block(2, &chunk, "Rust 책");
        // page·section 모두 None이라 헤더 = "[S2] (책: ...)" 정도.
        assert!(s.starts_with("[S2] (책: Rust 책)\n본문"));
    }

    #[test]
    fn build_context_orders_kept_by_ascending_score() {
        // hybrid_search는 점수 내림차순으로 결과를 준다. build_context 출력은 점수 오름차순.
        let retrieved = vec![
            rc(1, 0.9, "A high score", None, Some("§A")),
            rc(2, 0.5, "B mid score", None, Some("§B")),
            rc(3, 0.1, "C low score", None, Some("§C")),
        ];
        // 충분히 큰 budget → 모두 들어감.
        let bundle = build_context(&retrieved, "Book", 10_000);
        assert_eq!(bundle.citation_index_map.len(), 3);
        // 출력 순서 = 점수 오름차순 → C, B, A.
        assert_eq!(bundle.citation_index_map[0].chunk_id, 3); // C
        assert_eq!(bundle.citation_index_map[1].chunk_id, 2); // B
        assert_eq!(bundle.citation_index_map[2].chunk_id, 1); // A
        // 마커 번호는 출력 순서대로 1, 2, 3 — *원래 점수 순위가 아닌* 출력 순서.
        assert_eq!(bundle.citation_index_map[0].marker, "S1");
        assert_eq!(bundle.citation_index_map[1].marker, "S2");
        assert_eq!(bundle.citation_index_map[2].marker, "S3");
        // sources_block에 [S1] [S2] [S3] 모두 등장.
        assert!(bundle.sources_block.contains("[S1] "));
        assert!(bundle.sources_block.contains("[S2] "));
        assert!(bundle.sources_block.contains("[S3] "));
        // truncation 메타.
        assert_eq!(bundle.truncation.kept, 3);
        assert_eq!(bundle.truncation.dropped, 0);
        assert_eq!(bundle.truncation.budget, 10_000);
    }

    #[test]
    fn build_context_truncates_under_budget() {
        // 각 청크 ~300 토큰 (1000자 본문). budget을 작게 잡아 1~2건만 들어가도록.
        let body = "가".repeat(1000);
        let retrieved = vec![
            rc(1, 0.9, &body, None, None),
            rc(2, 0.5, &body, None, None),
            rc(3, 0.1, &body, None, None),
        ];
        // 각 source ≈ 300 + 20(header) = 320 토큰. 700 budget이면 ~2건.
        let bundle = build_context(&retrieved, "B", 700);
        assert!(bundle.truncation.kept <= 2);
        assert!(bundle.truncation.dropped >= 1);
        assert_eq!(bundle.citation_index_map.len(), bundle.truncation.kept);
    }

    #[test]
    fn build_context_empty_input_yields_empty_bundle() {
        let bundle = build_context(&[], "Book", 1000);
        assert!(bundle.sources_block.is_empty());
        assert!(bundle.citation_index_map.is_empty());
        assert_eq!(bundle.truncation.kept, 0);
        assert_eq!(bundle.truncation.dropped, 0);
        // 시스템 프롬프트는 *항상* 포함.
        assert_eq!(bundle.system_prompt, SYSTEM_PROMPT);
    }

    #[test]
    fn parse_citations_recognizes_in_range_markers() {
        let resp = "이건 사실입니다 [S1]. 또 다른 사실 [S2].";
        let parsed = parse_citations(resp, 3);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].marker, "S1");
        assert!(parsed[0].in_range);
        assert_eq!(parsed[1].marker, "S2");
        assert!(parsed[1].in_range);
    }

    #[test]
    fn parse_citations_flags_out_of_range_markers() {
        let resp = "[S1] 정상. [S99] 환각. [S0] 0번 자체가 무효.";
        let parsed = parse_citations(resp, 3);
        assert_eq!(parsed.len(), 3);
        assert!(parsed[0].in_range);
        assert!(!parsed[1].in_range);
        assert!(!parsed[2].in_range);
    }

    #[test]
    fn parse_citations_no_marker_returns_empty() {
        let resp = "마커가 전혀 없는 응답입니다.";
        let parsed = parse_citations(resp, 3);
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_citations_streaming_partial_marker_yields_no_match() {
        // 닫는 `]`까지 와야 매칭. 부분 입력은 스킵.
        let parsed = parse_citations("열린 마커 [S1 만 있음", 3);
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_citations_span_offsets_match_input() {
        let resp = "abc [S1] def [S2] ghi";
        let parsed = parse_citations(resp, 2);
        assert_eq!(parsed.len(), 2);
        assert_eq!(&resp[parsed[0].span.0..parsed[0].span.1], "[S1]");
        assert_eq!(&resp[parsed[1].span.0..parsed[1].span.1], "[S2]");
    }
}
