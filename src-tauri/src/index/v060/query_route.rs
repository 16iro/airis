// v0.6.x PR (D-109) — 쿼리 적응형 검색 라우팅.
//
// WeKnora의 "automatic retriever selection"을 airis에 맞춰 이식. airis는 지금 항상
// Dense(mE5 벡터) + BM25(FTS5)를 *균등 가중* RRF로 병합한다. 본 모듈은 질문 유형을
// 판정해 RRF 가중치를 살짝 조절한다 — 키워드형이면 BM25 쪽, 개념형이면 Dense 쪽.
//
// SUGGESTION 결정 = (B) LLM 분류. 단 "왕복 지연" 절충안:
//   * query rewriting 호출이 *어차피 일어날 때*(SearchStrength::Balanced/Accurate) 그
//     프롬프트에 유형 분류를 끼워 *한 번에* 받는다 → 추가 LLM 왕복 없음.
//   * rewriting이 생략되는 Fast 모드에선 분류도 생략 → `QueryClass::Balanced`(균등 RRF).
//   * LLM 출력 파싱 실패/blank → 휴리스틱 폴백 → 그래도 모호하면 Balanced.
//
// 안전망: 어떤 경우에도 폴백은 *균등 RRF(Balanced)*. 라우팅이 틀려도 현행보다 나빠지지
// 않게 한 쪽 검색기를 0으로 만들지 않는다(가중치는 0.7~1.3 범위의 완만한 기울임).

#![allow(dead_code)]

use futures_util::StreamExt;
use tracing::{debug, warn};

use crate::error::AppResult;
use crate::index::v043::rewriter::HistoryTurn;
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role};

/// 질문 유형 — RRF 가중치 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryClass {
    /// 키워드/정확매칭형 (고유명사·코드·숫자·짧은 용어) → BM25 가중 ↑.
    Keyword,
    /// 개념/설명형 (서술형 질문·관계·비교) → Dense 가중 ↑.
    Conceptual,
    /// 모호/혼합 → 균등 RRF (현행과 동일, 안전 폴백).
    Balanced,
}

impl QueryClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Conceptual => "conceptual",
            Self::Balanced => "balanced",
        }
    }

    /// LLM/사용자 입력 문자열 → QueryClass. 미상이면 None.
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "keyword" | "키워드" => Some(Self::Keyword),
            "conceptual" | "개념" => Some(Self::Conceptual),
            "balanced" | "균형" | "혼합" => Some(Self::Balanced),
            _ => None,
        }
    }

    /// RRF 가중치 (w_vec, w_fts). Dense=vector, FTS=BM25.
    ///
    /// 완만한 기울임 — 한쪽을 0으로 만들지 않아 라우팅 오판 시에도 반대 검색기가 보강한다.
    pub fn rrf_weights(self) -> (f64, f64) {
        match self {
            Self::Keyword => (0.7, 1.3),
            Self::Conceptual => (1.3, 0.7),
            Self::Balanced => (1.0, 1.0),
        }
    }
}

/// 라우팅 결과 — rewritten query + 분류.
#[derive(Debug, Clone)]
pub struct RoutedQuery {
    pub rewritten: String,
    pub class: QueryClass,
}

/// 분류 출력 토큰 한도 — rewriting 1줄 + 분류 1줄이라 작게.
const ROUTE_MAX_TOKENS: u32 = 256;

/// 재작성 + 분류 통합 시스템 프롬프트. v043 rewriter 프롬프트에 *유형 한 줄*을 더한 형태.
const ROUTE_SYSTEM_PROMPT: &str = "당신은 한국어 학습 도우미의 검색 쿼리 처리기입니다. \
사용자의 마지막 질문을 앞 대화 맥락을 참고해 처리한 뒤 *정확히 두 줄*로 출력합니다.\n\n\
첫 줄 = 대명사·생략을 풀어 쓴 한 줄 검색 쿼리 (앞에 \"재작성: \").\n\
둘째 줄 = 질문 유형 (앞에 \"유형: \"), 다음 셋 중 하나:\n\
  - keyword  : 고유명사·코드·숫자·정확한 용어를 찾는 질문\n\
  - conceptual : 개념 설명·관계·비교·이유를 묻는 서술형 질문\n\
  - balanced : 애매하거나 둘 다 해당\n\n\
규칙:\n\
1) 출력은 정확히 두 줄. 다른 설명·따옴표 금지.\n\
2) 영어 용어는 그대로 두세요.\n\
3) 대명사(\"이것\",\"그게\")만 맥락으로 풀어 쓰고, 의도가 명확하면 그대로.\n\n\
예시 1\n\
이전 대화:\n\
사용자: GameBoy 에뮬레이터의 PPU가 뭐예요?\n\
도우미: PPU는 Picture Processing Unit으로 화면 출력을 담당합니다.\n\
질문: 이거 어떻게 구현하지?\n\
재작성: GameBoy PPU(Picture Processing Unit) 구현 방법\n\
유형: conceptual\n\n\
예시 2\n\
이전 대화:\n\
(없음)\n\
질문: TCP 3-way handshake\n\
재작성: TCP 3-way handshake\n\
유형: keyword";

