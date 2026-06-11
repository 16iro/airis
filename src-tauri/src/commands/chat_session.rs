// v0.6.x (D-113~D-115) — 챗 세션 (스터디 내 대화 스레드).
//
// 한 스터디 = 하나의 연속 스레드였던 것을 *여러 세션*으로 분리. 본 모듈은 세션 CRUD
// Tauri 커맨드 + llm.rs(chat_send/run_stream)가 쓰는 DB 헬퍼를 담는다.
//
// 정책 (확정 결정):
//   * D-113: 진입 시 가장 최근 세션 이어보기 + "새 대화" 버튼. 빈 세션은 이탈 시 자동 삭제.
//   * D-114: 기존 메시지는 마이그 v23에서 'legacy-'||slug 세션으로 무손실 이관.
//   * D-115: 제목은 chat_send가 첫 메시지로 결정적 placeholder를 즉시 설정, run_stream이
//            첫 응답 후 LLM(Haiku)로 교체(best-effort). 본 모듈은 DB set/get만 제공.
//
// session_id는 chat_messages에 FK 없이 컬럼으로만 (recall_attempts 선례). 세션 삭제 시
// 메시지 정리는 `chat_session_delete`가 명시 수행.

#![allow(dead_code)]

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::AppState;

/// 결정적 placeholder 제목 최대 길이(문자). 첫 사용자 메시지를 이만큼 잘라 임시 제목으로.
pub const TITLE_PLACEHOLDER_MAX_CHARS: usize = 40;

/// 한 챗 세션.
#[derive(Debug, Clone, Serialize)]
pub struct ChatSession {
    pub id: String,
    pub study_slug: String,
    /// None이면 아직 제목 미정 — 프론트가 placeholder("새 대화") 표시.
    pub title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// 이 세션의 메시지 수 (빈 세션 판별·UI 표시용).
    pub message_count: i64,
}

// ===== Tauri 커맨드 =========================================================

/// 스터디의 세션 목록 — 최근 갱신 순. message_count 포함.
#[tauri::command]
pub fn chat_sessions_list(
    state: State<'_, AppState>,
    study_slug: String,
) -> AppResult<Vec<ChatSession>> {
    let db = state.db.lock().expect("db mutex");
    list_sessions(db.conn(), &study_slug)
}

/// 새 빈 세션 생성 (제목 없음 → placeholder). "새 대화" 버튼이 호출.
#[tauri::command]
pub fn chat_session_create(
    state: State<'_, AppState>,
    study_slug: String,
) -> AppResult<ChatSession> {
    // study_slug 실존 검증.
    {
        let db = state.db.lock().expect("db mutex");
        let exists: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM studies WHERE slug = ?1",
            params![study_slug],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(AppError::NotFound {
                message: format!("스터디 '{study_slug}'를 찾을 수 없습니다"),
            });
        }
    }
    let id = format!("sess-{}", Uuid::new_v4());
    let db = state.db.lock().expect("db mutex");
    db.conn().execute(
        "INSERT INTO chat_sessions (id, study_slug, title, created_at, updated_at) \
         VALUES (?1, ?2, NULL, datetime('now'), datetime('now'))",
        params![id, study_slug],
    )?;
    fetch_session(db.conn(), &id)?.ok_or_else(|| AppError::Internal {
        message: "방금 만든 세션을 다시 읽지 못했습니다".into(),
    })
}

/// 세션 제목 수동 변경.
#[tauri::command]
pub fn chat_session_rename(
    state: State<'_, AppState>,
    session_id: String,
    title: String,
) -> AppResult<()> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput {
            message: "제목이 비어 있습니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    let n = db.conn().execute(
        "UPDATE chat_sessions SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![trimmed, session_id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound {
            message: format!("세션 '{session_id}'를 찾을 수 없습니다"),
        });
    }
    Ok(())
}

/// 세션 삭제 — 소속 메시지도 함께 정리 (FK 없으므로 명시 삭제).
#[tauri::command]
pub fn chat_session_delete(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    let db = state.db.lock().expect("db mutex");
    delete_session(db.conn(), &session_id)
}

/// 빈 세션(메시지 0개)이면 삭제 — D-113 "빈 세션 이탈 시 자동 삭제" 용. 비어있지 않으면 no-op.
/// 삭제 여부를 bool로 반환 (프론트가 목록 갱신 판단).
#[tauri::command]
pub fn chat_session_delete_if_empty(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<bool> {
    let db = state.db.lock().expect("db mutex");
    delete_if_empty(db.conn(), &session_id)
}

// ===== DB 헬퍼 (llm.rs가 재사용) ============================================

/// 스터디 세션 목록 — 최근 갱신 순 + message_count.
pub fn list_sessions(conn: &Connection, study_slug: &str) -> AppResult<Vec<ChatSession>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.study_slug, s.title, s.created_at, s.updated_at, \
                (SELECT COUNT(*) FROM chat_messages m WHERE m.session_id = s.id) AS cnt \
         FROM chat_sessions s \
         WHERE s.study_slug = ?1 \
         ORDER BY s.updated_at DESC, s.id DESC",
    )?;
    let rows = stmt
        .query_map(params![study_slug], row_to_session)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 단일 세션 조회 (message_count 포함).
