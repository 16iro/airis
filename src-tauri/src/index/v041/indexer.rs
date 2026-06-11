// v0.4.1 인덱서 — pipeline orchestrator.
//
// 책 한 권을 받아 청크 시퀀스 → DB INSERT까지 한 cycle을 돈다. 임베딩 호출 자리는
// 본 PR(2)에서 *stub*. 실제 fastembed 호출은 PR 3 retrieval에서 채운다.
//
// 책임:
//   1. indexing_jobs row 생성/업데이트 — `index:progress` 이벤트(v0.3.2 도입)의 영속 backing.
//   2. parser → chunker → chunks INSERT (chunks_fts 트리거 자동 동기화).
//   3. parent_id / prev_chunk_id / next_chunk_id를 ord 인덱스 → 실제 DB id로 변환.
//   4. 임베딩 호출 자리(stub) — PR 3 retrieval가 채움.
//
// 호출 패턴:
//   * `commands::book` 또는 `commands::search`(재인덱싱)에서 호출.
//   * D-076 직렬 큐 — 여러 책을 한 번에 인덱싱하지 않는다 (jobs 테이블 status='running'은
//     동시 1개만, 큐 직렬화는 PR 3/4에서 commands 레이어가 책임).
//   * 본 함수는 *동기*. 호출 측이 `tokio::task::spawn_blocking`으로 격리.

#![allow(dead_code)]

use std::path::Path;

use rusqlite::{params, Connection, Transaction};

use crate::cache::embedding::EmbeddingCache;
use crate::error::AppResult;
use crate::index::v041::chunker::{chunk_md_sections, chunk_pdf_pages, ChunkRecord};
use crate::index::v041::embedder::{passage_prefix, Embedder, EMBED_BATCH};
use crate::index::v041::vector_store::{ensure_vec0, upsert_embedding};
use crate::parsers::types::Section;

/// v0.4.1 PR 4 모델 식별자 — embedding_cache 키 도출에 사용.
/// 모델 변경 시(향후 mE5-base 등) 별도 row가 자연스레 분리.
const T1_MODEL_ID: &str = "me5-small";

/// 책 인덱싱의 입력 — 파서 결과에서 챙겨 와야 하는 정보만 추린 형태.
#[derive(Debug, Clone)]
pub enum BookSource<'a> {
    /// MD/HTML — heading 기반 섹션 시퀀스.
    Sections(&'a [Section]),
    /// PDF — 페이지별 텍스트 시퀀스 (1-base가 아니라 0-base 인덱스).
    Pages(&'a [String]),
}

/// 인덱싱 결과 요약 — 호출 측 진행률 이벤트·로깅 용.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexOutcome {
    pub job_id: i64,
    pub chunks_inserted: usize,
    /// 실제 임베딩이 t1(=mE5-small)에 들어간 청크 수. PR 2 stub에서는 0.
    /// PR 3에서 fastembed 호출 후 vector_store::upsert_embedding로 채워짐.
    pub embeddings_inserted: usize,
}

/// 진행 상태 enum — indexing_jobs.status 컬럼의 안전한 한국어/영어 매핑.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
}

