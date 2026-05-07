// v0.4.4 PR 5 — BYOK 임베딩 슬라이스.
//
// 본 슬라이스는 *어댑터 trait + 1개 구현(Voyage)* + settings·keyring 통합 + 라우팅 stub
// 까지만. 실제 인덱싱 통합(`indexer.rs`의 fastembed 호출 분기)은 v0.4.4.1 또는 v0.4.5에서
// 박는다 — 차원 mismatch(voyage-3-lite=512d vs mE5-small=384d) 처리와 `vectors_byok`
// 별도 테이블 신설을 함께 다뤄야 하기 때문.
//
// 결정 (decision-log D-095 — 본 PR에서 락인):
//   * 1차 어댑터 = Voyage `voyage-3-lite` (한국어 양호, 비용 합리).
//   * 폴백(자리만, 본 PR엔 미구현) = Gemini Embedding `text-embedding-004` (사용자
//     Gemini 구독 활용 가능).
//   * API 키 = keyring 별도 entry (`voyage-api-key` / `gemini-embedding-api-key`).
//     기존 anthropic/openai/gemini 키와 분리 — provider 전환과 무관.
//   * 차원 mismatch 처리 = 본 PR엔 *어댑터 추상*만. 실제 적재 테이블 분기는 후속.
//
// 본 모듈은 *호출 0건* — `commands::byok` + `commands::dev_acceptance::dev_byok_routing_check`
// 가 진입점. dead_code 허용 (PR 5 이후 라우팅 통합 시 자연 해소).
#![allow(dead_code)]

pub mod byok_embedding;
