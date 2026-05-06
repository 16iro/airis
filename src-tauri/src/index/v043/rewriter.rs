// v0.4.3 PR 1 (D-086) — Query rewriting layer.
//
// 사용자 질문 + 대화 히스토리(최근 4턴)를 받아 *대명사·생략을 풀어 쓴 1줄 검색 쿼리*로
// 재작성한다. retrieval(hybrid_search)·HyDE 모두에게 더 풍부한 입력을 줘서 검색 품질을
// 끌어올리는 게 목표 (architecture §4.7.1).
//
// 동작:
//   1. history는 *최근 4턴*만 사용. 4턴 이하면 그대로. 0턴이면 query 자체가 잘 정렬된
//      검색어인 경우가 많아 그대로 반환할 수도 있지만, 사용자가 "이거", "그게" 같은
//      대명사를 넣은 첫 질문도 있으므로 *항상 호출*하고 LLM이 변경 없으면 동일 텍스트를
//      돌려주는 정책 (graceful — 호출 비용은 Haiku 1회로 작음).
//   2. provider.fast_model()을 model로 박은 ChatRequest를 만든다.
//      `cache_breakpoints`는 빈 Vec — 매 호출 query 컨텍스트가 달라 cache 효과 X.
//   3. provider.chat_stream을 호출하고 *모든 TextDelta를 누적*한다 (non-streaming 효과 —
//      짧은 응답이라 ms 단위로 끝남). Done 이벤트를 받으면 누적 텍스트를 후처리해 첫 줄만 사용.
//   4. 후처리:
//      * 양 끝 공백 제거.
//      * 첫 줄(`\n` 이전) 만 사용 — LLM이 설명을 덧붙여도 한 줄로 잘라냄.
//      * 마크다운 인용·따옴표·접두 라벨(`재작성:`, `검색어:`, `Query:` 등) 제거.
//      * 빈 문자열로 환원되면 *원본 query 그대로 반환*.
//   5. 폴백: provider 호출이 어떤 식으로든 에러면 *원본 query 그대로*. 에러는 warn log
//      만 남김 — chat 흐름을 막지 않는다.
//
// 비용·지연: Haiku 4.5 호출 1회 (architecture §4.12). 실측 ~150ms·~50 토큰.
// claude-code 어댑터에서는 사용자 구독 quota만 사용 (PR 24, D-066).

#![allow(dead_code)]

use futures_util::StreamExt;
use tracing::{debug, warn};

use crate::error::AppResult;
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role};

/// rewriter가 받을 단일 history 턴. `commands::llm::ChatHistoryMessage`보다 작은 표면.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryTurn {
    pub role: Role,
    pub content: String,
}

/// 최근 N턴 윈도우 — D-086 default. 4턴이면 user·assistant 합쳐 8개 메시지까지.
pub const HISTORY_WINDOW_TURNS: usize = 4;

/// rewriting 출력 토큰 한도 — 1줄짜리 짧은 검색어라 작게 잡아도 충분.
const REWRITE_MAX_TOKENS: u32 = 256;

