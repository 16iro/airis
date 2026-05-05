-- v14 — v0.4.1 PR 5 — A/B 비교 측정 테이블.
--
-- A/B dev panel(handoff §2 PR 5)이 사용자 선택을 영속하는 *전용* 테이블.
-- 기존 indexing/log 테이블 재활용 X — 측정 책임이 분리돼 있어야 export·집계가 단순.
--
-- 운영 원칙:
--   * 사용자 머신 로컬 only. handoff에 명시된 "체감 품질 7/10" 게이트 1을 지원.
--   * 응답 텍스트는 디버그용으로 보관 — 진단 시 사용자가 어느 줄이 좋다고 봤는지 직접 확인.
--   * forward-only — chunks 흐름과 같은 무파괴 마이그 정책.
--   * forge_at은 INTEGER millisecond UNIX (다른 v13 chunks 테이블과 일관 — 컨벤션은
--     일부러 v0.4.x 신설 테이블의 *epoch ms* 흐름을 따라간다).

CREATE TABLE ab_compare_choices (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    -- 같은 query를 여러 번 쳐도 익명 dedup 가능하도록 SHA-256 hash 보관 (raw query는 미보관 = 프라이버시 안전선).
    query_hash      TEXT    NOT NULL,
    -- 디버그용 raw query — 사용자 머신만 영속이라 OK.
    query_text      TEXT    NOT NULL,
    -- 두 응답 본문 — 사용자가 "왜 이걸 골랐는지" 사후 점검할 수 있게 보관.
    baseline_text   TEXT    NOT NULL,
    v041_text       TEXT    NOT NULL,
    -- 'baseline' | 'v041' | 'tie'.
    chose           TEXT    NOT NULL CHECK (chose IN ('baseline', 'v041', 'tie')),
    -- 사용자 자유 메모 (선택). 무엇이 더 좋았는지 한 줄 코멘트.
    note            TEXT,
    -- handle 추적 — 두 응답이 동시 stream 됐던 chat_send_ab_compare 콜의 handle 문자열.
    handle          TEXT    NOT NULL,
    created_at      INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000)
);

CREATE INDEX idx_ab_compare_chose      ON ab_compare_choices(chose);
CREATE INDEX idx_ab_compare_created_at ON ab_compare_choices(created_at);
