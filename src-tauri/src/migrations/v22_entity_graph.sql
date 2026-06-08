-- v22 — v0.6.x (D-111) 경량 GraphRAG: chunk_entities 동시출현 테이블.
-- 청크별 추출 엔티티(키워드) 저장. 검색 시 같은 엔티티를 공유하는 청크를 1홉 확장.
-- chunks ON DELETE CASCADE — 책/청크 삭제 시 자동 정리. book_id는 책별 격리 + 빠른 필터.

PRAGMA foreign_keys = OFF;

CREATE TABLE chunk_entities (
    chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    book_id  TEXT    NOT NULL,
    entity   TEXT    NOT NULL,
    weight   REAL    NOT NULL DEFAULT 1.0,
    PRIMARY KEY (chunk_id, entity)
);

-- 확장 쿼리 핵심 경로: book_id + entity IN (...) → chunk_id 집계.
CREATE INDEX idx_chunk_entities_book_entity ON chunk_entities(book_id, entity);
-- seed 엔티티 조회: chunk_id IN (...).
CREATE INDEX idx_chunk_entities_chunk ON chunk_entities(chunk_id);

PRAGMA foreign_key_check;
PRAGMA foreign_keys = ON;
