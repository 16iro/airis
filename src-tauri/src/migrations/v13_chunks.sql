-- v13 — v0.4.1 PR 1 — RAG 엔진 chunks 인프라.
--
-- D-073~D-077 채택분 (mE5-small 384d / sqlite-vec 0.1.9 / claude-code subprocess /
-- 섹션 부모 / 모델 cache appdata 강제) 토대를 까는 마이그레이션.
--
-- 무파괴 원칙: 기존 paragraphs / paragraphs_fts *변경 없음*. 신설만.
-- 책별로 chunks 적재 안 됐으면 v0.3.2 paragraphs FTS 폴백이 그대로 작동한다.
--
-- 신설:
--   * chunks         = 새 청킹 엔진의 검색 단위. 부모(섹션) / 이웃(prev/next) 링크 포함.
--   * chunks_fts     = FTS5 (BM25). content=chunks 외부 콘텐츠 모드. 트리거 동기화.
--   * vectors_t1     = mE5-small INT8 (384d) 임베딩 BLOB. tier 1 (=fastembed-rs) 자리.
--                      sqlite-vec 가상 테이블 vec0는 *PR 2/3에서 vector_store가 생성*한다.
--                      이유: PR 1 검증 = 마이그 forward-only + 트리거. vec0는 fastembed
--                      차원이 코드에서 결정되므로 vector_store가 책임지는 게 단순.
--   * indexing_jobs  = 책별 인덱싱 진행 상태 (v0.3.2 index:progress 이벤트의 영속 backing).
--
-- 외래키 형태: book_id TEXT — 기존 books.id (UUID, TEXT) 컬럼 컨벤션을 그대로 따른다.
-- (참고: handoff에는 source_id INTEGER로 표기됐지만 이 코드베이스의 books.id가
-- TEXT라서 paragraphs 테이블이 선택한 book_id TEXT 패턴을 일관되게 사용한다.)

-- 1) chunks — 새 엔진의 청크 단위.
CREATE TABLE chunks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id         TEXT    NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    ord             INTEGER NOT NULL,             -- 책 내 청크 순서 (0-base)
    text            TEXT    NOT NULL,             -- 청크 본문
    page            INTEGER,                      -- PDF 페이지 (1-base, 옵션)
    span_start      INTEGER,                      -- 원문 char offset 시작 (옵션)
    span_end        INTEGER,                      -- 원문 char offset 끝 (옵션)
    parent_id       INTEGER REFERENCES chunks(id) ON DELETE SET NULL, -- 섹션/페이지 부모 (auto-merging)
    prev_chunk_id   INTEGER REFERENCES chunks(id) ON DELETE SET NULL, -- sentence window 확장
    next_chunk_id   INTEGER REFERENCES chunks(id) ON DELETE SET NULL,
    section_path    TEXT,                         -- "Ch04/§State" 또는 "p.42" (PDF 폴백)
    token_count     INTEGER,                      -- §4.7.3 패킹용 (글자수/4 + 안전 마진 휴리스틱)
    created_at      INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000)
);

-- 검색 경로 보조: 책 + 순서 / 부모 단위 / 페이지 단위.
CREATE INDEX idx_chunks_book_ord    ON chunks(book_id, ord);
CREATE INDEX idx_chunks_parent      ON chunks(parent_id);
CREATE INDEX idx_chunks_book_page   ON chunks(book_id, page);
CREATE INDEX idx_chunks_book_section ON chunks(book_id, section_path);

-- 2) chunks_fts — FTS5 외부 콘텐츠 모드. paragraphs_fts 패턴과 동일 (unicode61).
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    text,
    content='chunks',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 1'
);

-- 3) FTS 동기화 트리거 — paragraphs_fts와 같은 형식.
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
    INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
END;

-- 4) vectors_t1 — tier 1 임베딩 BLOB (mE5-small INT8, 384d, little-endian f32).
--    sqlite-vec vec0 가상 테이블은 vector_store::ensure_vec0가 PR 3에서 생성한다.
--    (이유: 차원이 fastembed 모델 출력에 의존 — 마이그 SQL 텍스트보다 코드가 단일 책임)
--
--    chunks 삭제 시 자동 정리 (FK CASCADE). 차원·인코딩은 vector_store 책임.
CREATE TABLE vectors_t1 (
    chunk_id   INTEGER PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
    embedding  BLOB    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000)
);
-- chunk_id가 PRIMARY KEY라 별도 인덱스 불필요. KNN 시 vec0 가상 테이블이 책임.

-- 5) indexing_jobs — 책별 인덱싱 영속 진행 상태.
--    tier: 0 = future (BGE-M3 등 v0.4.2+), 1 = mE5-small (v0.4.1 default), 2 = future
--    status: queued → running → completed / failed / paused
CREATE TABLE indexing_jobs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id         TEXT    NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    status          TEXT    NOT NULL CHECK (status IN ('queued', 'running', 'paused', 'completed', 'failed')),
    tier            INTEGER NOT NULL DEFAULT 1 CHECK (tier IN (0, 1, 2)),
    progress_chunks INTEGER NOT NULL DEFAULT 0,
    total_chunks    INTEGER,
    started_at      INTEGER,
    finished_at     INTEGER,
    error           TEXT
);
CREATE INDEX idx_indexing_jobs_book   ON indexing_jobs(book_id);
CREATE INDEX idx_indexing_jobs_status ON indexing_jobs(status);
