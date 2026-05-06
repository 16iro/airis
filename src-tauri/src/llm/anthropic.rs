// Anthropic Messages API 어댑터.
// 호출 흐름:
//   1. POST /v1/messages (stream=true)
//   2. 4xx → 즉시 에러, 5xx·rate-limit → 재시도/큐
//   3. 200 → bytes_stream → SseParser → JSON 해석 → ChatEvent 발사
//
// 백오프: 429만 1s/2s/4s/8s ±20% jitter (8.6 절).
// 5xx는 큐 적재 (PR 6 워커가 처리). v0.1엔 큐 적재만, 워커 X.

use std::time::Duration;

use async_stream::try_stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};

use super::sse::{SseParseError, SseParser};
use super::{CacheBreakpoint, ChatEvent, ChatRequest, ChatStream, LlmProvider, Usage};
use crate::error::{AppError, AppResult};
use crate::secrets;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const PROVIDER: &str = "anthropic";
const MAX_RATE_LIMIT_RETRIES: u32 = 4;
const REQUEST_TIMEOUT_SECS: u64 = 60;

pub struct AnthropicProvider {
    client: reqwest::Client,
}

impl AnthropicProvider {
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

/// v0.4.3 PR 1 (D-086) — Anthropic provider의 빠른 보조 모델 (query rewriting·HyDE 등).
/// architecture §4.12 표 — Claude 라우팅에서 작업이 가벼운 항목은 Haiku 4.5 사용.
const ANTHROPIC_FAST_MODEL: &str = "claude-haiku-4-5";

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn fast_model(&self) -> &str {
        ANTHROPIC_FAST_MODEL
    }

    async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream> {
        let api_key = secrets::get(PROVIDER)?;

        let body = build_request_body(&request);

        // 백오프 루프 — rate limit만 재시도. 4xx/5xx/네트워크는 즉시 분기.
        let response = send_with_backoff(&self.client, &api_key, &body).await?;
        let stream = response.bytes_stream();

        // SSE → ChatEvent 변환 스트림. async-stream의 try_stream! 매크로로 작성.
        let event_stream = try_stream! {
            let mut parser = SseParser::new();
            let mut usage = Usage::default();
            let mut bytes_stream = Box::pin(stream);

            while let Some(chunk_result) = bytes_stream.next().await {
                let chunk = chunk_result.map_err(|e| AppError::LlmApi {
                    message: format!("[SSE-WIRE] network read: {e}"),
                })?;

                let sse_events = parser.feed(&chunk).map_err(map_sse_wire_err)?;

                for sse_event in sse_events {
                    match sse_event.event_type.as_deref() {
                        Some("content_block_delta") => {
                            if let Some(text) = extract_text_delta(&sse_event.data)? {
                                yield ChatEvent::TextDelta { text };
                            }
                        }
                        Some("message_start") => {
                            update_usage_from_message_start(&sse_event.data, &mut usage);
                        }
                        Some("message_delta") => {
                            update_usage_from_message_delta(&sse_event.data, &mut usage);
                        }
                        Some("message_stop") => {
                            yield ChatEvent::Done { usage: usage.clone() };
                        }
                        Some("error") => {
                            let err = parse_error_event(&sse_event.data);
                            error!(target: "llm", provider = PROVIDER, %err, "stream error event");
                            Err(err)?;
                        }
                        Some("ping") | Some("content_block_start") | Some("content_block_stop") => {
                            // 알지만 v0.1에 의미 없는 이벤트 — 그냥 흘려보냄.
                        }
                        Some(other) => {
                            warn!(
                                target: "llm",
                                provider = PROVIDER,
                                event_type = %other,
                                "[SSE-EVENT-UNKNOWN] skipping"
                            );
                        }
                        None => {
                            warn!(
                                target: "llm",
                                provider = PROVIDER,
                                "[SSE-EVENT-UNKNOWN] event without type field"
                            );
                        }
                    }
                }
            }
        };

        Ok(Box::pin(event_stream))
    }
}

