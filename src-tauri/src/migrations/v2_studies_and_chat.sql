-- v2 — 스터디 단위 도입 + 챗 히스토리·책 영속 + failed_llm_jobs FK 재구성.
--
-- v1엔 failed_llm_jobs 한 테이블만 있었고 study_slug는 단순 TEXT였다.
-- v2부터 studies가 *진짜 테이블*이 되며 chat_messages·books가 그 슬러그를 FK로 참조한다.
--
-- 핵심 제약:
--   * SQLite는 ALTER TABLE ADD FOREIGN KEY를 지원하지 않는다.
--     기존 failed_llm_jobs에 FK를 붙이려면 CREATE NEW + COPY + RENAME 패턴이 유일.
--   * INSERT ... SELECT 시점에 PRAGMA foreign_keys = ON 상태이므로,
--     모든 study_slug 행이 studies에 *미리* 존재해야 한다.
--     → 1)에서 studies 생성 → 2)에서 기존 슬러그 자동 보존 → 5)에서 데이터 복사.
--
-- 출처: design/architecture/db-schema.md (테이블 정의), v0.2 핸드오프 PR 8 결정 1.

-- 1) studies 테이블 추가.
--    is_active 컬럼 + partial unique index 조합으로 "동시에 활성 스터디 2개"를 DB가 차단.
CREATE TABLE studies (
    slug         TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    language     TEXT NOT NULL DEFAULT 'ko',
    created_at   TEXT NOT NULL,
    last_opened  TEXT,
    is_active    INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX idx_studies_active
    ON studies(is_active) WHERE is_active = 1;

-- 2) v1에서 사용 중이던 study_slug들을 studies에 자동 보존.
--    (v0.1 사용자가 'default' 슬러그로 큐를 쌓아둔 경우 그대로 살림 — FK 위반 방지)
INSERT INTO studies (slug, name, language, created_at, is_active)
SELECT DISTINCT study_slug, study_slug, 'ko', datetime('now'), 0
FROM failed_llm_jobs
WHERE study_slug NOT IN (SELECT slug FROM studies);

-- 3) chat_messages 테이블 추가 (메모리만이던 챗 히스토리 → DB 영속).
CREATE TABLE chat_messages (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug       TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    role             TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content          TEXT NOT NULL,
    section_ref      TEXT,
    page_ref         INTEGER,
    created_at       TEXT NOT NULL,
    cache_hit_tokens INTEGER NOT NULL DEFAULT 0,
    creation_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens    INTEGER NOT NULL DEFAULT 0,
    model            TEXT,
    cost_usd         REAL NOT NULL DEFAULT 0
);
CREATE INDEX idx_chat_study_time
    ON chat_messages(study_slug, created_at DESC);

-- 4) books 테이블 추가. PR 10~11에서 본격 사용 — v2엔 스키마만.
CREATE TABLE books (
    id            TEXT PRIMARY KEY,
    study_slug    TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    role          TEXT NOT NULL CHECK (role IN ('main', 'sub')),
    role_note     TEXT,
    title         TEXT NOT NULL,
    author        TEXT,
    source_path   TEXT NOT NULL,
    file_format   TEXT NOT NULL CHECK (file_format IN ('md', 'pdf', 'html', 'txt')),
    file_size     INTEGER NOT NULL,
    file_hash     TEXT NOT NULL,
    added_at      TEXT NOT NULL,
    last_modified TEXT,
    indexed_at    TEXT,
    metadata_json TEXT
);
CREATE INDEX idx_books_study ON books(study_slug);

-- 5) failed_llm_jobs 재구성 — FK + ON DELETE CASCADE 추가.
--    SQLite는 기존 테이블에 FK를 ALTER로 추가할 수 없으므로 CREATE+COPY+RENAME.
ALTER TABLE failed_llm_jobs RENAME TO failed_llm_jobs_old;

CREATE TABLE failed_llm_jobs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    job_type      TEXT NOT NULL CHECK (job_type IN (
                    'chat', 'memory_update', 'recall_eval',
                    'pomodoro_eval', 'meta_extract', 'response_validation'
                  )),
    payload_json  TEXT NOT NULL,
    error         TEXT,
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_attempt  TEXT,
    next_retry_at TEXT,
    created_at    TEXT NOT NULL,
    UNIQUE(study_slug, job_type, payload_json)
);

INSERT INTO failed_llm_jobs (
    id, study_slug, job_type, payload_json, error,
    attempts, last_attempt, next_retry_at, created_at
)
SELECT id, study_slug, job_type, payload_json, error,
       attempts, last_attempt, next_retry_at, created_at
FROM failed_llm_jobs_old;

DROP TABLE failed_llm_jobs_old;

CREATE INDEX idx_queue_retry
    ON failed_llm_jobs(next_retry_at)
    WHERE next_retry_at IS NOT NULL;
