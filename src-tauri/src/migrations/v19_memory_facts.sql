-- v19 — v0.5 PR 1 (D-097/D-098) — memory_facts DB 스키마.
--
-- 학습 진단표 저장소. LLM extraction + 결정적 정규식으로 자동 INSERT.
-- D-010 b "1회 확인" 정책 부분 supersede — chat extraction 자동 INSERT.
-- reports queue (MemoryPanelContent) 사후 정정 도구.
--
-- kind TEXT CHECK: 5섹션 = preference/correction/progress/meta/goal (TEXT라 향후 추가 무파괴).
-- source TEXT CHECK: 삽입 경로 식별. 정규식=trigger, 임베딩=citation, 수동=manual 등.
-- confidence REAL: 0.0~1.0. 시스템 프롬프트 주입 필터 = confidence >= 0.5 AND status='active'.
-- status TEXT CHECK: active/archived/expired.
-- created_at/updated_at: UNIX epoch (초).
--
-- memory_fact_chunks: fact ↔ chunk 연관 (chunk_id는 논리 참조 — 책별 분리라 FK 없음).
--
-- PRAGMA 패턴: v0.4.4 hotfix (BUG-003) 그대로 — FK off + 트랜잭션 + FK on + check.

PRAGMA foreign_keys = OFF;

CREATE TABLE memory_facts (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  study_id      TEXT    NOT NULL,
  kind          TEXT    NOT NULL CHECK (kind IN ('preference','correction','progress','meta','goal')),
  content       TEXT    NOT NULL,
  source        TEXT    NOT NULL CHECK (source IN ('user','trigger','srs','metacog','recall','citation','manual')),
  confidence    REAL    NOT NULL DEFAULT 1.0,
  status        TEXT    NOT NULL DEFAULT 'active' CHECK (status IN ('active','archived','expired')),
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

-- 5섹션 reports 뷰용: study + kind + status 조합 쿼리 최적화.
CREATE INDEX idx_memory_facts_study_kind_status
    ON memory_facts(study_id, kind, status);

-- "최근 N일 추가" 섹션용: study + status + created_at 역순 정렬 최적화.
CREATE INDEX idx_memory_facts_study_status_created
    ON memory_facts(study_id, status, created_at DESC);

-- fact ↔ chunk 연관.
-- chunk_id: 논리 참조 (책별 분리된 chunks 테이블 — FK 강제 X, 무결성은 코드 레벨).
CREATE TABLE memory_fact_chunks (
  fact_id    INTEGER NOT NULL REFERENCES memory_facts(id) ON DELETE CASCADE,
  chunk_id   INTEGER NOT NULL,
  similarity REAL    NOT NULL,
  PRIMARY KEY (fact_id, chunk_id)
);

CREATE INDEX idx_memory_fact_chunks_chunk
    ON memory_fact_chunks(chunk_id);

PRAGMA foreign_key_check;
PRAGMA foreign_keys = ON;