/// QueryRouter — rewriting + 분류를 LLM 1회로 처리. provider는 호출 시점 인자(D-066 hot-swap 호환).
#[derive(Debug, Default, Clone, Copy)]
pub struct QueryRouter;

impl QueryRouter {
    pub fn new() -> Self {
        Self
    }

    /// 통합 호출. 실패/미지정 시 *원본 query + 휴리스틱 분류*로 graceful 폴백.
    pub async fn route(
        &self,
        history: &[HistoryTurn],
        query: &str,
        provider: &dyn LlmProvider,
    ) -> AppResult<RoutedQuery> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(RoutedQuery {
                rewritten: query.to_string(),
                class: QueryClass::Balanced,
            });
        }
        let fast_model = provider.fast_model();
        if fast_model.is_empty() {
            // mock 등 fast_model 미구현 — 원본 + 휴리스틱.
            return Ok(RoutedQuery {
                rewritten: query.to_string(),
                class: classify_heuristic(trimmed),
            });
        }

        let user_prompt = build_user_prompt(history, trimmed);
        let request = ChatRequest {
            model: fast_model.to_string(),
            system: Some(ROUTE_SYSTEM_PROMPT.to_string()),
            messages: vec![Message {
                role: Role::User,
                content: user_prompt,
            }],
            max_tokens: ROUTE_MAX_TOKENS,
            cache_breakpoints: Vec::new(),
        };

        match collect_text(provider, request).await {
            Ok(raw) => {
                let (rewritten, class) = parse_route_output(&raw, trimmed);
                debug!(
                    target: "v060.query_route",
                    class = class.as_str(),
                    "query routed"
                );
                Ok(RoutedQuery { rewritten, class })
            }
            Err(e) => {
                warn!(
                    target: "v060.query_route",
                    error = %e,
                    "query routing 실패 — 원본 query + 휴리스틱 폴백"
                );
                Ok(RoutedQuery {
                    rewritten: query.to_string(),
                    class: classify_heuristic(trimmed),
                })
            }
        }
    }
}

/// 시간순 history(최근 4턴) + 현재 질문 → user prompt. v043 rewriter와 동일 윈도우 정책.
fn build_user_prompt(history: &[HistoryTurn], query: &str) -> String {
    const WINDOW: usize = 4;
    let cap = WINDOW * 2;
    let window: &[HistoryTurn] = if history.len() <= cap {
        history
    } else {
        &history[history.len() - cap..]
    };
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

/// provider 호출을 non-streaming처럼 — 모든 TextDelta 누적, Done에서 break.
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

/// LLM 두 줄 출력 파싱. 라벨(`재작성:`, `유형:`)을 찾아 추출.
///
/// 폴백:
///   * rewritten 비면 원본 query.
///   * class 못 찾으면 *재작성문 기준* 휴리스틱 분류.
fn parse_route_output(raw: &str, original: &str) -> (String, QueryClass) {
    let mut rewritten: Option<String> = None;
    let mut class: Option<QueryClass> = None;

    for line in raw.lines() {
        let line = line.trim();
        if let Some(r) = strip_one_of(line, &["재작성:", "재작성 :", "Rewritten:", "rewritten:"]) {
            let r = strip_outer_quotes(r.trim());
            if !r.is_empty() {
                rewritten = Some(r.to_string());
            }
        } else if let Some(c) = strip_one_of(line, &["유형:", "유형 :", "Type:", "type:", "class:"]) {
            class = QueryClass::parse(c.trim());
        }
    }

    // 라벨이 전혀 없는 경우(LLM이 라벨 무시) — 첫 줄을 rewritten으로.
    if rewritten.is_none() {
        if let Some(first) = raw.lines().map(str::trim).find(|l| !l.is_empty()) {
            let first = strip_outer_quotes(first);
            if !first.is_empty() {
                rewritten = Some(first.to_string());
            }
        }
    }

    let rewritten = rewritten.unwrap_or_else(|| original.to_string());
    let class = class.unwrap_or_else(|| classify_heuristic(&rewritten));
    (rewritten, class)
}

/// 라벨 prefix 중 하나를 떼고 나머지 반환.
fn strip_one_of<'a>(s: &'a str, labels: &[&str]) -> Option<&'a str> {
    for label in labels {
        if let Some(rest) = s.strip_prefix(label) {
            return Some(rest);
        }
    }
    None
}

