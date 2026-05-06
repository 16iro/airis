// v0.4.3 retrieval·응답 품질 슬라이스.
//
// 본 모듈은 v041/v042의 기존 retrieval/context 모듈을 *건드리지 않고* 새 진입점만
// 추가한다 (HANDOFF §9 권고대로 함수 시그니처 옵션 인자 누적).
//
//   * PR 1 (D-086) — `rewriter`: query rewriting + 검색 강도 토글.
//   * PR 2 (D-088) — `post_retrieval`: sentence window 확장 + auto-merging + MMR.
//   * PR 3+       — HyDE / reranker / 대화 압축 등.

#![allow(dead_code)]

pub mod post_retrieval;
pub mod rewriter;
