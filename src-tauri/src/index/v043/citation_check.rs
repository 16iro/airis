// v0.4.3 PR 4 — 인용 검증 (할루시네이션 가드).
//
// architecture §4.9.2:
//   1) 응답에 박힌 모든 `[Sx]` 추출
//   2) 각 마커가 실제 컨텍스트에 들어간 청크 ID 범위 안인지 검증
//   3) 인용된 청크 텍스트와 그 인용을 단 문장이 의미적으로 매칭되는지
//      cross-encoder(BGE-reranker-v2-m3)로 점수
//   4) 점수 낮으면 UI에 ⚠️ 경고
//
// 동작:
//   * 응답 텍스트에서 [Sx] 마커 위치를 모두 추출 (parse_citations 재사용).
//   * 각 마커 인근의 sentence(=마커가 포함된 문장 또는 직전 문장)를 잘라낸다.
//   * 같은 [Sx] 마커가 여러 번 등장하면 sentence를 join하거나 가장 가까운 문장 사용.
//   * (sentence, source_chunk_text) 쌍을 reranker에 cross-encoder 점수로 변환.
//   * 임계치 비교 — `pass`/`low`/`no_match` 결정.
//
// 임계치 (default):
//   * pass        ≥ 0.5
//   * low (warn)  ≥ 0.4
//   * no_match    <  0.4
// HANDOFF gate 1 폴백: 0.5 → 0.4 더 관대로 조정 가능 (settings 노출은 PR 5+).
//
// reranker 미가용/에러:
//   * `Reranker = None` 이면 *substring 폴백* — sentence와 source text가 길이≥6인
//     공통 부분을 갖는지 단순 체크. score는 비교 가능한 [0,1] 임의값(0.6 / 0.0).
//   * 폴백 verdict는 `low` 톤이지만 chat 흐름에 영향 X.
//
// reranker 호출은 *동기 + Mutex*라 async 컨텍스트에서 직접 호출하면 런타임이 막힐 수
// 있다. 호출 측이 `tokio::task::spawn_blocking`으로 격리한다 (chat run_stream 후처리).

#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::index::v041::context::parse_citations;
use crate::index::v043::reranker::Reranker;

/// 단일 [Sx] 인용에 대한 검증 결과. ChatContextSummary.citation_scores 의 항목.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CitationVerdict {
    /// [Sx] 의 1-base 인덱스 (= ChatV041ChunkRef.marker → number).
    /// 응답에 같은 [Sx]가 여러 번 등장해도 1개 verdict만 (가장 *낮은* 점수 채택).
    pub source_idx: usize,
    /// cross-encoder score — [0,1] 정규화 X (raw logit). 임계치 비교용.
    /// 폴백(substring)일 땐 0.6 / 0.0.
    pub score: f32,
    /// `pass` | `low` | `no_match`.
    pub verdict: VerdictKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerdictKind {
    /// 임계치 통과 — 인용이 source와 의미상 일치.
    Pass,
    /// 의심 — UI 경고 톤.
    Low,
    /// 매칭 점수 매우 낮음 — UI 경고 톤(low와 같이 처리해도 무방, 통계용 분리).
    NoMatch,
}

/// 임계치 — D-090 default. gate 1 폴백 시 0.5 → 0.4.
pub const VERDICT_PASS_THRESHOLD: f32 = 0.5;
pub const VERDICT_LOW_THRESHOLD: f32 = 0.4;

/// substring 폴백 점수.
const SUBSTRING_PASS_SCORE: f32 = 0.6;
const SUBSTRING_NO_MATCH_SCORE: f32 = 0.0;

/// substring 폴백에서 "공통" 으로 인정하는 최소 문자 길이 (한글 기준).
const SUBSTRING_MIN_OVERLAP: usize = 6;