/// rewriting 시스템 프롬프트 — 한국어 학습 시나리오에 맞춘 few-shot 2건 포함.
/// 책 무관(특정 책 이름 X) 일반 학습 도우미 톤.
const REWRITE_SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미의 검색 쿼리 재작성기입니다. \
사용자의 마지막 질문을 그 앞 대화 맥락을 참고해 *대명사·생략을 풀어 쓴 한 줄 검색 쿼리*로 다시 씁니다.\n\n\
규칙:\n\
1) 출력은 *오직 한 줄*. 설명·머리말·따옴표·라벨(예: \"검색어:\")을 붙이지 마세요.\n\
2) 책 이름이 분명하면 유지, 모르면 임의 추가하지 마세요.\n\
3) 사용자 의도가 이미 명확하면 그대로 두되, 대명사(\"이것\", \"그게\", \"저거\")만 풀어 쓰세요.\n\
4) 영어 용어는 그대로 두세요 (검색 정확도 ↑).\n\n\
예시 1\n\
이전 대화:\n\
사용자: GameBoy 에뮬레이터의 PPU가 뭐예요?\n\
도우미: PPU는 Picture Processing Unit으로 화면 출력을 담당합니다.\n\
질문: 이거 어떻게 구현하지?\n\
재작성: GameBoy PPU(Picture Processing Unit) 구현 방법\n\n\
예시 2\n\
이전 대화:\n\
사용자: Rust에서 Result와 Option 차이가 뭐야?\n\
도우미: Result는 성공·실패, Option은 값의 유무를 표현합니다.\n\
질문: 그럼 언제 어떤 걸 써야 해?\n\
재작성: Rust Result vs Option 사용 기준";

/// QueryRewriter — 짧은 LLM 호출로 query를 정규화. provider는 *호출 시점에 인자로* 받는다
/// (state.llm 교체에 영향받지 않게 — D-066 hot-swap 호환).
#[derive(Debug, Default, Clone, Copy)]
pub struct QueryRewriter;

impl QueryRewriter {
    pub fn new() -> Self {
        Self
    }

    /// rewriting 본 호출. 실패 시 *원본 query를 그대로* 반환 — graceful (chat 흐름 보호).
    ///
    /// `history`는 *시간순*(가장 오래된 것 → 최근). 본 함수가 마지막 4턴만 잘라 사용.
    /// `query`는 사용자 *이번 차례 질문*. history에 포함되지 *않은* 신규 입력.
    /// `provider`는 활성 LLM 어댑터 — `fast_model()`이 비어있는 mock 등은 *원본 그대로* 반환.
    pub async fn rewrite(
        &self,
        history: &[HistoryTurn],
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
                target: "v043.rewriter",
                "fast_model 미지정 — query rewriting skip"
            );
            return Ok(query.to_string());
        }

        let user_prompt = build_user_prompt(history, trimmed);
        let request = ChatRequest {
            model: fast_model.to_string(),
            system: Some(REWRITE_SYSTEM_PROMPT.to_string()),
            messages: vec![Message {
                role: Role::User,
                content: user_prompt,
            }],
            max_tokens: REWRITE_MAX_TOKENS,
            cache_breakpoints: Vec::new(),
        };

        match collect_text(provider, request).await {
            Ok(raw) => {
                let cleaned = postprocess(&raw);
                if cleaned.is_empty() {
                    debug!(
                        target: "v043.rewriter",
                        "rewriter 출력 비어 있음 — 원본 query 사용"
                    );
                    Ok(query.to_string())
                } else {
                    debug!(
                        target: "v043.rewriter",
                        original_len = trimmed.len(),
                        rewritten_len = cleaned.len(),
                        "query rewriting 적용"
                    );
                    Ok(cleaned)
                }
            }
            Err(e) => {
                warn!(
                    target: "v043.rewriter",
                    error = %e,
                    "query rewriting 실패 — 원본 query로 폴백"
                );
                Ok(query.to_string())
            }
        }
    }
}

/// 시간순 history → user prompt(이전 대화 + 현재 질문) 직렬화.
///
/// 마지막 N턴(=user/assistant 쌍 N개 ≈ 메시지 2N개) 만 사용. 그 앞은 무시 — query rewriting
/// 만 위해 길게 끌고 갈 가치 X. 비어 있으면 "이전 대화 없음" 표기.
fn build_user_prompt(history: &[HistoryTurn], query: &str) -> String {
    let window = recent_window(history, HISTORY_WINDOW_TURNS);
    let mut buf = String::new();
    buf.push_str("이전 대화:\n");
    if window.is_empty() {
        buf.push_str("(없음)\n");
    } else {
        for turn in window {
            let label = match turn.role {
                Role::User => "사용자",
                Role::Assistant => "도우미",
            };
            buf.push_str(label);
            buf.push_str(": ");
            buf.push_str(turn.content.trim());
            buf.push('\n');
        }
    }
    buf.push_str("질문: ");
    buf.push_str(query);
    buf.push_str("\n재작성:");
    buf
}

