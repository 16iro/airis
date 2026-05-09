// LLM 프로바이더 추상화 (D-005).
// trait LlmProvider — chat_stream 한 메서드만 v0.1.
// v0.2부터 embed·available_models 등 추가 예정.

mod sse;
pub mod extraction;

// 통합 테스트(`tests/v043_rewriter_smoke.rs`)가 mock provider를 외부 크레이트 경로로
// 사용 — cfg(test) 게이트를 풀어 외부에서도 가져갈 수 있게 한다. 일반 빌드에 추가
// 코드 사이즈는 무시할 수준이며, runtime 진입점에서 호출하지 않으므로 사용자 빌드
// 영향 없음.
pub mod mock;

pub mod anthropic;
pub mod claude_cli;
pub mod codex_cli;
pub mod gemini;
pub mod gemini_cli;
pub mod openai;

use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;
use serde::Serialize;

use crate::error::AppResult;

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    /// 호출자가 제안하는 prompt cache 경계 (D-036).
    /// 어댑터가 활용 — Anthropic은 cache_control={type:"ephemeral"} 박음.
    /// OpenAI는 자동 prefix 캐시라 무시. Gemini cachedContents는 v0.3+로 이연.
    pub cache_breakpoints: Vec<CacheBreakpoint>,
}

/// 캐시 경계 위치 — 호출자가 *어디에* cache_control 박을지 명시.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheBreakpoint {
    /// system prompt 마지막에 cache_control.
    System,
    /// messages[idx]의 마지막에 cache_control. idx 범위 밖이면 어댑터가 무시.
    Message(usize),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// 스트림 한 단위. TextDelta는 즉시 UI에 흘리고, Done은 종료 시 1회.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum ChatEvent {
    TextDelta { text: String },
    Done { usage: Usage },
}

/// LLM usage 메타데이터. db.chat_messages·비용 가시화에 사용.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
}

/// chat_stream의 반환 — `Send + 'static` Stream을 box로 동적 디스패치.
pub type ChatStream = Pin<Box<dyn Stream<Item = AppResult<ChatEvent>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 스트리밍 호출. 백오프·재시도는 구현체 내부에서 처리.
    async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream>;

    /// v0.4.3 PR 1 (D-086) — query rewriting·HyDE·follow-up 분류 등 *작은 보조 LLM 호출*
    /// 에 쓸 빠르고 저렴한 모델 이름 (architecture §4.12).
    ///
    /// 호출자는 `chat_stream`에 `ChatRequest { model: provider.fast_model(...), ... }` 형태로
    /// 박아 보낸다. `chat_stream`은 모델 이름을 그대로 wire에 실어 보내므로 별도 분기 X.
    ///
    /// 디폴트 구현은 빈 문자열 — *항상* `provider.fast_model().is_empty()`로 가용성을
    /// 점검할 것 (mock 등 고정 모델 어댑터 호환).
    fn fast_model(&self) -> &str {
        ""
    }
}
