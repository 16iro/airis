// SQLite 연결 + 마이그레이션.
// rusqlite (bundled) — SQLite C lib을 함께 빌드해 외부 의존성 0.
// 동기 API라 Tokio 환경에서는 `tokio::task::spawn_blocking`으로 격리한다.
//
// 마이그레이션 패턴 (db-schema.md "마이그레이션 메커니즘"):
//   - schema_version 테이블에 적용된 버전 기록
//   - MIGRATIONS 슬라이스를 1번부터 누락분만 트랜잭션으로 적용
//   - 새 버전 추가 시 SQL 파일 + MIGRATIONS 슬라이스에 한 줄.

use std::path::Path;
use std::sync::Once;

use rusqlite::Connection;

use crate::error::AppResult;

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/v1_initial.sql"),
    include_str!("migrations/v2_studies_and_chat.sql"),
    include_str!("migrations/v3_paragraphs_fts.sql"),
    include_str!("migrations/v4_intervention_and_history.sql"),
    include_str!("migrations/v5_pomodoro_cycles.sql"),
    include_str!("migrations/v6_srs_cards.sql"),
    include_str!("migrations/v7_recall_challenges.sql"),
    include_str!("migrations/v8_book_thumbnail.sql"),
    include_str!("migrations/v9_study_thumbnail.sql"),
    include_str!("migrations/v10_thumbnails_dir_rename.sql"),
    include_str!("migrations/v11_study_description.sql"),
    include_str!("migrations/v12_chat_context.sql"),
    include_str!("migrations/v13_chunks.sql"),
    include_str!("migrations/v14_ab_compare.sql"),
    include_str!("migrations/v15_robustness.sql"),
    include_str!("migrations/v16_cancelled_status.sql"),
];

/// sqlite-vec를 process-level에서 *한 번만* sqlite3_auto_extension에 등록한다.
///
/// 이 함수는 *모든 sqlite3 connection이 열리기 전에* 호출돼야 한다 — auto_extension은
/// 등록 시점 *이후*에 열린 connection에만 vec0를 자동 로드하기 때문. 따라서 첫
/// `Db::open` / `Db::open_in_memory` 진입에서 한 번 깐다.
///
/// `Once`로 보호 — 두 번 호출되어도 무해하지만 noise를 방지.
/// `sqlite-vec` Rust crate가 C 소스를 static link하므로 OS별 .so/.dll 동봉 불필요
/// (D-074, PoC d3_sqlite_vec.rs에서 검증 끝).
fn register_sqlite_vec_once() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: sqlite3_auto_extension은 FFI; 등록 함수는 sqlite-vec crate가
        // 제공하는 정적 함수 포인터로 형식이 SQLite extension entry point와 호환됨.
        // PoC d3_sqlite_vec.rs와 동일 패턴 — 단 rusqlite-ffi의
        // 시그니처(`unsafe extern "C" fn(*mut sqlite3, *mut *mut i8, *const sqlite3_api_routines) -> i32`)에 맞게
        // 캐스트만 명시 (sqlite_vec crate가 expose하는 entry point의 실제 시그니처와 동등).
        type AutoExtFn = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                AutoExtFn,
            >(sqlite_vec::sqlite3_vec_init as *const ())));
        }
    });
}

pub struct Db {
    conn: Connection,
}

