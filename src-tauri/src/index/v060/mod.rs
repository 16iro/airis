// v0.6.x RAG 보강 슬라이스 — WeKnora에서 골라 이식한 4개 최적화.
//
// 기존 v041/v042/v043 모듈을 *건드리지 않고* 새 진입점만 추가한다 (HANDOFF §9 원칙).
//
//   * D-108 — `passage_clean`: 검색→리랭크 사이 규칙 기반 청크 정제.
//   * D-109 — `query_route`  : LLM 1회로 rewriting+분류 → RRF 가중 라우팅.
//   * D-110 — `trace`        : 경량 RAG 파이프라인 관측성(설정 토글, 평소 비용 0).
//   * D-111 — `graph`        : 경량 로컬 GraphRAG (SQLite 동시출현 1홉 확장).
//
// 청킹 보강(D-112)은 v041 chunker를 확장하므로 본 슬라이스가 아니라 v041 측에 둔다.

#![allow(dead_code)]

pub mod graph;
pub mod passage_clean;
pub mod query_route;
pub mod trace;
