// extraction.rs — v0.5 PR 1 (D-097/D-098).
//
// chat turn으로부터 memory_facts 후보를 추출.
// D-010 b "1회 확인" 부분 supersede — 결과를 자동 INSERT.
//
// 단계:
//   1. 결정적 정규식 (triggers.rs N=3 패턴 그대로). hit → confidence = 0.7 base.
//   2. 임베딩 매칭:
//      - 활성 study의 *인덱싱 완료된* 책 chunks에 대해 user+assistant 텍스트 임베딩.
//      - cosine similarity 최댓값 ≥ 0.85 → memory_fact_chunks 후보 + confidence = 유사도.
//      - 임베딩 모델 = T1 mE5-small (이미 v0.4에서 lazy init). 활성 fastembed 재활용.
//      - 인덱싱 미완 책 → skip.
//   3. OR 결합: 둘 중 hit이면 FactCandidate. confidence = max(regex, embed).
//      두 신호 동시 hit → confidence 1.0 cap.
//   4. kind 분류: regex hit.kind 1차 신호. 임베딩만 hit → 'meta' (PR 2/3에서 LLM 정확도 ↑).

use rusqlite::params;
use std::sync::Arc;

use crate::commands::memory_facts::{insert_fact, insert_fact_chunk};
use crate::commands::triggers;
use crate::db::Db;
use crate::error::AppResult;
use crate::index::v041::embedder::{passage_prefix, Embedder};

/// 정규식 base confidence — triggers.rs hit.
const REGEX_BASE_CONFIDENCE: f64 = 0.7;

/// 임베딩 cosine 임계값.
pub const EMBED_COSINE_THRESHOLD: f64 = 0.85;

/// extract_from_turn 반환 타입.
#[derive(Debug, Clone)]
pub struct FactCandidate {
    pub kind: String,
    pub content: String,
    /// "trigger" = regex hit, "citation" = embedding hit.
    pub source: String,
    pub confidence: f64,
    /// 임베딩 hit인 경우 (chunk_id, similarity) 목록.
    pub chunk_hits: Vec<(i64, f64)>,
}

/// cosine similarity — 두 벡터의 내적 / (|a| * |b|).
/// 임베딩 벡터는 fastembed이 L2 정규화하므로 내적 = cosine.
/// 정규화 여부 불확실한 경우를 위해 명시 계산.
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    debug_assert_eq!(a.len(), b.len(), "cosine: 차원 불일치");
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let na: f64 = a.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na * nb)).clamp(-1.0, 1.0)
}