pub fn fetch_session(conn: &Connection, session_id: &str) -> AppResult<Option<ChatSession>> {
    let row = conn
        .query_row(
            "SELECT s.id, s.study_slug, s.title, s.created_at, s.updated_at, \
                    (SELECT COUNT(*) FROM chat_messages m WHERE m.session_id = s.id) AS cnt \
             FROM chat_sessions s WHERE s.id = ?1",
            params![session_id],
            row_to_session,
        )
        .optional()?;
    Ok(row)
}

fn row_to_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatSession> {
    Ok(ChatSession {
        id: r.get(0)?,
        study_slug: r.get(1)?,
        title: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
        message_count: r.get(5)?,
    })
}

/// 세션이 해당 스터디에 실존하는지 검증 (chat_send 진입 가드).
pub fn session_belongs_to_study(
    conn: &Connection,
    session_id: &str,
    study_slug: &str,
) -> AppResult<bool> {
    let cnt: i64 = conn.query_row(
        "SELECT COUNT(*) FROM chat_sessions WHERE id = ?1 AND study_slug = ?2",
        params![session_id, study_slug],
        |r| r.get(0),
    )?;
    Ok(cnt > 0)
}

/// 세션의 updated_at 갱신 (새 메시지 도착 시 — 목록 정렬 최신화).
pub fn touch_session(conn: &Connection, session_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE chat_sessions SET updated_at = datetime('now') WHERE id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// 세션 제목이 비어있는지 (NULL 또는 빈 문자열).
pub fn title_is_unset(conn: &Connection, session_id: &str) -> AppResult<bool> {
    let title: Option<Option<String>> = conn
        .query_row(
            "SELECT title FROM chat_sessions WHERE id = ?1",
            params![session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?;
    Ok(match title {
        Some(Some(t)) => t.trim().is_empty(),
        Some(None) => true,
        None => false, // 세션 부재 — 설정 안 함.
    })
}

/// 세션 제목 설정 (placeholder/LLM 결과 공통). updated_at은 건드리지 않음(정렬 영향 X).
pub fn set_title(conn: &Connection, session_id: &str, title: &str) -> AppResult<()> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE chat_sessions SET title = ?1 WHERE id = ?2",
        params![trimmed, session_id],
    )?;
    Ok(())
}

/// 세션의 사용자 메시지 수 (첫 메시지 판별·제목 트리거용).
pub fn user_message_count(conn: &Connection, session_id: &str) -> AppResult<i64> {
    let cnt: i64 = conn.query_row(
        "SELECT COUNT(*) FROM chat_messages WHERE session_id = ?1 AND role = 'user'",
        params![session_id],
        |r| r.get(0),
    )?;
    Ok(cnt)
}

/// 스터디의 가장 최근 세션 id를 반환. 없으면 새로 만든다 (retry 등 세션 컨텍스트가 없는
/// 경로용). 신규 세션 id = `sess-<uuid>`.
pub fn most_recent_or_create(conn: &Connection, study_slug: &str) -> AppResult<String> {
    let recent: Option<String> = conn
        .query_row(
            "SELECT id FROM chat_sessions WHERE study_slug = ?1 \
             ORDER BY updated_at DESC, id DESC LIMIT 1",
            params![study_slug],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = recent {
        return Ok(id);
    }
    let id = format!("sess-{}", Uuid::new_v4());
    conn.execute(
        "INSERT INTO chat_sessions (id, study_slug, title, created_at, updated_at) \
         VALUES (?1, ?2, NULL, datetime('now'), datetime('now'))",
        params![id, study_slug],
    )?;
    Ok(id)
}

/// 첫 사용자 메시지를 placeholder 제목으로 절단 (문자 경계 안전).
pub fn placeholder_title(query: &str) -> String {
    let trimmed = query.trim().replace('\n', " ");
    let head: String = trimmed.chars().take(TITLE_PLACEHOLDER_MAX_CHARS).collect();
    if trimmed.chars().count() > TITLE_PLACEHOLDER_MAX_CHARS {
        format!("{head}…")
    } else {
        head
    }
}

/// 세션 + 소속 메시지 삭제.
pub fn delete_session(conn: &Connection, session_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM chat_messages WHERE session_id = ?1",
        params![session_id],
    )?;
    let n = conn.execute(
        "DELETE FROM chat_sessions WHERE id = ?1",
        params![session_id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound {
            message: format!("세션 '{session_id}'를 찾을 수 없습니다"),
        });
    }
    Ok(())
}

/// 메시지 0개면 세션 삭제 (D-113 빈 세션 정리). 삭제했으면 true.
pub fn delete_if_empty(conn: &Connection, session_id: &str) -> AppResult<bool> {
    let cnt: i64 = conn.query_row(
        "SELECT COUNT(*) FROM chat_messages WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )?;
    if cnt > 0 {
        return Ok(false);
    }
    let n = conn.execute(
        "DELETE FROM chat_sessions WHERE id = ?1",
        params![session_id],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn seed_study(conn: &Connection, slug: &str) {
        conn.execute(
            "INSERT INTO studies (slug, name, created_at) VALUES (?1, ?1, datetime('now'))",
            params![slug],
        )
        .unwrap();
    }

    fn new_session(conn: &Connection, slug: &str) -> String {
        let id = format!("sess-{}", slug);
        conn.execute(
            "INSERT INTO chat_sessions (id, study_slug, created_at, updated_at) \
             VALUES (?1, ?2, datetime('now'), datetime('now'))",
            params![id, slug],
        )
        .unwrap();
        id
    }

    fn add_msg(conn: &Connection, slug: &str, sid: &str, role: &str, content: &str) {
        conn.execute(
            "INSERT INTO chat_messages (study_slug, role, content, created_at, session_id) \
             VALUES (?1, ?2, ?3, datetime('now'), ?4)",
            params![slug, role, content, sid],
        )
        .unwrap();
    }

    #[test]
    fn placeholder_title_truncates_and_strips_newlines() {
        assert_eq!(placeholder_title("짧은 질문"), "짧은 질문");
        let long = "가".repeat(60);
        let t = placeholder_title(&long);
        assert!(t.ends_with('…'));
        assert_eq!(t.chars().count(), TITLE_PLACEHOLDER_MAX_CHARS + 1);
        assert_eq!(placeholder_title("줄1\n줄2"), "줄1 줄2");
    }

    #[test]
    fn list_orders_by_updated_desc_with_counts() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        let a = new_session(db.conn(), "s1");
        // a에 메시지 2개.
        add_msg(db.conn(), "s1", &a, "user", "q");
        add_msg(db.conn(), "s1", &a, "assistant", "ans");
        let list = list_sessions(db.conn(), "s1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, a);
        assert_eq!(list[0].message_count, 2);
        assert!(list[0].title.is_none());
    }

    #[test]
    fn delete_session_removes_messages_too() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        let a = new_session(db.conn(), "s1");
        add_msg(db.conn(), "s1", &a, "user", "q");
        delete_session(db.conn(), &a).unwrap();
        let msgs: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM chat_messages WHERE session_id = ?1",
                params![a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(msgs, 0);
        assert!(fetch_session(db.conn(), &a).unwrap().is_none());
    }

    /// 명시 id로 세션 INSERT (한 스터디에 여러 세션 만들 때 — PK 충돌 회피).
    fn new_session_id(conn: &Connection, id: &str, slug: &str) {
        conn.execute(
            "INSERT INTO chat_sessions (id, study_slug, created_at, updated_at) \
             VALUES (?1, ?2, datetime('now'), datetime('now'))",
            params![id, slug],
        )
        .unwrap();
    }

    #[test]
    fn delete_if_empty_only_deletes_empty() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        new_session_id(db.conn(), "empty-s1", "s1");
        new_session_id(db.conn(), "full-s1", "s1");
        add_msg(db.conn(), "s1", "full-s1", "user", "q");

        assert!(delete_if_empty(db.conn(), "empty-s1").unwrap(), "빈 세션 삭제됨");
        assert!(!delete_if_empty(db.conn(), "full-s1").unwrap(), "비어있지 않으면 보존");
        assert!(fetch_session(db.conn(), "full-s1").unwrap().is_some());
    }

    #[test]
    fn title_set_and_unset_detection() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        let a = new_session(db.conn(), "s1");
        assert!(title_is_unset(db.conn(), &a).unwrap(), "신규 세션은 제목 없음");
        set_title(db.conn(), &a, "PBR 렌더링").unwrap();
        assert!(!title_is_unset(db.conn(), &a).unwrap());
        assert_eq!(
            fetch_session(db.conn(), &a).unwrap().unwrap().title.as_deref(),
            Some("PBR 렌더링")
        );
    }

    #[test]
    fn session_belongs_to_study_check() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        seed_study(db.conn(), "s2");
        let a = new_session(db.conn(), "s1");
        assert!(session_belongs_to_study(db.conn(), &a, "s1").unwrap());
        assert!(!session_belongs_to_study(db.conn(), &a, "s2").unwrap());
    }

    #[test]
    fn user_message_count_counts_only_user() {
        let db = Db::open_in_memory_for_test();
        seed_study(db.conn(), "s1");
        let a = new_session(db.conn(), "s1");
        add_msg(db.conn(), "s1", &a, "user", "q1");
        add_msg(db.conn(), "s1", &a, "assistant", "a1");
        add_msg(db.conn(), "s1", &a, "user", "q2");
        assert_eq!(user_message_count(db.conn(), &a).unwrap(), 2);
    }
}
