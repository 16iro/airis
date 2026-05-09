// 테스트용 mock 프로바이더. 미리 큐잉된 ChatEvent들을 그대로 흘려보낸다.
// chat_send command 통합 테스트·UI 시뮬레이션에 사용.

use async_stream::try_stream;
use async_trait::async_trait;

use super::{ChatEvent, ChatRequest, ChatStream, LlmProvider};
use crate::error::AppResult;

pub struct MockProvider {
    events: Vec<ChatEvent>,
    /// fast_model() 반환값. 빈 문자열이면 LLM fast-path skip.
    model: String,
}

impl MockProvider {
    pub fn new(events: Vec<ChatEvent>) -> Self {
        Self {
            events,
            model: String::new(),
        }
    }

    /// fast_model() 이 non-empty 값을 반환하도록 설정.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// 텍스트 청크들을 순서대로 흘려보내고 마지막에 Done 이벤트.
    pub fn from_text_chunks(chunks: &[&str]) -> Self {
        let mut events: Vec<ChatEvent> = chunks
            .iter()
            .map(|t| ChatEvent::TextDelta {
                text: (*t).to_string(),
            })
            .collect();
        events.push(ChatEvent::Done {
            usage: Default::default(),
        });
        Self::new(events)
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn chat_stream(&self, _request: ChatRequest) -> AppResult<ChatStream> {
        let events = self.events.clone();
        let stream = try_stream! {
            for event in events {
                yield event;
            }
        };
        Ok(Box::pin(stream))
    }

    fn fast_model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Message, Role};
    use futures_util::StreamExt;

    fn req() -> ChatRequest {
        ChatRequest {
            model: "test".into(),
            system: None,
            messages: vec![Message {
                role: Role::User,
                content: "hi".into(),
            }],
            max_tokens: 1024,
            cache_breakpoints: Vec::new(),
        }
    }

    #[tokio::test]
    async fn streams_pre_recorded_chunks_in_order() {
        let provider = MockProvider::from_text_chunks(&["안", "녕", "하세요"]);
        let mut stream = provider.chat_stream(req()).await.unwrap();

        let mut text = String::new();
        let mut got_done = false;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                ChatEvent::TextDelta { text: t } => text.push_str(&t),
                ChatEvent::Done { .. } => got_done = true,
            }
        }
        assert_eq!(text, "안녕하세요");
        assert!(got_done);
    }
}