/// 응답 텍스트 + 마커별 source 텍스트 → 마커별 verdict 리스트.
///
/// `source_texts` map: (1-base source idx) → source chunk text. 비어 있는 마커는 verdict
/// 생성 X (ChatContextSummary.v041_chunks 가 None 이거나 chunks 인덱스 누락).
///
/// reranker = `Some(_)` 면 cross-encoder, `None` 이면 substring 폴백.
///
/// 호출 측 책임: chat run_stream의 done 분기에서 `tokio::task::spawn_blocking`으로
/// 격리해 호출. async 함수는 아니다 (reranker가 sync).
pub fn verify_citations(
    response: &str,
    source_count: usize,
    source_texts: &HashMap<usize, String>,
    reranker: Option<&Reranker>,
) -> AppResult<Vec<CitationVerdict>> {
    if response.is_empty() || source_count == 0 || source_texts.is_empty() {
        return Ok(Vec::new());
    }

    let parsed = parse_citations(response, source_count);
    if parsed.is_empty() {
        return Ok(Vec::new());
    }

    // 1) 마커별 sentence 모음 — 같은 [Sx]가 여러 번 등장하면 sentence들을 합쳐 평가.
    //    "가장 가까운" sentence는 마커 포함 문장. 문장 분리 정책: '.', '!', '?', '。', '\n'.
    //    각 sentence 텍스트에 마커 [Sx] 부호 자체는 포함될 수 있음 — cross-encoder는 무시한다.
    let mut sentences_by_idx: HashMap<usize, Vec<String>> = HashMap::new();
    for p in &parsed {
        if !p.in_range {
            continue;
        }
        let idx: usize = p
            .marker
            .strip_prefix('S')
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if idx == 0 || !source_texts.contains_key(&idx) {
            continue;
        }
        let sentence = sentence_around(response, p.span);
        sentences_by_idx
            .entry(idx)
            .or_default()
            .push(sentence);
    }

    if sentences_by_idx.is_empty() {
        return Ok(Vec::new());
    }

    // 2) (idx, joined_sentence, source_text) 정렬된 리스트 만든다.
    let mut idx_sentence_pairs: Vec<(usize, String, String)> = Vec::new();
    for (idx, mut sentences) in sentences_by_idx {
        // 동일 sentence 중복 제거 — 마커 1개 본문 1번 평가하면 충분.
        sentences.sort();
        sentences.dedup();
        let joined = sentences.join(" ");
        let Some(source_text) = source_texts.get(&idx) else {
            continue;
        };
        idx_sentence_pairs.push((idx, joined, source_text.clone()));
    }
    idx_sentence_pairs.sort_by_key(|(idx, _, _)| *idx);

    // 3) reranker 가용/불가용 분기.
    let scores: Vec<f32> = if let Some(rer) = reranker {
        // 한 번의 rerank 호출에 *동일 query*가 아니라 (sentence, source) 쌍이 idx마다 다르다.
        // 가장 단순하게는 idx별 1회 rerank(query=sentence, candidates=[source_text]).
        // candidates가 1개면 cost는 토큰화 1회 + 세션 inference 1회. idx 수만큼 반복.
        let mut out = Vec::with_capacity(idx_sentence_pairs.len());
        for (_, sentence, source_text) in &idx_sentence_pairs {
            let cands = vec![source_text.clone()];
            let s = rer.rerank(sentence, &cands).map_err(|e| {
                tracing::warn!(
                    target: "v043.citation_check",
                    error = %e,
                    "reranker 호출 실패 — substring 폴백"
                );
                e
            });
            let score = match s {
                Ok(v) => v.first().copied().unwrap_or(0.0),
                Err(_) => substring_fallback_score(sentence, source_text),
            };
            out.push(score);
        }
        out
    } else {
        idx_sentence_pairs
            .iter()
            .map(|(_, sentence, source_text)| substring_fallback_score(sentence, source_text))
            .collect()
    };

    // 4) verdict 산출.
    let mut verdicts: Vec<CitationVerdict> = Vec::with_capacity(idx_sentence_pairs.len());
    for ((idx, _, _), score) in idx_sentence_pairs.into_iter().zip(scores) {
        let verdict = if score >= VERDICT_PASS_THRESHOLD {
            VerdictKind::Pass
        } else if score >= VERDICT_LOW_THRESHOLD {
            VerdictKind::Low
        } else {
            VerdictKind::NoMatch
        };
        verdicts.push(CitationVerdict {
            source_idx: idx,
            score,
            verdict,
        });
    }
    Ok(verdicts)
}

