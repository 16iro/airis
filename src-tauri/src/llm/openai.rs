// OpenAI Chat Completions API 어댑터.
//
// 호출 흐름:
//   1. POST /v1/chat/completions (stream=true, stream_options.include_usage=true)
//   2. 4xx → 즉시 에러, 5xx·rate-limit → 재시도/큐 (Anthropic과 동일)
//   3. 200 → bytes_stream → SseParser → JSON 해석 → ChatEvent 발사
//
// 응답 형식 (data 줄):
//   {"id":"chatcmpl-...","choices":[{"delta":{"content":"hello"},...}]}
//   ...
//   {"id":"...","choices":[{"delta":{},"finish_reason":"stop"}],"usage":{...}}
//   data: [DONE]    ← 종료 마커

use std::time::Duration;

use async_stream::try_stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};

use super::sse::{SseParseError, SseParser};
use super::{ChatEvent, ChatRequest, ChatStream, LlmProvider, Usage};
use crate::error::{AppError, AppResult};
use crate::secrets;

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const PROVIDER: &str = "openai";
const MAX_RATE_LIMIT_RETRIES: u32 = 4;
const REQUEST_TIMEOUT_SECS: u64 = 60;
const DONE_MARKER: &str = "[DONE]";

pub struct OpenAiProvider {
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| AppError::Internal {
                message: format!("http client init: {e}"),
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream> {
        let api_key = secrets::get(PROVIDER)?;
        let body = build_request_body(&request);
        let response = send_with_backoff(&self.client, &api_key, &body).await?;
        let stream = response.bytes_stream();

        let event_stream = try_stream! {
            let mut parser = SseParser::new();
            let mut usage = Usage::default();
            let mut emitted_done = false;
            let mut bytes_stream = Box::pin(stream);

            while let Some(chunk_result) = bytes_stream.next().await {
                let chunk = chunk_result.map_err(|e| AppError::LlmApi {
                    message: format!("[SSE-WIRE] network read: {e}"),
                })?;

                let sse_events = parser.feed(&chunk).map_err(map_sse_wire_err)?;

                for sse_event in sse_events {
                    let data = sse_event.data.trim();
                    if data == DONE_MARKER {
                        if !emitted_done {
                            yield ChatEvent::Done { usage: usage.clone() };
                            emitted_done = true;
                        }
                        continue;
                    }
                    if data.is_empty() {
                        continue;
                    }

                    let v: Value = serde_json::from_str(data).map_err(|e| AppError::LlmApi {
                        message: format!(
                            "[SSE-JSON] openai chunk: {e}: {}",
                            truncate(data, 256)
                        ),
                    })?;

                    if let Some(text) = extract_text_delta(&v) {
                        yield ChatEvent::TextDelta { text };
                    }
                    update_usage(&v, &mut usage);
                    if let Some(finish) = first_finish_reason(&v) {
                        debug!(target: "llm", provider = PROVIDER, finish, "finish_reason");
                    }
                }
            }
            // 안전망: [DONE] 안 와도 stream 끝났으면 Done 1회.
            if !emitted_done {
                yield ChatEvent::Done { usage };
            }
        };

        Ok(Box::pin(event_stream))
    }
}

fn build_request_body(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = &req.system {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for m in &req.messages {
        messages.push(json!({
            "role": m.role.as_str(),
            "content": m.content,
        }));
    }
    json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
    })
}

async fn send_with_backoff(
    client: &reqwest::Client,
    api_key: &str,
    body: &Value,
) -> AppResult<reqwest::Response> {
    for attempt in 0..MAX_RATE_LIMIT_RETRIES {
        let result = client
            .post(OPENAI_URL)
            .header("authorization", format!("Bearer {api_key}"))
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await;

        match result {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    info!(target: "llm", provider = PROVIDER, attempt, "request ok");
                    return Ok(resp);
                }
                let code = status.as_u16();
                if code == 429 {
                    let delay_ms = backoff_delay_ms(attempt);
                    warn!(
                        target: "llm",
                        provider = PROVIDER,
                        attempt,
                        delay_ms,
                        "rate limited, backing off"
                    );
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    continue;
                }
                let err_body = resp.text().await.unwrap_or_default();
                debug!(target: "llm", provider = PROVIDER, code, err_body, "non-success status");
                if code == 401 || code == 403 {
                    return Err(AppError::AuthRequired);
                }
                if (400..500).contains(&code) {
                    return Err(AppError::LlmApi {
                        message: format!("HTTP {code}: {}", truncate(&err_body, 256)),
                    });
                }
                return Err(AppError::LlmApi {
                    message: format!("HTTP {code}: server error"),
                });
            }
            Err(e) if e.is_connect() || e.is_timeout() => {
                error!(target: "llm", provider = PROVIDER, error = %e, "network error");
                return Err(AppError::NetworkUnavailable);
            }
            Err(e) => {
                return Err(AppError::LlmApi {
                    message: format!("http: {e}"),
                });
            }
        }
    }

    Err(AppError::RateLimited {
        retry_after_seconds: backoff_delay_ms(MAX_RATE_LIMIT_RETRIES) / 1000,
    })
}

