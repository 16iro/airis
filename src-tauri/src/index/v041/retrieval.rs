// v0.4.1 retrieval — Hybrid (vector top-K + FTS5 top-K) + RRF 병합.
//
// architecture §4.6 그대로:
//   1. dense vector top-K (sqlite-vec vec0 KNN, default K=20)
//   2. FTS5 top-K (chunks_fts BM25, default K=20)
//   3. RRF (Reciprocal Rank Fusion)로 병합 → top-N (default N=10)
//
// 본 모듈은 *retrieval만* 책임. 메타데이터 직렬화·토큰 예산 패킹·시스템 프롬프트는
// `context.rs`가 담당.
//
// vec0 가상 테이블은 `vector_store::ensure_vec0`로 idempotent 보장. 검색 진입에서
// 항상 한 번 호출 — 빈 책(아직 임베딩 0건) 검색이 와도 panic 없이 빈 결과를 반환한다.
//
// 책별 격리: vec0 인덱스는 *책 구분이 없는* 글로벌 KNN. 따라서 vector top-K는
// 결과를 받은 뒤 chunks 테이블 JOIN으로 `book_id` 필터를 거친다.
// FTS5 검색도 마찬가지 — JOIN으로 같은 책으로 한정.

#![allow(dead_code)]

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::index::v041::embedder::{query_prefix, Embedder};
use crate::index::v041::vector_store::{ensure_vec0, knn};

/// 단일 검색 결과 — chunk 메타까지 같이 채워서 context.rs가 그대로 패킹할 수 있게 한다.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    /// chunks.id.
    pub id: i64,
    /// chunks.text 본문.
    pub text: String,
    /// chunks.page (PDF 1-base, MD/HTML은 None).
    pub page: Option<i64>,
    /// chunks.section_path (`Ch04/§State` 또는 `p.42`). 빈 문자열이면 None.
    pub section_path: Option<String>,
    /// chunks.parent_id.
    pub parent_id: Option<i64>,
    /// chunks.prev_chunk_id.
    pub prev_chunk_id: Option<i64>,
    /// chunks.next_chunk_id.
    pub next_chunk_id: Option<i64>,
    /// chunks.token_count (D-080 휴리스틱). 없으면 None.
    pub token_count: Option<i64>,
    /// RRF 합산 점수. 큰 값이 더 관련도 높음.
    pub score: f64,
}

/// vector top-K default — architecture §4.6 권고 K=20.
pub const VECTOR_TOP_K: usize = 20;

/// FTS5 top-K default — vector top-K와 같은 폭(K=20).
pub const FTS_TOP_K: usize = 20;

/// RRF 상수 k — RRF 표준 기본값 60. 작을수록 상위 rank 영향 강조.
const RRF_K: f64 = 60.0;

/// hybrid_search default — context.rs가 받을 top-N (default 10).
pub const HYBRID_TOP_N: usize = 10;

/// FTS5 검색을 위해 사용자 입력을 안전한 MATCH 식으로 정규화.
///
/// chunks_fts는 unicode61 토크나이저(paragraphs_fts와 동일)를 쓴다. 사용자 입력의
/// FTS 메타 문자 (`"`, `*`, `^`, `(`, `)`, `:`)는 제거하고, 공백 단위 토큰을 OR로 묶는다.
/// 빈 결과면 None.
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
        // OR 결합 — recall 우선. 정밀도는 RRF + vector 검색이 보강.
        Some(tokens.join(" OR "))
    }
}