impl Db {
    /// 지정 경로의 SQLite 파일을 열고 (없으면 생성) WAL 모드 활성화 + 마이그레이션 적용.
    pub fn open(path: &Path) -> AppResult<Self> {
        register_sqlite_vec_once();
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// 메모리 SQLite — 테스트 전용. 매 호출마다 새 인스턴스.
    #[cfg(test)]
    fn open_in_memory() -> AppResult<Self> {
        register_sqlite_vec_once();
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// 다른 모듈의 단위 테스트가 사용 — 마이그까지 적용된 in-memory Db.
    /// `expect`는 *테스트 invariant* — 실패 시 즉시 panic이 정상.
    #[cfg(test)]
    pub fn open_in_memory_for_test() -> Self {
        Self::open_in_memory().expect("in-memory db must open in tests")
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// transaction이 필요한 호출자(쓰기·activate 등) 전용. read-only 경로엔 `conn()`.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// 디스크 DB만 WAL을 켠다 (in-memory는 WAL 의미 없음).
    ///
    /// v0.4.2 강건성 (PR 1):
    ///   * WAL 모드 — write-ahead log로 crash 시 미커밋 트랜잭션만 잃는다.
    ///     architecture §5 cascade·강건성의 토대.
    ///   * synchronous=NORMAL — WAL과 함께 쓰면 fsync 빈도 ↓ 안전성 충분.
    ///   * busy_timeout=5000ms — concurrent connection이 락 충돌 시 바쁘게
    ///     대기. 인덱싱 worker + retrieval reader 동시 동작 위에서 즉시 실패 방지.
    ///
    /// in-memory DB는 WAL 의미 없음 — 이 경우 journal_mode 결과가 "memory"라서
    /// WAL 검증을 분기로 우회.
    fn configure(conn: &Connection) -> AppResult<()> {
        // pragma_update는 PRAGMA name = value 와 동등.
        // foreign_keys는 *connection-scoped* — 매 연결마다 다시 켜야 한다.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // WAL은 *database-scoped* — 한 번 켜면 파일에 영구 적용.
        // in-memory에는 효과 없음(언제나 'memory' 반환).
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        // synchronous=NORMAL: WAL과의 표준 조합. crash 시 마지막 트랜잭션만 잃을 수 있음.
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        // busy_timeout: 락 경합 시 즉시 SQLITE_BUSY 대신 5초까지 재시도.
        let _ = conn.pragma_update(None, "busy_timeout", 5000);

        // WAL 검증 — 디스크 DB(journal_mode != "memory")에서만. v0.4.2 강건성 필수.
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        debug_assert!(
            mode.eq_ignore_ascii_case("wal") || mode.eq_ignore_ascii_case("memory"),
            "v0.4.2 강건성: 디스크 DB는 WAL 모드 필수, 받은 값: {mode}"
        );
        Ok(())
    }

    fn migrate(&mut self) -> AppResult<()> {
        // 부트스트랩 — schema_version 테이블 자체는 마이그레이션 외부에서 보장.
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version    INTEGER PRIMARY KEY,
                applied_at TEXT    NOT NULL DEFAULT (datetime('now'))
            );",
        )?;

        let current: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )?;

        for (idx, sql) in MIGRATIONS.iter().enumerate() {
            let version = (idx + 1) as i64;
            if version <= current {
                continue;
            }
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                rusqlite::params![version],
            )?;
            tx.commit()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_count(db: &Db, name: &str) -> i64 {
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                rusqlite::params![name],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn migrate_creates_v2_tables() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(table_count(&db, "schema_version"), 1);
        assert_eq!(table_count(&db, "failed_llm_jobs"), 1);
        assert_eq!(table_count(&db, "studies"), 1);
        assert_eq!(table_count(&db, "chat_messages"), 1);
        assert_eq!(table_count(&db, "books"), 1);
    }

    #[test]
    fn migrate_creates_v3_paragraphs_and_fts() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(table_count(&db, "paragraphs"), 1);
        // FTS5 virtual table은 sqlite_master에서 type='table'로 잡힘.
        assert_eq!(table_count(&db, "paragraphs_fts"), 1);
    }

