// v0.4.4 PR 5 (D-095) — BYOK 클라우드 임베딩 어댑터.
//
// 어댑터 trait `CloudEmbedder` + 1개 구현 `VoyageEmbedder`. 폴백 자리(Gemini)는
// 본 PR에 미구현 — settings·keyring 자리만 잡아두고 실제 호출은 후속 슬라이스.
//
// 호출 흐름 (Voyage):
//   1. POST https://api.voyageai.com/v1/embeddings
//      body = { "input": [...], "model": "voyage-3-lite", "input_type": "document" | "query" }
//   2. 4xx → 즉시 에러 (잘못된 키 / 형식 오류 등 사용자 액션 필요)
//   3. 5xx → 즉시 에러 (PR 5 범위엔 큐 적재 X — 임베딩은 인덱싱 시점이라 잡 단위 재시도)
//   4. 200 → response.data[].embedding 추출 → Vec<Vec<f32>>
//
// 본 PR은 *어댑터 + 호출 형식 + 단위 테스트*까지. 실제 인덱서 분기·`vectors_byok` 테이블
// 신설은 v0.4.4.1 또는 v0.4.5에서.
//
// 차원 mismatch 주의: voyage-3-lite는 *512차원*. mE5-small INT8(384d)·BGE-M3(1024d)와
// 모두 다르다. 본 PR은 *어댑터 dim()* 가시화까지만.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

use crate::error::{AppError, AppResult};

const REQUEST_TIMEOUT_SECS: u64 = 30;

/// 클라우드 임베딩 어댑터 — `embed_passages` / `embed_query` 두 메서드만 노출.
///
/// 모델·차원 구분(passage vs query)은 어댑터 내부에서 처리한다. 호출 측은 평문 청크/질의를
/// 그대로 넘기면 된다 — fastembed `Embedder` 패턴(`passage_prefix`/`query_prefix`)과 달리
/// 본 어댑터는 *입력 전처리*까지 책임 (Voyage는 prefix 대신 `input_type` 필드 사용).
#[async_trait]
pub trait CloudEmbedder: Send + Sync {
    /// 어댑터 이름 — 로그·UI 표시용. `"voyage-3-lite"` 같은 모델 식별자.
    fn name(&self) -> &str;

    /// 임베딩 차원. settings·migration 단계에서 *vec0 가상 테이블 dim*과 일관성 검증.
    /// voyage-3-lite = 512 / Gemini text-embedding-004 = 768.
    fn dim(&self) -> usize;

    /// 청크 본문 배열 → 임베딩 벡터 배열. mE5의 `passage:` prefix 같은 전처리는 어댑터 내부.
    async fn embed_passages(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>>;

    /// 단일 사용자 질의 → 임베딩 벡터. 어댑터가 query input_type으로 전송.
    async fn embed_query(&self, query: &str) -> AppResult<Vec<f32>>;
}

/// 본 PR이 지원하는 BYOK provider — Voyage만. Gemini 자리는 settings·keyring에만 잡아두고
/// 어댑터 구현은 후속 슬라이스.
///
/// `serde(rename_all = "lowercase")` — TypeScript 측 union literal과 직접 매칭.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ByokProvider {
    /// Voyage AI — 본 PR 1차 구현.
    Voyage,
    /// Google Gemini Embedding — 본 PR엔 자리만(어댑터 미구현). v0.4.5+.
    Gemini,
}

impl ByokProvider {
    /// keyring entry 식별자. 기존 `anthropic`/`openai`/`gemini` 키와 분리되도록 별도 prefix.
    pub fn keyring_id(&self) -> &'static str {
        match self {
            Self::Voyage => "voyage-byok-embedding",
            Self::Gemini => "gemini-byok-embedding",
        }
    }

    /// 추천(기본) 모델 — UI dropdown 초기값.
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Voyage => "voyage-3-lite",
            Self::Gemini => "text-embedding-004",
        }
    }

    /// 모델 이름 → 차원 매핑. UI에 안내값 표시 + 어댑터 호출 후 차원 검증에 사용.
    /// 알 수 없는 모델은 None 반환 — 어댑터가 응답을 받은 뒤 실측 차원으로 보정.
    pub fn known_dim(model: &str) -> Option<usize> {
        match model {
            "voyage-3" => Some(1024),
            "voyage-3-lite" => Some(512),
            "text-embedding-004" => Some(768),
            _ => None,
        }
    }
}

