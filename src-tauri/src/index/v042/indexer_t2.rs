// T2 인덱서 — BGE-M3 백그라운드 빌드 (worker.embed_batch + vector_store_t2 어댑터).
//
// 책임 (HANDOFF §1.2):
//   1. `vectors_t2_vec0` 가상 테이블 ensure (idempotent).
//   2. indexing_jobs row 생성 (tier='t2_bge-m3', 즉 db_tier=2).
//   3. 청크 시퀀스를 BATCH_SIZE(=worker.rs 32)씩 묶어 embed → vec0 + worker.embed_batch.
//   4. PauseGate.wait_if_paused() + cancel 점검을 *매 batch 시작 전*. cooperative.
//   5. 실패 청크 → record_embed_failure (3회 누적 → 'failed' skip).
//   6. 호출 측이 manifest_t2 ready 전환 + active_index 핫스왑 (build_t2 같은 상위
//      함수 또는 PR 3 commands가 책임). 본 모듈은 *DB 채우기*만.
//
// trait Embedder 추상화:
//   `EmbedderLike` trait — T2/T1 어느 쪽이든 같은 worker 진입을 가능케 한다.
//   본 PR은 T2만 호출 측 구현. v0.4.3+ T1 호출도 wire한다.
//
// 호출 측 (PR 3·5):
//   * commands::book::start_t2_build(book_id) — 백그라운드 task.
//   * 진행 중 사용자 chat 진입 시 PR 5 cooperative pause 트리거.
//
// 본 PR 단위 테스트는 `mock embedder` 패턴으로 worker.embed_batch 통합 검증 — fastembed
// 다운로드 비용을 안 들이고 indexer 흐름의 정확성만 검사.

#![allow(dead_code)]

use std::sync::atomic::Ordering;

use rusqlite::{params, Connection};

use crate::cache::embedding::EmbeddingCache;
use crate::error::AppResult;
use crate::index::v041::f32_bytes;
use crate::index::v042::embedder_t2::{EmbedderT2, T2_EMBED_BATCH};
use crate::index::v042::vector_store_t2::{ensure_vec0_t2, VEC0_TABLE_T2};
use crate::index::v042::worker::{
    record_embed_failure, EmbedFailureOutcome, IndexingWorker, Tier,
};

/// v0.4.2 PR 4 — T2 embedding_cache 모델 식별자.
const T2_MODEL_ID: &str = "bge-m3";

/// T2 인덱싱 결과 — 호출 측 진행 보고에 사용.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct T2Outcome {
    pub job_id: i64,
    /// 성공적으로 vectors_t2에 들어간 청크 수.
    pub embeddings_inserted: usize,
    /// 3회 실패로 skip된 청크 수.
    pub skipped: usize,
    /// 사용자/시스템 cancel로 도중 멈춘 경우 true.
    pub cancelled: bool,
}

/// 임베더 추상화 — T2/T1 worker 통합을 위해. 본 PR은 T2 구현만.
///
/// dyn 디스패치 — fastembed enum 분기는 호출 측이 책임. 본 trait는 *passage 임베딩*
/// 한 면만. retrieval은 별도 retrieval_v042::QueryEmbedder를 쓴다(query/passage 분리).
pub trait PassageEmbedder: Send + Sync {
    /// 임베딩 차원 — vec0 가상 테이블 strict 검증의 호출 측 ground truth.
    fn dim(&self) -> usize;
    /// 청크 본문 배열 → 임베딩 벡터 배열. caller는 prefix(필요 시)를 미리 적용.
    /// BGE-M3는 prefix X, mE5는 "passage: " 강제.
    fn embed_passages(&self, chunks: &[String]) -> AppResult<Vec<Vec<f32>>>;
}

impl PassageEmbedder for EmbedderT2 {
    fn dim(&self) -> usize {
        EmbedderT2::DIM
    }

    fn embed_passages(&self, chunks: &[String]) -> AppResult<Vec<Vec<f32>>> {
        EmbedderT2::embed_passages(self, chunks)
    }
}

