//! v0.4.3 PR 1 smoke test — Query rewriting layer 통합.
//!
//! 검증:
//!   1. fast_model 미지정 provider(MockProvider) → rewriter가 *원본 query 그대로* 반환.
//!   2. fast_model 채워진 mock(rewrite output 흉내) → rewriter가 첫 줄 정규화한 출력 반환.
//!   3. provider 에러 → 폴백으로 *원본 query*.
//!   4. RewritePolicy 라우팅 — settings.search_strength 매핑이 should_rewrite/should_hyde
//!      이 의도대로 분기.
//!
//! 본 smoke는 hybrid_search·DB·임베더 모두 *건드리지 않는다* — rewriter 함수 단독 통합
//! 만 책임. retrieval·DB 통합은 v041_retrieval_smoke / v042_cache_smoke가 담당하므로
//! 같은 트리에서 회귀 보장 (PR 2가 sentence window·MMR을 더하면 그 smoke가 추가됨).

use airis_lib::error::AppError;
use airis_lib::index::v043::rewriter::{HistoryTurn, QueryRewriter, RewritePolicy};
use airis_lib::llm::mock::MockProvider;
use airis_lib::llm::{ChatEvent, ChatRequest, ChatStream, LlmProvider, Role, Usage};
use async_trait::async_trait;

/// fast_model이 채워진 mock — rewriter가 *실제로 호출*되는 흐름 검증.
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

    async fn chat_stream(
        &self,
        _request: ChatRequest,
    ) -> Result<ChatStream, AppError> {
        let text = self.response.clone();
        let stream = async_stream::try_stream! {
            yield ChatEvent::TextDelta { text };
            yield ChatEvent::Done { usage: Usage::default() };
        };
        Ok(Box::pin(stream))
    }
}

struct AlwaysFailFastProvider;

#[async_trait]
impl LlmProvider for AlwaysFailFastProvider {
    fn fast_model(&self) -> &str {
        "fake-haiku"
    }

    async fn chat_stream(
        &self,
        _request: ChatRequest,
    ) -> Result<ChatStream, AppError> {
        Err(AppError::LlmApi {
            message: "smoke fail".into(),
        })
    }
}

#[tokio::test]
async fn rewriter_skips_when_fast_model_empty() {
    // MockProvider는 fast_model() = "" 폴백 → 원본 그대로.
    let provider = MockProvider::from_text_chunks(&["unused"]);
    let rewriter = QueryRewriter::new();
    let history = vec![HistoryTurn {
        role: Role::User,
        content: "이전 질문".into(),
    }];
    let out = rewriter
        .rewrite(&history, "이거 어떻게?", &provider)
        .await
        .unwrap();
    assert_eq!(out, "이거 어떻게?");
}

#[tokio::test]
async fn rewriter_returns_first_line_when_provider_responds() {
    // LLM이 부연 설명을 같이 주면 첫 줄만 사용.
    let provider = FastFakeProvider::new("PPU 구현 방법\n부연: ...");
    let rewriter = QueryRewriter::new();
    let history = vec![
        HistoryTurn {
            role: Role::User,
            content: "PPU란 뭐?".into(),
        },
        HistoryTurn {
            role: Role::Assistant,
            content: "Picture Processing Unit입니다.".into(),
        },
    ];
    let out = rewriter
        .rewrite(&history, "이거 어떻게 구현?", &provider)
        .await
        .unwrap();
    assert_eq!(out, "PPU 구현 방법");
}

#[tokio::test]
async fn rewriter_falls_back_to_original_on_error() {
    let provider = AlwaysFailFastProvider;
    let rewriter = QueryRewriter::new();
    let out = rewriter.rewrite(&[], "원본 질문", &provider).await.unwrap();
    assert_eq!(out, "원본 질문");
}

#[tokio::test]
async fn rewrite_policy_skip_branch_avoids_provider_call() {
    // 호출자가 RewritePolicy를 사전 검사해 rewriter를 건너뛰는 흐름 시뮬레이션.
    // commands::llm::chat_send가 이 패턴을 그대로 사용.
    let policy = RewritePolicy::Skip;
    assert!(!policy.should_rewrite());
    assert!(!policy.should_hyde());
}

#[tokio::test]
async fn rewrite_policy_rewrite_branch_calls_only_rewriter() {
    let policy = RewritePolicy::Rewrite;
    assert!(policy.should_rewrite());
    assert!(!policy.should_hyde(), "PR 1에선 HyDE 비활성 — Balanced default");
}

#[tokio::test]
async fn rewrite_policy_accurate_branch_enables_both_rewriter_and_hyde() {
    let policy = RewritePolicy::RewriteAndHyde;
    assert!(policy.should_rewrite());
    assert!(policy.should_hyde(), "Accurate 모드는 PR 3 HyDE까지 ON");
}
