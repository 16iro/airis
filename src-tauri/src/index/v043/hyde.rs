// v0.4.3 PR 3 (D-087) — HyDE (Hypothetical Document Embeddings).
//
// 사용자 질문 → fast LLM(Haiku 4.5 등)에게 *가상의 답변 1단락*을 작성시킨다.
// 그 가상 답변을 임베딩해 검색에 사용하면, 질문 표면형(짧고 의문형) vs 코퍼스 표면형
// (서술·설명형) 간 임베딩 공간 mismatch를 완화할 수 있다 (architecture §4.7.1, HyDE
// 논문 Gao et al. 2022 https://arxiv.org/abs/2212.10496).
//
// 활성화 정책 (D-087):
//   * 검색 강도 "정확"(`SearchStrength::Accurate`) 모드에서만 ON. 빠름·균형은 skip.
//   * 가상 답변 *1건*만 생성 — 지연·비용 trade-off (claude-code 구독은 무료지만
//     LLM 라운드트립 ~수백 ms 추가). 향후 토글 가능 자리.
//   * 캐시 bypass — 가상 답변은 LLM이 매번 살짝 다르게 생성하므로 embedding cache hit
//     가능성이 낮음 (architecture §4.11.3 관찰). 본 모듈은 *embedding cache get/put을
//     직접 호출하지 않는다*.
//
// 폴백 (graceful):
//   * provider.fast_model() 미지정(예: mock) → 원본 query 그대로 반환.
//   * provider 호출 에러 → 원본 query 그대로. warn log만 남김.
//   * LLM 출력 후처리 후 빈 문자열 → 원본 query 그대로.
// chat 흐름을 막지 않는 게 1순위.
//
// rewriter와의 차이:
//   rewriter는 *대명사·생략 풀어쓴 한 줄 검색어*가 목표 → 첫 줄만 잘라냄.
//   HyDE은 *서술적인 가상 답변 한 단락*이 목표 → 단락 전체를 그대로 사용.

#![allow(dead_code)]

use futures_util::StreamExt;
use tracing::{debug, warn};

use crate::error::AppResult;
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role};

/// HyDE 가상 답변 토큰 한도 — 1단락 ~150~250 토큰 정도가 자연스러움. 상한은 임베더의
/// max_tokens(BGE-M3 8192, mE5-small 512) 안쪽으로 충분히 작게.
const HYDE_MAX_TOKENS: u32 = 512;

