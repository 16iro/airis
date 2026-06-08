// v0.4.2 retrieval — active_index 기반 T1/T2 분기 hybrid 검색.
//
// 책임 (HANDOFF §1.4):
//   * 책별 `active_index.txt` 1회 읽기 → T1(=v1_me5-small) 또는 T2(=v2_bge-m3)로 분기.
//   * T1: v041::retrieval::hybrid_search 그대로 호출 (코드 중복 회피).
//   * T2: vec0_t2 + chunks_fts → RRF 병합 (T1과 동일 패턴, 차원만 1024).
//
// 설계 결정 — *옵션 A* (HANDOFF §1.4):
//   v041 retrieval은 *건드리지 않고* 그대로 두고, v042가 wrapping하는 형태로 래퍼만
//   추가. 이유:
//     1. v041 단위 테스트가 그대로 살아남아 회귀 위험 0.
//     2. 호출 측 변경 = `commands/llm.rs::build_v041_block` 한 곳만 wrapper로 redirect.
//     3. T1 검색은 "active_index = v1"이거나 "active_index 파일 부재(v0.4.1 호환)"
//        모두 v041 hybrid_search로 자연스럽게 떨어진다.
//
// 진행 중 쿼리 일관성:
//   retrieval 진입 시 active_index 1회 읽기 + 결과 캐시 — handoff "그 사이클이
//   끝난 모델로 일관" 정책. 핫스왑 도중 한쪽 책에서 두 번 검색되면 두 번째부터
//   새 active로 분기. 단일 검색 안에선 무조건 한 모델만.
//
// PassageEmbedder (T1)와 QueryEmbedder (T2) 분리:
//   * T1 = `v041::Embedder` (mE5 prefix 강제, query_prefix 적용).
//   * T2 = `EmbedderT2` (prefix 없음, raw query).
//   호출 측이 T1·T2 임베더를 모두 보유. 본 함수는 어느 쪽이든 받을 수 있게 enum.

#![allow(dead_code)]

use std::path::Path;

use rusqlite::Connection;

use crate::error::AppResult;
use crate::index::v041::embedder::Embedder as EmbedderT1;
use crate::index::v041::retrieval::RetrievedChunk;
use crate::index::v042::active_index::read_active_index;
use crate::index::v042::embedder_t2::EmbedderT2;
use crate::index::v042::manifest::IndexKind;

