// 인덱싱 모듈 — 책 → paragraphs → FTS5.
//
// PR 11 범위 (D-064 결정에 따른 단순화):
//   * `chunker`  = 섹션 본문을 ~500자 청크로 분할 (한국어 문장 경계 보존).
//   * `keyword`  = paragraphs INSERT — FTS5는 트리거가 자동 동기화.
//   * 검색은 commands/search.rs::search_sections이 paragraphs_fts MATCH 사용.
//
// 임베딩·하이브리드는 v0.3+ (D-064/D-065 supersede).
//
// commands::book(이 PR 후속)에서 호출 들어오면 dead_code 경고 자동 해소.
#![allow(dead_code)]

pub mod chunker;
pub mod keyword;
