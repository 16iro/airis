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

// v0.4.1 RAG 엔진. 기존 chunker/keyword(paragraphs FTS)와 *공존* — 책별 chunks 적재
// 여부에 따라 폴백 (v0.4.1_HANDOFF §5 무파괴 원칙).
pub mod v041;

// v0.4.2 cascade·강건성 토대 — DB v15 위에서 동작하는 IndexingWorker + 재개 헬퍼.
// PR 1은 worker/resume 함수만, 실제 호출은 PR 2(T2 인덱서)~PR 3(UI)에서 시작.
pub mod v042;