fn backoff_delay_ms(attempt: u32) -> u64 {
    let base = 1000_u64 * 2_u64.pow(attempt);
    let jitter = (rand::random::<u64>()) % (base / 5);
    base + jitter
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

fn map_sse_wire_err(e: SseParseError) -> AppError {
    match e {
        SseParseError::Wire { reason, raw } => AppError::LlmApi {
            message: format!("[SSE-WIRE] {reason}: {raw}"),
        },
    }
}

/// `choices[0].delta.content` 추출. 없으면 None.
fn extract_text_delta(v: &Value) -> Option<String> {
    v.pointer("/choices/0/delta/content")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// `usage` 필드가 들어오면 누적 (보통 마지막 chunk).
/// OpenAI 필드명: prompt_tokens·completion_tokens·prompt_tokens_details.cached_tokens
fn update_usage(v: &Value, usage: &mut Usage) {
    let Some(u) = v.get("usage") else { return };
    if let Some(p) = u.get("prompt_tokens").and_then(|x| x.as_u64()) {
        usage.input_tokens = p as u32;
    }
    if let Some(c) = u.get("completion_tokens").and_then(|x| x.as_u64()) {
        usage.output_tokens = c as u32;
    }
    if let Some(cached) = u
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|x| x.as_u64())
    {
        usage.cache_read_input_tokens = cached as u32;
    }
}

fn first_finish_reason(v: &Value) -> Option<&str> {
    v.pointer("/choices/0/finish_reason")
        .and_then(|t| t.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Message, Role};

    #[test]
    fn build_body_includes_stream_and_system_as_system_role() {
        let req = ChatRequest {
            model: "gpt-4.1".into(),
            system: Some("you help".into()),
            messages: vec![Message {
                role: Role::User,
                content: "hi".into(),
            }],
            max_tokens: 1024,
            cache_breakpoints: Vec::new(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["model"], "gpt-4.1");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "you help");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "hi");
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn build_body_omits_system_message_when_none() {
        let req = ChatRequest {
            model: "gpt-4.1".into(),
            system: None,
            messages: vec![Message {
                role: Role::User,
                content: "hi".into(),
            }],
            max_tokens: 100,
            cache_breakpoints: Vec::new(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn extract_text_delta_returns_content() {
        let data: Value = serde_json::from_str(
            r#"{"choices":[{"delta":{"content":"hello"},"index":0,"finish_reason":null}]}"#,
        )
        .unwrap();
        assert_eq!(extract_text_delta(&data).as_deref(), Some("hello"));
    }

    #[test]
    fn extract_text_delta_none_when_empty() {
        let data: Value =
            serde_json::from_str(r#"{"choices":[{"delta":{},"index":0,"finish_reason":"stop"}]}"#)
                .unwrap();
        assert_eq!(extract_text_delta(&data), None);
    }

    #[test]
    fn update_usage_populates_input_output_and_cached() {
        let v: Value = serde_json::from_str(
            r#"{
                "choices":[],
                "usage":{
                    "prompt_tokens":1234,
                    "completion_tokens":42,
                    "prompt_tokens_details":{"cached_tokens":200}
                }
            }"#,
        )
        .unwrap();
        let mut usage = Usage::default();
        update_usage(&v, &mut usage);
        assert_eq!(usage.input_tokens, 1234);
        assert_eq!(usage.output_tokens, 42);
        assert_eq!(usage.cache_read_input_tokens, 200);
    }

    #[test]
    fn update_usage_noop_when_field_absent() {
        let v: Value = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
        let mut usage = Usage {
            input_tokens: 99,
            ..Default::default()
        };
        update_usage(&v, &mut usage);
        assert_eq!(usage.input_tokens, 99); // unchanged
    }
}