impl JobStatus {
    fn as_db_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// 책 1권을 chunks(+chunks_fts 트리거 + 임베딩)에 적재한다 — 동기 함수.
///
/// 동작:
///   1. indexing_jobs INSERT (status='running', tier=1, progress 0).
///   2. chunker로 ChunkRecord 시퀀스 생성.
///   3. 트랜잭션 안에서 chunks INSERT 2-pass:
///      - pass 1 = 모든 청크 `parent_id=NULL, prev_chunk_id=NULL, next_chunk_id=NULL`로
///        INSERT → ord → chunks.id 맵 만든다.
///      - pass 2 = parent/prev/next를 chunker가 채운 ord 인덱스로 UPDATE.
///      - pass 3 = embedder가 있으면 `passage_prefix`를 붙인 본문 배치 임베딩 → vec0 +
///        vectors_t1 upsert. progress_chunks를 batch(=`EMBED_BATCH`)마다 갱신.
///   4. status='completed' 마무리.
///   5. 실패 시 트랜잭션 롤백 + status='failed' + error 메시지.
///
/// `embedder=None`이면 chunks·FTS만 적재 (PR 4가 명시적으로 reindex 강제 시점에 None
/// 호출은 하지 않을 예정). 임베딩 인스턴스를 호출 측에서 굳이 넘기는 형태인 이유: 모델
/// 다운로드 비용을 호출 시점에 한 번만 노출 + 인스턴스 재사용으로 여러 책 인덱싱 시
/// init 비용 절감.
///
/// `app_data_dir`은 `Embedder::new` 호출에 쓰일 cache 경로 — 본 함수가 직접 init하지는
/// 않지만, 시그니처를 함께 받아 두어 PR 4 commands 진입에서 일관 전달이 단순.
/// 호출 측은 `tokio::task::spawn_blocking`으로 격리해 async 런타임을 막지 않는다.
pub fn index_book(
    conn: &mut Connection,
    book_id: &str,
    src: BookSource<'_>,
    embedder: Option<&Embedder>,
    app_data_dir: &Path,
) -> AppResult<IndexOutcome> {
    index_book_with_cache(conn, book_id, src, embedder, app_data_dir, None)
}

/// `index_book` + embedding cache hook (v0.4.2 PR 4 D-084).
///
/// `cache=Some(...)` 이면 batch 단위로 cache lookup → miss만 fastembed 호출 → put.
/// `cache=None`이면 기존 인덱서 흐름과 동일.
pub fn index_book_with_cache(
    conn: &mut Connection,
    book_id: &str,
    src: BookSource<'_>,
    embedder: Option<&Embedder>,
    app_data_dir: &Path,
    cache: Option<&EmbeddingCache>,
) -> AppResult<IndexOutcome> {
    // app_data_dir는 embedder가 None이면 사용되지 않음 — 시그니처 일관성을 위해 받음.
    // PR 4가 명시 reindex 시점에 항상 Some(embedder)로 호출 (D-076 직렬 큐).
    let _ = app_data_dir;

    // 1. indexing_jobs row 생성.
    let job_id = create_running_job(conn, book_id)?;

    // 2. chunker.
    let records: Vec<ChunkRecord> = match src {
        BookSource::Sections(sections) => chunk_md_sections(sections),
        BookSource::Pages(pages) => chunk_pdf_pages(pages),
    };

    let total = records.len();
    set_total_chunks(conn, job_id, total)?;

    if records.is_empty() {
        finalize_job(conn, job_id, 0, JobStatus::Completed, None)?;
        return Ok(IndexOutcome {
            job_id,
            chunks_inserted: 0,
            embeddings_inserted: 0,
        });
    }

    // 3. 트랜잭션 안에서 INSERT 2-pass + (옵션) 임베딩 pass 3.
    let result = (|| -> AppResult<usize> {
        // vec0 가상 테이블이 임베딩 적재 *전*에 존재해야 한다. 트랜잭션 시작 전에 ensure.
        if embedder.is_some() {
            ensure_vec0(conn)?;
        }

        let tx = conn.transaction()?;
        // v0.6.x — 멱등 재인덱싱: 적재 전 이 책의 기존 청크를 정리한다. 이게 없으면
        // reindex(또는 add-flow 재진입)마다 chunks가 *중복 누적*된다 (실측 4× 중복 발견).
        // FK CASCADE(vectors_t1/t2·chunk_entities) + 트리거(chunks_fts)는 chunks DELETE로
        // 자동 정리되지만, vec0 가상 테이블은 CASCADE가 안 되므로 rowid로 먼저 삭제.
        clear_existing_chunks(&tx, book_id)?;
        let id_by_ord = insert_chunks_pass1(&tx, book_id, &records)?;
        update_chunks_pass2(&tx, &records, &id_by_ord)?;

        // 4. 임베딩 — embedder가 주어지면 batch 단위로 진행률 갱신.
        let embeddings_inserted = if let Some(emb) = embedder {
            embed_pass3(&tx, &records, &id_by_ord, emb, job_id, cache)?
        } else {
            0
        };

        tx.commit()?;
        Ok(embeddings_inserted)
    })();

    match result {
        Ok(embeddings_inserted) => {
            finalize_job(conn, job_id, total, JobStatus::Completed, None)?;
            // v0.6.x (D-111) — 경량 GraphRAG 엔티티 인덱스 구축. 인덱싱 *완료 후* 별도 단계
            // (5분 임베딩 예산 밖). 실패해도 인덱싱은 성공으로 둔다 — 그래프는 검색 보강용
            // 부가 기능이라 graceful. 엔티티 인덱스가 없으면 검색 시 graph 확장이 no-op.
            match crate::index::v060::graph::rebuild_book_entities(conn, book_id) {
                Ok(n) => tracing::debug!(
                    target: "v060.graph",
                    book_id,
                    entity_rows = n,
                    "엔티티 인덱스 구축 완료"
                ),
                Err(e) => tracing::warn!(
                    target: "v060.graph",
                    book_id,
                    error = %e,
                    "엔티티 인덱스 구축 실패 — graph 확장 비활성 (검색은 정상)"
                ),
            }
            Ok(IndexOutcome {
                job_id,
                chunks_inserted: total,
                embeddings_inserted,
            })
        }
        Err(e) => {
            // 실패 — 트랜잭션 자동 롤백, jobs row만 갱신.
            let msg = format!("{e}");
            finalize_job(conn, job_id, 0, JobStatus::Failed, Some(&msg))?;
            Err(e)
        }
    }
}

// ----- 멱등 재인덱싱 — 기존 청크 정리 -------------------------------------

/// 해당 책의 기존 청크 + 의존 행을 정리한다. 재인덱싱/재진입 시 중복 누적 방지.
///
/// 순서: vec0 가상 테이블(`vectors_t1_vec0`/`vectors_t2_vec0`)은 FK CASCADE 대상이
/// 아니므로 chunk id로 *먼저* 삭제. 그다음 `chunks` DELETE가 vectors_t1/vectors_t2/
/// chunk_entities(FK CASCADE) + chunks_fts(트리거)를 연쇄 정리한다.
fn clear_existing_chunks(conn: &Connection, book_id: &str) -> AppResult<()> {
    for vec0 in ["vectors_t1_vec0", "vectors_t2_vec0"] {
        if table_exists(conn, vec0)? {
            conn.execute(
                &format!(
                    "DELETE FROM {vec0} WHERE rowid IN \
                     (SELECT id FROM chunks WHERE book_id = ?1)"
                ),
                params![book_id],
            )?;
        }
    }
    conn.execute("DELETE FROM chunks WHERE book_id = ?1", params![book_id])?;
    Ok(())
}

/// sqlite_master에 해당 이름의 테이블이 존재하는지 (vec0는 lazy 생성이라 부재 가능).
fn table_exists(conn: &Connection, name: &str) -> AppResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
        params![name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

// ----- jobs row 헬퍼 -------------------------------------------------------

fn create_running_job(conn: &Connection, book_id: &str) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO indexing_jobs (book_id, status, tier, progress_chunks, started_at) \
         VALUES (?1, 'running', 1, 0, CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
        params![book_id],
    )?;
    Ok(conn.last_insert_rowid())
}

fn set_total_chunks(conn: &Connection, job_id: i64, total: usize) -> AppResult<()> {
    conn.execute(
        "UPDATE indexing_jobs SET total_chunks = ?1 WHERE id = ?2",
        params![total as i64, job_id],
    )?;
    Ok(())
}

fn finalize_job(
    conn: &Connection,
    job_id: i64,
    progress: usize,
    status: JobStatus,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE indexing_jobs SET \
             status = ?1, \
             progress_chunks = ?2, \
             finished_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
             error = ?3 \
         WHERE id = ?4",
        params![status.as_db_str(), progress as i64, error, job_id],
    )?;
    Ok(())
}

