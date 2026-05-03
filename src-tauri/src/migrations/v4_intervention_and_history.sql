-- v4 — F11.6 능력 착각 5지표 + F7.2 반복 검색 + F12 정합성 검사 결과 영속.
--
-- 출처: design/architecture/db-schema.md.
-- v0.2 시점엔 *데이터 누적*만 시작. 사용자 가시 alert는 v0.3+.

CREATE TABLE intervention_signals (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    signal_type   TEXT NOT NULL CHECK (signal_type IN (
                    'repeat_search', 'short_dwell', 'progress_recall_gap',
                    'self_report_low', 'forced_output_miss',
                    'rabbit_hole', 'goal_drift', 'pace_vs_deadline', 'no_zoom_out'
                  )),
    severity      REAL NOT NULL DEFAULT 0,
    metadata_json TEXT,
    fired_at      TEXT NOT NULL,
    user_dismissed INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_signals_study ON intervention_signals(study_slug, fired_at DESC);

CREATE TABLE search_history (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    query         TEXT NOT NULL,
    query_norm    TEXT NOT NULL,
    result_count  INTEGER NOT NULL,
    searched_at   TEXT NOT NULL
);
CREATE INDEX idx_search_norm ON search_history(study_slug, query_norm, searched_at DESC);

CREATE TABLE consistency_check_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    study_slug    TEXT REFERENCES studies(slug) ON DELETE CASCADE,
    check_type    TEXT NOT NULL CHECK (check_type IN (
                    'memory_active_conflict', 'index_stale',
                    'cross_book_term_conflict', 'regression_test'
                  )),
    triggered_by  TEXT NOT NULL CHECK (triggered_by IN ('event', 'manual', 'scheduled')),
    issues_json   TEXT NOT NULL DEFAULT '[]',
    checked_at    TEXT NOT NULL
);
CREATE INDEX idx_consistency_study ON consistency_check_log(study_slug, checked_at DESC);