/// 시간순 history에서 마지막 `turns`개 user/assistant *턴*만 추린다.
///
/// 1턴 = (user, assistant) 1쌍. 마지막 메시지가 user(=새 질문 직전 미응답)인 경우는
/// 그 user도 포함 — partial turn으로 취급. 단순 슬라이딩 윈도우 (메시지 2N개 cap)로
/// 충분 (rewriting 입력 잡음 컨트롤이 아니라 "최근 컨텍스트 전달"이 목적).
fn recent_window(history: &[HistoryTurn], turns: usize) -> &[HistoryTurn] {
    let cap = turns.saturating_mul(2);
    if history.len() <= cap {
        history
    } else {
        &history[history.len() - cap..]
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

/// LLM 출력 후처리:
///   * 양 끝 공백 제거.
///   * 첫 줄만 사용 (LLM이 부연 설명 붙이면 잘라냄).
///   * 잔여 따옴표·라벨 제거.
fn postprocess(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // 첫 줄만 — `\n` 또는 `\r\n` 어느 쪽이든.
    let first_line = trimmed.split(['\n', '\r']).next().unwrap_or("").trim();
    let stripped = strip_label(first_line);
    let unquoted = strip_outer_quotes(stripped);
    unquoted.trim().to_string()
}

/// 한국어/영어 흔한 prefix 라벨을 제거 — `재작성:`, `검색어:`, `Query:`, `Rewritten:` 등.
fn strip_label(s: &str) -> &str {
    const LABELS: &[&str] = &[
        "재작성:",
        "재작성 :",
        "검색어:",
        "검색어 :",
        "쿼리:",
        "쿼리 :",
        "Query:",
        "query:",
        "Rewritten:",
        "rewritten:",
        "Search:",
        "search:",
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
    // 한국어 인용 부호 「」 — UTF-8 3바이트.
    if let (Some(start), Some(end)) = (s.chars().next(), s.chars().last()) {
        if start == '「' && end == '」' {
            let s_trim = s.trim_start_matches('「').trim_end_matches('」');
            return s_trim;
        }
    }
    s
}

// =============================================================================
// 검색 강도 토글 — D-086 정책 (settings.rs::SearchStrength 와 1:1 매핑).
// rewriter 모듈에 두어 *retrieval 전 단계 정책*이 한 곳에 모이게 한다 (PR 3 HyDE도 같은 enum 참조).
// =============================================================================

/// 검색 강도. `Settings::search_strength`와 동일 의미.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewritePolicy {
    /// 빠름 — rewriting 생략. 즉시 검색.
    Skip,
    /// 균형 (default) — query rewriting ON, HyDE OFF.
    Rewrite,
    /// 정확 — query rewriting ON + HyDE ON (HyDE 활성화는 PR 3).
    RewriteAndHyde,
}

impl RewritePolicy {
    /// rewriting을 실제로 호출할지.
    pub fn should_rewrite(self) -> bool {
        matches!(self, Self::Rewrite | Self::RewriteAndHyde)
    }

    /// HyDE를 호출할지 (PR 3 진입점).
    pub fn should_hyde(self) -> bool {
        matches!(self, Self::RewriteAndHyde)
    }

    /// PR 2 (D-088) — sentence window/auto-merging/MMR 후처리를 수행할지.
    /// 빠름은 속도 우선이라 skip, Balanced/Accurate은 ON.
    pub fn should_postprocess(self) -> bool {
        matches!(self, Self::Rewrite | Self::RewriteAndHyde)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockProvider;
    use crate::llm::{ChatEvent, ChatRequest, ChatStream, LlmProvider, Usage};
    use async_trait::async_trait;
    use std::sync::Mutex;

    // -----------------------------------------------------------------------
    // 유틸 단위
    // -----------------------------------------------------------------------

    #[test]
    fn recent_window_returns_full_history_when_under_cap() {
        let h = vec![turn(Role::User, "a"), turn(Role::Assistant, "b")];
        let w = recent_window(&h, 4);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn recent_window_caps_at_2n_messages() {
        // 6턴 = 12메시지. window=4턴 = 8메시지로 잘림.
        let mut h: Vec<HistoryTurn> = Vec::new();
        for i in 0..6 {
            h.push(turn(Role::User, &format!("u{i}")));
            h.push(turn(Role::Assistant, &format!("a{i}")));
        }
        let w = recent_window(&h, 4);
        assert_eq!(w.len(), 8);
        // 가장 최근 8개 = u2..u5 + a2..a5.
        assert_eq!(w.first().unwrap().content, "u2");
        assert_eq!(w.last().unwrap().content, "a5");
    }

    #[test]
    fn build_user_prompt_includes_label_and_query() {
        let h = vec![
            turn(Role::User, "PPU가 뭐?"),
            turn(Role::Assistant, "Picture Processing Unit입니다."),
        ];
        let p = build_user_prompt(&h, "이거 어떻게 구현해?");
        assert!(p.contains("사용자: PPU가 뭐?"));
        assert!(p.contains("도우미: Picture Processing Unit입니다."));
        assert!(p.contains("질문: 이거 어떻게 구현해?"));
        assert!(p.ends_with("재작성:"));
    }

    #[test]
    fn build_user_prompt_with_empty_history_uses_placeholder() {
        let p = build_user_prompt(&[], "처음 질문");
        assert!(p.contains("이전 대화:\n(없음)\n"));
        assert!(p.contains("질문: 처음 질문"));
    }

    #[test]
    fn postprocess_takes_first_line_only() {
        let raw = "GameBoy PPU 구현 방법\n\n부연 설명: PPU는...";
        assert_eq!(postprocess(raw), "GameBoy PPU 구현 방법");
    }

    #[test]
    fn postprocess_strips_label_prefix() {
        assert_eq!(postprocess("재작성: Rust Result 사용법"), "Rust Result 사용법");
        assert_eq!(postprocess("Query: foo bar"), "foo bar");
        assert_eq!(postprocess("검색어 : 한국어 학습"), "한국어 학습");
    }

    #[test]
    fn postprocess_strips_outer_quotes() {
        assert_eq!(postprocess("\"hello world\""), "hello world");
        assert_eq!(postprocess("'안녕'"), "안녕");
        assert_eq!(postprocess("「PPU 구현」"), "PPU 구현");
    }

    #[test]
    fn postprocess_returns_empty_for_blank_input() {
        assert_eq!(postprocess(""), "");
        assert_eq!(postprocess("   \n\n"), "");
    }

    #[test]
    fn rewrite_policy_routing() {
        assert!(!RewritePolicy::Skip.should_rewrite());
        assert!(!RewritePolicy::Skip.should_hyde());
        assert!(!RewritePolicy::Skip.should_postprocess());
        assert!(RewritePolicy::Rewrite.should_rewrite());
        assert!(!RewritePolicy::Rewrite.should_hyde());
        assert!(RewritePolicy::Rewrite.should_postprocess());
        assert!(RewritePolicy::RewriteAndHyde.should_rewrite());
        assert!(RewritePolicy::RewriteAndHyde.should_hyde());
        assert!(RewritePolicy::RewriteAndHyde.should_postprocess());
    }

    // -----------------------------------------------------------------------
    // 통합 단위 — MockProvider 래핑
    // -----------------------------------------------------------------------

    /// MockProvider는 fast_model() 디폴트(`""`) 라 rewriter가 무조건 skip 한다.
    /// 본 테스트는 *fast_model 미지정 폴백* 경로 검증.
    #[tokio::test]
    async fn rewrite_skips_when_provider_has_no_fast_model() {
        let provider = MockProvider::from_text_chunks(&["GameBoy PPU 구현"]);
        let rewriter = QueryRewriter::new();
        let history = vec![turn(Role::User, "이거 뭐?")];
        let out = rewriter
            .rewrite(&history, "이거 어떻게 해?", &provider)
            .await
            .unwrap();
        assert_eq!(
            out, "이거 어떻게 해?",
            "fast_model이 비어있는 provider는 query rewriting skip"
        );
    }

    #[tokio::test]
    async fn rewrite_returns_first_line_with_fast_model() {
        let provider = FastMockProvider::with_text("GameBoy PPU 구현 방법\n부연: 어쩌고");
        let rewriter = QueryRewriter::new();
        let history = vec![
            turn(Role::User, "GameBoy 에뮬레이터의 PPU가 뭐예요?"),
            turn(Role::Assistant, "Picture Processing Unit입니다."),
        ];
        let out = rewriter
            .rewrite(&history, "이거 어떻게 구현하지?", &provider)
            .await
            .unwrap();
        assert_eq!(out, "GameBoy PPU 구현 방법");
    }

    #[tokio::test]
    async fn rewrite_falls_back_to_original_on_provider_error() {
        let provider = FailingFastProvider;
        let rewriter = QueryRewriter::new();
        let out = rewriter
            .rewrite(&[], "이거 어떻게 해?", &provider)
            .await
            .unwrap();
        assert_eq!(out, "이거 어떻게 해?");
    }

    #[tokio::test]
    async fn rewrite_falls_back_to_original_when_llm_outputs_blank() {
        let provider = FastMockProvider::with_text("   \n   ");
        let rewriter = QueryRewriter::new();
        let out = rewriter.rewrite(&[], "원본 질문", &provider).await.unwrap();
        assert_eq!(out, "원본 질문");
    }

    #[tokio::test]
    async fn rewrite_uses_only_recent_4_turns() {
        // history 6턴(12 msg). FastMockProvider가 본 prompt를 capture해 검증.
        let provider = FastMockProvider::with_capture("간단한 검색어");
        let rewriter = QueryRewriter::new();
        let mut history = Vec::new();
        for i in 0..6 {
            history.push(turn(Role::User, &format!("UQ{i}")));
            history.push(turn(Role::Assistant, &format!("AR{i}")));
        }
        let _ = rewriter
            .rewrite(&history, "마지막 질문", &provider)
            .await
            .unwrap();
        let captured = provider.captured();
        // window=4턴이라 UQ0,AR0,UQ1,AR1 은 *없어야* 함, UQ2~UQ5/AR2~AR5는 있어야 함.
        assert!(!captured.contains("UQ0"));
        assert!(!captured.contains("UQ1"));
        assert!(captured.contains("UQ2"));
        assert!(captured.contains("AR5"));
        assert!(captured.contains("질문: 마지막 질문"));
    }

    // -----------------------------------------------------------------------
    // 헬퍼 — fast_model을 채우고 입력 prompt를 capture할 수 있는 mock.
    // -----------------------------------------------------------------------

    fn turn(role: Role, content: &str) -> HistoryTurn {
        HistoryTurn {
            role,
            content: content.to_string(),
        }
    }

    /// fast_model이 채워진 mock — query rewriting 흐름이 *실제로* 호출되는지 검증용.
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
        fn with_capture(text: &str) -> Self {
            Self::with_text(text)
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
            // 마지막 user 메시지를 capture (build_user_prompt 출력).
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

    /// 항상 에러를 반환하는 mock — 폴백 경로 검증.
    struct FailingFastProvider;

    #[async_trait]
    impl LlmProvider for FailingFastProvider {
        fn fast_model(&self) -> &str {
            "test-haiku"
        }

        async fn chat_stream(&self, _request: ChatRequest) -> AppResult<ChatStream> {
            Err(crate::error::AppError::LlmApi {
                message: "rewriter mock failure".into(),
            })
        }
    }
}
