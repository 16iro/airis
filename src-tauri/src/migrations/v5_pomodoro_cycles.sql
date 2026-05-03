-- v5 — F9.3 Pomodoro 사이클 로그 (PR 20).
-- 출처: db-schema.md `pomodoro_cycles`. v2에서 누락됐던 것을 v5로 추가.

CREATE TABLE pomodoro_cycles (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    phase         TEXT NOT NULL CHECK (phase IN ('focus', 'break')),
    duration_min  INTEGER NOT NULL DEFAULT 25,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    completed     INTEGER NOT NULL DEFAULT 0,
    interruption  TEXT
);
CREATE INDEX idx_pomodoro_study ON pomodoro_cycles(study_slug, started_at DESC);