/// settings.json에 영속되는 BYOK 설정. None이면 BYOK 비활성 (기본).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByokConfig {
    pub provider: ByokProvider,
    /// 사용자가 dropdown에서 고른 모델. 어댑터 init 시 그대로 전달.
    pub model: String,
}

impl ByokConfig {
    /// 차원 안내값 — `ByokProvider::known_dim`과 동일하지만 인스턴스 메서드로 한 번 더 노출.
    pub fn known_dim(&self) -> Option<usize> {
        ByokProvider::known_dim(&self.model)
    }
}

// ---- Voyage 어댑터 ---------------------------------------------------------

/// Voyage AI 임베딩 어댑터. https://docs.voyageai.com/reference/embeddings-api
///
/// 호출 형식 (POST /v1/embeddings):
///   ```json
///   {
///     "input": ["text1", "text2"],
///     "model": "voyage-3-lite",
///     "input_type": "document" | "query"
///   }
///   ```
///
/// 응답 형식:
///   ```json
///   { "data": [{ "embedding": [...] }, ...] }
///   ```
pub struct VoyageEmbedder {
    api_key: String,
    model: String,
    dim: usize,
    base_url: String,
    client: reqwest::Client,
}

impl VoyageEmbedder {
    pub const DEFAULT_BASE_URL: &'static str = "https://api.voyageai.com/v1/embeddings";

    /// 새 Voyage 어댑터 — keyring에서 키를 꺼내 보관 + reqwest client init.
    /// 차원은 모델 이름에서 lookup. 알 수 없는 모델이면 voyage-3-lite 기본값(512d) 폴백.
    pub fn new(api_key: String, model: String) -> AppResult<Self> {
        let dim =
            ByokProvider::known_dim(&model).unwrap_or(512);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| AppError::Internal {
                message: format!("voyage http client init: {e}"),
            })?;
        Ok(Self {
            api_key,
            model,
            dim,
            base_url: Self::DEFAULT_BASE_URL.to_string(),
            client,
        })
    }

    /// 테스트용 base_url override. 실제 코드 경로는 `new`만 사용.
    #[cfg(test)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    async fn call_embed(
        &self,
        inputs: Vec<&str>,
        input_type: &'static str,
    ) -> AppResult<Vec<Vec<f32>>> {
        let body = json!({
            "input": inputs,
            "model": self.model,
            "input_type": input_type,
        });

        let resp = self
            .client
            .post(&self.base_url)
            .header("authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    AppError::NetworkUnavailable
                } else {
                    AppError::LlmApi {
                        message: format!("voyage request: {e}"),
                    }
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let code = status.as_u16();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(map_http_error(code, &err_text));
        }

        let parsed: VoyageResponse = resp.json().await.map_err(|e| AppError::LlmApi {
            message: format!("voyage response parse: {e}"),
        })?;

        let mut out: Vec<Vec<f32>> = Vec::with_capacity(parsed.data.len());
        for item in parsed.data {
            if item.embedding.len() != self.dim {
                // 응답 차원이 known_dim과 다르면 *명시 에러* — 인덱스 적재 전에 빠르게 차단.
                return Err(AppError::Internal {
                    message: format!(
                        "voyage dim mismatch: expected {} / got {} (model={})",
                        self.dim,
                        item.embedding.len(),
                        self.model,
                    ),
                });
            }
            out.push(item.embedding);
        }
        Ok(out)
    }
}

#[async_trait]
impl CloudEmbedder for VoyageEmbedder {
    fn name(&self) -> &str {
        &self.model
    }

    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed_passages(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inputs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.call_embed(inputs, "document").await
    }

