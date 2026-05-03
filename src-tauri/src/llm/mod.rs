// LLM 프로바이더 추상화 (D-005).
// trait LlmProvider — chat_stream 한 메서드만 v0.1.
// v0.2부터 embed·available_models 등 추가 예정.

mod sse;

#[cfg(test)]
pub mod mock;

pub mod anthropic;
pub mod gemini;
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
}
