// SQLite 연결 + 마이그레이션.
// rusqlite (bundled) — SQLite C lib을 함께 빌드해 외부 의존성 0.
// 동기 API라 Tokio 환경에서는 `tokio::task::spawn_blocking`으로 격리한다.
//
// 마이그레이션 패턴 (db-schema.md "마이그레이션 메커니즘"):
//   - schema_version 테이블에 적용된 버전 기록
//   - MIGRATIONS 슬라이스를 1번부터 누락분만 트랜잭션으로 적용
//   - 새 버전 추가 시 SQL 파일 + MIGRATIONS 슬라이스에 한 줄.

use std::path::Path;

use rusqlite::Connection;

use crate::error::AppResult;

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/v1_initial.sql"),
    include_str!("migrations/v2_studies_and_chat.sql"),
    include_str!("migrations/v3_paragraphs_fts.sql"),
];

pub struct Db {
    conn: Connection,
}

impl Db {
    /// 지정 경로의 SQLite 파일을 열고 (없으면 생성) WAL 모드 활성화 + 마이그레이션 적용.
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// 메모리 SQLite — 테스트 전용. 매 호출마다 새 인스턴스.
    #[cfg(test)]
    fn open_in_memory() -> AppResult<Self> {
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
    fn configure(conn: &Connection) -> AppResult<()> {
        // pragma_update는 PRAGMA name = value 와 동등.
        // foreign_keys는 *connection-scoped* — 매 연결마다 다시 켜야 한다.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // WAL은 *database-scoped* — 한 번 켜면 파일에 영구 적용.
        // in-memory에는 효과 없음.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
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
