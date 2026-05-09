-- v21 — v0.5 PR 4 (D-101) recall_attempts 테이블.
-- 회상 챌린지 시도 내역 영속. gate 4 (응답률 ≥ 50%) 측정 데이터.
-- FK 없음 — chunk_id는 chunks.id 참조 의미지만 CASCADE 복잡성 회피.

PRAGMA foreign_keys = OFF;

CREATE TABLE recall_attempts (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug   TEXT NOT NULL,
    chunk_id     INTEGER NOT NULL,
    trigger_id   TEXT NOT NULL,
    strength     TEXT NOT NULL CHECK (strength IN ('weak', 'medium', 'strong')),
    outcome      TEXT NOT NULL CHECK (outcome IN ('correct', 'incorrect', 'dismissed', 'timeout', 'skipped')),
    fired_at     TEXT NOT NULL,
    responded_at TEXT
);

CREATE INDEX idx_recall_study_fired ON recall_attempts(study_slug, fired_at DESC);
CREATE INDEX idx_recall_outcome ON recall_attempts(study_slug, outcome);

PRAGMA foreign_key_check;
PRAGMA foreign_keys = ON;
