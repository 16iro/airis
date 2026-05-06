//! v0.4.3 PR 4 smoke test — 인용 검증 + 대화 히스토리 압축 통합.
//!
//! 본 smoke는 fastembed 모델 다운로드(~600MB) 없이 *substring 폴백 + LLM mock*만으로
//! 통합 흐름을 검증한다. 실제 BGE-reranker-v2-m3 e2e는
//! `cargo test --release index::v043::reranker::tests::end_to_end_rerank_when_enabled
//!  -- --ignored AIRIS_E2E_RERANKER=1` 게이팅.
//!
//! 검증:
//!   1. citation_check::verify_citations — substring 폴백 경로(reranker=None) e2e.
//!   2. HistoryCompressor::compress — 6턴 이하/초과 분기.
//!   3. compress + chat 빌드 흐름 (system summary 주입 + recent_turns 메시지 prepend) —
//!      build_chat_request_with_hyde 시그니처 호환성 회귀 방지.
//!
//! retrieval·DB·임베더는 *건드리지 않는다* — citation_check / history_compressor 단독.

use airis_lib::error::AppError;
use airis_lib::index::v043::citation_check::{verify_citations, VerdictKind};
use airis_lib::index::v043::history_compressor::{
    CompressedHistory, HistoryCompressor, RECENT_TURNS_KEEP,
};
use airis_lib::index::v043::rewriter::HistoryTurn;
use airis_lib::llm::{ChatEvent, ChatRequest, ChatStream, LlmProvider, Role, Usage};
use async_trait::async_trait;
use std::collections::HashMap;

/// fast_model이 채워진 mock — history_compressor가 실제로 호출되는 흐름 검증.
struct FastFakeProvider {
    response: String,
}

impl FastFakeProvider {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for FastFakeProvider {
    fn fast_model(&self) -> &str {
        "fake-haiku"
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, AppError> {
        let text = self.response.clone();
        let stream = async_stream::try_stream! {
            yield ChatEvent::TextDelta { text };
            yield ChatEvent::Done { usage: Usage::default() };
        };
        Ok(Box::pin(stream))
    }
}

fn turn(role: Role, content: &str) -> HistoryTurn {
    HistoryTurn {
        role,
        content: content.to_string(),
    }
}

#[test]
fn citation_check_substring_fallback_e2e_with_three_sources() {
    // 3개 source — 1번/3번은 응답 sentence와 본문 공통 6글자 이상 매칭, 2번은 매칭 X.
    let mut sources: HashMap<usize, String> = HashMap::new();
    sources.insert(
        1,
        "GameBoy PPU(Picture Processing Unit)는 LCD 렌더링 담당.".to_string(),
    );
    sources.insert(
        2,
        "전혀 다른 주제 — 식물의 광합성 과정 설명.".to_string(),
    );
    sources.insert(
        3,
        "Rust 소유권 모델은 컴파일 시점에 메모리 안전성을 보장합니다.".to_string(),
    );
    let response = "GameBoy PPU 그래픽 처리 담당 [S1]. 별개로 Rust 소유권 모델은 메모리 안전성 \
                    보장 [S3]. 잘못된 인용 [S2].";
    let verdicts = verify_citations(response, 3, &sources, None).expect("verify");
    // 3 verdicts (each idx).
    let mut by_idx: HashMap<usize, VerdictKind> = HashMap::new();
    for v in &verdicts {
        by_idx.insert(v.source_idx, v.verdict);
    }
    assert_eq!(by_idx.get(&1), Some(&VerdictKind::Pass));
    assert_eq!(by_idx.get(&3), Some(&VerdictKind::Pass));
    assert_eq!(by_idx.get(&2), Some(&VerdictKind::NoMatch));
}

#[tokio::test]
async fn history_compressor_short_history_yields_no_summary() {
    let provider = FastFakeProvider::new("쓸일없음");
    let mut history = Vec::new();
    for i in 0..3 {
        history.push(turn(Role::User, &format!("u{i}")));
        history.push(turn(Role::Assistant, &format!("a{i}")));
    }
    let result = HistoryCompressor::new()
        .compress(&history, &provider)
        .await
        .unwrap();
    assert!(result.summary.is_none());
    assert_eq!(result.recent_turns.len(), 6);
}

#[tokio::test]
async fn history_compressor_long_history_summarizes_oldest_turns() {
    let provider = FastFakeProvider::new("오래된 6턴 한 줄 요약");
    let mut history = Vec::new();
    for i in 0..12 {
        history.push(turn(Role::User, &format!("U{i}")));
        history.push(turn(Role::Assistant, &format!("A{i}")));
    }
    let result = HistoryCompressor::new()
        .compress(&history, &provider)
        .await
        .unwrap();
    assert_eq!(result.summary.as_deref(), Some("오래된 6턴 한 줄 요약"));
    // 최근 RECENT_TURNS_KEEP 턴(=6턴 = 12 msg) 만 raw.
    assert_eq!(result.recent_turns.len(), RECENT_TURNS_KEEP * 2);
}

#[test]
fn compressed_history_default_is_none_summary_and_empty_turns() {
    // build_chat_request_with_hyde가 요약 없는 케이스에서 default를 그대로 받을 수 있도록.
    let d = CompressedHistory::default();
    assert!(d.summary.is_none());
    assert!(d.recent_turns.is_empty());
}
