// Gemini Generative Language API 어댑터.
//
// 호출 흐름:
//   1. POST /v1beta/models/{model}:streamGenerateContent?alt=sse
//      Header: x-goog-api-key
//   2. 4xx → 즉시 에러, 5xx·rate-limit → 재시도/큐
//   3. 200 → bytes_stream → SseParser → JSON 해석 → ChatEvent 발사
//
// 응답 형식 (data 줄):
//   {"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":...,"safetyRatings":[...]}],"usageMetadata":{...}}
//
// safety / blockReason:
//   * promptFeedback.blockReason 또는 candidates[0].finishReason in ("SAFETY","RECITATION")
//   * 결정 #2 (handoff): 배너 알림 + 응답 그대로 표시. 재생성 X.
//   * 본 어댑터는 그 신호를 *로그*만 남김. UI 배너는 상위에서 처리 (PR 17).

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

const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const PROVIDER: &str = "gemini";
const MAX_RATE_LIMIT_RETRIES: u32 = 4;
const REQUEST_TIMEOUT_SECS: u64 = 60;

pub struct GeminiProvider {
    client: reqwest::Client,
}

impl GeminiProvider {
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

/// v0.4.3 PR 1 (D-086) — Gemini provider의 빠른 보조 모델 (architecture §4.12 표).
/// `gemini-flash-latest`는 Google이 노출하는 alias로 본격 검증은 v0.4.4 BUG-001 fix와
/// 함께(BUG-001은 stream 누적 — non-stream 한 번 호출에선 영향 X). PR 1에선 *시그니처만*
/// 박아두고 실제 호출은 사용 시점에 검증.
const GEMINI_FAST_MODEL: &str = "gemini-flash-latest";

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn fast_model(&self) -> &str {
        GEMINI_FAST_MODEL
    }

    async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream> {
        let api_key = secrets::get(PROVIDER)?;
        let body = build_request_body(&request);
        let url = build_url(&request.model);
        let response = send_with_backoff(&self.client, &url, &api_key, &body).await?;
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
                    if data.is_empty() {
                        continue;
                    }
                    let v: Value = serde_json::from_str(data).map_err(|e| AppError::LlmApi {
                        message: format!(
                            "[SSE-JSON] gemini chunk: {e}: {}",
                            truncate(data, 256)
                        ),
                    })?;

                    if let Some(reason) = block_reason(&v) {
                        warn!(
                            target: "llm",
                            provider = PROVIDER,
                            reason,
                            "content blocked by safety filter"
                        );
                    }

                    if let Some(text) = extract_text_delta(&v) {
                        yield ChatEvent::TextDelta { text };
                    }
                    update_usage(&v, &mut usage);

                    if let Some(finish) = first_finish_reason(&v) {
                        debug!(target: "llm", provider = PROVIDER, finish, "finish_reason");
                        if !emitted_done {
                            yield ChatEvent::Done { usage: usage.clone() };
                            emitted_done = true;
                        }
                    }
                }
            }
            // 안전망: finishReason 안 와도 stream 끝났으면 Done 1회.
            if !emitted_done {
                yield ChatEvent::Done { usage };
            }
        };

        Ok(Box::pin(event_stream))
    }
}

fn build_url(model: &str) -> String {
    format!("{GEMINI_BASE}/{model}:streamGenerateContent?alt=sse")
}

fn build_request_body(req: &ChatRequest) -> Value {
    // Gemini는 system_instruction을 별도 필드. messages는 contents 배열.
    let contents: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            json!({
                "role": gemini_role(m.role.as_str()),
                "parts": [{ "text": m.content }],
            })
        })
        .collect();

    let mut body = json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": req.max_tokens,
        },
    });

    if let Some(system) = &req.system {
        body["systemInstruction"] = json!({
            "parts": [{ "text": system }],
        });
    }

    body
}

/// 우리 trait의 role("user"·"assistant") → Gemini role("user"·"model") 변환.
fn gemini_role(role: &str) -> &str {
    match role {
        "assistant" => "model",
        other => other,
    }
}

