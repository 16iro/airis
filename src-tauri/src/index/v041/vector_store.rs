// v0.4.1 vector_store — sqlite-vec vec0 가상 테이블 + vectors_t1 BLOB 페어.
//
// 두 테이블 역할 분담:
//   * vectors_t1   = chunk_id ↔ embedding BLOB 영속 (DB v13에서 만든다).
//   * vectors_t1_vec0 = sqlite-vec vec0 가상 테이블. KNN 검색 인덱스. rowid = chunk_id.
//                       *마이그레이션이 아니라 코드*가 생성. 차원 strict이라 fastembed
//                       모델 dim에 맞춰야 하기 때문.
//
// PR 1 범위:
//   * 함수 시그니처 + SQL 문자열만. 실제 호출(INSERT 후 vec0 동기, KNN 쿼리)은 PR 3.
//   * f32 → BLOB 변환은 v041::f32_bytes 사용.
//
// vec0 가상 테이블 차원 strict (HANDOFF §9):
//   다른 차원 INSERT 거부 → mE5-small=384 일관 강제.

#![allow(dead_code)]

use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::index::v041::embedder::Embedder;
use crate::index::v041::f32_bytes;

/// vec0 가상 테이블 이름 — `vectors_t1` 영속 BLOB 테이블의 KNN 인덱스 짝.
pub const VEC0_TABLE: &str = "vectors_t1_vec0";

/// vec0 가상 테이블이 없으면 만든다. 차원 = `Embedder::DIM` (384).
///
/// CREATE VIRTUAL TABLE IF NOT EXISTS는 SQLite 3.35+에서 vec0를 받는다 — bundled
/// rusqlite의 SQLite 버전(3.46+)에서 안전. 이미 있으면 noop.
///
/// PR 3가 인덱싱 시작 직전 1회 호출, retrieval 시작 시도에서도 idempotent하게 호출.
pub fn ensure_vec0(conn: &Connection) -> AppResult<()> {
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {table} USING vec0(\
            embedding FLOAT[{dim}]\
        )",
        table = VEC0_TABLE,
        dim = Embedder::DIM,
    );
    conn.execute(&sql, [])?;
    Ok(())
}

/// chunk_id에 임베딩을 *upsert*. 두 테이블에 동시 반영:
///   1. vectors_t1            — 영속 BLOB (book 삭제 CASCADE로 자동 정리)
///   2. vectors_t1_vec0       — KNN 인덱스 (rowid = chunk_id, 차원 strict)
///
/// 호출 측에서 트랜잭션을 잡아주는 게 권장. 본 함수 자체는 트랜잭션 시작/커밋 X.
///
/// PR 1: 시그니처만. 실제 호출은 PR 3 retrieval에서 INSERT 경로로 이동.
pub fn upsert_embedding(
    conn: &Connection,
    chunk_id: i64,
    embedding: &[f32],
) -> AppResult<()> {
    let bytes = f32_bytes(embedding);

    // 영속 BLOB — chunk_id가 PRIMARY KEY라 ON CONFLICT REPLACE로 upsert.
    conn.execute(
        "INSERT INTO vectors_t1 (chunk_id, embedding) VALUES (?1, ?2) \
         ON CONFLICT(chunk_id) DO UPDATE SET embedding = excluded.embedding",
        params![chunk_id, bytes],
    )?;

    // vec0 인덱스 — rowid가 PK 역할. 같은 rowid 재INSERT는 거부되므로 DELETE+INSERT.
    let delete_sql = format!("DELETE FROM {VEC0_TABLE} WHERE rowid = ?1");
    conn.execute(&delete_sql, params![chunk_id])?;
    let insert_sql = format!("INSERT INTO {VEC0_TABLE}(rowid, embedding) VALUES (?1, ?2)");
    conn.execute(&insert_sql, params![chunk_id, bytes])?;

    Ok(())
}

/// 단일 chunk_id의 임베딩 BLOB을 가져온다 (vectors_t1, KNN과 무관). 디버그/검증 용.
pub fn get_embedding(conn: &Connection, chunk_id: i64) -> AppResult<Option<Vec<u8>>> {
    let mut stmt = conn.prepare("SELECT embedding FROM vectors_t1 WHERE chunk_id = ?1")?;
    let mut rows = stmt.query(params![chunk_id])?;
    if let Some(row) = rows.next()? {
        let bytes: Vec<u8> = row.get(0)?;
        Ok(Some(bytes))
    } else {
        Ok(None)
    }
}

/// vec0 인덱스에서 쿼리 벡터의 top-K KNN — chunk_id + distance를 점수 오름차순으로.
///
/// PR 1: 시그니처만. 실제 RRF 병합은 PR 3 retrieval에서 호출.
pub fn knn(
    conn: &Connection,
    query: &[f32],
    k: usize,
) -> AppResult<Vec<(i64, f64)>> {
    let q_bytes = f32_bytes(query);
    // 라인 continuation `\`는 *그 다음 줄의 leading whitespace까지 제거*해서 토큰이 붙는다.
    // SQL 키워드 사이에 명시 공백을 둔다.
    let sql = format!(
        "SELECT rowid, distance FROM {VEC0_TABLE} \
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