/// 한 책의 T2 인덱스를 빌드. *전체 청크가 아니라*, 호출 측이 미리 결정한 pending 청크
/// 시퀀스만 처리 — resume_pending_jobs 결과를 그대로 받을 수 있게 분리.
///
/// `chunks`는 `(chunk_id, raw_text)` 페어 시퀀스. ord 오름차순 권장.
/// `worker`는 호출 측이 만들어 보유 — pause/cancel 외부 트리거 가능하게.
///
/// SQL transactions:
///   * 한 배치(=`T2_EMBED_BATCH`)당 *한 트랜잭션*. worker.embed_batch가
///     vectors_t2 + chunks.embed_status_t2='done' + indexing_jobs.progress 셋을 atomic.
///   * vec0 인덱스 INSERT는 같은 트랜잭션 안에서 별도 호출(non-trivial하므로 TODO 주석).
///
/// 본 PR 단위 테스트는 mock embedder + 작은 청크 시퀀스로 검증.
pub fn build_t2_for_chunks<E: PassageEmbedder + ?Sized>(
    conn: &mut Connection,
    job_id: i64,
    chunks: &[(i64, String)],
    embedder: &E,
    worker: &IndexingWorker,
) -> AppResult<T2Outcome> {
    build_t2_for_chunks_with_cache(conn, job_id, chunks, embedder, worker, None)
}

/// `build_t2_for_chunks` + embedding cache hook (v0.4.2 PR 4 D-084).
pub fn build_t2_for_chunks_with_cache<E: PassageEmbedder + ?Sized>(
    conn: &mut Connection,
    job_id: i64,
    chunks: &[(i64, String)],
    embedder: &E,
    worker: &IndexingWorker,
    cache: Option<&EmbeddingCache>,
) -> AppResult<T2Outcome> {
    // vec0 가상 테이블 ensure — idempotent.
    ensure_vec0_t2(conn)?;

    // 차원 검증 — embedder dim이 1024여야 함 (BGE-M3).
    if embedder.dim() != EmbedderT2::DIM {
        return Err(crate::error::AppError::Internal {
            message: format!(
                "build_t2: embedder.dim()={} ≠ EmbedderT2::DIM={}",
                embedder.dim(),
                EmbedderT2::DIM
            ),
        });
    }

    let mut total_inserted = 0_usize;
    let mut total_skipped = 0_usize;

    for batch in chunks.chunks(T2_EMBED_BATCH) {
        // cancel 점검 — 매 batch 시작 전.
        if worker.cancel.load(Ordering::SeqCst) {
            return Ok(T2Outcome {
                job_id,
                embeddings_inserted: total_inserted,
                skipped: total_skipped,
                cancelled: true,
            });
        }
        // pause 점검 — pause 상태면 resume까지 블록.
        worker.pause_gate.wait_if_paused();

        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();

        // 1) Cache lookup — miss만 fastembed 호출. lookup은 별도 short-lived tx.
        let mut vectors: Vec<Vec<f32>> = vec![Vec::new(); batch.len()];
        let mut miss_indices: Vec<usize> = Vec::new();

        if let Some(c) = cache {
            let items: Vec<(String, String)> = texts
                .iter()
                .map(|t| (t.clone(), T2_MODEL_ID.to_string()))
                .collect();
            // Connection은 conn 핸들 그대로 — get_batch는 read-only(SELECT) + last_hit_at UPDATE.
            // commit_batch_t2와는 트랜잭션 분리 (lookup 시점에 일관성 깨짐 X — cache는 idempotent).
            let cached = c.get_batch(conn, &items)?;
            for (i, slot) in cached.into_iter().enumerate() {
                if let Some(v) = slot {
                    if v.len() == EmbedderT2::DIM {
                        vectors[i] = v;
                    } else {
                        miss_indices.push(i);
                    }
                } else {
                    miss_indices.push(i);
                }
            }
        } else {
            for i in 0..batch.len() {
                miss_indices.push(i);
            }
        }

        // 2) miss만 fastembed 호출.
        let embed_result = if miss_indices.is_empty() {
            Ok(Vec::<Vec<f32>>::new())
        } else {
            let to_embed: Vec<String> = miss_indices.iter().map(|&i| texts[i].clone()).collect();
            embedder.embed_passages(&to_embed)
        };

        match embed_result {
            Ok(new_vecs) => {
                if new_vecs.len() != miss_indices.len() {
                    return Err(crate::error::AppError::Internal {
                        message: format!(
                            "build_t2: embedder가 {} 입력에 {} 결과 반환",
                            miss_indices.len(),
                            new_vecs.len()
                        ),
                    });
                }
                // miss 슬롯 채움 + cache put.
                for (slot_idx, v) in miss_indices.iter().zip(new_vecs) {
                    if let Some(c) = cache {
                        // 인덱서 트랜잭션과 별개 — cache는 항상 영속(다음 배치도 활용).
                        c.put(conn, &texts[*slot_idx], T2_MODEL_ID, EmbedderT2::DIM, &v)?;
                    }
                    vectors[*slot_idx] = v;
                }

                let chunk_ids: Vec<i64> = batch.iter().map(|(id, _)| *id).collect();
                // 단일 트랜잭션 — vec0 + vectors_t2 BLOB + chunks.embed_status_t2 +
                // indexing_jobs.progress 4개 모두 atomic. 크래시 시 마지막 미커밋 배치만 손실.
                commit_batch_t2(conn, job_id, &chunk_ids, &vectors)?;
                total_inserted += batch.len();
            }
            Err(e) => {
                // 배치 단위 실패. 청크별로 attempts++ + last_error.
                let msg = format!("{e}");
                for (chunk_id, _) in batch.iter() {
                    let outcome = record_embed_failure(conn, *chunk_id, Tier::T2BgeM3, &msg)?;
                    if outcome == EmbedFailureOutcome::Skipped {
                        total_skipped += 1;
                    }
                }
                // 잡 자체는 *계속* — 다음 배치 진행. 무한 재시도는 attempts 카운터가 차단.
            }
        }
    }

    Ok(T2Outcome {
        job_id,
        embeddings_inserted: total_inserted,
        skipped: total_skipped,
        cancelled: false,
    })
}