/// 양 끝 따옴표 한 겹 제거 (rewriter::strip_outer_quotes와 동일 정책).
fn strip_outer_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.chars().count() < 2 {
        return s;
    }
    let first = s.chars().next().unwrap();
    let last = s.chars().last().unwrap();
    if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
        let inner: String = s.chars().collect::<Vec<_>>()[1..s.chars().count() - 1]
            .iter()
            .collect();
        // &str 반환 위해 byte offset으로 자른다.
        let start = s.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0);
        let end = s
            .char_indices()
            .nth(s.chars().count() - 1)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        let _ = inner;
        return &s[start..end];
    }
    if first == '「' && last == '」' {
        return s.trim_start_matches('「').trim_end_matches('」');
    }
    s
}

/// 규칙 기반 분류 — LLM 미사용/폴백 경로. "확실한 키워드형"만 Keyword, 명확한 서술형만
/// Conceptual, 나머지는 Balanced (보수적).
pub fn classify_heuristic(query: &str) -> QueryClass {
    let q = query.trim();
    if q.is_empty() {
        return QueryClass::Balanced;
    }

    // --- 키워드형 강한 신호 ---
    // 따옴표로 감싼 정확 문구.
    let has_quote = q.contains('"') || q.contains('「') || q.contains('\'');
    // 코드/식별자 신호: 백틱, ::, (), <>, snake_case, 점 표기(file.ext), 슬래시 경로.
    let has_code = q.contains('`')
        || q.contains("::")
        || q.contains("()")
        || q.contains("->")
        || q.contains('_')
        || q.contains('/')
        || has_dotted_identifier(q);
    // 대문자 약어(2~6자 ALL CAPS, 예: TCP, HTTP, PPU).
    let has_acronym = q
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| {
            let len = tok.chars().count();
            (2..=6).contains(&len) && tok.chars().all(|c| c.is_ascii_uppercase())
        });

    // --- 개념형 강한 신호 ---
    let conceptual_markers = [
        "왜", "어떻게", "무엇", "뭐", "차이", "비교", "관계", "설명", "이유", "원리",
        "why", "how", "what", "explain", "difference", "compare", "relationship", "concept",
    ];
    let lower = q.to_ascii_lowercase();
    let has_conceptual = conceptual_markers.iter().any(|m| lower.contains(m));
    // 질문 길이(공백 토큰 수) — 길면 서술형 가능성.
    let token_count = q.split_whitespace().count();

    // --- 판정 (보수적 우선순위) ---
    // 1) 코드/따옴표는 강한 키워드 신호 — 개념 마커보다 우선.
    if has_code || has_quote {
        return QueryClass::Keyword;
    }
    // 2) 명시적 개념 마커 + 어느 정도 길이 → Conceptual.
    if has_conceptual && token_count >= 2 {
        return QueryClass::Conceptual;
    }
    // 3) 아주 짧고(≤3토큰) 약어 포함 → Keyword (예: "TCP handshake").
    if token_count <= 3 && has_acronym {
        return QueryClass::Keyword;
    }
    // 4) 그 외 — 안전 폴백.
    QueryClass::Balanced
}