async fn send_with_backoff(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &Value,
) -> AppResult<reqwest::Response> {
    for attempt in 0..MAX_RATE_LIMIT_RETRIES {
        let result = client
            .post(url)
            .header("x-goog-api-key", api_key)
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

/// `candidates[0].content.parts[0].text` 추출. parts가 여러 개면 모두 concat.
fn extract_text_delta(v: &Value) -> Option<String> {
    let parts = v.pointer("/candidates/0/content/parts")?.as_array()?;
    let mut combined = String::new();
    for p in parts {
        if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
            combined.push_str(t);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

/// `usageMetadata` — 매 chunk마다 누적값. 마지막 chunk 기준 final.
fn update_usage(v: &Value, usage: &mut Usage) {
    let Some(u) = v.get("usageMetadata") else {
        return;
    };
    if let Some(p) = u.get("promptTokenCount").and_then(|x| x.as_u64()) {
        usage.input_tokens = p as u32;
    }
    if let Some(c) = u.get("candidatesTokenCount").and_then(|x| x.as_u64()) {
        usage.output_tokens = c as u32;
    }
    if let Some(cached) = u.get("cachedContentTokenCount").and_then(|x| x.as_u64()) {
        usage.cache_read_input_tokens = cached as u32;
    }
}

fn first_finish_reason(v: &Value) -> Option<&str> {
    v.pointer("/candidates/0/finishReason")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
}

/// promptFeedback.blockReason 또는 finishReason ∈ {SAFETY, RECITATION, ...}.
fn block_reason(v: &Value) -> Option<String> {
    if let Some(reason) = v
        .pointer("/promptFeedback/blockReason")
        .and_then(|t| t.as_str())
    {
        return Some(format!("promptFeedback.blockReason={reason}"));
    }
    let finish = v
        .pointer("/candidates/0/finishReason")
        .and_then(|t| t.as_str())?;
    if matches!(
        finish,
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT"
    ) {
        Some(format!("finishReason={finish}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Message, Role};

    #[test]
    fn build_url_inserts_model() {
        let url = build_url("gemini-2.5-pro");
        assert!(url.starts_with(GEMINI_BASE));
        assert!(url.contains("/gemini-2.5-pro:streamGenerateContent?alt=sse"));
    }

    #[test]
    fn build_body_uses_model_role_and_system_instruction() {
        let req = ChatRequest {
            model: "gemini-2.5-pro".into(),
            system: Some("you help".into()),
            messages: vec![Message {
                role: Role::User,
                content: "안녕".into(),
            }],
            max_tokens: 2048,
            cache_breakpoints: Vec::new(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "안녕");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "you help");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 2048);
    }

    #[test]
    fn build_body_assistant_role_maps_to_model() {
        let req = ChatRequest {
            model: "gemini-2.5-flash".into(),
            system: None,
            messages: vec![Message {
                role: Role::Assistant,
                content: "이전 응답".into(),
            }],
            max_tokens: 100,
            cache_breakpoints: Vec::new(),
        };
        let body = build_request_body(&req);
        assert_eq!(body["contents"][0]["role"], "model");
    }

    #[test]
    fn extract_text_delta_concats_parts() {
        let data: Value = serde_json::from_str(
            r#"{"candidates":[{"content":{"parts":[{"text":"hel"},{"text":"lo"}],"role":"model"}}]}"#,
        )
        .unwrap();
        assert_eq!(extract_text_delta(&data).as_deref(), Some("hello"));
    }

    #[test]
    fn extract_text_delta_none_when_no_parts() {
        let data: Value =
            serde_json::from_str(r#"{"candidates":[{"finishReason":"STOP"}]}"#).unwrap();
        assert_eq!(extract_text_delta(&data), None);
    }

    #[test]
    fn update_usage_populates_prompt_and_candidates() {
        let v: Value = serde_json::from_str(
            r#"{"usageMetadata":{
                "promptTokenCount":120,
                "candidatesTokenCount":42,
                "cachedContentTokenCount":50
            }}"#,
        )
        .unwrap();
        let mut usage = Usage::default();
        update_usage(&v, &mut usage);
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 42);
        assert_eq!(usage.cache_read_input_tokens, 50);
    }

    #[test]
    fn block_reason_detects_prompt_feedback() {
        let v: Value =
            serde_json::from_str(r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#).unwrap();
        let reason = block_reason(&v).unwrap();
        assert!(reason.contains("blockReason=SAFETY"));
    }

    #[test]
    fn block_reason_detects_finish_safety() {
        let v: Value =
            serde_json::from_str(r#"{"candidates":[{"finishReason":"SAFETY"}]}"#).unwrap();
        let reason = block_reason(&v).unwrap();
        assert!(reason.contains("finishReason=SAFETY"));
    }

    #[test]
    fn block_reason_none_for_normal_stop() {
        let v: Value = serde_json::from_str(r#"{"candidates":[{"finishReason":"STOP"}]}"#).unwrap();
        assert_eq!(block_reason(&v), None);
    }
}
