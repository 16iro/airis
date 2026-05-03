-- v6 — F8 SRS 카드 (PR 21).
-- db-schema.md `srs_cards` 그대로. v2/v3에서 누락된 것을 v6로 추가.

CREATE TABLE srs_cards (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    front         TEXT NOT NULL,
    back          TEXT NOT NULL,
    section_ref   TEXT,
    page_ref      INTEGER,
    e_factor      REAL NOT NULL DEFAULT 2.5,
    interval_days INTEGER NOT NULL DEFAULT 0,
    repetitions   INTEGER NOT NULL DEFAULT 0,
    due_date      TEXT NOT NULL,
    last_reviewed TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_srs_due ON srs_cards(study_slug, due_date);
