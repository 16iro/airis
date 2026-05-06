-- v17 — v0.4.4 PR 3 (D-093) — books.file_format CHECK constraint에 'docx' 추가.
--
-- v2_studies_and_chat.sql에서 books.file_format은 ('md', 'pdf', 'html', 'txt')로 고정.
-- DOCX 등록을 위해 CHECK 확장. SQLite는 ALTER COLUMN CHECK 직접 변경 X — 테이블 재생성
-- 패턴(v16과 동일):
--   1. books_new를 새 CHECK로 만들고
--   2. INSERT INTO ... SELECT * FROM books (구 CHECK 통과 데이터는 새 CHECK도 통과)
--   3. 구 테이블 DROP 후 RENAME.
--
-- 인덱스도 같은 이름으로 다시 생성 (DROP TABLE은 인덱스를 함께 제거).
--
-- 주의:
--   * v8_book_thumbnail.sql이 ALTER로 추가한 thumbnail_path 컬럼도 새 정의에 보존.
--   * FK 참조: books.id를 가리키는 외래키 테이블(paragraphs, chunks, indexing_jobs 등)은
--     SQLite의 RENAME 시 자동 갱신(`PRAGMA foreign_keys=OFF` 없이도 안전 — RENAME은
--     대상 테이블 이름만 바꿈, 다른 테이블의 FK 정의 텍스트는 영향 X).
--   * 그러나 *기존 데이터 보존*을 위해 INSERT-DROP-RENAME 트랜잭션 안 일관성이 중요.
--   * 무파괴(forward-only) — 기존 row의 file_format이 ('md', 'pdf', 'html', 'txt') 중
--     하나이므로 새 CHECK도 통과.

-- 1) 새 테이블 — v8 thumbnail 컬럼까지 포함한 *현재 형태* + 'docx' 추가.
CREATE TABLE books_new (
    id              TEXT PRIMARY KEY,
    study_slug      TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('main', 'sub')),
    role_note       TEXT,
    title           TEXT NOT NULL,
    author          TEXT,
    source_path     TEXT NOT NULL,
    file_format     TEXT NOT NULL CHECK (file_format IN ('md', 'pdf', 'html', 'txt', 'docx')),
    file_size       INTEGER NOT NULL,
    file_hash       TEXT NOT NULL,
    added_at        TEXT NOT NULL,
    last_modified   TEXT,
    indexed_at      TEXT,
    metadata_json   TEXT,
    thumbnail_path  TEXT
);

-- 2) 데이터 이관.
INSERT INTO books_new (
    id, study_slug, role, role_note, title, author, source_path,
    file_format, file_size, file_hash, added_at, last_modified,
    indexed_at, metadata_json, thumbnail_path
)
SELECT
    id, study_slug, role, role_note, title, author, source_path,
    file_format, file_size, file_hash, added_at, last_modified,
    indexed_at, metadata_json, thumbnail_path
FROM books;

-- 3) 구 테이블 DROP + RENAME.
DROP TABLE books;
ALTER TABLE books_new RENAME TO books;

-- 4) 인덱스 재생성 — DROP TABLE이 인덱스를 함께 제거.
CREATE INDEX idx_books_study ON books(study_slug);
