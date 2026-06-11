-- v23 — v0.6.x (D-113~D-115) 챗 세션 분리.
-- 한 스터디 = 하나의 연속 스레드였던 것을 *여러 세션(대화 스레드)*로 분리한다.
-- 기존 메시지는 무손실 이관: 메시지가 있는 스터디마다 결정적 id('legacy-'||slug)의
-- 기본 세션 1개를 만들고 그 스터디 메시지 전부를 귀속시킨다.
--
-- chat_messages.session_id 는 FK 없음 (recall_attempts 선례 — CASCADE 복잡성 회피).
-- 세션 삭제 시 메시지 정리는 백엔드 커맨드가 책임. 스터디 삭제는 studies CASCADE가
-- chat_messages·chat_sessions 양쪽을 정리.

PRAGMA foreign_keys = OFF;

CREATE TABLE chat_sessions (
    id          TEXT PRIMARY KEY,
    study_slug  TEXT NOT NULL REFERENCES studies(slug) ON DELETE CASCADE,
    title       TEXT,                         -- NULL이면 프론트가 placeholder 표시
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_chat_sessions_study ON chat_sessions(study_slug, updated_at DESC);

ALTER TABLE chat_messages ADD COLUMN session_id TEXT;
CREATE INDEX idx_chat_messages_session ON chat_messages(session_id, created_at);

-- 이관: 메시지 있는 스터디마다 기본 세션 1개 + 귀속.
INSERT INTO chat_sessions (id, study_slug, title, created_at, updated_at)
SELECT 'legacy-' || study_slug,
       study_slug,
       '이전 대화',
       MIN(created_at),
       MAX(created_at)
FROM chat_messages
WHERE session_id IS NULL
GROUP BY study_slug;

UPDATE chat_messages
SET session_id = 'legacy-' || study_slug
WHERE session_id IS NULL;

PRAGMA foreign_key_check;
PRAGMA foreign_keys = ON;
