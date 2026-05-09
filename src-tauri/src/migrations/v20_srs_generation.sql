-- v20 — v0.5 PR 2 (D-099/D-103) — SRS on-demand card generation columns.
--
-- 기존 srs_cards에 생성 이력 3 컬럼 추가:
--   source_chunk_id    : 카드를 생성한 source chunk.id (논리 참조 — 책별 분리라 FK 없음)
--   generation_method  : 생성 경로 (DEFAULT 'legacy' — 기존 row 자동 backfill)
--   citation_score     : citation_check 통과 점수 (NULL = 수동/레거시)
--
-- DEFAULT 'legacy': 기존 row는 ALTER 시점에 자동 'legacy' backfill (SQLite 동작).
-- 신규 INSERT는 명시값 강제 (알고리즘·명령별로 'deterministic_cloze' 등 지정).
--
-- CHECK 제약은 새 컬럼에만 적용 (기존 row = 'legacy'는 DEFAULT 값이라 CHECK 통과).
--
-- source_chunk_id FK 미설정 이유: chunks 테이블이 book_id 기반으로 분리돼 있어
-- memory_fact_chunks (v19) 패턴과 동일하게 코드 레벨 무결성만 보장.
--
-- PRAGMA 패턴: v0.4.4 hotfix (BUG-003) 동일 — FK off + 트랜잭션 + FK on + check.

PRAGMA foreign_keys = OFF;

ALTER TABLE srs_cards ADD COLUMN source_chunk_id INTEGER;

ALTER TABLE srs_cards ADD COLUMN generation_method TEXT NOT NULL DEFAULT 'legacy'
    CHECK (generation_method IN (
        'manual',
        'legacy',
        'deterministic_cloze',
        'deterministic_match',
        'deterministic_order',
        'llm_mc4'
    ));

ALTER TABLE srs_cards ADD COLUMN citation_score REAL;

CREATE INDEX idx_srs_cards_source_chunk ON srs_cards(source_chunk_id);
CREATE INDEX idx_srs_cards_generation_method ON srs_cards(generation_method);

PRAGMA foreign_key_check;
PRAGMA foreign_keys = ON;