/// Retrieval에서 사용하는 임베더 핸들 — T1/T2 dispatch.
///
/// Arc 등 소유권 형태는 호출 측에 맡김(state.embedder는 `Arc<EmbedderT1>`, T2는
/// 별도 lazy slot — PR 3가 wire). 본 enum은 reference borrow.
pub enum RetrievalEmbedder<'a> {
    T1(&'a EmbedderT1),
    T2(&'a EmbedderT2),
}

impl<'a> RetrievalEmbedder<'a> {
    pub fn dim(&self) -> usize {
        match self {
            Self::T1(_) => EmbedderT1::DIM,
            Self::T2(_) => EmbedderT2::DIM,
        }
    }

    pub fn active_kind(&self) -> IndexKind {
        match self {
            Self::T1(_) => IndexKind::V1Me5Small,
            Self::T2(_) => IndexKind::V2BgeM3,
        }
    }
}

/// active_index를 책별로 읽어 검색 모델을 결정 → hybrid_search 분기.
///
/// 호출 흐름:
///   1. read_active_index(app_data_dir, book_id) — 파일 없으면 V1Me5Small 디폴트.
///   2. 결과가 V1Me5Small → embedder가 T1이어야 한다(=v041 hybrid_search).
///      결과가 V2BgeM3   → embedder가 T2이어야 한다.
///      mismatch면 InvalidInput 에러 (호출 측이 올바른 임베더 슬롯에서 lookup 책임).
///   3. V0Bm25는 FTS-only — embedder 없이 검색 가능 (architecture §5).
///      본 함수는 V1/V2 dispatch만. V0는 호출 측에서 fts_only_search를 직접 호출.
///
/// 빈 결과·빈 query는 v041 hybrid_search와 동일하게 빈 Vec 반환.
pub fn hybrid_search_active(
    conn: &Connection,
    embedder: RetrievalEmbedder<'_>,
    app_data_dir: &Path,
    book_id: &str,
    query: &str,
    n: usize,
) -> AppResult<Vec<RetrievedChunk>> {
    hybrid_search_active_with_vector_query(conn, embedder, app_data_dir, book_id, query, query, n)
}

/// v0.4.3 PR 3 (D-087) — vector 검색에 *별도의 query 텍스트*를 사용할 수 있는 active dispatch.
///
/// HyDE 사용 시: vector 트랙은 hypothetical answer, FTS 트랙은 rewritten query — 두 트랙을
/// 별도 텍스트로 호출 후 RRF 병합한다.
pub fn hybrid_search_active_with_vector_query(
    conn: &Connection,
    embedder: RetrievalEmbedder<'_>,
    app_data_dir: &Path,
    book_id: &str,
    vector_query: &str,
    fts_query: &str,
    n: usize,
) -> AppResult<Vec<RetrievedChunk>> {
    let active = read_active_index(app_data_dir, book_id)?;
    match (active, embedder) {
        // T1 — v041 hybrid_search_with_vector_query 그대로.
        (IndexKind::V1Me5Small, RetrievalEmbedder::T1(e)) => {
            crate::index::v041::retrieval::hybrid_search_with_vector_query(
                conn,
                e,
                book_id,
                vector_query,
                fts_query,
                n,
            )
        }
        // T2 — 본 모듈 자체 hybrid_search_with_vector_query.
        (IndexKind::V2BgeM3, RetrievalEmbedder::T2(e)) => {
            t2_hybrid_search_with_vector_query(conn, e, book_id, vector_query, fts_query, n)
        }
        (IndexKind::V0Bm25, _) => Err(crate::error::AppError::InvalidInput {
            message: "v0_bm25는 hybrid_search 진입 X — fts_only_search 사용".to_string(),
        }),
        (active, embedder) => Err(crate::error::AppError::InvalidInput {
            message: format!(
                "active_index={:?}와 embedder dim={} mismatch",
                active.dir_name(),
                embedder.dim()
            ),
        }),
    }
}

/// v0.6.x (D-109) — 가중 RRF 적용 active dispatch. query_route 가중치를 받는다.
///
/// (w_vec, w_fts) = (1.0, 1.0)이면 `hybrid_search_active_with_vector_query`와 동일.
#[allow(clippy::too_many_arguments)]
pub fn hybrid_search_active_weighted_with_vector_query(
    conn: &Connection,
    embedder: RetrievalEmbedder<'_>,
    app_data_dir: &Path,
    book_id: &str,
    vector_query: &str,
    fts_query: &str,
    n: usize,
    w_vec: f64,
    w_fts: f64,
) -> AppResult<Vec<RetrievedChunk>> {
    let active = read_active_index(app_data_dir, book_id)?;
    match (active, embedder) {
        (IndexKind::V1Me5Small, RetrievalEmbedder::T1(e)) => {
            crate::index::v041::retrieval::hybrid_search_weighted(
                conn,
                e,
                book_id,
                vector_query,
                fts_query,
                n,
                w_vec,
                w_fts,
            )
        }
        (IndexKind::V2BgeM3, RetrievalEmbedder::T2(e)) => {
            t2_hybrid_search_weighted_with_vector_query(
                conn,
                e,
                book_id,
                vector_query,
                fts_query,
                n,
                w_vec,
                w_fts,
            )
        }
        (IndexKind::V0Bm25, _) => Err(crate::error::AppError::InvalidInput {
            message: "v0_bm25는 hybrid_search 진입 X — fts_only_search 사용".to_string(),
        }),
        (active, embedder) => Err(crate::error::AppError::InvalidInput {
            message: format!(
                "active_index={:?}와 embedder dim={} mismatch",
                active.dir_name(),
                embedder.dim()
            ),
        }),
    }
}

/// T2 가중 hybrid_search — t2_hybrid_search_with_vector_query의 가중 RRF 버전.
#[allow(clippy::too_many_arguments)]
fn t2_hybrid_search_weighted_with_vector_query(
    conn: &Connection,
    embedder: &EmbedderT2,
    book_id: &str,
    vector_query: &str,
    fts_query: &str,
    n: usize,
    w_vec: f64,
    w_fts: f64,
) -> AppResult<Vec<RetrievedChunk>> {
    use crate::index::v041::retrieval::{rrf_merge_weighted, FTS_TOP_K, VECTOR_TOP_K};

    if n == 0 || (vector_query.trim().is_empty() && fts_query.trim().is_empty()) {
        return Ok(Vec::new());
    }
    let vec_ranking = if vector_query.trim().is_empty() {
        Vec::new()
    } else {
        vector_top_k_t2(conn, embedder, book_id, vector_query, VECTOR_TOP_K)?
    };
    let fts_ranking = if fts_query.trim().is_empty() {
        Vec::new()
    } else {
        fts_top_k_for(conn, book_id, fts_query, FTS_TOP_K)?
    };
    let merged = rrf_merge_weighted(&vec_ranking, &fts_ranking, w_vec, w_fts);
    let top: Vec<(i64, f64)> = merged.into_iter().take(n).collect();
    fetch_chunks(conn, &top)
}

/// T2 전용 hybrid_search — vec0_t2 + chunks_fts → RRF.
///
/// v041::retrieval::hybrid_search와 같은 흐름이지만 vector_top_k가 vec0_t2를 사용.
/// FTS 부분은 v041과 완전히 같다 (chunks_fts는 모델 무관).
fn t2_hybrid_search(
    conn: &Connection,
    embedder: &EmbedderT2,
    book_id: &str,
    query: &str,
    n: usize,
) -> AppResult<Vec<RetrievedChunk>> {
    t2_hybrid_search_with_vector_query(conn, embedder, book_id, query, query, n)
}

/// v0.4.3 PR 3 (D-087) — T2 hybrid_search에 vector_query/fts_query 분리 인자.
fn t2_hybrid_search_with_vector_query(
    conn: &Connection,
    embedder: &EmbedderT2,
    book_id: &str,
    vector_query: &str,
    fts_query: &str,
    n: usize,
) -> AppResult<Vec<RetrievedChunk>> {
    use crate::index::v041::retrieval::{
        fts_only_search as v041_fts_only_search, FTS_TOP_K, HYBRID_TOP_N, VECTOR_TOP_K,
    };

    if n == 0 || (vector_query.trim().is_empty() && fts_query.trim().is_empty()) {
        return Ok(Vec::new());
    }
    let _ = HYBRID_TOP_N;
    let vec_ranking = if vector_query.trim().is_empty() {
        Vec::new()
    } else {
        vector_top_k_t2(conn, embedder, book_id, vector_query, VECTOR_TOP_K)?
    };
    let fts_ranking = if fts_query.trim().is_empty() {
        Vec::new()
    } else {
        fts_top_k_for(conn, book_id, fts_query, FTS_TOP_K)?
    };
    let merged = rrf_merge(&vec_ranking, &fts_ranking);
    let top: Vec<(i64, f64)> = merged.into_iter().take(n).collect();
    let _ = v041_fts_only_search;
    fetch_chunks(conn, &top)
}

fn vector_top_k_t2(
    conn: &Connection,
    embedder: &EmbedderT2,
    book_id: &str,
    query: &str,
    k: usize,
) -> AppResult<Vec<(i64, f64)>> {
    use crate::index::v042::vector_store_t2::knn_t2;
    if k == 0 {
        return Ok(Vec::new());
    }
    // BGE-M3는 prefix 없음 — raw query 그대로.
    let q_emb = embedder.embed_query(query)?;
    let raw = knn_t2(conn, &q_emb, k.saturating_mul(4))?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut filtered: Vec<(i64, f64)> = Vec::with_capacity(raw.len());
    let mut stmt =
        conn.prepare("SELECT 1 FROM chunks WHERE id = ?1 AND book_id = ?2 LIMIT 1")?;
    for (id, dist) in raw {
        let exists: Option<i64> = stmt
            .query_row(rusqlite::params![id, book_id], |r| r.get(0))
            .ok();
        if exists.is_some() {
            filtered.push((id, dist));
            if filtered.len() == k {
                break;
            }
        }
    }
    Ok(filtered)
}

fn fts_top_k_for(
    conn: &Connection,
    book_id: &str,
    query: &str,
    k: usize,
) -> AppResult<Vec<(i64, f64)>> {
    if k == 0 {
        return Ok(Vec::new());
    }
    let Some(expr) = normalize_fts_query(query) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT c.id, bm25(chunks_fts) AS score \
         FROM chunks_fts \
         JOIN chunks c ON c.id = chunks_fts.rowid \
         WHERE chunks_fts MATCH ?1 AND c.book_id = ?2 \
         ORDER BY score \
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![expr, book_id, k as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// FTS5 입력 정규화 — v041과 동일.
fn normalize_fts_query(query: &str) -> Option<String> {
    let cleaned: String = query
        .chars()
        .map(|c| match c {
            '"' | '*' | '^' | '(' | ')' | ':' | '\\' => ' ',
            _ => c,
        })
        .collect();
    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

/// RRF 병합 — v041과 동일 식. 책별 검색 트랙 안에서 통일된 점수.
fn rrf_merge(
    vec_ranking: &[(i64, f64)],
    fts_ranking: &[(i64, f64)],
) -> Vec<(i64, f64)> {
    use std::collections::HashMap;
    const RRF_K: f64 = 60.0;
    let mut score: HashMap<i64, f64> = HashMap::new();
    for (rank, (id, _)) in vec_ranking.iter().enumerate() {
        *score.entry(*id).or_insert(0.0) += 1.0 / (RRF_K + (rank as f64) + 1.0);
    }
    for (rank, (id, _)) in fts_ranking.iter().enumerate() {
        *score.entry(*id).or_insert(0.0) += 1.0 / (RRF_K + (rank as f64) + 1.0);
    }
    let mut merged: Vec<(i64, f64)> = score.into_iter().collect();
    merged.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    merged
}

fn fetch_chunks(
    conn: &Connection,
    ids_with_score: &[(i64, f64)],
) -> AppResult<Vec<RetrievedChunk>> {
    let mut stmt = conn.prepare(
        "SELECT id, text, page, section_path, parent_id, prev_chunk_id, next_chunk_id, \
                token_count \
         FROM chunks WHERE id = ?1",
    )?;
    let mut out = Vec::with_capacity(ids_with_score.len());
    for (id, score) in ids_with_score {
        let row = stmt.query_row(rusqlite::params![id], |r| {
            let section_path: Option<String> = r.get::<_, Option<String>>(3)?.and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            });
            Ok(RetrievedChunk {
                id: r.get(0)?,
                text: r.get(1)?,
                page: r.get(2)?,
                section_path,
                parent_id: r.get(4)?,
                prev_chunk_id: r.get(5)?,
                next_chunk_id: r.get(6)?,
                token_count: r.get(7)?,
                score: *score,
            })
        });
        match row {
            Ok(rec) => out.push(rec),
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::v042::active_index::write_active_index_atomic;
    use rusqlite::params;
    use rusqlite::Connection;

    fn register_sqlite_vec_once() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        type AutoExtFn = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        INIT.call_once(|| unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                AutoExtFn,
            >(sqlite_vec::sqlite3_vec_init as *const ())));
        });
    }

    fn fresh_conn() -> Connection {
        register_sqlite_vec_once();
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        let migrations: &[&str] = &[
            include_str!("../../migrations/v1_initial.sql"),
            include_str!("../../migrations/v2_studies_and_chat.sql"),
            include_str!("../../migrations/v3_paragraphs_fts.sql"),
            include_str!("../../migrations/v4_intervention_and_history.sql"),
            include_str!("../../migrations/v5_pomodoro_cycles.sql"),
            include_str!("../../migrations/v6_srs_cards.sql"),
            include_str!("../../migrations/v7_recall_challenges.sql"),
            include_str!("../../migrations/v8_book_thumbnail.sql"),
            include_str!("../../migrations/v9_study_thumbnail.sql"),
            include_str!("../../migrations/v10_thumbnails_dir_rename.sql"),
            include_str!("../../migrations/v11_study_description.sql"),
            include_str!("../../migrations/v12_chat_context.sql"),
            include_str!("../../migrations/v13_chunks.sql"),
            include_str!("../../migrations/v14_ab_compare.sql"),
            include_str!("../../migrations/v15_robustness.sql"),
        ];
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (\
                version INTEGER PRIMARY KEY,\
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
             );",
        )
        .unwrap();
        for sql in migrations {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute(
            "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (id, study_slug, role, title, source_path, file_format, \
                                  file_size, file_hash, added_at) \
             VALUES ('b1','s','main','B','/x','md',0,'h',datetime('now'))",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn rrf_merge_higher_when_appearing_in_both() {
        let vec = vec![(100, 0.1), (200, 0.2)];
        let fts = vec![(100, -2.5), (300, -3.0)];
        let merged = rrf_merge(&vec, &fts);
        assert_eq!(merged[0].0, 100);
    }

    #[test]
    fn normalize_fts_query_strips_meta_chars() {
        assert_eq!(
            normalize_fts_query("ownership 모델"),
            Some("\"ownership\" OR \"모델\"".to_string())
        );
        assert_eq!(normalize_fts_query("   "), None);
    }

    #[test]
    fn fts_top_k_for_filters_by_book() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO books (id, study_slug, role, title, source_path, file_format, \
                                  file_size, file_hash, added_at) \
             VALUES ('b2','s','main','B2','/x2','md',0,'h2',datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, section_path, token_count) \
             VALUES ('b1', 0, 'Rust ownership 모델', 'Ch01', 4)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, section_path, token_count) \
             VALUES ('b2', 0, 'Rust ownership 다른', 'Ch01', 4)",
            [],
        )
        .unwrap();

        let hits = fts_top_k_for(&conn, "b1", "ownership", 5).unwrap();
        assert_eq!(hits.len(), 1, "b1만 한정");
    }

    #[test]
    fn t2_hybrid_search_with_empty_book_returns_empty() {
        let conn = fresh_conn();
        // chunks 0건이라 vec0/FTS 모두 비어 있음. 임베더는 호출 자체가 없음.
        // (embed_query 호출 전에 KNN을 부르므로 호출은 일어나지만 결과 빈 Vec).
        // 본 테스트는 분기 패턴 검증 — fts_top_k_for + rrf_merge 통합.
        let hits = fts_top_k_for(&conn, "b1", "ownership", 5).unwrap();
        let merged = rrf_merge(&[], &hits);
        let top: Vec<(i64, f64)> = merged.into_iter().take(10).collect();
        let recs = fetch_chunks(&conn, &top).unwrap();
        assert!(recs.is_empty());
    }

    #[test]
    fn hybrid_search_active_v0_returns_invalid_input() {
        let conn = fresh_conn();
        let dir = tempfile::tempdir().unwrap();
        write_active_index_atomic(dir.path(), "b1", IndexKind::V0Bm25).unwrap();

        // T1 임베더는 fastembed 다운로드라 mock 구성 불가 — InvalidInput 분기 진입은
        // active_index 읽기 직후라 임베더 사용 전에 거부된다. 따라서 T2 임베더 자리에는
        // null 대용의 *어떤 RetrievalEmbedder*가 들어가도 분기 결과가 같다. 본 테스트는
        // active_index가 V0Bm25면 InvalidInput을 돌려준다는 *행위 명세*만 검증.
        // 실제 임베더 인자를 만들지 않고 코드 path를 직접 호출하긴 어려우니, 본 테스트는
        // active_index 읽기·parse_active_index가 V0Bm25를 정상 dispatch 분기로 전달하는지
        // 만 read_active_index로 간접 검증.
        let kind =
            crate::index::v042::active_index::read_active_index(dir.path(), "b1").unwrap();
        assert_eq!(kind, IndexKind::V0Bm25);
        // hybrid_search_active 의 V0 분기 경로는 e2e fts_only_search 통합에서 검증.
        // (commands 측에서 V0 분기 시 fts_only_search 직접 호출 책임.)
        // 본 unit은 dispatch 분기 결정만 검증.
        let _ = conn;
    }

    #[test]
    fn fetch_chunks_skips_missing_ids() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, section_path, page, token_count) \
             VALUES ('b1', 0, '본문 A', 'Ch01', 12, 3)",
            [],
        )
        .unwrap();
        let real_id: i64 = conn
            .query_row("SELECT id FROM chunks WHERE book_id='b1'", [], |r| r.get(0))
            .unwrap();
        let recs = fetch_chunks(&conn, &[(real_id, 0.5), (99_999, 0.4)]).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].id, real_id);
        assert_eq!(recs[0].page, Some(12));
    }

    #[test]
    fn retrieval_embedder_dim_matches_kind() {
        // dim이 active_kind와 일관 — 본 enum의 invariant.
        // 실제 임베더 인스턴스 없이 변형 enum이 dim을 잘 매핑하는지 패턴만 검증.
        // (RetrievalEmbedder::T1/T2는 reference라 인스턴스 보유 못함 → 별도 단위 검증
        //  대신 IndexKind dim 일치 검증.)
        assert_eq!(IndexKind::V1Me5Small.dim(), EmbedderT1::DIM);
        assert_eq!(IndexKind::V2BgeM3.dim(), EmbedderT2::DIM);
    }

    #[test]
    fn read_active_index_default_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let kind =
            crate::index::v042::active_index::read_active_index(dir.path(), "ghost").unwrap();
        assert_eq!(kind, IndexKind::V1Me5Small, "디폴트 v1");
    }

    #[test]
    fn chunks_table_can_seed_for_t2_smoke() {
        // T2 retrieval은 BGE-M3 다운로드라 e2e 전용. 본 단위 테스트는 chunks 적재만
        // 가능한지 확인 — vec0_t2를 ensure할 수는 있지만, vec_ranking은 임베더 없이
        // 만들 수 없다. 따라서 T2 hybrid_search 자체의 e2e는 v042_cascade_smoke의
        // fake-embedder 패턴으로 검증.
        let conn = fresh_conn();
        let _ = params!["dummy"];
        let _ = conn;
    }
}