    async fn embed_query(&self, query: &str) -> AppResult<Vec<f32>> {
        let inputs = vec![query];
        let mut vecs = self.call_embed(inputs, "query").await?;
        vecs.pop().ok_or_else(|| AppError::LlmApi {
            message: "voyage embed_query: empty data array".to_string(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageDatum>,
}

#[derive(Debug, Deserialize)]
struct VoyageDatum {
    embedding: Vec<f32>,
}

fn map_http_error(code: u16, body: &str) -> AppError {
    let snippet = truncate(body, 256);
    if code == 401 || code == 403 {
        // 키 자체가 무효 — 사용자 재입력 필요. Settings UI가 명시 안내하도록 AuthRequired.
        AppError::AuthRequired
    } else if code == 429 {
        // Voyage rate limit. 본 PR은 큐 X — 사용자에게 그대로 노출.
        AppError::RateLimited {
            retry_after_seconds: 30,
        }
    } else {
        // 4xx (401·403·429 외) + 5xx 모두 동일 LlmApi 메시지로 노출. 5xx 큐 적재는 본 PR 범위 X
        // — 임베딩은 인덱싱 잡 단위라 잡 자체가 재시도되면서 자연스레 회복.
        AppError::LlmApi {
            message: format!("voyage HTTP {code}: {snippet}"),
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dim_returns_canonical_dims() {
        assert_eq!(ByokProvider::known_dim("voyage-3-lite"), Some(512));
        assert_eq!(ByokProvider::known_dim("voyage-3"), Some(1024));
        assert_eq!(ByokProvider::known_dim("text-embedding-004"), Some(768));
        assert_eq!(ByokProvider::known_dim("does-not-exist"), None);
    }

    #[test]
    fn provider_keyring_ids_are_distinct_from_llm_keys() {
        // 기존 anthropic/openai/gemini LLM 키와 절대 충돌 X.
        assert_ne!(ByokProvider::Voyage.keyring_id(), "gemini");
        assert_ne!(ByokProvider::Voyage.keyring_id(), "openai");
        assert!(ByokProvider::Voyage.keyring_id().contains("byok"));
        assert!(ByokProvider::Gemini.keyring_id().contains("byok"));
    }

    #[test]
    fn provider_default_model_matches_known_dim() {
        // dropdown 초기값과 known_dim 매핑 일관성.
        let voyage = ByokProvider::Voyage.default_model();
        assert_eq!(voyage, "voyage-3-lite");
        assert!(ByokProvider::known_dim(voyage).is_some());

        let gemini = ByokProvider::Gemini.default_model();
        assert_eq!(gemini, "text-embedding-004");
        assert!(ByokProvider::known_dim(gemini).is_some());
    }

    #[test]
    fn provider_lowercase_serialization() {
        // frontend TS literal과 직접 매칭.
        assert_eq!(
            serde_json::to_string(&ByokProvider::Voyage).unwrap(),
            "\"voyage\""
        );
        assert_eq!(
            serde_json::to_string(&ByokProvider::Gemini).unwrap(),
            "\"gemini\""
        );
    }

    #[test]
    fn voyage_embedder_new_resolves_dim_from_model() {
        let e = VoyageEmbedder::new("test-key".into(), "voyage-3-lite".into()).unwrap();
        assert_eq!(e.dim(), 512);
        assert_eq!(e.name(), "voyage-3-lite");

        let e = VoyageEmbedder::new("test-key".into(), "voyage-3".into()).unwrap();
        assert_eq!(e.dim(), 1024);
    }

    #[test]
    fn voyage_embedder_unknown_model_falls_back_to_512() {
        // 알 수 없는 모델은 voyage-3-lite 차원으로 폴백 — 응답 차원 mismatch 에러로 잡힘.
        let e = VoyageEmbedder::new("test-key".into(), "voyage-future".into()).unwrap();
        assert_eq!(e.dim(), 512);
    }

    #[test]
    fn map_http_error_401_is_auth_required() {
        let err = map_http_error(401, "invalid api key");
        assert!(matches!(err, AppError::AuthRequired));
        let err = map_http_error(403, "forbidden");
        assert!(matches!(err, AppError::AuthRequired));
    }

    #[test]
    fn map_http_error_429_is_rate_limited() {
        let err = map_http_error(429, "rate limit");
        assert!(matches!(err, AppError::RateLimited { .. }));
    }

    #[test]
    fn map_http_error_400_other_is_llm_api() {
        let err = map_http_error(400, "bad request — model not found");
        match err {
            AppError::LlmApi { message } => {
                assert!(message.contains("400"));
                assert!(message.contains("model not found"));
            }
            other => panic!("expected LlmApi, got {other:?}"),
        }
    }

    #[test]
    fn truncate_keeps_short_strings_intact() {
        assert_eq!(truncate("short", 256), "short");
    }

    #[test]
    fn truncate_appends_ellipsis_when_oversized() {
        let long = "x".repeat(300);
        let t = truncate(&long, 16);
        assert!(t.starts_with(&"x".repeat(16)));
        assert!(t.ends_with('…'));
    }

    #[test]
    fn byok_config_lowercase_round_trip() {
        let cfg = ByokConfig {
            provider: ByokProvider::Voyage,
            model: "voyage-3-lite".into(),
        };
        let s = serde_json::to_string(&cfg).unwrap();
        assert!(s.contains("\"provider\":\"voyage\""));
        assert!(s.contains("\"model\":\"voyage-3-lite\""));

        let back: ByokConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn byok_config_known_dim_passthrough() {
        let cfg = ByokConfig {
            provider: ByokProvider::Voyage,
            model: "voyage-3-lite".into(),
        };
        assert_eq!(cfg.known_dim(), Some(512));

        let cfg = ByokConfig {
            provider: ByokProvider::Gemini,
            model: "text-embedding-004".into(),
        };
        assert_eq!(cfg.known_dim(), Some(768));
    }

    // ---- HTTP 통합 테스트 (자체 mock 서버) -------------------------------------
    //
    // mockito/wiremock 추가 의존 없이 std + tokio가 이미 트리에 있으므로 간단한
    // TcpListener 위에서 fixed response를 돌려준다. Content-Length는 본문 byte 수에서
    // 자동 계산 — 수기 매핑 실수 회피.

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// 단일 요청 mock 서버. 받은 raw HTTP body를 반환하고, status·body로부터 자동 조립한
    /// HTTP/1.1 응답을 1회 송신. body는 byte 길이 그대로 Content-Length 박힘.
    fn spawn_mock_server(
        status: u16,
        body: &'static str,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/v1/embeddings");
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .ok();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .ok();

                // request body 끝까지 읽기 — Content-Length 확인 후 정확히 그만큼 더 읽는다.
                // 단순 테스트라 헤더 8KB 이내 가정.
                let mut buf = [0u8; 8192];
                let mut acc: Vec<u8> = Vec::new();
                let mut header_end: Option<usize> = None;
                while header_end.is_none() {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            acc.extend_from_slice(&buf[..n]);
                            if let Some(idx) = find_header_terminator(&acc) {
                                header_end = Some(idx + 4);
                            }
                        }
                        Err(_) => break,
                    }
                }
                if let Some(hend) = header_end {
                    let header_str = String::from_utf8_lossy(&acc[..hend]).to_string();
                    let cl = parse_content_length(&header_str).unwrap_or(0);
                    let already_body = acc.len() - hend;
                    let mut to_read = cl.saturating_sub(already_body);
                    while to_read > 0 {
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                acc.extend_from_slice(&buf[..n]);
                                to_read = to_read.saturating_sub(n);
                            }
                            Err(_) => break,
                        }
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&acc).into_owned());

                let status_line = match status {
                    200 => "HTTP/1.1 200 OK",
                    401 => "HTTP/1.1 401 Unauthorized",
                    _ => "HTTP/1.1 500 Internal Server Error",
                };
                let body_bytes = body.as_bytes();
                let mut response = Vec::new();
                response.extend_from_slice(status_line.as_bytes());
                response.extend_from_slice(b"\r\n");
                response.extend_from_slice(b"Content-Type: application/json\r\n");
                response.extend_from_slice(
                    format!("Content-Length: {}\r\n", body_bytes.len()).as_bytes(),
                );
                response.extend_from_slice(b"Connection: close\r\n\r\n");
                response.extend_from_slice(body_bytes);
                let _ = stream.write_all(&response);
                let _ = stream.flush();
                // 클라이언트가 body를 받기 전에 close되지 않도록 잠시 대기.
                thread::sleep(Duration::from_millis(50));
            }
        });
        (url, rx)
    }

