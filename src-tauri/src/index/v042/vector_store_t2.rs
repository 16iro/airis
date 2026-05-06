// T2 vector_store — sqlite-vec vec0 가상 테이블(`vectors_t2_vec0`) + vectors_t2 BLOB 페어.
//
// v041 vector_store 패턴 그대로, 차원만 1024(=BGE-M3)로 분기.
//
// 두 테이블 역할:
//   * vectors_t2          = chunk_id ↔ embedding BLOB 영속 (DB v15에서 만든다).
//   * vectors_t2_vec0     = sqlite-vec vec0. KNN 검색 인덱스. rowid = chunk_id.
//                            *마이그가 아니라 코드*가 생성. 차원 strict이라 BGE-M3 dim
//                            (1024)에 맞춰야 하기 때문.

#![allow(dead_code)]

use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::index::v041::f32_bytes;
use crate::index::v042::embedder_t2::EmbedderT2;

/// vec0 가상 테이블 이름 — `vectors_t2` 영속 BLOB 테이블의 KNN 인덱스 짝.
pub const VEC0_TABLE_T2: &str = "vectors_t2_vec0";

/// vec0 가상 테이블이 없으면 만든다. 차원 = `EmbedderT2::DIM` (1024).
///
/// 이미 있으면 noop. v041::vector_store::ensure_vec0와 같은 패턴 — idempotent.
pub fn ensure_vec0_t2(conn: &Connection) -> AppResult<()> {
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {table} USING vec0(\
            embedding FLOAT[{dim}]\
        )",
        table = VEC0_TABLE_T2,
        dim = EmbedderT2::DIM,
    );
    conn.execute(&sql, [])?;
    Ok(())
}

/// chunk_id에 임베딩을 *upsert*. 두 테이블에 동시 반영:
///   1. vectors_t2            — 영속 BLOB
///   2. vectors_t2_vec0       — KNN 인덱스 (rowid = chunk_id, 차원 strict 1024)
///
/// 호출 측에서 트랜잭션을 잡아주는 게 권장. 본 함수 자체는 트랜잭션 시작/커밋 X.
/// indexer_t2가 worker.embed_batch와 함께 단일 트랜잭션 안에서 호출.
pub fn upsert_embedding_t2(
    conn: &Connection,
    chunk_id: i64,
    embedding: &[f32],
) -> AppResult<()> {
    let bytes = f32_bytes(embedding);

    // 영속 BLOB — chunk_id가 PRIMARY KEY라 ON CONFLICT REPLACE로 upsert.
    conn.execute(
        "INSERT INTO vectors_t2 (chunk_id, embedding) VALUES (?1, ?2) \
         ON CONFLICT(chunk_id) DO UPDATE SET embedding = excluded.embedding",
        params![chunk_id, bytes],
    )?;

    // vec0 인덱스 — 같은 rowid 재INSERT는 거부되므로 DELETE+INSERT.
    let delete_sql = format!("DELETE FROM {VEC0_TABLE_T2} WHERE rowid = ?1");
    conn.execute(&delete_sql, params![chunk_id])?;
    let insert_sql = format!("INSERT INTO {VEC0_TABLE_T2}(rowid, embedding) VALUES (?1, ?2)");
    conn.execute(&insert_sql, params![chunk_id, bytes])?;

    Ok(())
}

/// vec0 인덱스에서 쿼리 벡터의 top-K KNN — chunk_id + distance를 점수 오름차순으로.
///
/// v041::vector_store::knn 패턴 그대로. SQL 라인 continuation 버그 회피 — 명시 공백.
pub fn knn_t2(
    conn: &Connection,
    query: &[f32],
    k: usize,
) -> AppResult<Vec<(i64, f64)>> {
    let q_bytes = f32_bytes(query);
    let sql = format!(
        "SELECT rowid, distance FROM {VEC0_TABLE_T2} \
         WHERE embedding MATCH ?1 \
         ORDER BY distance \
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![q_bytes, k as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // 책 + 청크 1개 — vectors_t2 FK 충족용.
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

    /// 1024차원 결정적 가짜 임베딩 — i 인덱스만 1.0인 one-hot.
    fn one_hot_1024(i: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; 1024];
        if i < 1024 {
            v[i] = 1.0;
        }
        v
    }

    fn insert_chunk(conn: &Connection, ord: i64) -> i64 {
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, token_count) VALUES ('b1', ?1, ?2, 1)",
            params![ord, format!("c{ord}")],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn ensure_vec0_t2_is_idempotent() {
        let conn = fresh_conn();
        ensure_vec0_t2(&conn).unwrap();
        ensure_vec0_t2(&conn).unwrap(); // 재호출 — noop이어야 함.
        // 가상 테이블 존재 확인.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![VEC0_TABLE_T2],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn upsert_embedding_t2_round_trip_via_knn() {
        let conn = fresh_conn();
        ensure_vec0_t2(&conn).unwrap();
        let id_a = insert_chunk(&conn, 0);
        let id_b = insert_chunk(&conn, 1);

        upsert_embedding_t2(&conn, id_a, &one_hot_1024(0)).unwrap();
        upsert_embedding_t2(&conn, id_b, &one_hot_1024(7)).unwrap();

        // 인덱스 0인 query → id_a가 distance 0.
        let hits = knn_t2(&conn, &one_hot_1024(0), 2).unwrap();
        assert_eq!(hits[0].0, id_a, "distance 가장 작은 row가 id_a");
        // distance 자체는 implementation detail — 정렬 순서만 검증.
        assert!(hits[0].1 <= hits[1].1);
    }

    #[test]
    fn upsert_embedding_t2_replaces_existing() {
        let conn = fresh_conn();
        ensure_vec0_t2(&conn).unwrap();
        let id = insert_chunk(&conn, 0);

        upsert_embedding_t2(&conn, id, &one_hot_1024(0)).unwrap();
        // 같은 chunk_id에 다른 벡터로 upsert — 기존 row 교체.
        upsert_embedding_t2(&conn, id, &one_hot_1024(50)).unwrap();

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM vectors_t2 WHERE chunk_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "upsert는 1행 유지");

        // 인덱스 50인 query → 같은 id 1위.
        let hits = knn_t2(&conn, &one_hot_1024(50), 1).unwrap();
        assert_eq!(hits[0].0, id);
    }

    #[test]
    fn knn_t2_returns_empty_when_index_empty() {
        let conn = fresh_conn();
        ensure_vec0_t2(&conn).unwrap();
        let hits = knn_t2(&conn, &one_hot_1024(0), 5).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn upsert_embedding_t2_dimension_strict_rejects_wrong_dim() {
        let conn = fresh_conn();
        ensure_vec0_t2(&conn).unwrap();
        let id = insert_chunk(&conn, 0);
        // vec0 가상 테이블이 1024차원 strict이라 384차원 INSERT는 실패해야 한다.
        let bad = vec![0.0_f32; 384];
        let r = upsert_embedding_t2(&conn, id, &bad);
        assert!(r.is_err(), "1024≠384 차원 mismatch는 vec0 INSERT 거부");
    }
}
