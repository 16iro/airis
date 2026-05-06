// v0.4.3 PR 4 (D-089) — 대화 히스토리 압축.
//
// architecture §4.10.2:
//   슬라이딩 윈도우 + 요약 누적 하이브리드.
//
// D-089 정책 (확정):
//   * 최근 6턴(=user/assistant 쌍 6개 ≈ 메시지 12개) raw 유지.
//   * 6턴 초과 시 가장 오래된 4턴(메시지 8개)을 *Haiku 한 줄 요약*으로 누적.
//   * 매 chat 호출마다 점진 압축 — 한 번에 다 하지 않음 (요약 횟수 ↓, 비용 ↓).
//   * 요약 모델 = `provider.fast_model()`. 미지정/에러 시 *가장 오래된 turn drop만* (graceful).
//   * raw 영속(chat_messages)에는 손대지 않는다 — 요약은 LLM 입력에만 사용.
//     (HANDOFF §9: "원본 turn은 별도 영속, 요약은 LLM 입력에만 사용")
//
// 입력: HistoryTurn[] 시간순(가장 오래된 → 최근).
// 출력: CompressedHistory { summary: Option<String>, recent_turns: Vec<HistoryTurn> }.
//   * summary = Some(_) 면 system prompt에 "이전 대화 요약" 으로 주입.
//   * recent_turns = LLM messages에 그대로 들어갈 user/assistant turn.
//
// 폴백:
//   * fast_model 미지정 → summary=None, recent_turns=마지막 RECENT_TURNS_KEEP*2 메시지.
//   * 요약 LLM 호출 에러 → 동일.
//
// 비용·지연: 요약은 turn 5+이상부터만 호출 (대부분의 chat은 1~3턴이라 호출 횟수 작음).
// claude-code 어댑터에선 사용자 구독 quota만 사용.

#![allow(dead_code)]

use futures_util::StreamExt;
use tracing::{debug, warn};

use crate::error::AppResult;
use crate::index::v043::rewriter::HistoryTurn;
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role};

/// 최근 raw 유지 턴 수 — D-089 default.
pub const RECENT_TURNS_KEEP: usize = 6;

/// 한 번 압축 시 가장 오래된 N턴을 요약으로 옮긴다 — 점진 압축의 step size.
pub const COMPRESS_TURNS_PER_STEP: usize = 4;

/// 요약 출력 토큰 한도 — 짧은 한 줄 요약이라 작게.
const SUMMARIZE_MAX_TOKENS: u32 = 384;

/// 시스템 프롬프트 — 한국어 학습 시나리오, 한 줄 요약.
const SUMMARIZE_SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미의 대화 요약기입니다. \
주어진 *과거 대화 일부*를 한 줄로 요약합니다.\n\n\
규칙:\n\
1) 출력은 *오직 한 줄*. 머리말·라벨(예: \"요약:\")·따옴표·번호를 붙이지 마세요.\n\
2) 사용자가 무엇을 물었고 도우미가 어떤 결론을 줬는지를 *간결한 한 문장*으로 요약하세요.\n\
3) 영어 기술 용어는 그대로 두세요.\n\
4) 분량은 30~80자 사이가 자연스럽습니다.\n\n\
예시\n\
대화:\n\
사용자: GameBoy 에뮬레이터의 PPU가 뭐예요?\n\
도우미: PPU는 Picture Processing Unit으로 화면 출력을 담당합니다.\n\
사용자: 어떻게 구현해?\n\
도우미: HBlank/VBlank/OAM/픽셀 전송 4모드 상태 머신으로 구현합니다.\n\
요약: GameBoy PPU 정의 + 4모드 상태 머신 구현 방식 논의.";

/// 압축 결과 — recent_turns는 LLM messages, summary는 system prompt 주입용.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompressedHistory {
    /// 누적 요약 (이전 누적이 있으면 새 요약을 *append*). None이면 압축 없음.
    pub summary: Option<String>,
    /// 최근 raw 턴 리스트 — 시간순(가장 오래된 → 최근).
    pub recent_turns: Vec<HistoryTurn>,
}