/// `foo.bar` / `file.rs` 같은 점 표기 식별자가 있는지 (URL 문장부호 제외 보수적 검사).
fn has_dotted_identifier(q: &str) -> bool {
    q.split_whitespace().any(|tok| {
        let dots = tok.matches('.').count();
        dots >= 1
            && tok.len() >= 3
            && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
            // "다." 같은 한국어 종결은 위 all() ascii 조건에서 자연 배제.
            && !tok.ends_with('.')
            && !tok.starts_with('.')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockProvider;
    use crate::llm::{ChatStream, Usage};
    use async_trait::async_trait;

    #[test]
    fn weights_tilt_toward_correct_retriever() {
        let (vk, fk) = QueryClass::Keyword.rrf_weights();
        assert!(fk > vk, "키워드형은 BM25(fts) 가중 ↑");
        let (vc, fc) = QueryClass::Conceptual.rrf_weights();
        assert!(vc > fc, "개념형은 Dense(vector) 가중 ↑");
        assert_eq!(QueryClass::Balanced.rrf_weights(), (1.0, 1.0));
    }

    #[test]
    fn weights_never_zero_a_retriever() {
        // 라우팅 오판 안전망 — 어느 쪽도 0이 아님.
        for c in [QueryClass::Keyword, QueryClass::Conceptual, QueryClass::Balanced] {
            let (v, f) = c.rrf_weights();
            assert!(v > 0.0 && f > 0.0, "{:?} 가중치 0 금지", c);
        }
    }

    #[test]
    fn class_parse_accepts_ko_and_en() {
        assert_eq!(QueryClass::parse("keyword"), Some(QueryClass::Keyword));
        assert_eq!(QueryClass::parse("  Conceptual "), Some(QueryClass::Conceptual));
        assert_eq!(QueryClass::parse("개념"), Some(QueryClass::Conceptual));
        assert_eq!(QueryClass::parse("균형"), Some(QueryClass::Balanced));
        assert_eq!(QueryClass::parse("garbage"), None);
    }

    #[test]
    fn heuristic_detects_keyword_signals() {
        assert_eq!(classify_heuristic("\"정확한 문구\""), QueryClass::Keyword);
        assert_eq!(classify_heuristic("Vec::push 사용법"), QueryClass::Keyword);
        assert_eq!(classify_heuristic("main.rs 구조"), QueryClass::Keyword);
        assert_eq!(classify_heuristic("TCP handshake"), QueryClass::Keyword);
    }

    #[test]
    fn heuristic_detects_conceptual_signals() {
        assert_eq!(
            classify_heuristic("프로세스와 스레드의 차이가 뭐야"),
            QueryClass::Conceptual
        );
        assert_eq!(
            classify_heuristic("왜 가비지 컬렉션이 필요한가"),
            QueryClass::Conceptual
        );
        assert_eq!(
            classify_heuristic("explain how scheduling works"),
            QueryClass::Conceptual
        );
    }

    #[test]
    fn heuristic_falls_back_to_balanced() {
        assert_eq!(classify_heuristic("운영체제"), QueryClass::Balanced);
        assert_eq!(classify_heuristic(""), QueryClass::Balanced);
    }

    #[test]
    fn heuristic_code_beats_conceptual_marker() {
        // 코드 신호가 개념 마커보다 우선 (보수적: 정확매칭 우선).
        assert_eq!(
            classify_heuristic("Vec::push 는 어떻게 동작해"),
            QueryClass::Keyword
        );
    }

    #[test]
    fn parse_route_output_extracts_both_lines() {
        let raw = "재작성: GameBoy PPU 구현 방법\n유형: conceptual";
        let (r, c) = parse_route_output(raw, "이거 어떻게 구현해");
        assert_eq!(r, "GameBoy PPU 구현 방법");
        assert_eq!(c, QueryClass::Conceptual);
    }

    #[test]
    fn parse_route_output_handles_missing_class_with_heuristic() {
        let raw = "재작성: Vec::push 사용법";
        let (r, c) = parse_route_output(raw, "original");
        assert_eq!(r, "Vec::push 사용법");
        assert_eq!(c, QueryClass::Keyword, "class 누락 → 휴리스틱");
    }

    #[test]
    fn parse_route_output_handles_unlabeled_first_line() {
        let raw = "그냥 한 줄 출력";
        let (r, _c) = parse_route_output(raw, "original");
        assert_eq!(r, "그냥 한 줄 출력");
    }

    #[test]
    fn parse_route_output_strips_quotes() {
        let raw = "재작성: \"따옴표 제거\"\n유형: keyword";
        let (r, c) = parse_route_output(raw, "x");
        assert_eq!(r, "따옴표 제거");
        assert_eq!(c, QueryClass::Keyword);
    }

    #[tokio::test]
    async fn route_falls_back_when_no_fast_model() {
        // MockProvider는 fast_model="" → LLM 호출 없이 원본 + 휴리스틱.
        let provider = MockProvider::from_text_chunks(&["ignored"]);
        let routed = QueryRouter::new()
            .route(&[], "Vec::push 사용법", &provider)
            .await
            .unwrap();
        assert_eq!(routed.rewritten, "Vec::push 사용법");
        assert_eq!(routed.class, QueryClass::Keyword);
    }

    #[tokio::test]
    async fn route_parses_llm_output_with_fast_model() {
        let provider = RouteMock {
            text: "재작성: TCP 3-way handshake 동작\n유형: keyword".to_string(),
        };
        let routed = QueryRouter::new()
            .route(&[], "이거 동작", &provider)
            .await
            .unwrap();
        assert_eq!(routed.rewritten, "TCP 3-way handshake 동작");
        assert_eq!(routed.class, QueryClass::Keyword);
    }

    struct RouteMock {
        text: String,
    }

    #[async_trait]
    impl LlmProvider for RouteMock {
        fn fast_model(&self) -> &str {
            "test-haiku"
        }
        async fn chat_stream(&self, _request: ChatRequest) -> AppResult<ChatStream> {
            let text = self.text.clone();
            let stream = async_stream::try_stream! {
                yield ChatEvent::TextDelta { text };
                yield ChatEvent::Done { usage: Usage::default() };
            };
            Ok(Box::pin(stream))
        }
    }
}