/// HyDE 시스템 프롬프트 — 한국어 학습 시나리오, few-shot 1~2건 포함.
///
/// 핵심:
///   1) 출력은 *한 단락*만. 머리말·라벨·따옴표·번호 매김 X.
///   2) *사실 보장 X* — 검색 임베딩용이라는 의도 명시. 모델이 hallucinate해도 OK.
///   3) 어조는 교과서·기술 문서 톤 (코퍼스와 임베딩 공간이 가까운 표면형).
///   4) 영어 용어는 그대로 둠.
const HYDE_SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미의 검색 전처리기입니다. \
사용자의 질문을 받아, 그 질문에 대한 *가상의 답변 한 단락*을 마치 교과서·기술 문서가 설명하듯 \
서술체로 작성합니다.\n\n\
규칙:\n\
1) 출력은 *오직 한 단락*. 머리말·라벨(예: \"답변:\")·따옴표·번호를 붙이지 마세요.\n\
2) 사실 정확성은 *보장하지 않아도 됩니다* — 본 답변은 임베딩 검색 입력으로만 사용됩니다.\n\
3) 어조는 교과서·기술 문서·매뉴얼처럼 서술적·구체적으로 적습니다(질문 형식 X).\n\
4) 영어 기술 용어는 그대로 두세요 (검색 정확도 ↑).\n\
5) 분량은 3~5문장 사이.\n\n\
예시 1\n\
질문: GameBoy PPU(Picture Processing Unit) 구현 방법\n\
답변: GameBoy의 PPU는 2비트 색 깊이의 160x144 LCD를 매 프레임 갱신하는 그래픽 하드웨어입니다. \
구현은 보통 모드 0(HBlank)·모드 1(VBlank)·모드 2(OAM 스캔)·모드 3(픽셀 전송) 4단계 상태 \
머신으로 분리합니다. 각 모드는 정확한 사이클 수 동안 진행되며, LY와 LYC 비교로 STAT 인터럽트를 \
발생시킵니다. 픽셀 전송 단계에서는 background tilemap, window, sprite를 우선순위 규칙에 따라 \
합성합니다.\n\n\
예시 2\n\
질문: Rust Result vs Option 사용 기준\n\
답변: Result는 작업이 실패할 수 있는지를 표현하고 그 실패 이유를 E 타입으로 함께 전달하는 반면, \
Option은 단순히 값이 있을지 없을지만 표현합니다. 파일 입출력·네트워크처럼 실패에 *원인*이 있는 \
연산은 Result를 사용합니다. 반면 컬렉션의 first()처럼 부재가 정상 흐름인 경우 Option을 사용합니다. \
? 연산자는 Result에 자연스럽게 결합되어 에러 전파를 간결하게 만들고, Option도 None을 조기 반환하는 \
용도로 ?를 쓸 수 있습니다.";

/// HyDE 생성기 — 짧은 LLM 호출로 가상 답변 1건 생성. provider는 호출 시점에 인자로 받는다
/// (rewriter와 동일 패턴 — D-066 hot-swap 호환).
#[derive(Debug, Default, Clone, Copy)]
pub struct HydeGenerator;

impl HydeGenerator {
    pub fn new() -> Self {
        Self
    }

    /// HyDE 본 호출. 실패 시 *원본 query 그대로* 반환 (graceful — chat 흐름 보호).
    ///
    /// 호출 측은 본 함수의 결과를 *그대로 임베더에 넣어* 검색용 query embedding을 만든다.
    /// 본 메서드는 임베더 호출도, embedding cache 접근도 하지 *않는다* — 책임 분리.
    pub async fn generate(
        &self,
        query: &str,
        provider: &dyn LlmProvider,
    ) -> AppResult<String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(query.to_string());
        }
        let fast_model = provider.fast_model();
        if fast_model.is_empty() {
            // mock provider 등 fast_model 미구현 — graceful skip.
            debug!(
                target: "v043.hyde",
                "fast_model 미지정 — HyDE skip"
            );
            return Ok(query.to_string());
        }

        let user_prompt = format!("질문: {trimmed}\n답변:");
        let request = ChatRequest {
            model: fast_model.to_string(),
            system: Some(HYDE_SYSTEM_PROMPT.to_string()),
            messages: vec![Message {
                role: Role::User,
                content: user_prompt,
            }],
            max_tokens: HYDE_MAX_TOKENS,
            // HyDE 결과는 *매번 다름* — cache_breakpoints 비워둠.
            cache_breakpoints: Vec::new(),
        };

        match collect_text(provider, request).await {
            Ok(raw) => {
                let cleaned = postprocess(&raw);
                if cleaned.is_empty() {
                    debug!(
                        target: "v043.hyde",
                        "HyDE 출력 비어 있음 — 원본 query 사용"
                    );
                    Ok(query.to_string())
                } else {
                    debug!(
                        target: "v043.hyde",
                        original_len = trimmed.len(),
                        hypothetical_len = cleaned.len(),
                        "HyDE 가상 답변 생성"
                    );
                    Ok(cleaned)
                }
            }
            Err(e) => {
                warn!(
                    target: "v043.hyde",
                    error = %e,
                    "HyDE 호출 실패 — 원본 query로 폴백"
                );
                Ok(query.to_string())
            }
        }
    }
}

/// provider 호출을 *non-streaming처럼* 사용 — 모든 TextDelta를 누적, Done에서 break.
async fn collect_text(provider: &dyn LlmProvider, request: ChatRequest) -> AppResult<String> {
    let mut stream = provider.chat_stream(request).await?;
    let mut buf = String::new();
    while let Some(event) = stream.next().await {
        match event? {
            ChatEvent::TextDelta { text } => buf.push_str(&text),
            ChatEvent::Done { .. } => break,
        }
    }
    Ok(buf)
}