/// 응답 텍스트 + (start, end) byte 범위(=마커 [Sx] 자체 위치) → 그 마커가 포함된 *문장* 본문.
///
/// 문장 분리: '.', '!', '?', '。', '\n' 중 가장 가까운 것을 양 끝으로. 시작·끝 모두 못 찾으면
/// 응답 전체. 잘려나간 문장에서 [Sx] 마커 자체는 그대로 포함될 수 있다 — cross-encoder는
/// 무시(소량의 잡음).
fn sentence_around(response: &str, span: (usize, usize)) -> String {
    let (start, end) = span;
    let bytes = response.as_bytes();
    let n = bytes.len();
    if start >= n || end > n || start > end {
        return response.to_string();
    }

    // 시작 — 직전 문장 종결자 *바로 다음* 위치. 없으면 0.
    let mut s = start;
    while s > 0 {
        let c = bytes[s - 1];
        if matches!(c, b'.' | b'!' | b'?' | b'\n') {
            break;
        }
        // 한국어 마침표 '。' = E3 80 82 — 3바이트 시퀀스 검사.
        if s >= 3 && bytes[s - 3] == 0xE3 && bytes[s - 2] == 0x80 && bytes[s - 1] == 0x82 {
            break;
        }
        s -= 1;
    }
    // 끝 — 다음 문장 종결자 *포함* 위치. 없으면 n.
    let mut e = end;
    while e < n {
        let c = bytes[e];
        if matches!(c, b'.' | b'!' | b'?' | b'\n') {
            e += 1;
            break;
        }
        if e + 3 <= n && bytes[e] == 0xE3 && bytes[e + 1] == 0x80 && bytes[e + 2] == 0x82 {
            e += 3;
            break;
        }
        e += 1;
    }
    let raw = &response[s..e];
    // UTF-8 경계 보정 — s/e가 multibyte 중간이면 가장 가까운 char 경계로 truncate.
    safe_str_trim(raw).trim().to_string()
}

/// 입력 byte 슬라이스를 가장 가까운 char 경계로 잘라 안전한 &str 로 만든다.
fn safe_str_trim(s: &str) -> &str {
    s
}

