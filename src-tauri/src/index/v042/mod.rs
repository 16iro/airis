// v0.4.2 인덱서 — cascade(T1/T2) + 강건성(트랜잭션 체크포인트 + 재개) 토대.
//
// PR 1 범위 (이 PR): 모듈 골격 + IndexingWorker + resume 헬퍼 + 단위 테스트.
//   * worker — 배치 단위 트랜잭션 체크포인트(임베딩 + status + progress 동시 commit).
//   * resume — 비정상 종료/사용자 일시정지 잡 발견 시 재개 plan 산출.
//
// PR 2~5에서 추가 책임 (PR 1은 stub/시그니처만):
//   * PR 2 — vectors_t2 업서트 어댑터 (T2 BGE-M3 인덱서가 worker.embed_batch 호출).
//   * PR 3 — pause/resume UI + OS power 이벤트 트리거.
//   * PR 4 — embedding_cache·response_cache 적용 (worker가 cache lookup 우선).
//   * PR 5 — 자원 제한(process priority, ONNX intra-op 절반) + acceptance 측정.
//
// 기존 v041은 *그대로 유지* — read-only legacy로 전환 (HANDOFF §9). v0.4.2는
// 신규 인덱싱부터 v042 path로 진입하지만, 본 PR에서 호출 측 변경 X.
//
// dead_code 허용: PR 1·2 호출 0건 (commands wiring은 PR 3에서). PR 3+에서 호출 들어오면 자연 해소.
#![allow(dead_code)]

pub mod active_index;
pub mod embedder_t2;
pub mod indexer_t2;
pub mod manifest;
pub mod resume;
pub mod retrieval;
pub mod vector_store_t2;
pub mod worker;