    fn find_header_terminator(buf: &[u8]) -> Option<usize> {
        // CRLF CRLF 검색.
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }

    fn parse_content_length(headers: &str) -> Option<usize> {
        for line in headers.lines() {
            let lower = line.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("content-length:") {
                return rest.trim().parse::<usize>().ok();
            }
        }
        None
    }

    #[tokio::test]
    async fn embed_passages_parses_voyage_response() {
        let body = "{\"data\":[{\"embedding\":[0.1,0.2,0.3]},{\"embedding\":[0.4,0.5,0.6]}]}";
        let (url, _rx) = spawn_mock_server(200, body);
        let mut e = VoyageEmbedder::new("test-key".into(), "voyage-test".into()).unwrap();
        // 가상 모델 — known_dim에 없으므로 폴백 512. 응답이 3차원이라 dim 검증 위해 직접 set.
        e.dim = 3;
        let e = e.with_base_url(url);

        let result = e
            .embed_passages(&["hello".to_string(), "world".to_string()])
            .await
            .expect("embed ok");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec![0.1f32, 0.2, 0.3]);
        assert_eq!(result[1], vec![0.4f32, 0.5, 0.6]);
    }

    #[tokio::test]
    async fn embed_query_uses_query_input_type() {
        let body = "{\"data\":[{\"embedding\":[0.7,0.8,0.9]}]}";
        let (url, rx) = spawn_mock_server(200, body);
        let mut e = VoyageEmbedder::new("test-key".into(), "voyage-test".into()).unwrap();
        e.dim = 3;
        let e = e.with_base_url(url);

        let q = e.embed_query("질문").await.expect("query ok");
        assert_eq!(q, vec![0.7f32, 0.8, 0.9]);

        let raw_request = rx.recv_timeout(Duration::from_secs(2)).expect("request");
        assert!(
            raw_request.contains("\"input_type\":\"query\""),
            "expected input_type=query, got: {raw_request}"
        );
        assert!(
            raw_request
                .to_lowercase()
                .contains("authorization: bearer test-key"),
            "expected bearer auth header, got: {raw_request}"
        );
    }

    #[tokio::test]
    async fn embed_passages_401_returns_auth_required() {
        let body = "{\"detail\":\"Invalid API key.\"}";
        let (url, _rx) = spawn_mock_server(401, body);
        let e = VoyageEmbedder::new("bad-key".into(), "voyage-3-lite".into())
            .unwrap()
            .with_base_url(url);

        let err = e
            .embed_passages(&["x".to_string()])
            .await
            .expect_err("should fail");
        assert!(matches!(err, AppError::AuthRequired));
    }

    #[tokio::test]
    async fn embed_passages_dim_mismatch_returns_internal() {
        // 응답이 4개 elem이지만 known_dim=512 기대 → 명시 에러.
        let body = "{\"data\":[{\"embedding\":[0.1,0.2,0.3,0.4]}]}";
        let (url, _rx) = spawn_mock_server(200, body);
        let e = VoyageEmbedder::new("test-key".into(), "voyage-3-lite".into())
            .unwrap()
            .with_base_url(url);
        let err = e
            .embed_passages(&["x".to_string()])
            .await
            .expect_err("dim mismatch should fail");
        match err {
            AppError::Internal { message } => {
                assert!(
                    message.contains("dim mismatch"),
                    "expected dim mismatch message, got: {message}"
                );
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn embed_passages_empty_input_short_circuits() {
        // 호출 자체가 일어나지 않으므로 mock 서버 X — 호출 0건이면 통과.
        let e = VoyageEmbedder::new("test-key".into(), "voyage-3-lite".into()).unwrap();
        let r = e.embed_passages(&[]).await.expect("empty ok");
        assert!(r.is_empty());
    }
}