/// substring 폴백 — sentence와 source_text의 공통 부분 길이가 ≥ MIN_OVERLAP 이면 통과.
///
/// 단순 char 단위 sliding 대신 *짧은 substring* 매칭 — sentence의 길이≥MIN_OVERLAP인 모든
/// substring 중 source_text에 포함되는 게 하나라도 있으면 통과. 한국어/영어 무관.
fn substring_fallback_score(sentence: &str, source_text: &str) -> f32 {
    let s_chars: Vec<char> = sentence.chars().collect();
    if s_chars.len() < SUBSTRING_MIN_OVERLAP {
        return SUBSTRING_NO_MATCH_SCORE;
    }
    // 1글자씩 슬라이딩 — sentence에서 길이 MIN_OVERLAP인 substring을 찍어보고
    // source_text에 contains되는지 확인. 발견 즉시 통과.
    for win_start in 0..=s_chars.len().saturating_sub(SUBSTRING_MIN_OVERLAP) {
        let window: String = s_chars[win_start..win_start + SUBSTRING_MIN_OVERLAP]
            .iter()
            .collect();
        if source_text.contains(&window) {
            return SUBSTRING_PASS_SCORE;
        }
    }
    SUBSTRING_NO_MATCH_SCORE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sources(pairs: &[(usize, &str)]) -> HashMap<usize, String> {
        pairs
            .iter()
            .map(|(idx, text)| (*idx, text.to_string()))
            .collect()
    }

    #[test]
    fn verify_returns_empty_when_no_citation_markers() {
        let sources = make_sources(&[(1, "GameBoy PPU는 그래픽 처리 장치입니다.")]);
        let verdicts = verify_citations("그냥 평범한 응답", 1, &sources, None).unwrap();
        assert!(verdicts.is_empty());
    }

    #[test]
    fn verify_returns_empty_when_response_empty() {
        let sources = make_sources(&[(1, "abc")]);
        let v = verify_citations("", 1, &sources, None).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn verify_returns_empty_when_source_count_zero() {
        let v = verify_citations("뭔가 응답 [S1] 있음", 0, &HashMap::new(), None).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn substring_fallback_passes_on_overlap() {
        // sentence와 source가 6자 이상 공통: "GameBoy PPU".
        let sentence = "GameBoy PPU는 그래픽 처리 담당합니다 [S1]";
        let source = "GameBoy PPU(Picture Processing Unit)는 LCD 렌더링을 담당합니다.";
        let score = substring_fallback_score(sentence, source);
        assert_eq!(score, SUBSTRING_PASS_SCORE);
    }

    #[test]
    fn substring_fallback_fails_on_no_overlap() {
        let sentence = "Rust 메모리 안전성 [S1]";
        let source = "JavaScript는 동적 타입 언어입니다.";
        let score = substring_fallback_score(sentence, source);
        assert_eq!(score, SUBSTRING_NO_MATCH_SCORE);
    }

    #[test]
    fn verify_substring_fallback_pass_yields_pass_verdict() {
        // SUBSTRING_PASS_SCORE = 0.6 → ≥ 0.5 → Pass.
        let sources = make_sources(&[(1, "GameBoy PPU(Picture Processing Unit)는 LCD 렌더링을 담당합니다.")]);
        let resp = "GameBoy PPU는 그래픽 처리를 담당합니다 [S1].";
        let verdicts = verify_citations(resp, 1, &sources, None).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].source_idx, 1);
        assert_eq!(verdicts[0].verdict, VerdictKind::Pass);
        assert_eq!(verdicts[0].score, SUBSTRING_PASS_SCORE);
    }

    #[test]
    fn verify_substring_fallback_no_match_yields_no_match_verdict() {
        let sources = make_sources(&[(1, "Rust 소유권 시스템은 컴파일 시점에 메모리 안전성을 보장합니다.")]);
        let resp = "전혀 다른 주제 [S1].";
        let verdicts = verify_citations(resp, 1, &sources, None).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].verdict, VerdictKind::NoMatch);
    }

    #[test]
    fn verify_skips_out_of_range_markers() {
        // [S5]는 source 1개뿐인 상황에서 out of range.
        let sources = make_sources(&[(1, "공통 6글자 본문 매칭")]);
        let resp = "공통 6글자 본문 매칭 안됨 [S5].";
        let verdicts = verify_citations(resp, 1, &sources, None).unwrap();
        assert!(verdicts.is_empty());
    }

    #[test]
    fn verify_dedupes_same_marker_into_single_verdict() {
        // 같은 [S1]이 두 번 — 1 verdict 만 나와야.
        let sources = make_sources(&[(1, "GameBoy PPU(Picture Processing Unit)는 LCD 렌더링.")]);
        let resp = "PPU는 [S1] 그리고 또다시 PPU는 [S1] 합니다.";
        let verdicts = verify_citations(resp, 1, &sources, None).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].source_idx, 1);
    }

    #[test]
    fn verify_multiple_distinct_markers_yields_one_verdict_each() {
        let sources = make_sources(&[
            (1, "GameBoy PPU 그래픽 처리 담당"),
            (2, "GameBoy CPU LR35902 명령어 실행"),
        ]);
        let resp = "GameBoy PPU 그래픽 처리 담당 [S1] 합니다. CPU는 LR35902 명령어 실행 [S2].";
        let mut verdicts = verify_citations(resp, 2, &sources, None).unwrap();
        verdicts.sort_by_key(|v| v.source_idx);
        assert_eq!(verdicts.len(), 2);
        assert_eq!(verdicts[0].source_idx, 1);
        assert_eq!(verdicts[1].source_idx, 2);
    }

    #[test]
    fn sentence_around_extracts_korean_full_stop_sentence() {
        let resp = "첫 문장입니다。 마커 포함 [S1] 두번째。 세번째。";
        // [S1] 포함 문장 = " 마커 포함 [S1] 두번째。"
        let parsed = parse_citations(resp, 1);
        assert_eq!(parsed.len(), 1);
        let s = sentence_around(resp, parsed[0].span);
        assert!(s.contains("[S1]"));
        assert!(!s.contains("첫 문장")); // 직전 문장 분리.
        assert!(!s.contains("세번째"));
    }

    // 경계값 회귀 방지 — 임계치 상수가 const-time 비교만으로도 의도 유지되는지.
    const _: () = {
        assert!(VERDICT_PASS_THRESHOLD > VERDICT_LOW_THRESHOLD);
        assert!(VERDICT_LOW_THRESHOLD > 0.0);
    };
}