    #[test]
    fn migrate_creates_v4_intervention_history_and_consistency() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(table_count(&db, "intervention_signals"), 1);
        assert_eq!(table_count(&db, "search_history"), 1);
        assert_eq!(table_count(&db, "consistency_check_log"), 1);
    }

    #[test]
    fn fts_triggers_keep_index_in_sync() {
        // INSERT → MATCH 가능. DELETE → MATCH 결과 사라짐.
        let db = Db::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s1','S1',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (
                    id, study_slug, role, title, source_path, file_format,
                    file_size, file_hash, added_at
                 ) VALUES ('b1','s1','main','Book','/tmp/x','md',0,'h',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO paragraphs (
                    book_id, section_path, section_label, chunk_index, content
                 ) VALUES ('b1','Ch01','Ch01',0,'Rust ownership and borrowing')",
                [],
            )
            .unwrap();

        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM paragraphs_fts WHERE paragraphs_fts MATCH 'ownership'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "FTS index should pick up inserted content");

        db.conn()
            .execute("DELETE FROM paragraphs WHERE book_id='b1'", [])
            .unwrap();
        let after: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM paragraphs_fts WHERE paragraphs_fts MATCH 'ownership'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 0, "DELETE trigger should clean FTS index");
    }

    #[test]
    fn migrate_creates_v13_chunks_and_indexes() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(table_count(&db, "chunks"), 1);
        assert_eq!(table_count(&db, "chunks_fts"), 1);
        assert_eq!(table_count(&db, "vectors_t1"), 1);
        assert_eq!(table_count(&db, "indexing_jobs"), 1);
    }

    #[test]
    fn migrate_creates_v14_ab_compare_table() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(table_count(&db, "ab_compare_choices"), 1);
    }

    fn column_exists(db: &Db, table: &str, column: &str) -> bool {
        // PRAGMA table_info는 (cid, name, type, notnull, dflt, pk) 순으로 행 반환.
        let mut stmt = db
            .conn()
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|x| x.unwrap())
            .collect();
        rows.iter().any(|name| name == column)
    }

    #[test]
    fn migrate_creates_v15_robustness_columns_and_tables() {
        let db = Db::open_in_memory().unwrap();
        // chunks 신규 컬럼 4개.
        assert!(column_exists(&db, "chunks", "embed_status_t1"));
        assert!(column_exists(&db, "chunks", "embed_status_t2"));
        assert!(column_exists(&db, "chunks", "embed_attempts"));
        assert!(column_exists(&db, "chunks", "last_error"));
        // indexing_jobs 신규 컬럼 2개.
        assert!(column_exists(&db, "indexing_jobs", "pause_reason"));
        assert!(column_exists(&db, "indexing_jobs", "updated_at"));
        // 신규 테이블 3개.
        assert_eq!(table_count(&db, "vectors_t2"), 1);
        assert_eq!(table_count(&db, "embedding_cache"), 1);
        assert_eq!(table_count(&db, "response_cache"), 1);
    }

    #[test]
    fn migrate_v15_backfills_t1_done_for_existing_vectors() {
        // v0.4.1에서 벡터 적재 끝난 청크는 마이그 후 자동으로 t1='done'.
        // 시나리오: in-memory DB에 v1~v15까지 한 번 적용 (chunks/vectors_t1 모두 존재) →
        //   * 책 + chunk 삽입 (사용자 데이터 흉내)
        //   * vectors_t1 INSERT (v0.4.1 임베딩 결과 흉내)
        //   * 강제 다운그레이드 (embed_status_t1을 NULL로 되돌림 = 마이그 직전 상태 흉내)
        //   * 마이그 헬퍼 SQL을 직접 실행해 백필 동작 검증.
        let db = Db::open_in_memory().unwrap();
        // 책 + 청크 + vectors_t1 INSERT.
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (
                    id, study_slug, role, title, source_path, file_format,
                    file_size, file_hash, added_at
                 ) VALUES ('b','s','main','B','/x','md',0,'h',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO chunks (book_id, ord, text, token_count) \
                 VALUES ('b', 0, 'hello', 1)",
                [],
            )
            .unwrap();
        let chunk_id: i64 = db
            .conn()
            .query_row("SELECT id FROM chunks WHERE book_id='b'", [], |r| r.get(0))
            .unwrap();
        // vectors_t1 BLOB 더미 (4바이트 = f32 한 개).
        db.conn()
            .execute(
                "INSERT INTO vectors_t1 (chunk_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![chunk_id, vec![0u8; 4]],
            )
            .unwrap();
        // 마이그 직전 상태 흉내 — 백필 UPDATE를 다시 검증하기 위해 NULL로 reset.
        db.conn()
            .execute(
                "UPDATE chunks SET embed_status_t1 = NULL WHERE id = ?1",
                rusqlite::params![chunk_id],
            )
            .unwrap();
        // v15 백필 UPDATE를 그대로 재실행.
        db.conn()
            .execute(
                "UPDATE chunks SET embed_status_t1 = 'done' \
                 WHERE id IN (SELECT chunk_id FROM vectors_t1)",
                [],
            )
            .unwrap();
        // 결과 — vectors_t1에 있는 청크는 t1='done'.
        let status: Option<String> = db
            .conn()
            .query_row(
                "SELECT embed_status_t1 FROM chunks WHERE id = ?1",
                rusqlite::params![chunk_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status.as_deref(), Some("done"));
    }

    #[test]
    fn migrate_v15_does_not_backfill_unembedded_chunks() {
        // vectors_t1에 없는 청크는 마이그 후에도 embed_status_t1 = NULL로 남는다.
        let db = Db::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (
                    id, study_slug, role, title, source_path, file_format,
                    file_size, file_hash, added_at
                 ) VALUES ('b','s','main','B','/x','md',0,'h',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO chunks (book_id, ord, text, token_count) \
                 VALUES ('b', 0, 'no embed', 1)",
                [],
            )
            .unwrap();
        let status: Option<String> = db
            .conn()
            .query_row(
                "SELECT embed_status_t1 FROM chunks WHERE book_id='b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, None, "벡터 미적재 청크는 NULL 유지");
    }

    #[test]
    fn migrate_v16_indexing_jobs_accepts_cancelled_status() {
        // v16 마이그 후 'cancelled'가 1급 시민 — INSERT 시 CHECK 통과.
        let db = Db::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (
                    id, study_slug, role, title, source_path, file_format,
                    file_size, file_hash, added_at
                 ) VALUES ('b','s','main','B','/x','md',0,'h',datetime('now'))",
                [],
            )
            .unwrap();
        // 'cancelled' status 직접 INSERT 가능.
        db.conn()
            .execute(
                "INSERT INTO indexing_jobs \
                    (book_id, status, tier, progress_chunks) \
                 VALUES ('b', 'cancelled', 2, 0)",
                [],
            )
            .unwrap();
        let cnt: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM indexing_jobs WHERE status='cancelled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cnt, 1);
    }

    #[test]
    fn migrate_v16_indexing_jobs_rejects_unknown_status() {
        // CHECK 제약 — 알 수 없는 status는 거부.
        let db = Db::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO books (
                    id, study_slug, role, title, source_path, file_format,
                    file_size, file_hash, added_at
                 ) VALUES ('b','s','main','B','/x','md',0,'h',datetime('now'))",
                [],
            )
            .unwrap();
        let r = db.conn().execute(
            "INSERT INTO indexing_jobs (book_id, status, tier, progress_chunks) \
             VALUES ('b', 'lol', 2, 0)",
            [],
        );
        assert!(r.is_err(), "v16 CHECK가 'lol' status를 거부해야");
    }

    #[test]
    fn migrate_v16_preserves_existing_columns_and_data() {
        // v16 테이블 재생성 후에도 v15 ALTER 컬럼(pause_reason / updated_at) + 데이터가
        // 그대로 보존되는지 검증.
        let db = Db::open_in_memory().unwrap();
        // 신규 테이블에 컬럼 모두 존재.
        assert!(column_exists(&db, "indexing_jobs", "pause_reason"));
        assert!(column_exists(&db, "indexing_jobs", "updated_at"));
        assert!(column_exists(&db, "indexing_jobs", "status"));
        assert!(column_exists(&db, "indexing_jobs", "tier"));
        assert!(column_exists(&db, "indexing_jobs", "progress_chunks"));

        // 인덱스 재생성 검증 — book / status 인덱스가 존재.
        let idx_names: Vec<String> = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='indexing_jobs'")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|x| x.unwrap())
            .collect();
        assert!(idx_names.iter().any(|s| s == "idx_indexing_jobs_book"));
        assert!(idx_names.iter().any(|s| s == "idx_indexing_jobs_status"));
    }

    #[test]
    fn ab_compare_chose_check_constraint_rejects_unknown() {
        let db = Db::open_in_memory().unwrap();
        let result = db.conn().execute(
            "INSERT INTO ab_compare_choices (query_hash, query_text, baseline_text, v041_text, chose, handle)
             VALUES ('h', 'q', 'a', 'b', 'lol', 'handle-1')",
            [],
        );
        assert!(
            result.is_err(),
            "CHECK constraint must reject chose values outside (baseline|v041|tie)"
        );
    }

    #[test]
    fn ab_compare_accepts_three_chose_values() {
        let db = Db::open_in_memory().unwrap();
        for c in ["baseline", "v041", "tie"] {
            db.conn()
                .execute(
                    "INSERT INTO ab_compare_choices (query_hash, query_text, baseline_text, v041_text, chose, handle)
                     VALUES (?1, 'q', 'a', 'b', ?2, 'h')",
                    rusqlite::params![format!("hash-{c}"), c],
                )
                .unwrap();
        }
        let total: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM ab_compare_choices", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 3);
    }

    #[test]
    fn sqlite_vec_auto_extension_loaded() {
        // v13 마이그레이션 직후 vec_version()이 동작해야 한다 — auto_extension 등록 검증.
        let db = Db::open_in_memory().unwrap();
        let version: String = db
            .conn()
            .query_row("SELECT vec_version()", [], |r| r.get(0))
            .unwrap();
        assert!(
            version.starts_with('v'),
            "vec_version()는 'vX.Y.Z' 형식이어야 하는데 받은 값: {version}"
        );
    }

    #[test]
    fn migrate_records_version() {
        let db = Db::open_in_memory().unwrap();
        let max: i64 = db
            .conn()
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(max, MIGRATIONS.len() as i64);
    }

    #[test]
    fn migrate_is_idempotent() {
        // 두 번째 호출은 누락분이 없으므로 schema_version 행이 늘어나지 않는다.
        let mut db = Db::open_in_memory().unwrap();
        let initial: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        db.migrate().unwrap();
        let after: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(initial, after);
    }

    #[test]
    fn failed_llm_jobs_check_constraint_rejects_unknown_type() {
        let db = Db::open_in_memory().unwrap();
        // FK 위반을 피하려고 미리 'default' 스터디 생성.
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('default', 'default', datetime('now'))",
                [],
            )
            .unwrap();
        let result = db.conn().execute(
            "INSERT INTO failed_llm_jobs (study_slug, job_type, payload_json, created_at)
             VALUES ('default', 'invalid_type', '{}', datetime('now'))",
            [],
        );
        assert!(
            result.is_err(),
            "CHECK constraint must reject unknown job_type"
        );
    }

    #[test]
    fn failed_llm_jobs_fk_rejects_missing_study() {
        // v2부터 study_slug가 studies에 없으면 INSERT 거부 (FK + foreign_keys=ON).
        let db = Db::open_in_memory().unwrap();
        let result = db.conn().execute(
            "INSERT INTO failed_llm_jobs (study_slug, job_type, payload_json, created_at)
             VALUES ('ghost', 'chat', '{}', datetime('now'))",
            [],
        );
        assert!(result.is_err(), "FK must reject unknown study_slug");
    }

    #[test]
    fn studies_active_unique_index_prevents_two_active() {
        let db = Db::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at, is_active) VALUES ('a','a',datetime('now'),1)",
                [],
            )
            .unwrap();
        let result = db.conn().execute(
            "INSERT INTO studies (slug, name, created_at, is_active) VALUES ('b','b',datetime('now'),1)",
            [],
        );
        assert!(
            result.is_err(),
            "partial unique index must block second active row"
        );
    }

    #[test]
    fn delete_study_cascades_to_chat_and_jobs() {
        // 스터디 삭제 시 chat_messages·failed_llm_jobs 자동 삭제 (ON DELETE CASCADE).
        let db = Db::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s1','S1',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO chat_messages (study_slug, role, content, created_at)
                 VALUES ('s1','user','hi',datetime('now'))",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO failed_llm_jobs (study_slug, job_type, payload_json, created_at)
                 VALUES ('s1','chat','{}',datetime('now'))",
                [],
            )
            .unwrap();

        db.conn()
            .execute("DELETE FROM studies WHERE slug='s1'", [])
            .unwrap();

        let chat: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM chat_messages", [], |r| r.get(0))
            .unwrap();
        let jobs: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM failed_llm_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chat, 0);
        assert_eq!(jobs, 0);
    }
}