/// HistoryCompressor — 짧은 LLM 호출로 가장 오래된 N턴을 한 줄 요약으로 옮긴다.
/// rewriter/HyDE와 동일 패턴 — provider는 호출 시점 인자.
#[derive(Debug, Default, Clone)]
pub struct HistoryCompressor {
    /// 누적된 요약 — 호출 측이 보존(예: chat 세션 store)하면 매 호출마다 append.
    /// 본 구조체 자체는 in-memory only (영속 X — D-089 정책).
    accumulated_summary: Option<String>,
}

impl HistoryCompressor {
    pub fn new() -> Self {
        Self {
            accumulated_summary: None,
        }
    }

    /// 외부에서 누적된 요약을 주입(향후 영속 옵션). 본 PR에선 사용 X.
    pub fn with_summary(summary: Option<String>) -> Self {
        Self {
            accumulated_summary: summary,
        }
    }

    /// 압축 본 호출.
    ///
    /// `history`는 *시간순*(가장 오래된 → 최근). turns ≤ RECENT_TURNS_KEEP 면 압축 없음.
    /// 초과 시 가장 오래된 COMPRESS_TURNS_PER_STEP 턴을 요약하고, 나머지(최근 RECENT_TURNS_KEEP
    /// 턴)는 raw로 유지.
    ///
    /// 폴백: fast_model 미지정/요약 호출 에러 → 가장 오래된 turn을 *drop만* 하고 summary는 None.
    pub async fn compress(
        &self,
        history: &[HistoryTurn],
        provider: &dyn LlmProvider,
    ) -> AppResult<CompressedHistory> {
        // 1) 충분히 짧으면 압축 X.
        let total_turns = count_turns(history);
        if total_turns <= RECENT_TURNS_KEEP {
            return Ok(CompressedHistory {
                summary: self.accumulated_summary.clone(),
                recent_turns: history.to_vec(),
            });
        }

        // 2) 압축 대상 = 가장 오래된 COMPRESS_TURNS_PER_STEP 턴(=user/assistant 쌍).
        //    raw 보존 = 그 외 = 마지막 RECENT_TURNS_KEEP 턴.
        let recent_msg_cap = RECENT_TURNS_KEEP.saturating_mul(2);
        let recent_start = history.len().saturating_sub(recent_msg_cap);
        let recent_turns: Vec<HistoryTurn> = history[recent_start..].to_vec();

        // 압축 대상 = history[..recent_start] 중 *가장 오래된 COMPRESS_TURNS_PER_STEP*2 메시지*
        // 만 새로 요약. 그 앞쪽은 이미 누적 요약에 들어갔거나(외부 보존) 일단 drop.
        // 본 PR에선 단순히: 압축 대상 = history[..recent_start] 전체. 매 호출마다 동일
        // *지수적 압축* 발생 — turns 폭증 시 reduce 비용↑이지만 D-089는 step 1회 정책.
        let to_summarize: Vec<HistoryTurn> = history[..recent_start].to_vec();

        let fast_model = provider.fast_model();
        if fast_model.is_empty() || to_summarize.is_empty() {
            // graceful — recent만 반환, summary는 누적 그대로.
            debug!(
                target: "v043.history_compressor",
                "fast_model 미지정 또는 압축 대상 0 — 압축 skip, recent만 유지"
            );
            return Ok(CompressedHistory {
                summary: self.accumulated_summary.clone(),
                recent_turns,
            });
        }

        let user_prompt = build_summarize_prompt(&to_summarize);
        let request = ChatRequest {
            model: fast_model.to_string(),
            system: Some(SUMMARIZE_SYSTEM_PROMPT.to_string()),
            messages: vec![Message {
                role: Role::User,
                content: user_prompt,
            }],
            max_tokens: SUMMARIZE_MAX_TOKENS,
            cache_breakpoints: Vec::new(),
        };

        match collect_text(provider, request).await {
            Ok(raw) => {
                let cleaned = postprocess(&raw);
                if cleaned.is_empty() {
                    debug!(
                        target: "v043.history_compressor",
                        "요약 출력 비어 있음 — 누적 요약 그대로 유지"
                    );
                    Ok(CompressedHistory {
                        summary: self.accumulated_summary.clone(),
                        recent_turns,
                    })
                } else {
                    let new_summary = match self.accumulated_summary.as_ref() {
                        Some(prev) if !prev.is_empty() => format!("{prev} {cleaned}"),
                        _ => cleaned,
                    };
                    debug!(
                        target: "v043.history_compressor",
                        summary_len = new_summary.len(),
                        recent_turns = recent_turns.len(),
                        "history 압축 적용"
                    );
                    Ok(CompressedHistory {
                        summary: Some(new_summary),
                        recent_turns,
                    })
                }
            }
            Err(e) => {
                warn!(
                    target: "v043.history_compressor",
                    error = %e,
                    "history 요약 실패 — 가장 오래된 turn drop만 (graceful)"
                );
                Ok(CompressedHistory {
                    summary: self.accumulated_summary.clone(),
                    recent_turns,
                })
            }
        }
    }
}