// ----- chunks INSERT 2-pass ------------------------------------------------

/// pass 1: parent/prev/next 모두 NULL로 chunks INSERT. ord → 실제 chunks.id 매핑 반환.
///
/// HashMap을 쓰지 않고 Vec<i64>로 반환 — ord는 dense 0-base라 인덱스 자체가 키.
fn insert_chunks_pass1(
    tx: &Transaction<'_>,
    book_id: &str,
    records: &[ChunkRecord],
) -> AppResult<Vec<i64>> {
    let mut stmt = tx.prepare(
        "INSERT INTO chunks \
            (book_id, ord, text, page, span_start, span_end, \
             parent_id, prev_chunk_id, next_chunk_id, section_path, token_count) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, ?7, ?8)",
    )?;

    let mut ids = Vec::with_capacity(records.len());
    for r in records {
        stmt.execute(params![
            book_id,
            r.ord as i64,
            r.text,
            r.page.map(|p| p as i64),
            r.span_start.map(|x| x as i64),
            r.span_end.map(|x| x as i64),
            r.section_path,
            r.token_count as i64,
        ])?;
        ids.push(tx.last_insert_rowid());
    }
    Ok(ids)
}

/// pass 2: parent/prev/next를 ord 인덱스 → 실제 chunks.id로 변환해 UPDATE.
fn update_chunks_pass2(
    tx: &Transaction<'_>,
    records: &[ChunkRecord],
    id_by_ord: &[i64],
) -> AppResult<()> {
    let mut stmt = tx.prepare(
        "UPDATE chunks SET parent_id = ?1, prev_chunk_id = ?2, next_chunk_id = ?3 WHERE id = ?4",
    )?;

    for r in records {
        let parent = r.parent_ord.map(|o| id_by_ord[o]);
        let prev = r.prev_ord.map(|o| id_by_ord[o]);
        let next = r.next_ord.map(|o| id_by_ord[o]);
        let id = id_by_ord[r.ord];
        stmt.execute(params![parent, prev, next, id])?;
    }
    Ok(())
}