/// 한 배치를 *단일 트랜잭션*으로 commit:
///   1. vec0_t2 인덱스 DELETE+INSERT (rowid=chunk_id, 차원 1024 strict).
///   2. vectors_t2 BLOB upsert (chunk_id PK ON CONFLICT REPLACE).
///   3. chunks.embed_status_t2='done' UPDATE.
///   4. indexing_jobs.progress_chunks += N + updated_at = now() UPDATE.
///
/// 4개가 모두 한 트랜잭션 안에서 commit/rollback. crash 시 마지막 미커밋 배치만 손실
/// (acceptance gate 1: 손실 ≤ 1배치). worker.embed_batch와 같은 invariant이지만
/// vec0 INSERT까지 같은 트랜잭션에 묶기 위해 indexer_t2 안에서 직접 트랜잭션을 잡는다.
fn commit_batch_t2(
    conn: &mut Connection,
    job_id: i64,
    chunk_ids: &[i64],
    embeddings: &[Vec<f32>],
) -> AppResult<()> {
    if chunk_ids.len() != embeddings.len() {
        return Err(crate::error::AppError::Internal {
            message: format!(
                "commit_batch_t2 length mismatch: {} ids vs {} vectors",
                chunk_ids.len(),
                embeddings.len()
            ),
        });
    }
    if chunk_ids.is_empty() {
        return Ok(());
    }
    let n = chunk_ids.len() as i64;
    let tx = conn.transaction()?;

    // 1. vec0_t2 — DELETE+INSERT (vec0는 같은 rowid 재INSERT 거부).
    {
        let delete_sql = format!("DELETE FROM {VEC0_TABLE_T2} WHERE rowid = ?1");
        let mut del_stmt = tx.prepare(&delete_sql)?;
        let insert_sql = format!("INSERT INTO {VEC0_TABLE_T2}(rowid, embedding) VALUES (?1, ?2)");
        let mut ins_stmt = tx.prepare(&insert_sql)?;
        for (chunk_id, vec) in chunk_ids.iter().zip(embeddings.iter()) {
            del_stmt.execute(params![chunk_id])?;
            ins_stmt.execute(params![chunk_id, f32_bytes(vec)])?;
        }
    }

    // 2. vectors_t2 BLOB upsert.
    {
        let mut stmt = tx.prepare(
            "INSERT INTO vectors_t2 (chunk_id, embedding) VALUES (?1, ?2) \
             ON CONFLICT(chunk_id) DO UPDATE SET embedding = excluded.embedding",
        )?;
        for (chunk_id, vec) in chunk_ids.iter().zip(embeddings.iter()) {
            stmt.execute(params![chunk_id, f32_bytes(vec)])?;
        }
    }

    // 3. chunks.embed_status_t2 = 'done'.
    {
        let mut stmt = tx.prepare("UPDATE chunks SET embed_status_t2 = 'done' WHERE id = ?1")?;
        for chunk_id in chunk_ids {
            stmt.execute(params![chunk_id])?;
        }
    }

    // 4. indexing_jobs.progress_chunks += N + updated_at = now().
    tx.execute(
        "UPDATE indexing_jobs SET \
            progress_chunks = progress_chunks + ?1, \
            updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
         WHERE id = ?2",
        params![n, job_id],
    )?;

    tx.commit()?;
    Ok(())
}