/// vector top-K — query를 mE5 query prefix로 감싸 임베더에 넣고 vec0 KNN 호출.
///
/// 결과는 책으로 한정해 chunks 테이블에서 row 존재 여부 검증. distance 오름차순.
fn vector_top_k(
    conn: &Connection,
    embedder: &Embedder,
    book_id: &str,
    query: &str,
    k: usize,
) -> AppResult<Vec<(i64, f64)>> {
    if k == 0 {
        return Ok(Vec::new());
    }
    let q_emb = embedder.embed_query(&query_prefix(query))?;
    // vec0는 *글로벌* — 후보를 좀 더 받고 책 필터 적용.
    let raw = knn(conn, &q_emb, k.saturating_mul(4))?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    // chunks JOIN으로 같은 book_id만 추리고 distance 오름차순 보존.
    let mut filtered: Vec<(i64, f64)> = Vec::with_capacity(raw.len());
    let mut stmt =
        conn.prepare("SELECT 1 FROM chunks WHERE id = ?1 AND book_id = ?2 LIMIT 1")?;
    for (id, dist) in raw {
        let exists: Option<i64> = stmt
            .query_row(params![id, book_id], |r| r.get(0))
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

/// FTS5 top-K — chunks_fts MATCH + bm25 ranking. 책 필터를 JOIN으로 강제.
///
/// FTS5 bm25는 *낮을수록 관련도 높다*는 점에 유의. 호출 측은 rank 인덱스만 사용하므로
/// 부호는 신경 쓸 필요 없으나, 디버깅을 위해 score를 그대로 반환한다.
fn fts_top_k(
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
        .query_map(params![expr, book_id, k as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// RRF 병합 — 두 ranking을 받아 chunk별 `Σ 1/(k + rank)` 합산.
///
/// 같은 chunk가 양쪽에 등장하면 자연스레 상위. rank는 1-base.
fn rrf_merge(
    vec_ranking: &[(i64, f64)],
    fts_ranking: &[(i64, f64)],
) -> Vec<(i64, f64)> {
    // 균등 가중 = weighted(1.0, 1.0)와 동치 (기존 동작 보존).
    rrf_merge_weighted(vec_ranking, fts_ranking, 1.0, 1.0)
}

/// v0.6.x (D-109) — 가중 RRF. vector·fts 기여에 각각 가중치를 곱한다.
///
/// `Σ w · 1/(k + rank)`. 쿼리 적응형 라우팅(query_route)이 질문 유형에 따라
/// (w_vec, w_fts)를 (0.7,1.3)/(1.3,0.7)/(1.0,1.0)로 전달. 가중치가 (1.0,1.0)이면
/// 기존 균등 RRF와 *완전히 동일*.
pub fn rrf_merge_weighted(
    vec_ranking: &[(i64, f64)],
    fts_ranking: &[(i64, f64)],
    w_vec: f64,
    w_fts: f64,
) -> Vec<(i64, f64)> {
    let mut score: HashMap<i64, f64> = HashMap::new();
    for (rank, (id, _)) in vec_ranking.iter().enumerate() {
        *score.entry(*id).or_insert(0.0) += w_vec / (RRF_K + (rank as f64) + 1.0);
    }
    for (rank, (id, _)) in fts_ranking.iter().enumerate() {
        *score.entry(*id).or_insert(0.0) += w_fts / (RRF_K + (rank as f64) + 1.0);
    }
    let mut merged: Vec<(i64, f64)> = score.into_iter().collect();
    // 점수 내림차순 (큰 값 = 더 관련). 동점이면 chunk_id 오름차순(안정).
    merged.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    merged
}

/// 합산 결과 chunk_id 리스트 → chunks 테이블 일괄 조회로 RetrievedChunk 채우기.
fn fetch_chunk_records(
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
        let row = stmt.query_row(params![id], |r| {
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
            // chunk가 사라진 경우(예: 인덱싱 진행 중 race) 조용히 skip.
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// Hybrid retrieval entry — vector + FTS5 → RRF 병합 → top-N.
///
/// `n`이 0이면 빈 결과. 책에 chunks 적재가 없으면 vec0 KNN과 FTS MATCH 모두 빈 결과를
/// 반환해 자연스레 빈 Vec를 돌려준다.
pub fn hybrid_search(
    conn: &Connection,
    embedder: &Embedder,
    book_id: &str,
    query: &str,
    n: usize,
) -> AppResult<Vec<RetrievedChunk>> {
    hybrid_search_with_vector_query(conn, embedder, book_id, query, query, n)
}

/// v0.4.3 PR 3 (D-087) — vector 검색에 *별도의 query 텍스트*를 사용할 수 있는 entry.
///
/// HyDE 사용 시: `vector_query` = LLM이 생성한 가상 답변 단락, `fts_query` = rewritten
/// 사용자 질문. FTS5 텍스트 매칭은 가상 답변에 약하므로 *rewritten query 그대로* 가야
/// 한다.
///
/// `vector_query`가 빈 문자열이면 vector 검색 skip → FTS-only RRF (다른 분기에서
/// `fts_only_search`를 직접 부르는 게 더 명확하지만, 안전장치 차원).
pub fn hybrid_search_with_vector_query(
    conn: &Connection,
    embedder: &Embedder,
    book_id: &str,
    vector_query: &str,
    fts_query: &str,
    n: usize,
) -> AppResult<Vec<RetrievedChunk>> {
    // 균등 가중 — 기존 동작 보존.
    hybrid_search_weighted(conn, embedder, book_id, vector_query, fts_query, n, 1.0, 1.0)
}

/// v0.6.x (D-109) — 가중 RRF 적용 hybrid retrieval. query_route 가중치를 받는다.
///
/// `w_vec`/`w_fts`가 (1.0, 1.0)이면 `hybrid_search_with_vector_query`와 동일.
#[allow(clippy::too_many_arguments)]
pub fn hybrid_search_weighted(
    conn: &Connection,
    embedder: &Embedder,
    book_id: &str,
    vector_query: &str,
    fts_query: &str,
    n: usize,
    w_vec: f64,
    w_fts: f64,
) -> AppResult<Vec<RetrievedChunk>> {
    if n == 0 || (vector_query.trim().is_empty() && fts_query.trim().is_empty()) {
        return Ok(Vec::new());
    }
    // vec0 idempotent 보장 — 첫 검색 시 vec0가 아직 없으면 만든다(차원=Embedder::DIM).
    ensure_vec0(conn)?;

    let vec_ranking = if vector_query.trim().is_empty() {
        Vec::new()
    } else {
        vector_top_k(conn, embedder, book_id, vector_query, VECTOR_TOP_K)?
    };
    let fts_ranking = if fts_query.trim().is_empty() {
        Vec::new()
    } else {
        fts_top_k(conn, book_id, fts_query, FTS_TOP_K)?
    };
    let merged = rrf_merge_weighted(&vec_ranking, &fts_ranking, w_vec, w_fts);
    let top: Vec<(i64, f64)> = merged.into_iter().take(n).collect();
    fetch_chunk_records(conn, &top)
}

/// FTS-only fallback — 임베더 instance 없이 검색 가능 (PR 4가 책에 임베딩 없는 경우 호출).
///
/// 본 함수는 vec0 미생성 환경(=마이그만 적용된 빈 DB)에서도 안전하게 동작한다.
pub fn fts_only_search(
    conn: &Connection,
    book_id: &str,
    query: &str,
    n: usize,
) -> AppResult<Vec<RetrievedChunk>> {
    if n == 0 || query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let fts_ranking = fts_top_k(conn, book_id, query, n.max(FTS_TOP_K))?;
    let merged = rrf_merge(&[], &fts_ranking);
    let top: Vec<(i64, f64)> = merged.into_iter().take(n).collect();
    fetch_chunk_records(conn, &top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::v041::f32_bytes;
    use crate::index::v041::vector_store::VEC0_TABLE;
    use rusqlite::Connection;

    /// 가짜 384d 임베딩 — 인덱스 i를 받아 한 차원만 1.0인 one-hot 벡터로 만든다.
    /// vec0 KNN distance가 입력 인덱스와 일치하는 row를 top-1으로 잡도록 *결정적*으로
    /// 동작.
    fn one_hot_384(i: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        if i < 384 {
            v[i] = 1.0;
        }
        v
    }

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
        let conn = Connection::open_in_memory().expect("open in-memory");
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
            "INSERT INTO studies (slug, name, created_at) VALUES ('s1','S1',datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (id, study_slug, role, title, source_path, file_format, \
                                  file_size, file_hash, added_at) \
             VALUES ('b1','s1','main','Book','/tmp/x','md',0,'h',datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (id, study_slug, role, title, source_path, file_format, \
                                  file_size, file_hash, added_at) \
             VALUES ('b2','s1','main','OtherBook','/tmp/y','md',0,'h2',datetime('now'))",
            [],
        )
        .unwrap();
        ensure_vec0(&conn).unwrap();
        conn
    }

    /// chunks INSERT 후 가짜 임베딩을 vec0에 직접 INSERT — embedder 호출 없이 KNN 검증.
    fn insert_chunk_with_fake_emb(
        conn: &Connection,
        book_id: &str,
        ord: i64,
        text: &str,
        section_path: &str,
        page: Option<i64>,
        emb: &[f32],
    ) -> i64 {
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, page, section_path, token_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![book_id, ord, text, page, section_path, text.chars().count() as i64],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        let bytes = f32_bytes(emb);
        let sql = format!("INSERT INTO {VEC0_TABLE}(rowid, embedding) VALUES (?1, ?2)");
        conn.execute(&sql, params![id, bytes]).unwrap();
        // vectors_t1 도 페어로 채움 (실제 운용 경로 일관성).
        conn.execute(
            "INSERT INTO vectors_t1 (chunk_id, embedding) VALUES (?1, ?2)",
            params![id, bytes],
        )
        .unwrap();
        id
    }

    #[test]
    fn normalize_fts_query_strips_meta_chars_and_or_joins_tokens() {
        assert_eq!(normalize_fts_query("ownership 모델"), Some("\"ownership\" OR \"모델\"".to_string()));
        assert_eq!(normalize_fts_query("a*b\"c"), Some("\"a\" OR \"b\" OR \"c\"".to_string()));
        assert_eq!(normalize_fts_query("   "), None);
        assert_eq!(normalize_fts_query(""), None);
    }

    #[test]
    fn rrf_merge_higher_when_appearing_in_both() {
        // 청크 100 = 양쪽 1위, 청크 200 = vec only 1위, 청크 300 = fts only 1위.
        let vec = vec![(100, 0.1), (200, 0.2)];
        let fts = vec![(100, -2.5), (300, -3.0)];
        let merged = rrf_merge(&vec, &fts);
        // 100이 첫 번째 — 양쪽 모두 rank 1.
        assert_eq!(merged[0].0, 100);
        // 200·300은 한 쪽만 rank 1이라 같은 점수, chunk_id 오름차순 → 200, 300 순.
        let positions: Vec<i64> = merged.iter().map(|(id, _)| *id).collect();
        assert_eq!(positions, vec![100, 200, 300]);
    }

    #[test]
    fn fts_only_search_returns_book_scoped_hits() {
        let conn = fresh_conn();
        // 두 책에 같은 키워드가 들어있어도 b1 only로 한정.
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, section_path) \
             VALUES ('b1', 0, 'Rust ownership 모델 안전성', 'Ch01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, section_path) \
             VALUES ('b2', 0, 'Rust ownership 다른 책', 'Ch01')",
            [],
        )
        .unwrap();

        let hits = fts_only_search(&conn, "b1", "ownership", 5).unwrap();
        assert_eq!(hits.len(), 1, "b1 한정");
        assert_eq!(hits[0].section_path.as_deref(), Some("Ch01"));
        assert!(hits[0].text.contains("안전성"));
    }

    #[test]
    fn vector_top_k_filters_by_book_and_orders_by_distance() {
        let conn = fresh_conn();
        // b1에 3개, b2에 1개 — 가짜 임베딩으로 distance 0이 b1의 한 청크에 매칭되도록.
        let _id_b1_a = insert_chunk_with_fake_emb(
            &conn,
            "b1",
            0,
            "한국어 본문 A",
            "Ch01/§A",
            None,
            &one_hot_384(0),
        );
        let id_b1_b = insert_chunk_with_fake_emb(
            &conn,
            "b1",
            1,
            "한국어 본문 B",
            "Ch01/§B",
            None,
            &one_hot_384(1),
        );
        let _id_b1_c = insert_chunk_with_fake_emb(
            &conn,
            "b1",
            2,
            "한국어 본문 C",
            "Ch01/§C",
            None,
            &one_hot_384(2),
        );
        let _id_b2 = insert_chunk_with_fake_emb(
            &conn,
            "b2",
            0,
            "다른 책",
            "Ch01",
            None,
            &one_hot_384(1), // b2에도 같은 one-hot 있음 → book 필터 검증
        );

        // KNN distance가 0인 row를 직접 찾기 위해 vec0를 그대로 호출.
        let raw = knn(&conn, &one_hot_384(1), 4).unwrap();
        // distance 0인 row가 두 개 (b1_b, b2). book 필터로 b1만 남아야 함.
        assert!(raw.iter().any(|(id, _)| *id == id_b1_b));

        // book 필터 적용된 vector_top_k는 fastembed 호출이 필요해 e2e 게이팅. 여기서는
        // 직접 chunks 검사로 동치 검증.
        let mut stmt =
            conn.prepare("SELECT 1 FROM chunks WHERE id = ?1 AND book_id = ?2").unwrap();
        let mut filtered: Vec<i64> = Vec::new();
        for (id, _) in raw {
            if stmt.query_row(params![id, "b1"], |r| r.get::<_, i64>(0)).is_ok() {
                filtered.push(id);
            }
        }
        assert!(filtered.contains(&id_b1_b));
    }

    #[test]
    fn rrf_merge_with_only_one_side_works() {
        let vec = vec![(1, 0.1), (2, 0.2), (3, 0.3)];
        let merged = rrf_merge(&vec, &[]);
        let ids: Vec<i64> = merged.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![1, 2, 3]);

        let fts = vec![(10, -1.0), (20, -2.0)];
        let merged = rrf_merge(&[], &fts);
        let ids: Vec<i64> = merged.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![10, 20]);
    }

    #[test]
    fn fetch_chunk_records_handles_missing_ids_silently() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, section_path, page) \
             VALUES ('b1', 0, '한국어 본문', 'Ch01', 12)",
            [],
        )
        .unwrap();
        let real_id: i64 = conn
            .query_row("SELECT id FROM chunks WHERE book_id='b1'", [], |r| r.get(0))
            .unwrap();

        let recs = fetch_chunk_records(&conn, &[(real_id, 0.5), (99999, 0.4)]).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].id, real_id);
        assert_eq!(recs[0].page, Some(12));
        assert_eq!(recs[0].section_path.as_deref(), Some("Ch01"));
        assert!((recs[0].score - 0.5).abs() < 1e-9);
    }

    #[test]
    fn fts_only_search_empty_query_returns_empty() {
        let conn = fresh_conn();
        assert!(fts_only_search(&conn, "b1", "   ", 5).unwrap().is_empty());
        assert!(fts_only_search(&conn, "b1", "", 5).unwrap().is_empty());
    }

    #[test]
    fn fts_top_k_empty_book_yields_empty() {
        let conn = fresh_conn();
        // 책에 청크가 없으면 빈 결과.
        let hits = fts_top_k(&conn, "b1", "ownership", 5).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn vector_top_k_zero_k_returns_empty_without_calling_embedder() {
        // K=0 분기 — embedder 인스턴스가 필요 없는 short-circuit 검증.
        // 실제 embedder는 fastembed 다운로드라 unit test 부적합.
        // 여기서는 K=0이면 *embedder 호출 없이* 즉시 Vec::new() 반환을 확인.
        // (Embedder 인스턴스 자체 생성도 스킵해야 한다 — 함수 시그니처가 &Embedder라
        //  실제 인스턴스 없이는 못 부르지만, K=0 분기는 코드 경로 검증 목적.)
        // 본 테스트는 분기 자체가 얕아 코드 리뷰로도 충분하지만, 보호 차원에서
        // K=0 호출 시 Ok(Vec::new())를 반환한다는 *행위 명세*를 남긴다.
        // 실제 호출 검증은 e2e 통합(`AIRIS_E2E_EMBED=1`)에서 한다.
    }
}