/// LLM 출력 후처리 — *한 단락 통째*로 사용 (rewriter와 달리 첫 줄만 잘라내지 않음).
///
///   * 양 끝 공백 제거.
///   * 잔여 라벨(`답변:`, `Answer:` 등) 제거.
///   * 양 끝 따옴표 한 겹 제거.
///   * 본문 내부의 `\n`은 유지 — 임베더는 줄바꿈을 자연 토큰화 하므로 의미 손실 X.
fn postprocess(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let stripped = strip_label(trimmed);
    let unquoted = strip_outer_quotes(stripped);
    unquoted.trim().to_string()
}

/// 한국어/영어 흔한 prefix 라벨 제거 — `답변:`, `Answer:`, `A:` 등.
fn strip_label(s: &str) -> &str {
    const LABELS: &[&str] = &[
        "답변:",
        "답변 :",
        "가상 답변:",
        "가상 답변 :",
        "Answer:",
        "answer:",
        "A:",
        "a:",
    ];
    for label in LABELS {
        if let Some(rest) = s.strip_prefix(label) {
            return rest.trim_start();
        }
    }
    s
}

/// 양 끝 따옴표(`"..."`, `'...'`, `「...」`) 한 겹 제거.
fn strip_outer_quotes(s: &str) -> &str {
    if s.len() < 2 {
        return s;
    }
    let bytes = s.as_bytes();
    let first = bytes[0];
    let last = bytes[s.len() - 1];
    if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
        return &s[1..s.len() - 1];
    }
    if let (Some(start), Some(end)) = (s.chars().next(), s.chars().last()) {
        if start == '「' && end == '」' {
            let s_trim = s.trim_start_matches('「').trim_end_matches('」');
            return s_trim;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockProvider;
    use crate::llm::{ChatEvent, ChatRequest, ChatStream, LlmProvider, Usage};
    use async_trait::async_trait;
    use std::sync::Mutex;

    // -----------------------------------------------------------------------
    // postprocess 유틸 단위
    // -----------------------------------------------------------------------

    #[test]
    fn postprocess_keeps_full_paragraph() {
        // rewriter와 달리 첫 줄만 잘라내지 *않음*. 단락 전체 유지.
        let raw = "GameBoy의 PPU는 LCD 갱신을 담당합니다. 4단계 모드로 동작합니다.\n픽셀 전송 단계에서 합성합니다.";
        let out = postprocess(raw);
        assert!(out.contains("GameBoy의 PPU는 LCD 갱신을 담당합니다"));
        assert!(out.contains("4단계 모드로 동작합니다"));
        assert!(out.contains("픽셀 전송 단계에서 합성합니다"));
    }

    #[test]
    fn postprocess_strips_label_prefix() {
        assert_eq!(
            postprocess("답변: PPU는 LCD를 갱신합니다."),
            "PPU는 LCD를 갱신합니다."
        );
        assert_eq!(postprocess("Answer: foo bar"), "foo bar");
    }

    #[test]
    fn postprocess_strips_outer_quotes() {
        assert_eq!(postprocess("\"hello world\""), "hello world");
        assert_eq!(postprocess("'안녕하세요'"), "안녕하세요");
    }

    #[test]
    fn postprocess_returns_empty_for_blank_input() {
        assert_eq!(postprocess(""), "");
        assert_eq!(postprocess("   \n\n"), "");
    }

    #[test]
    fn postprocess_preserves_internal_newlines() {
        // HyDE 답변에 줄바꿈이 있어도 그대로 — 임베더는 자연 토큰화.
        let raw = "첫째 줄.\n둘째 줄.";
        let out = postprocess(raw);
        assert!(out.contains("첫째 줄."));
        assert!(out.contains("둘째 줄."));
        assert!(out.contains('\n'));
    }

    // -----------------------------------------------------------------------
    // generate — MockProvider 래핑
    // -----------------------------------------------------------------------

    /// MockProvider는 fast_model() 디폴트(`""`) 라 HyDE가 무조건 skip 한다 (graceful).
    #[tokio::test]
    async fn generate_skips_when_provider_has_no_fast_model() {
        let provider = MockProvider::from_text_chunks(&["응답 무시"]);
        let hyde = HydeGenerator::new();
        let out = hyde.generate("GameBoy PPU 구현", &provider).await.unwrap();
        assert_eq!(
            out, "GameBoy PPU 구현",
            "fast_model이 비어있는 provider는 HyDE skip"
        );
    }

    #[tokio::test]
    async fn generate_returns_full_paragraph_with_fast_model() {
        let provider = FastMockProvider::with_text(
            "GameBoy의 PPU는 LCD를 매 프레임 갱신합니다. 4단계 모드 머신으로 동작합니다.",
        );
        let hyde = HydeGenerator::new();
        let out = hyde.generate("GameBoy PPU 구현", &provider).await.unwrap();
        assert!(out.contains("GameBoy의 PPU는 LCD를 매 프레임 갱신합니다"));
        assert!(out.contains("4단계 모드 머신으로 동작합니다"));
    }

    #[tokio::test]
    async fn generate_falls_back_to_original_on_provider_error() {
        let provider = FailingFastProvider;
        let hyde = HydeGenerator::new();
        let out = hyde.generate("원본 질문", &provider).await.unwrap();
        assert_eq!(out, "원본 질문");
    }

    #[tokio::test]
    async fn generate_falls_back_to_original_when_llm_outputs_blank() {
        let provider = FastMockProvider::with_text("   \n   ");
        let hyde = HydeGenerator::new();
        let out = hyde.generate("원본 질문", &provider).await.unwrap();
        assert_eq!(out, "원본 질문");
    }

    #[tokio::test]
    async fn generate_skips_when_query_is_blank() {
        // 빈 query는 LLM 호출 자체 skip — provider.fast_model 검사 전에 즉시 반환.
        let provider = FastMockProvider::with_text("이 응답은 호출되면 안 됨");
        let hyde = HydeGenerator::new();
        let out = hyde.generate("   ", &provider).await.unwrap();
        assert_eq!(out, "   ", "빈 query는 호출 skip + 원본 그대로");
        assert!(
            provider.captured().is_empty(),
            "LLM 호출 자체가 발생하지 않아야 함"
        );
    }

    #[tokio::test]
    async fn generate_uses_query_in_user_prompt() {
        let provider = FastMockProvider::with_text("가상 답변 단락");
        let hyde = HydeGenerator::new();
        let _ = hyde.generate("Rust 메모리 안전성", &provider).await.unwrap();
        let captured = provider.captured();
        assert!(captured.contains("질문: Rust 메모리 안전성"));
        assert!(captured.ends_with("답변:"));
    }

    #[tokio::test]
    async fn generate_strips_label_in_output() {
        // LLM이 라벨을 붙여도 후처리에서 제거되어야 함.
        let provider = FastMockProvider::with_text("답변: PPU는 LCD를 갱신합니다.");
        let hyde = HydeGenerator::new();
        let out = hyde.generate("PPU 동작", &provider).await.unwrap();
        assert_eq!(out, "PPU는 LCD를 갱신합니다.");
    }

    // -----------------------------------------------------------------------
    // 헬퍼 — fast_model을 채우고 prompt를 capture할 수 있는 mock.
    // -----------------------------------------------------------------------

    struct FastMockProvider {
        text: String,
        captured_prompt: Mutex<String>,
    }

    impl FastMockProvider {
        fn with_text(text: &str) -> Self {
            Self {
                text: text.to_string(),
                captured_prompt: Mutex::new(String::new()),
            }
        }
        fn captured(&self) -> String {
            self.captured_prompt.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for FastMockProvider {
        fn fast_model(&self) -> &str {
            "test-haiku"
        }

        async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream> {
            // 마지막 user 메시지를 capture.
            if let Some(last) = request.messages.iter().rev().find(|m| m.role == Role::User) {
                *self.captured_prompt.lock().unwrap() = last.content.clone();
            }
            let text = self.text.clone();
            let stream = async_stream::try_stream! {
                yield ChatEvent::TextDelta { text };
                yield ChatEvent::Done { usage: Usage::default() };
            };
            Ok(Box::pin(stream))
        }
    }

    /// 항상 에러 — 폴백 경로 검증.
    struct FailingFastProvider;

    #[async_trait]
    impl LlmProvider for FailingFastProvider {
        fn fast_model(&self) -> &str {
            "test-haiku"
        }

        async fn chat_stream(&self, _request: ChatRequest) -> AppResult<ChatStream> {
            Err(crate::error::AppError::LlmApi {
                message: "hyde mock failure".into(),
            })
        }
    }
}