/// pass 3: passage prefix 적용한 본문을 batch(=`EMBED_BATCH`) 단위로 임베딩 → vec0 + vectors_t1
/// upsert. progress_chunks를 batch마다 갱신.
///
/// 같은 트랜잭션을 공유 — 임베딩이 실패하면 chunks 적재까지 롤백.
///
/// v0.4.2 PR 4 (D-084): `cache=Some(...)` 이면 batch 단위 cache lookup →
/// miss만 fastembed 호출 → put. 같은 텍스트 재인덱싱 시 fastembed 호출 절감.
/// trade-off: cache lookup 시점은 트랜잭션 안 — embed_passages가 트랜잭션 안에서
/// `&Connection` 매개변수와 충돌하지 않게 lookup/put 모두 같은 트랜잭션 핸들 사용.
fn embed_pass3(
    tx: &Transaction<'_>,
    records: &[ChunkRecord],
    id_by_ord: &[i64],
    embedder: &Embedder,
    job_id: i64,
    cache: Option<&EmbeddingCache>,
) -> AppResult<usize> {
    let mut total_inserted = 0_usize;
    let mut progress = 0_usize;

    for batch in records.chunks(EMBED_BATCH) {
        // prefixed text는 cache 키(=원본 text + ':' + model)와 분리. cache는 *raw text 기준*.
        // 같은 raw text는 같은 prefix가 붙으므로 prefix를 키에 포함시키지 않아도 등가.
        let raw_texts: Vec<String> = batch.iter().map(|r| r.text.clone()).collect();
        let mut vectors: Vec<Vec<f32>> = vec![Vec::new(); batch.len()];
        let mut miss_indices: Vec<usize> = Vec::new();

        if let Some(c) = cache {
            let items: Vec<(String, String)> = raw_texts
                .iter()
                .map(|t| (t.clone(), T1_MODEL_ID.to_string()))
                .collect();
            let cached = c.get_batch(tx, &items)?;
            for (i, slot) in cached.into_iter().enumerate() {
                if let Some(v) = slot {
                    if v.len() == Embedder::DIM {
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

        // miss만 fastembed 호출.
        if !miss_indices.is_empty() {
            let prefixed: Vec<String> = miss_indices
                .iter()
                .map(|&i| passage_prefix(&raw_texts[i]))
                .collect();
            let new_vecs = embedder.embed_passages(&prefixed)?;
            if new_vecs.len() != miss_indices.len() {
                return Err(crate::error::AppError::Internal {
                    message: format!(
                        "embed_pass3: fastembed가 {} 입력에 {} 결과 반환",
                        miss_indices.len(),
                        new_vecs.len()
                    ),
                });
            }
            for (slot_idx, v) in miss_indices.iter().zip(new_vecs) {
                if let Some(c) = cache {
                    // 같은 트랜잭션 안에서 cache put — 인덱서 트랜잭션이 commit돼야 cache row도 영속.
                    c.put(tx, &raw_texts[*slot_idx], T1_MODEL_ID, Embedder::DIM, &v)?;
                }
                vectors[*slot_idx] = v;
            }
        }

        for (rec, v) in batch.iter().zip(vectors.iter()) {
            let chunk_id = id_by_ord[rec.ord];
            upsert_embedding(tx, chunk_id, v)?;
            total_inserted += 1;
        }

        progress += batch.len();
        tx.execute(
            "UPDATE indexing_jobs SET progress_chunks = ?1 WHERE id = ?2",
            params![progress as i64, job_id],
        )?;
    }
    Ok(total_inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::types::SectionLevel;
    use rusqlite::Connection;

    /// 인메모리 DB에 v1~v13 일괄 적용 — db_smoke 테스트 패턴 그대로.
    /// 본 단위 테스트는 sqlite-vec 등록 X — chunks 적재만 검증해서 vec0 불필요.
    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("FK on");

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

        // FK 만족용 study + book.
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
        conn
    }

    fn mk_section(path: &str, body: &str) -> Section {
        Section {
            path: path.to_string(),
            display_label: path.to_string(),
            level: SectionLevel::Section,
            parent_path: None,
            page: None,
            body: body.to_string(),
        }
    }

    #[test]
    fn reindexing_is_idempotent_no_duplicate_chunks() {
        // v0.6.x 회귀 가드 — 재인덱싱이 청크를 중복 누적하면 안 된다 (실측 4× 중복 버그 수정).
        let mut conn = fresh_db();
        let sections = vec![mk_section("Ch01/§A", &"가나다라마 ".repeat(2000))];
        let first =
            index_book(&mut conn, "b1", BookSource::Sections(&sections), None, Path::new("/tmp"))
                .unwrap();
        assert!(first.chunks_inserted >= 2, "큰 섹션은 ≥2 청크");

        // 같은 책 재인덱싱.
        let second =
            index_book(&mut conn, "b1", BookSource::Sections(&sections), None, Path::new("/tmp"))
                .unwrap();
        assert_eq!(
            second.chunks_inserted, first.chunks_inserted,
            "재인덱싱 청크 수는 동일해야"
        );

        // DB 실제 총량 == 1회 분량 (중복 누적 X).
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks WHERE book_id='b1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total as usize, second.chunks_inserted, "중복 누적 없음");
        // ord 중복 없음.
        let distinct: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT ord) FROM chunks WHERE book_id='b1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(distinct, total, "ord 중복 없음");
    }

    #[test]
    fn empty_source_completes_with_zero_chunks() {
        let mut conn = fresh_db();
        let outcome =
            index_book(&mut conn, "b1", BookSource::Sections(&[]), None, Path::new("/tmp")).unwrap();
        assert_eq!(outcome.chunks_inserted, 0);
        assert_eq!(outcome.embeddings_inserted, 0);
        // jobs 테이블에 'completed' row.
        let status: String = conn
            .query_row(
                "SELECT status FROM indexing_jobs WHERE id = ?1",
                params![outcome.job_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
    }

    #[test]
    fn small_md_section_inserts_one_chunk_with_fts() {
        let mut conn = fresh_db();
        let sections = vec![mk_section(
            "Ch01/§Intro",
            "Rust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다.",
        )];
        let outcome = index_book(
            &mut conn,
            "b1",
            BookSource::Sections(&sections),
            None,
            Path::new("/tmp"),
        )
        .unwrap();
        assert_eq!(outcome.chunks_inserted, 1);

        // chunks 적재 확인.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE book_id='b1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // FTS 트리거 동기화 검증 — 'ownership' 키워드 검색.
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH 'ownership'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);

        // jobs 마무리 status.
        let job: (String, i64, Option<i64>) = conn
            .query_row(
                "SELECT status, progress_chunks, total_chunks FROM indexing_jobs \
                 WHERE id = ?1",
                params![outcome.job_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(job.0, "completed");
        assert_eq!(job.1, 1);
        assert_eq!(job.2, Some(1));
    }

    #[test]
    fn large_md_section_links_parent_and_neighbors() {
        let mut conn = fresh_db();
        // 청크 분할 강제 — 8000+자 본문.
        let body: String = (0..200)
            .map(|i| format!("문장 {i}번이고 본문이 길게 이어집니다. "))
            .collect::<String>()
            .repeat(4);
        let sections = vec![mk_section("Ch01/§Big", &body)];
        let outcome = index_book(
            &mut conn,
            "b1",
            BookSource::Sections(&sections),
            None,
            Path::new("/tmp"),
        )
        .unwrap();
        assert!(
            outcome.chunks_inserted >= 2,
            "8000+자는 최소 2 청크. 실제 {}",
            outcome.chunks_inserted
        );

        // ord 0 = 부모 (parent_id IS NULL).
        let parent: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM chunks WHERE book_id='b1' AND ord=0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(parent.is_none(), "첫 청크의 parent_id는 NULL");

        // ord 1 의 parent = ord 0 의 chunks.id.
        let row0_id: i64 = conn
            .query_row(
                "SELECT id FROM chunks WHERE book_id='b1' AND ord=0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let row1_parent: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM chunks WHERE book_id='b1' AND ord=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(row1_parent, Some(row0_id));

        // ord 1 의 prev = ord 0.
        let row1_prev: Option<i64> = conn
            .query_row(
                "SELECT prev_chunk_id FROM chunks WHERE book_id='b1' AND ord=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(row1_prev, Some(row0_id));

        // ord 0 의 next = ord 1 의 id.
        let row1_id: i64 = conn
            .query_row(
                "SELECT id FROM chunks WHERE book_id='b1' AND ord=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let row0_next: Option<i64> = conn
            .query_row(
                "SELECT next_chunk_id FROM chunks WHERE book_id='b1' AND ord=0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(row0_next, Some(row1_id));

        // 마지막 청크의 next_chunk_id IS NULL.
        let max_ord: i64 = conn
            .query_row(
                "SELECT MAX(ord) FROM chunks WHERE book_id='b1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let last_next: Option<i64> = conn
            .query_row(
                "SELECT next_chunk_id FROM chunks WHERE book_id='b1' AND ord = ?1",
                params![max_ord],
                |r| r.get(0),
            )
            .unwrap();
        assert!(last_next.is_none());
    }

    #[test]
    fn pdf_pages_indexer_uses_page_metadata() {
        let mut conn = fresh_db();
        let pages = vec![
            "첫 페이지의 본문입니다. 검색 키워드로 사용할 한국어 산문.".to_string(),
            "두 번째 페이지에는 ownership 영문 토큰도 포함됩니다.".to_string(),
        ];
        let outcome = index_book(
            &mut conn,
            "b1",
            BookSource::Pages(&pages),
            None,
            Path::new("/tmp"),
        )
        .unwrap();
        assert_eq!(outcome.chunks_inserted, 2);

        let (page0, path0): (Option<i64>, String) = conn
            .query_row(
                "SELECT page, section_path FROM chunks WHERE book_id='b1' AND ord=0",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(page0, Some(1));
        assert_eq!(path0, "p.1");

        let (page1, path1): (Option<i64>, String) = conn
            .query_row(
                "SELECT page, section_path FROM chunks WHERE book_id='b1' AND ord=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(page1, Some(2));
        assert_eq!(path1, "p.2");

        // 페이지 사이엔 prev/next/parent 모두 NULL (서로 다른 부모).
        let (parent, prev, next): (Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT parent_id, prev_chunk_id, next_chunk_id \
                 FROM chunks WHERE book_id='b1' AND ord=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(parent.is_none(), "페이지 부모 누수 X");
        assert!(prev.is_none(), "페이지 사이 prev 연결 X");
        assert!(next.is_none());
    }
}
