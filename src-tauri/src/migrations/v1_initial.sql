-- v1 — failed_llm_jobs 큐
-- v0.1 슬라이스에는 studies 테이블이 없으므로 study_slug 컬럼은 단순 TEXT.
-- studies 테이블이 추가되는 v0.2 마이그레이션에서 FK·CASCADE를 재구성한다.
-- 출처: design/architecture/db-schema.md "failed_llm_jobs" 절.

CREATE TABLE failed_llm_jobs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT NOT NULL,
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

CREATE INDEX idx_queue_retry
    ON failed_llm_jobs(next_retry_at)
    WHERE next_retry_at IS NOT NULL;
