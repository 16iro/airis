// v0.6.x PR (D-110) — RAG 파이프라인 관측성(트레이스).
//
// WeKnora의 "Langfuse식 span tree / 파싱 트레이스 타임라인"을 airis 제약(로컬 단일
// 사용자·평소 비용 0)에 맞춰 *경량 인메모리 트레이스*로 이식한 모듈. 한 chat 쿼리의
// 파싱→검색→정제→리랭크→패킹 각 단계의 시간·점수·버려진 source 수를 기록한다.
//
// 목적: 그 자체로 답을 좋게 만들지 않는 *계측 토대*. D-108(passage cleaning) /
// D-109(쿼리 라우팅) / D-111(GraphRAG) 등 다른 개선의 효과를 추측이 아니라 데이터로
// 검증하기 위한 도구.
//
// 비용 0 보장:
//   * `RagTrace::disabled()` 면 `begin`/`end`/`summary` 모두 사실상 no-op (push 안 함).
//   * 활성화는 `Settings::dev_rag_trace` (디폴트 false) → orchestration이 주입.
//   * 로컬 단일 사용자라 기록이 기기 밖으로 나가지 않음(프라이버시 부담 없음).
//
// 데이터 흐름: `RagTrace` → `finish()` → `Some(TraceReport)`(활성) / `None`(비활성).
// 활성 시 tracing::info로 1줄 로그 + report를 `ChatContextSummary`에 실어 dev panel이
// 표시할 수 있게 한다.

#![allow(dead_code)]

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 한 단계(span)의 트레이스 — 이름 + 소요 ms + 임의 필드(hits·dropped·score 등).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    /// 단계 이름 (`retrieval`, `passage_clean`, `rerank`, `pack` 등).
    pub name: String,
    /// 소요 시간 (밀리초, 소수 포함 — sub-ms 단계 가시화).
    pub ms: f64,
    /// 단계별 부가 필드. 평면화되어 JSON에 그대로 노출.
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

/// 한 쿼리 전체 트레이스 리포트 — span 목록 + 쿼리 수준 요약 필드.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TraceReport {
    pub spans: Vec<TraceSpan>,
    /// 쿼리 수준 요약(query_class·used_hyde·total_ms 등).
    #[serde(flatten)]
    pub summary: Map<String, Value>,
}

impl TraceReport {
    /// span들의 ms 합.
    pub fn total_ms(&self) -> f64 {
        self.spans.iter().map(|s| s.ms).sum()
    }
}

/// 진행 중 span 핸들 — `RagTrace::begin`이 반환. `note`로 필드를 누적하고 `RagTrace::end`로
/// 닫는다. 비활성 트레이스에서도 생성 자체는 cheap(Instant 1회).
pub struct Span {
    start: Instant,
    name: String,
    fields: Map<String, Value>,
}

impl Span {
    /// 필드 1개 추가 (builder 스타일).
    pub fn note(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.to_string(), value.into());
        self
    }
}

/// RAG 트레이스 수집기. 비활성이면 모든 기록이 no-op.
pub struct RagTrace {
    enabled: bool,
    report: TraceReport,
}

impl RagTrace {
    /// 활성/비활성 트레이스 생성.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            report: TraceReport::default(),
        }
    }

    /// 비활성 트레이스 — 모든 기록 no-op.
    pub fn disabled() -> Self {
        Self::new(false)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// span 시작. 비활성이어도 호출 가능(Instant만 잡음).
    pub fn begin(&self, name: &str) -> Span {
        Span {
            start: Instant::now(),
            name: name.to_string(),
            fields: Map::new(),
        }
    }

    /// span 종료 — 활성일 때만 소요 ms를 계산해 report에 push.
    pub fn end(&mut self, span: Span) {
        if !self.enabled {
            return;
        }
        let ms = span.start.elapsed().as_micros() as f64 / 1000.0;
        self.report.spans.push(TraceSpan {
            name: span.name,
            ms,
            fields: span.fields,
        });
    }

    /// 쿼리 수준 요약 필드 1개 기록 (활성일 때만).
    pub fn summary(&mut self, key: &str, value: impl Into<Value>) {
        if !self.enabled {
            return;
        }
        self.report.summary.insert(key.to_string(), value.into());
    }

    /// 트레이스 종료 — 활성이면 1줄 로그 + report 반환, 비활성이면 None.
    pub fn finish(mut self) -> Option<TraceReport> {
        if !self.enabled {
            return None;
        }
        let total = self.report.total_ms();
        self.report
            .summary
            .insert("total_ms".to_string(), Value::from(total));
        let span_names: Vec<String> = self
            .report
            .spans
            .iter()
            .map(|s| format!("{}={:.2}ms", s.name, s.ms))
            .collect();
        tracing::info!(
            target: "v060.trace",
            total_ms = total,
            spans = %span_names.join(" "),
            "rag trace"
        );
        Some(self.report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_trace_records_nothing() {
        let mut t = RagTrace::disabled();
        let sp = t.begin("retrieval").note("hits", 10_i64);
        t.end(sp);
        t.summary("query_class", "keyword");
        assert!(t.finish().is_none(), "비활성 트레이스는 None 반환");
    }

    #[test]
    fn enabled_trace_records_spans_and_fields() {
        let mut t = RagTrace::new(true);
        let sp = t.begin("retrieval").note("hits", 7_i64).note("book", "b1");
        t.end(sp);
        let sp2 = t.begin("rerank").note("dropped", 3_i64);
        t.end(sp2);
        t.summary("query_class", "conceptual");

        let report = t.finish().expect("활성 트레이스는 report 반환");
        assert_eq!(report.spans.len(), 2);
        assert_eq!(report.spans[0].name, "retrieval");
        assert_eq!(report.spans[0].fields.get("hits"), Some(&Value::from(7)));
        assert_eq!(
            report.spans[0].fields.get("book"),
            Some(&Value::from("b1"))
        );
        assert_eq!(report.spans[1].fields.get("dropped"), Some(&Value::from(3)));
        assert_eq!(
            report.summary.get("query_class"),
            Some(&Value::from("conceptual"))
        );
        // total_ms는 finish에서 자동 주입.
        assert!(report.summary.contains_key("total_ms"));
    }

    #[test]
    fn report_serializes_to_flat_json() {
        let mut t = RagTrace::new(true);
        let sp = t.begin("pack").note("tokens", 3200_i64);
        t.end(sp);
        t.summary("used_hyde", false);
        let report = t.finish().unwrap();
        let json = serde_json::to_value(&report).unwrap();
        // span 필드가 평면화돼야 함.
        assert_eq!(json["spans"][0]["name"], "pack");
        assert_eq!(json["spans"][0]["tokens"], 3200);
        assert_eq!(json["used_hyde"], false);
    }

    #[test]
    fn total_ms_sums_spans() {
        let mut report = TraceReport::default();
        report.spans.push(TraceSpan {
            name: "a".into(),
            ms: 1.5,
            fields: Map::new(),
        });
        report.spans.push(TraceSpan {
            name: "b".into(),
            ms: 2.25,
            fields: Map::new(),
        });
        assert!((report.total_ms() - 3.75).abs() < 1e-9);
    }
}
