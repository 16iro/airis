-- v7 — F7.7 회상 챌린지 (PR 22).
-- db-schema.md `recall_challenges` 그대로.

CREATE TABLE recall_challenges (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    chapter_ref   TEXT NOT NULL,
    user_input    TEXT NOT NULL,
    keywords_expected_json TEXT NOT NULL,
    keywords_present_json  TEXT NOT NULL,
    keywords_missing_json  TEXT NOT NULL,
    passed        INTEGER NOT NULL,
    challenged_at TEXT NOT NULL
);
CREATE INDEX idx_recall_study ON recall_challenges(study_slug, challenged_at DESC);