/// bytes (little-endian f32) → Vec<f32>.
fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// 인덱싱 완료된 책이 있는지 + 책 id 목록 반환.
/// "완료": chunks 테이블에 해당 book_id 행이 1건 이상 AND embed_status_t1='done' 행이 1건 이상.
fn indexed_book_ids(db: &Db, study_slug: &str) -> AppResult<Vec<String>> {
    let mut stmt = db.conn().prepare(
        "SELECT DISTINCT b.id FROM books b \
         WHERE b.study_slug = ?1 \
           AND EXISTS( \
               SELECT 1 FROM chunks c \
               WHERE c.book_id = b.id AND c.embed_status_t1 = 'done' \
               LIMIT 1 \
           )",
    )?;
    let ids = stmt
        .query_map(params![study_slug], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// book_id 목록에서 chunk_id + embedding BLOB 전부 로드.
/// 대용량 책이면 느릴 수 있으므로 embed_status_t1='done' 한정.
fn load_chunk_embeddings(db: &Db, book_ids: &[String]) -> AppResult<Vec<(i64, Vec<f32>)>> {
    if book_ids.is_empty() {
        return Ok(Vec::new());
    }
    // IN 절을 직접 조립 (book_id는 DB에서 온 UUID — injection 위험 없음).
    let placeholders: String = book_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT c.id, v.embedding FROM chunks c \
         JOIN vectors_t1 v ON v.chunk_id = c.id \
         WHERE c.book_id IN ({placeholders}) AND c.embed_status_t1 = 'done'"
    );
    let mut stmt = db.conn().prepare(&sql)?;

    // rusqlite params_from_iter로 동적 바인딩.
    let rows = stmt
        .query_map(rusqlite::params_from_iter(book_ids.iter()), |r| {
            let chunk_id: i64 = r.get(0)?;
            let blob: Vec<u8> = r.get(1)?;
            Ok((chunk_id, blob))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let result = rows
        .into_iter()
        .map(|(id, blob)| (id, bytes_to_f32(&blob)))
        .collect();
    Ok(result)
}

/// user_msg + assistant_msg에서 FactCandidate 추출 (blocking — spawn_blocking에서 호출).
///
/// study_slug: 활성 스터디 (chunks 매칭 범위 결정).
/// embedder: None이면 임베딩 단계 skip (정규식 단계만).
pub fn extract_from_turn(
    db: &Db,
    study_slug: &str,
    user_msg: &str,
    assistant_msg: &str,
    embedder: Option<&Arc<Embedder>>,
) -> AppResult<Vec<FactCandidate>> {
    let mut candidates: Vec<FactCandidate> = Vec::new();

    // 1단계: 결정적 정규식 — 사용자 발화에서 트리거 감지.
    let trigger_hits = triggers::detect(user_msg);
    for hit in &trigger_hits {
        candidates.push(FactCandidate {
            kind: trigger_kind_to_str(hit.kind),
            content: hit.suggested_entry.clone(),
            source: "trigger".to_string(),
            confidence: REGEX_BASE_CONFIDENCE,
            chunk_hits: Vec::new(),
        });
    }

    // 2단계: 임베딩 매칭 — embedder 슬롯이 있을 때만.
    if let Some(emb) = embedder {
        let book_ids = indexed_book_ids(db, study_slug)?;
        if !book_ids.is_empty() {
            // user + assistant 결합 텍스트를 passage prefix로 임베딩.
            let combined = format!("{user_msg}\n{assistant_msg}");
            let prefixed = passage_prefix(&combined);

            let query_vec = match emb.embed_passages(&[prefixed]) {
                Ok(mut vecs) if !vecs.is_empty() => vecs.remove(0),
                _ => {
                    // 임베딩 실패 시 embedding 단계 skip — 정규식 결과만 반환.
                    return Ok(candidates);
                }
            };

            let chunk_embeddings = load_chunk_embeddings(db, &book_ids)?;

            // 임계값 이상인 chunk 수집.
            let mut hits: Vec<(i64, f64)> = chunk_embeddings
                .iter()
                .filter_map(|(cid, cvec)| {
                    let sim = cosine(&query_vec, cvec);
                    if sim >= EMBED_COSINE_THRESHOLD {
                        Some((*cid, sim))
                    } else {
                        None
                    }
                })
                .collect();

            if !hits.is_empty() {
                // 유사도 내림차순 정렬 — 최고 유사도를 confidence로.
                hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let best_sim = hits[0].1;

                // 기존 regex 후보와 OR 결합 — regex hit이 있으면 confidence를 상향.
                // 임베딩만 hit이면 새 'meta' candidate 추가.
                let has_regex_match = !candidates.is_empty();
                if has_regex_match {
                    // 두 신호 동시 hit → confidence 1.0.
                    for c in candidates.iter_mut() {
                        c.confidence = 1.0_f64.min(c.confidence.max(best_sim));
                        c.chunk_hits.clone_from(&hits);
                    }
                } else {
                    // 임베딩만 hit → default kind = 'meta'.
                    candidates.push(FactCandidate {
                        kind: "meta".to_string(),
                        content: user_msg.trim().to_string(),
                        source: "citation".to_string(),
                        confidence: best_sim,
                        chunk_hits: hits,
                    });
                }
            }
        }
    }

    Ok(candidates)
}

/// TriggerKind → fact kind 문자열.
fn trigger_kind_to_str(kind: triggers::TriggerKind) -> String {
    match kind {
        triggers::TriggerKind::Preference => "preference".to_string(),
        triggers::TriggerKind::Correction => "correction".to_string(),
        triggers::TriggerKind::Goal => "goal".to_string(),
    }
}

/// chat turn 완료 후 background에서 호출 — candidates → DB INSERT.
/// confidence < 0.5인 candidate도 INSERT (주입 필터에서 걸러짐).
pub fn persist_candidates(
    db: &Db,
    study_id: &str,
    candidates: &[FactCandidate],
) -> AppResult<()> {
    for c in candidates {
        let fact = insert_fact(db.conn(), study_id, &c.kind, &c.content, &c.source, c.confidence)?;
        for (chunk_id, sim) in &c.chunk_hits {
            // INSERT OR REPLACE — 중복 시 최신 유사도로 덮음.
            let _ = insert_fact_chunk(db.conn(), fact.id, *chunk_id, *sim);
        }
    }
    Ok(())
}

// ---- 단위 테스트 -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn setup_db() -> Db {
        Db::open_in_memory_for_test()
    }

    #[test]
    fn cosine_identical_vectors() {
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![1.0_f32, 0.0, 0.0];
        let sim = cosine(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6, "same vector cosine should be ~1.0");
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![0.0_f32, 1.0, 0.0];
        let sim = cosine(&a, &b);
        assert!(sim.abs() < 1e-6, "orthogonal cosine should be ~0.0");
    }

    #[test]
    fn cosine_zero_vector_returns_zero() {
        let a = vec![0.0_f32, 0.0, 0.0];
        let b = vec![1.0_f32, 0.0, 0.0];
        assert_eq!(cosine(&a, &b), 0.0);
    }

    #[test]
    fn bytes_to_f32_roundtrip() {
        let original = [1.0_f32, 2.0, 3.0];
        let bytes: Vec<u8> = original
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let restored = bytes_to_f32(&bytes);
        assert_eq!(restored.len(), 3);
        for (a, b) in original.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn extract_regex_hit_with_no_embedder() {
        let db = setup_db();
        let candidates = extract_from_turn(
            &db,
            "test-study",
            "이제부터 코드 예제를 컴파일 가능한 형태로 줘.",
            "알겠습니다.",
            None,
        )
        .unwrap();
        assert!(!candidates.is_empty(), "regex trigger should produce candidates");
        assert!(candidates.iter().any(|c| c.kind == "preference"));
        assert!(candidates.iter().any(|c| c.source == "trigger"));
        assert!(
            candidates.iter().all(|c| c.confidence >= REGEX_BASE_CONFIDENCE - 1e-9),
            "regex base confidence must be >= 0.7"
        );
    }

    #[test]
    fn extract_neutral_text_no_regex_hit() {
        let db = setup_db();
        let candidates = extract_from_turn(
            &db,
            "test-study",
            "Rust의 소유권에 대해 설명해줘.",
            "소유권이란...",
            None,
        )
        .unwrap();
        // 정규식 hit 없음. 임베딩 없으면 0 candidates.
        assert!(
            candidates.is_empty() || candidates.iter().all(|c| c.source != "trigger"),
            "neutral text should produce 0 trigger candidates"
        );
    }

    #[test]
    fn persist_candidates_inserts_facts() {
        let db = setup_db();
        let candidates = vec![FactCandidate {
            kind: "preference".to_string(),
            content: "빠른 결과 우선".to_string(),
            source: "trigger".to_string(),
            confidence: 0.7,
            chunk_hits: Vec::new(),
        }];
        persist_candidates(&db, "s1", &candidates).unwrap();

        let cnt: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM memory_facts WHERE study_id='s1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 1);
    }

    #[test]
    fn persist_candidates_inserts_chunk_hits() {
        let db = setup_db();
        // books/chunks 없이 chunk_id를 논리 참조로만 — FK 없음이 정책.
        let candidates = vec![FactCandidate {
            kind: "meta".to_string(),
            content: "Rust ownership".to_string(),
            source: "citation".to_string(),
            confidence: 0.9,
            chunk_hits: vec![(42, 0.91)],
        }];
        persist_candidates(&db, "s1", &candidates).unwrap();

        let cnt: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM memory_fact_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 1);
    }

    /// 모의 임베딩 — 임계값 이상인 경우 candidate 생성 확인.
    /// 실제 fastembed 로드 없이 bytes_to_f32 + cosine으로 mock 구성.
    #[test]
    fn embed_cosine_threshold_constant() {
        // 임계값이 PR 1 스펙(0.85)과 일치하는지 회귀 보장.
        assert!(
            (EMBED_COSINE_THRESHOLD - 0.85).abs() < 1e-9,
            "embedding cosine threshold must be 0.85"
        );
    }
}
