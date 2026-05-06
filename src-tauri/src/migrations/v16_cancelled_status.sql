-- v16 — v0.4.2 PR 5 — indexing_jobs.status에 'cancelled' 추가.
--
-- v15까지 cancel_indexing_job은 'failed' + error='cancelled by user' 마커로 영속했다.
-- v16은 *명시 'cancelled'* status를 1급 시민으로 승격. UI/검색 분기가 status로 직접
-- 분기 가능. 기존 'failed' row는 *그대로 유지* — 소급 변환 X (단순성).
--
-- SQLite는 ALTER COLUMN CHECK 직접 변경 X — 테이블 재생성 패턴:
--   1. indexing_jobs_new를 새 CHECK로 만들고
--   2. INSERT INTO ... SELECT * FROM indexing_jobs (구 CHECK 통과 데이터는 새 CHECK도 통과)
--   3. 구 테이블 DROP 후 RENAME.
--
-- 인덱스도 같은 이름으로 다시 만든다 (DROP TABLE은 인덱스를 함께 제거).
--
-- 주의:
--   * v15 ALTER로 추가한 pause_reason / updated_at 컬럼도 새 테이블 정의에 보존.
--   * AUTOINCREMENT는 그대로 유지 — id 시퀀스가 끊기지 않게.
--   * sqlite_sequence row도 건드리지 않음 (자동 유지).

-- 1) 새 테이블.
CREATE TABLE indexing_jobs_new (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id         TEXT    NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    status          TEXT    NOT NULL CHECK (status IN ('queued', 'running', 'paused', 'completed', 'failed', 'cancelled')),
    tier            INTEGER NOT NULL DEFAULT 1 CHECK (tier IN (0, 1, 2)),
    progress_chunks INTEGER NOT NULL DEFAULT 0,
    total_chunks    INTEGER,
    started_at      INTEGER,
    finished_at     INTEGER,
    error           TEXT,
    pause_reason    TEXT,
    updated_at      INTEGER
);

-- 2) 데이터 이관.
INSERT INTO indexing_jobs_new (
    id, book_id, status, tier, progress_chunks, total_chunks,
    started_at, finished_at, error, pause_reason, updated_at
)
SELECT
    id, book_id, status, tier, progress_chunks, total_chunks,
    started_at, finished_at, error, pause_reason, updated_at
FROM indexing_jobs;

-- 3) 구 테이블 DROP + RENAME.
DROP TABLE indexing_jobs;
ALTER TABLE indexing_jobs_new RENAME TO indexing_jobs;

-- 4) 인덱스 재생성 (DROP TABLE이 인덱스를 함께 제거).
CREATE INDEX idx_indexing_jobs_book   ON indexing_jobs(book_id);
CREATE INDEX idx_indexing_jobs_status ON indexing_jobs(status);