/// turn 수 계산 — 메시지 2개 = 1턴(user+assistant). 홀수면 floor.
/// (마지막 메시지가 user인 partial turn은 1턴으로 셈 — 사용자 발화 자체가 turn 시작.)
fn count_turns(history: &[HistoryTurn]) -> usize {
    // 메시지 N개를 2로 나눈 뒤 ceiling — 마지막 user-only도 1턴.
    history.len().div_ceil(2)
}

/// 요약 입력 prompt 직렬화.
fn build_summarize_prompt(turns: &[HistoryTurn]) -> String {
    let mut buf = String::new();
    buf.push_str("대화:\n");
    for t in turns {
        let label = match t.role {
            Role::User => "사용자",
            Role::Assistant => "도우미",
        };
        buf.push_str(label);
        buf.push_str(": ");
        buf.push_str(t.content.trim());
        buf.push('\n');
    }
    buf.push_str("요약:");
    buf
}

/// rewriter::collect_text 와 동일 — 모든 TextDelta를 누적, Done에서 break.
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

/// rewriter::postprocess와 동일 정책 — 첫 줄·라벨 제거·따옴표 제거.
fn postprocess(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let first_line = trimmed.split(['\n', '\r']).next().unwrap_or("").trim();
    let stripped = strip_label(first_line);
    let unquoted = strip_outer_quotes(stripped);
    unquoted.trim().to_string()
}