/// indexing_jobs row 생성 — tier='t2_bge-m3' (db_tier=2). 호출 측이 build_t2_for_chunks
/// 진입 전에 호출.
pub fn create_t2_job(conn: &Connection, book_id: &str, total_chunks: usize) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO indexing_jobs \
            (book_id, status, tier, progress_chunks, total_chunks, started_at, updated_at) \
         VALUES (?1, 'running', 2, 0, ?2, \
                 CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
                 CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
        params![book_id, total_chunks as i64],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::v042::worker::{Tier, MAX_EMBED_ATTEMPTS};
    use rusqlite::Connection;
    use std::sync::Mutex;

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

    fn fresh_db() -> Connection {
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
        // 책 + 청크.
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

    fn insert_chunks(conn: &Connection, n: usize) -> Vec<i64> {
        let mut ids = Vec::with_capacity(n);
        for i in 0..n {
            conn.execute(
                "INSERT INTO chunks (book_id, ord, text, token_count) VALUES ('b1', ?1, ?2, 1)",
                params![i as i64, format!("chunk-{i}")],
            )
            .unwrap();
            ids.push(conn.last_insert_rowid());
        }
        ids
    }

    /// 1024d 결정적 가짜 임베딩 — i 인덱스만 1.0인 one-hot.
    fn one_hot_1024(i: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; 1024];
        if i < 1024 {
            v[i] = 1.0;
        }
        v
    }

    /// 결정적 mock — 청크 텍스트의 첫 글자 코드를 인덱스로 1024d one-hot.
    /// 같은 텍스트는 같은 벡터 → 테스트 안정.
    struct MockEmbedderT2 {
        call_count: Mutex<usize>,
        fail_after: Option<usize>,
    }

    impl MockEmbedderT2 {
        fn new() -> Self {
            Self {
                call_count: Mutex::new(0),
                fail_after: None,
            }
        }

        fn fail_after(n: usize) -> Self {
            Self {
                call_count: Mutex::new(0),
                fail_after: Some(n),
            }
        }
    }

    impl PassageEmbedder for MockEmbedderT2 {
        fn dim(&self) -> usize {
            EmbedderT2::DIM
        }

        fn embed_passages(&self, chunks: &[String]) -> AppResult<Vec<Vec<f32>>> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            if let Some(fa) = self.fail_after {
                if *count > fa {
                    return Err(crate::error::AppError::Internal {
                        message: "mock fail".to_string(),
                    });
                }
            }
            let mut out = Vec::with_capacity(chunks.len());
            for (i, _) in chunks.iter().enumerate() {
                // 결정적 — 호출 시퀀스 순서로 인덱스 부여.
                out.push(one_hot_1024((*count * 100 + i) % 1024));
            }
            Ok(out)
        }
    }

    /// 차원 mismatch 시뮬 — dim()=384 반환.
    struct MockEmbedderWrongDim;
    impl PassageEmbedder for MockEmbedderWrongDim {
        fn dim(&self) -> usize {
            384
        }
        fn embed_passages(&self, _: &[String]) -> AppResult<Vec<Vec<f32>>> {
            unreachable!("dim 검증에서 차단되어야 함")
        }
    }

    #[test]
    fn build_t2_indexes_small_chunk_sequence_completely() {
        let mut conn = fresh_db();
        let ids = insert_chunks(&conn, 5);
        let job_id = create_t2_job(&conn, "b1", 5).unwrap();
        let chunks: Vec<(i64, String)> = ids.iter().map(|id| (*id, format!("text-{id}"))).collect();
        let embedder = MockEmbedderT2::new();
        let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);

        let outcome = build_t2_for_chunks(&mut conn, job_id, &chunks, &embedder, &worker).unwrap();
        assert_eq!(outcome.embeddings_inserted, 5);
        assert_eq!(outcome.skipped, 0);
        assert!(!outcome.cancelled);

        // chunks.embed_status_t2='done' 5건.
        let done: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE embed_status_t2 = 'done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(done, 5);
        // vectors_t2 5건.
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM vectors_t2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5);
        // indexing_jobs.progress_chunks = 5.
        let progress: i64 = conn
            .query_row(
                "SELECT progress_chunks FROM indexing_jobs WHERE id = ?1",
                params![job_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(progress, 5);
    }

    #[test]
    fn build_t2_records_failure_and_continues() {
        // 첫 배치는 성공, 두 번째 배치는 실패.
        // BATCH_SIZE=32라 청크 33개로 두 배치.
        let mut conn = fresh_db();
        let ids = insert_chunks(&conn, 33);
        let job_id = create_t2_job(&conn, "b1", 33).unwrap();
        let chunks: Vec<(i64, String)> = ids.iter().map(|id| (*id, format!("text-{id}"))).collect();
        let embedder = MockEmbedderT2::fail_after(1); // 첫 호출만 성공.
        let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);

        let outcome = build_t2_for_chunks(&mut conn, job_id, &chunks, &embedder, &worker).unwrap();
        assert_eq!(outcome.embeddings_inserted, 32, "첫 배치만 성공");
        // 실패 청크는 1회만 시도라 attempts=1 → MAX(=3) 미만 → skipped 0.
        assert_eq!(outcome.skipped, 0);

        let done: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE embed_status_t2 = 'done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(done, 32);
    }

    #[test]
    fn build_t2_skips_chunks_after_max_failures() {
        let mut conn = fresh_db();
        let ids = insert_chunks(&conn, 1);
        // 미리 attempts=2로 채워서 한 번 더 실패하면 'failed' 마킹.
        conn.execute(
            "UPDATE chunks SET embed_attempts = 2 WHERE id = ?1",
            params![ids[0]],
        )
        .unwrap();

        let job_id = create_t2_job(&conn, "b1", 1).unwrap();
        let chunks: Vec<(i64, String)> = vec![(ids[0], "x".to_string())];
        let embedder = MockEmbedderT2::fail_after(0); // 무조건 실패.
        let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);

        let outcome = build_t2_for_chunks(&mut conn, job_id, &chunks, &embedder, &worker).unwrap();
        assert_eq!(outcome.embeddings_inserted, 0);
        assert_eq!(outcome.skipped, 1);

        // 'failed' 마킹 검증.
        let status: Option<String> = conn
            .query_row(
                "SELECT embed_status_t2 FROM chunks WHERE id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status.as_deref(), Some("failed"));
        // attempts = MAX_EMBED_ATTEMPTS = 3.
        let attempts: i64 = conn
            .query_row(
                "SELECT embed_attempts FROM chunks WHERE id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, MAX_EMBED_ATTEMPTS);
    }

    #[test]
    fn build_t2_respects_cancel_between_batches() {
        let mut conn = fresh_db();
        let ids = insert_chunks(&conn, 64);
        let job_id = create_t2_job(&conn, "b1", 64).unwrap();
        let chunks: Vec<(i64, String)> = ids.iter().map(|id| (*id, format!("t-{id}"))).collect();
        let embedder = MockEmbedderT2::new();
        let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);
        // 진입 전 cancel — 첫 배치 시작 전에 즉시 종료.
        worker.cancel();

        let outcome = build_t2_for_chunks(&mut conn, job_id, &chunks, &embedder, &worker).unwrap();
        assert!(outcome.cancelled);
        assert_eq!(outcome.embeddings_inserted, 0);
    }

    #[test]
    fn build_t2_rejects_wrong_dimension_embedder() {
        let mut conn = fresh_db();
        let _ids = insert_chunks(&conn, 1);
        let job_id = create_t2_job(&conn, "b1", 1).unwrap();
        let chunks = vec![(1, "x".to_string())];
        let embedder = MockEmbedderWrongDim;
        let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);
        let r = build_t2_for_chunks(&mut conn, job_id, &chunks, &embedder, &worker);
        assert!(r.is_err(), "embedder.dim()=384는 BGE-M3 1024와 mismatch");
    }

    #[test]
    fn build_t2_empty_chunks_is_noop() {
        let mut conn = fresh_db();
        let _ = insert_chunks(&conn, 0);
        let job_id = create_t2_job(&conn, "b1", 0).unwrap();
        let embedder = MockEmbedderT2::new();
        let worker = IndexingWorker::new(job_id, Tier::T2BgeM3);
        let outcome = build_t2_for_chunks(&mut conn, job_id, &[], &embedder, &worker).unwrap();
        assert_eq!(outcome.embeddings_inserted, 0);
        assert_eq!(outcome.skipped, 0);
        assert!(!outcome.cancelled);
    }
}
