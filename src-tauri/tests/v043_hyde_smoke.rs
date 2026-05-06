//! v0.4.3 PR 3 smoke test — HyDE (Hypothetical Document Embeddings) 통합.
//!
//! 검증:
//!   1. fast_model 미지정 provider(MockProvider) → HyDE가 *원본 query 그대로* 반환.
//!   2. fast_model 채워진 mock(가상 답변 단락) → HyDE가 *단락 통째*를 반환 (rewriter처럼
//!      첫 줄만 잘라내지 않음).
//!   3. provider 에러 → 폴백으로 *원본 query*.
//!   4. RewritePolicy Accurate가 should_hyde() = true 분기에 일치 — chat_send가
//!      HyDE 호출 트리거를 결정하는 데 사용.
//!
//! 본 smoke는 hybrid_search·DB·임베더 모두 *건드리지 않는다* — HyDE 함수 단독 통합만
//! 책임. retrieval·MMR query embedding 통합은 v041_retrieval_smoke /
//! v043_post_retrieval_smoke가 담당.

use airis_lib::error::AppError;
use airis_lib::index::v043::hyde::HydeGenerator;
use airis_lib::index::v043::rewriter::RewritePolicy;
use airis_lib::llm::mock::MockProvider;
use airis_lib::llm::{ChatEvent, ChatRequest, ChatStream, LlmProvider, Usage};
use async_trait::async_trait;

/// fast_model이 채워진 mock — HyDE가 *실제로 호출*되는 흐름 검증.
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

struct AlwaysFailFastProvider;

#[async_trait]
impl LlmProvider for AlwaysFailFastProvider {
    fn fast_model(&self) -> &str {
        "fake-haiku"
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, AppError> {
        Err(AppError::LlmApi {
            message: "hyde smoke fail".into(),
        })
    }
}

#[tokio::test]
async fn hyde_skips_when_fast_model_empty() {
    // MockProvider는 fast_model() = "" 폴백 → 원본 그대로.
    let provider = MockProvider::from_text_chunks(&["unused"]);
    let hyde = HydeGenerator::new();
    let out = hyde.generate("GameBoy PPU 구현", &provider).await.unwrap();
    assert_eq!(out, "GameBoy PPU 구현");
}

#[tokio::test]
async fn hyde_returns_full_paragraph_when_provider_responds() {
    // rewriter와 달리 단락 전체 유지 — 첫 줄만 잘라내지 않음.
    let provider = FastFakeProvider::new(
        "GameBoy의 PPU는 LCD를 갱신합니다.\n4단계 모드 머신으로 동작합니다.",
    );
    let hyde = HydeGenerator::new();
    let out = hyde.generate("GameBoy PPU 구현", &provider).await.unwrap();
    assert!(out.contains("GameBoy의 PPU는 LCD를 갱신합니다."));
    assert!(
        out.contains("4단계 모드 머신으로 동작합니다."),
        "두 번째 줄까지 포함되어야 — HyDE은 단락 전체 사용"
    );
}

#[tokio::test]
async fn hyde_falls_back_to_original_on_provider_error() {
    let provider = AlwaysFailFastProvider;
    let hyde = HydeGenerator::new();
    let out = hyde.generate("원본 질문", &provider).await.unwrap();
    assert_eq!(out, "원본 질문");
}

#[tokio::test]
async fn hyde_strips_label_prefix_in_output() {
    // LLM이 "답변:" 라벨을 붙여도 후처리가 제거.
    let provider = FastFakeProvider::new("답변: PPU는 LCD를 갱신합니다.");
    let hyde = HydeGenerator::new();
    let out = hyde.generate("PPU 동작", &provider).await.unwrap();
    assert_eq!(out, "PPU는 LCD를 갱신합니다.");
}

#[tokio::test]
async fn hyde_falls_back_when_llm_outputs_blank() {
    let provider = FastFakeProvider::new("   \n   ");
    let hyde = HydeGenerator::new();
    let out = hyde.generate("원본 질문", &provider).await.unwrap();
    assert_eq!(out, "원본 질문");
}

#[tokio::test]
async fn rewrite_policy_accurate_triggers_hyde() {
    // commands::llm::chat_send가 이 분기 신호로 HyDE 호출을 결정.
    let policy = RewritePolicy::RewriteAndHyde;
    assert!(policy.should_hyde(), "Accurate 모드는 PR 3 HyDE까지 ON");
    assert!(policy.should_rewrite());
    assert!(policy.should_postprocess());
}

#[tokio::test]
async fn rewrite_policy_balanced_skips_hyde() {
    let policy = RewritePolicy::Rewrite;
    assert!(!policy.should_hyde(), "Balanced 모드는 HyDE OFF");
    assert!(policy.should_rewrite());
}

#[tokio::test]
async fn rewrite_policy_fast_skips_hyde() {
    let policy = RewritePolicy::Skip;
    assert!(!policy.should_hyde(), "Fast 모드는 HyDE OFF");
    assert!(!policy.should_rewrite());
}