fn strip_label(s: &str) -> &str {
    const LABELS: &[&str] = &[
        "요약:",
        "요약 :",
        "Summary:",
        "summary:",
    ];
    for label in LABELS {
        if let Some(rest) = s.strip_prefix(label) {
            return rest.trim_start();
        }
    }
    s
}

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
            return s.trim_start_matches('「').trim_end_matches('」');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ChatEvent, ChatRequest, ChatStream, Usage};
    use async_trait::async_trait;
    use std::sync::Mutex;

    fn turn(role: Role, content: &str) -> HistoryTurn {
        HistoryTurn {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn count_turns_floors_message_pairs() {
        // 2 msg = 1턴, 3 msg = 2턴(partial), 4 msg = 2턴, 12 msg = 6턴.
        assert_eq!(count_turns(&[]), 0);
        assert_eq!(count_turns(&[turn(Role::User, "a")]), 1);
        assert_eq!(
            count_turns(&[turn(Role::User, "a"), turn(Role::Assistant, "b")]),
            1
        );
        assert_eq!(
            count_turns(&[
                turn(Role::User, "a"),
                turn(Role::Assistant, "b"),
                turn(Role::User, "c"),
            ]),
            2
        );
        let mut h = Vec::new();
        for i in 0..6 {
            h.push(turn(Role::User, &format!("u{i}")));
            h.push(turn(Role::Assistant, &format!("a{i}")));
        }
        assert_eq!(count_turns(&h), 6);
    }

    #[tokio::test]
    async fn compress_returns_no_summary_when_history_short() {
        // 6턴 이하 → summary=None, recent_turns=전체.
        let provider = FastMockProvider::with_text("이건 호출되지 말아야 함");
        let mut history = Vec::new();
        for i in 0..3 {
            history.push(turn(Role::User, &format!("u{i}")));
            history.push(turn(Role::Assistant, &format!("a{i}")));
        }
        let compressor = HistoryCompressor::new();
        let result = compressor.compress(&history, &provider).await.unwrap();
        assert!(result.summary.is_none());
        assert_eq!(result.recent_turns.len(), 6);
        // provider 호출 없었어야 함 — captured prompt 비어 있음.
        assert!(provider.captured().is_empty());
    }

    #[tokio::test]
    async fn compress_summarizes_oldest_turns_when_over_threshold() {
        // 12턴(=24 msg) → 가장 오래된 6턴(12 msg)을 요약, 최근 6턴(12 msg) raw.
        let provider = FastMockProvider::with_text("GameBoy PPU 구현 방식 논의 요약 한 줄");
        let mut history = Vec::new();
        for i in 0..12 {
            history.push(turn(Role::User, &format!("UQ{i}")));
            history.push(turn(Role::Assistant, &format!("AR{i}")));
        }
        let compressor = HistoryCompressor::new();
        let result = compressor.compress(&history, &provider).await.unwrap();
        assert_eq!(result.summary.as_deref(), Some("GameBoy PPU 구현 방식 논의 요약 한 줄"));
        assert_eq!(result.recent_turns.len(), 12); // 6턴 = 12 msg.
        // recent는 가장 최근 6턴(UQ6~UQ11 + AR6~AR11).
        assert_eq!(result.recent_turns.first().unwrap().content, "UQ6");
        assert_eq!(result.recent_turns.last().unwrap().content, "AR11");
        // 요약 prompt에 가장 오래된 turn 들어갔는지 — UQ0/AR0 등.
        let captured = provider.captured();
        assert!(captured.contains("UQ0"));
        assert!(captured.contains("AR5"));
        assert!(!captured.contains("UQ6"));
    }

    #[tokio::test]
    async fn compress_falls_back_when_provider_has_no_fast_model() {
        // fast_model = "" → 압축 skip, summary는 누적 그대로 (None), recent만 잘림.
        let provider = MockNoFast;
        let mut history = Vec::new();
        for i in 0..10 {
            history.push(turn(Role::User, &format!("U{i}")));
            history.push(turn(Role::Assistant, &format!("A{i}")));
        }
        let compressor = HistoryCompressor::new();
        let result = compressor.compress(&history, &provider).await.unwrap();
        assert!(result.summary.is_none());
        assert_eq!(result.recent_turns.len(), 12); // 6턴 raw.
    }

    #[tokio::test]
    async fn compress_falls_back_on_provider_error() {
        let provider = FailingFastProvider;
        let mut history = Vec::new();
        for i in 0..8 {
            history.push(turn(Role::User, &format!("U{i}")));
            history.push(turn(Role::Assistant, &format!("A{i}")));
        }
        let compressor = HistoryCompressor::new();
        let result = compressor.compress(&history, &provider).await.unwrap();
        // graceful — recent만, summary는 누적(None) 그대로.
        assert!(result.summary.is_none());
        assert_eq!(result.recent_turns.len(), 12);
    }

    #[tokio::test]
    async fn compress_appends_to_existing_accumulated_summary() {
        let provider = FastMockProvider::with_text("새 요약");
        let mut history = Vec::new();
        for i in 0..8 {
            history.push(turn(Role::User, &format!("U{i}")));
            history.push(turn(Role::Assistant, &format!("A{i}")));
        }
        let compressor = HistoryCompressor::with_summary(Some("이전 요약".to_string()));
        let result = compressor.compress(&history, &provider).await.unwrap();
        assert_eq!(result.summary.as_deref(), Some("이전 요약 새 요약"));
    }

    #[test]
    fn postprocess_takes_first_line_only_and_strips_label() {
        assert_eq!(postprocess("요약: PPU 구현 한 줄\n부연 설명"), "PPU 구현 한 줄");
        assert_eq!(postprocess("Summary: foo bar"), "foo bar");
    }

    // --------- mock providers ---------

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

    struct MockNoFast;

    #[async_trait]
    impl LlmProvider for MockNoFast {
        // fast_model 미지정 (default = "").
        async fn chat_stream(&self, _request: ChatRequest) -> AppResult<ChatStream> {
            let stream = async_stream::try_stream! {
                yield ChatEvent::Done { usage: Usage::default() };
            };
            Ok(Box::pin(stream))
        }
    }

    struct FailingFastProvider;

    #[async_trait]
    impl LlmProvider for FailingFastProvider {
        fn fast_model(&self) -> &str {
            "test-haiku"
        }
        async fn chat_stream(&self, _request: ChatRequest) -> AppResult<ChatStream> {
            Err(crate::error::AppError::LlmApi {
                message: "history_compressor mock failure".into(),
            })
        }
    }
}
