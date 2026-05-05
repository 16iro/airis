// v0.4.1 인덱서 — RAG 엔진 (chunks + vectors_t1 + chunks_fts + indexing_jobs).
//
// PR 1 범위 (이 PR): DB v13 마이그레이션 + 모듈 골격만. 실제 인덱싱 로직은 PR 2~5에서
// 채운다. 현재 stub은 컴파일 + 단위 테스트(prefix helper만)만 통과시키는 게 목표.
//
// 호출 측:
//   * PR 2 — chunker (D-078~D-080) — chunks INSERT (parent/prev/next 채움)
//   * PR 3 — Hybrid retrieval — vec0 + chunks_fts → RRF
//   * PR 4 — ChatContextChip 점프 + reindex UX
//   * PR 5 — A/B dev panel
//
// 기존 v0.3.2 `chunker`/`keyword`(paragraphs FTS)는 *그대로 남는다* — 책별 chunks 적재
// 여부에 따라 폴백 (handoff §5).
//
// dead_code 허용: PR 1은 *호출 0건*. PR 2~5에서 호출 들어오면 자연 해소.
#![allow(dead_code)]

pub mod chunker;
pub mod embedder;
pub mod indexer;
pub mod vector_store;

/// f32 슬라이스를 little-endian byte slice로 변환 (sqlite-vec 입력 형식).
///
/// sqlite-vec는 vec0 가상 테이블에 BLOB을 넣을 때 *little-endian f32* 만 받는다.
/// x86_64 / aarch64는 모두 little-endian이라 안전. 빅엔디안 타겟이 들어오면
/// 별도 변환이 필요하지만 airis 지원 OS는 모두 little-endian.
///
/// PoC `experiments/v040-poc/src/bin/d3_sqlite_vec.rs::f32_bytes`에서 그대로 이식.
pub fn f32_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32는 4바이트 POD. 메모리 레이아웃은 IEEE-754 single-precision으로 고정.
    // 결과 슬라이스는 입력과 같은 lifetime을 공유 (시그니처가 강제).
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_bytes_layout() {
        // f32 4바이트 × N개 = 4N 바이트. little-endian 호스트(테스트는 x86_64)에서
        // 1.0_f32는 [0x00, 0x00, 0x80, 0x3F]로 인코딩.
        let v = [1.0_f32];
        let bytes = f32_bytes(&v);
        assert_eq!(bytes.len(), 4);
        assert_eq!(bytes, &[0x00, 0x00, 0x80, 0x3F]);
    }

    #[test]
    fn f32_bytes_round_trip_via_pointer_cast() {
        // PoC와 동일 — bytes 슬라이스를 *const f32로 다시 읽으면 원본과 같은 비트 패턴.
        let v: Vec<f32> = (0..16).map(|i| i as f32 * 0.5).collect();
        let bytes = f32_bytes(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        // 첫 8바이트 = 0.0, 0.5 (little-endian)
        let zero: [u8; 4] = [0, 0, 0, 0];
        let half: [u8; 4] = [0, 0, 0, 0x3F];
        assert_eq!(&bytes[0..4], &zero);
        assert_eq!(&bytes[4..8], &half);
    }
}
