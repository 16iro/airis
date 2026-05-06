// v0.4.3 retrieval·응답 품질 슬라이스. PR 1은 query rewriting layer만.
//
// 본 모듈은 v041/v042의 기존 retrieval/context 모듈을 *건드리지 않고* 새 진입점만
// 추가한다 (HANDOFF §9 권고대로 함수 시그니처 옵션 인자 누적). PR 2~4가 sentence
// window·auto-merging·MMR·reranker 등을 같은 모듈 트리 아래에 더한다.

#![allow(dead_code)]

pub mod rewriter;