fn build_request_body(req: &ChatRequest) -> Value {
    // 메시지 — cache_breakpoints에 Message(idx)가 있으면 해당 메시지에 cache_control 박는다.
    let messages: Vec<Value> = req
        .messages
        .iter()
        .enumerate()
        .map(|(idx, m)| {
            let cached = req
                .cache_breakpoints
                .iter()
                .any(|b| matches!(b, CacheBreakpoint::Message(i) if *i == idx));
            if cached {
                json!({
                    "role": m.role.as_str(),
                    "content": [{
                        "type": "text",
                        "text": m.content,
                        "cache_control": { "type": "ephemeral" }
                    }],
                })
            } else {
                json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })
            }
        })
        .collect();

    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": messages,
        "stream": true,
    });

    if let Some(system) = &req.system {
        let system_cached = req
            .cache_breakpoints
            .iter()
            .any(|b| matches!(b, CacheBreakpoint::System));
        body["system"] = if system_cached {
            json!([{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" }
            }])
        } else {
            json!(system)
        };
    }

    body
}

async fn send_with_backoff(
    client: &reqwest::Client,
    api_key: &str,
    body: &Value,
) -> AppResult<reqwest::Response> {
    for attempt in 0..MAX_RATE_LIMIT_RETRIES {
        let result = client
            .post(ANTHROPIC_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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
                // 5xx — 즉시 에러 (PR 6 큐 워커가 적재 처리할 예정)
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

/// 1s, 2s, 4s, 8s 기반 + ±20% jitter.
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

/// content_block_delta 이벤트에서 text_delta.text 추출.
/// 형식이 다르면 [SSE-PAYLOAD-UNKNOWN] warn + None 반환.
fn extract_text_delta(data: &str) -> AppResult<Option<String>> {
    let v: Value = serde_json::from_str(data).map_err(|e| AppError::LlmApi {
        message: format!(
            "[SSE-JSON] content_block_delta: {e}: {}",
            truncate(data, 256)
        ),
    })?;

    let Some(delta) = v.get("delta") else {
        warn!(
            target: "llm",
            provider = PROVIDER,
            "[SSE-PAYLOAD-UNKNOWN] content_block_delta missing 'delta'"
        );
        return Ok(None);
    };

    let delta_type = delta.get("type").and_then(|t| t.as_str());
    if delta_type != Some("text_delta") {
        // input_json_delta·thinking_delta 등 v0.1 이외의 delta — 무시.
        return Ok(None);
    }

    Ok(delta
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string()))
}

fn update_usage_from_message_start(data: &str, usage: &mut Usage) {
    let Ok(v): Result<Value, _> = serde_json::from_str(data) else {
        warn!(target: "llm", provider = PROVIDER, "[SSE-JSON] message_start parse failed");
        return;
    };
    let Some(u) = v.pointer("/message/usage") else {
        return;
    };
    usage.input_tokens = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    usage.cache_creation_input_tokens = u
        .get("cache_creation_input_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
    usage.cache_read_input_tokens = u
        .get("cache_read_input_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as u32;
}

fn update_usage_from_message_delta(data: &str, usage: &mut Usage) {
    let Ok(v): Result<Value, _> = serde_json::from_str(data) else {
        warn!(target: "llm", provider = PROVIDER, "[SSE-JSON] message_delta parse failed");
        return;
    };
    if let Some(out) = v.pointer("/usage/output_tokens").and_then(|x| x.as_u64()) {
        usage.output_tokens = out as u32;
    }
}

fn parse_error_event(data: &str) -> AppError {
    let v: Value = serde_json::from_str(data).unwrap_or_default();
    let kind = v
        .pointer("/error/type")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown");
    let message = v
        .pointer("/error/message")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    AppError::LlmApi {
        message: format!("anthropic error event: {kind}: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Message, Role};

    #[test]
    fn build_body_includes_stream_true_and_max_tokens() {
        let req = ChatRequest {
            model: "claude-opus-4-7".into(),
            system: Some("you are a helper".into()),
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
        assert_eq!(body["model"], "claude-opus-4-7");
        assert_eq!(body["system"], "you are a helper");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn build_body_wraps_system_in_cache_control_when_breakpoint_present() {
        let req = ChatRequest {
            model: "claude-opus-4-7".into(),
            system: Some("cached system text".into()),
            messages: vec![Message {
                role: Role::User,
                content: "hi".into(),
            }],
            max_tokens: 1024,
            cache_breakpoints: vec![CacheBreakpoint::System],
        };
        let body = build_request_body(&req);
        // system이 array 형태로 wrap + cache_control 박힘.
        assert!(body["system"].is_array());
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "cached system text");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn build_body_wraps_message_in_cache_control_when_breakpoint_present() {
        let req = ChatRequest {
            model: "claude-opus-4-7".into(),
            system: None,
            messages: vec![Message {
                role: Role::User,
                content: "L1+L2 prefix".into(),
            }],
            max_tokens: 1024,
            cache_breakpoints: vec![CacheBreakpoint::Message(0)],
        };
        let body = build_request_body(&req);
        // 해당 메시지의 content가 array + cache_control 박힘.
        assert!(body["messages"][0]["content"].is_array());
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn build_body_omits_system_when_none() {
        let req = ChatRequest {
            model: "claude-opus-4-7".into(),
            system: None,
            messages: vec![],
            max_tokens: 100,
            cache_breakpoints: Vec::new(),
        };
        let body = build_request_body(&req);
        assert!(body.get("system").is_none());
    }

    #[test]
    fn extract_text_delta_returns_text() {
        let data = r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}"#;
        let out = extract_text_delta(data).unwrap();
        assert_eq!(out.as_deref(), Some("hello"));
    }

    #[test]
    fn extract_text_delta_returns_none_for_other_delta_types() {
        let data =
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"x"}}"#;
        let out = extract_text_delta(data).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn extract_text_delta_errors_on_invalid_json() {
        let data = r#"{not valid"#;
        let err = extract_text_delta(data).unwrap_err();
        match err {
            AppError::LlmApi { message } => {
                assert!(message.contains("[SSE-JSON]"));
            }
            other => panic!("expected LlmApi, got {other:?}"),
        }
    }

    #[test]
    fn parse_error_event_extracts_kind_and_message() {
        let data = r#"{"type":"error","error":{"type":"overloaded_error","message":"too busy"}}"#;
        let err = parse_error_event(data);
        match err {
            AppError::LlmApi { message } => {
                assert!(message.contains("overloaded_error"));
                assert!(message.contains("too busy"));
            }
            other => panic!("expected LlmApi, got {other:?}"),
        }
    }

    #[test]
    fn update_usage_from_message_start_populates_input_and_cache_tokens() {
        let data = r#"{
          "type":"message_start",
          "message":{
            "id":"m1",
            "role":"assistant",
            "usage":{
              "input_tokens": 1234,
              "cache_creation_input_tokens": 100,
              "cache_read_input_tokens": 200,
              "output_tokens": 0
            }
          }
        }"#;
        let mut usage = Usage::default();
        update_usage_from_message_start(data, &mut usage);
        assert_eq!(usage.input_tokens, 1234);
        assert_eq!(usage.cache_creation_input_tokens, 100);
        assert_eq!(usage.cache_read_input_tokens, 200);
    }

    #[test]
    fn update_usage_from_message_delta_updates_output_only() {
        let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
        let mut usage = Usage {
            input_tokens: 100,
            ..Default::default()
        };
        update_usage_from_message_delta(data, &mut usage);
        assert_eq!(usage.output_tokens, 42);
        assert_eq!(usage.input_tokens, 100); // unchanged
    }

    #[test]
    fn backoff_delay_grows_geometrically() {
        // jitter는 ±20% 이내라 비교 시 여유.
        let d0 = backoff_delay_ms(0);
        let d3 = backoff_delay_ms(3);
        assert!((1000..1200).contains(&d0));
        assert!((8000..9600).contains(&d3));
    }
}
