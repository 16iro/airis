// failed_llm_jobs 큐 헬퍼.
// v0.1 PR 6: *기록 + 명시 재시도*만. 자동 워커는 v0.2.
//
// dedup: UNIQUE(study_slug, job_type, payload_json) — 같은 입력 반복 실패 시
// 같은 row의 attempts++.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// chat 잡의 직렬화 페이로드. context 본문은 *재시도 시점 AppState*에서 다시 읽으므로
/// payload엔 사용자 입력(query·section_id)만 담는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatPayload {
    pub query: String,
    pub context_section_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedJob {
    pub id: i64,
    pub study_slug: String,
    pub job_type: String,
    /// chat 잡의 경우 사용자 질문. UI 리스트에 표시.
    pub query: String,
    pub error: Option<String>,
    pub attempts: i64,
    pub last_attempt: Option<String>,
    /// NULL이면 자동 retry 한도 초과 — 수동만 가능. ISO 8601.
    pub next_retry_at: Option<String>,
    pub created_at: String,
}

pub const JOB_TYPE_CHAT: &str = "chat";

/// 잡 INSERT 또는 (UNIQUE 충돌 시) attempts++ UPDATE. 반환 = 항상 row id.
/// next_retry_at은 *exponential backoff* 기반 (1m / 2m / 4m / 8m). 4회 후엔 NULL → 수동 retry만.
pub fn enqueue_or_update(
    conn: &Connection,
    study_slug: &str,
    payload: &ChatPayload,
    error: &str,
) -> AppResult<i64> {
    let payload_json = serde_json::to_string(payload).map_err(|e| AppError::Internal {
        message: format!("payload serialize: {e}"),
    })?;

    conn.execute(
        "INSERT INTO failed_llm_jobs (study_slug, job_type, payload_json, error, created_at, next_retry_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now', '+1 minute'))
         ON CONFLICT (study_slug, job_type, payload_json) DO UPDATE SET
            error = excluded.error,
            attempts = attempts + 1,
            last_attempt = datetime('now'),
            next_retry_at = CASE
                WHEN attempts + 1 >= 4 THEN NULL
                WHEN attempts + 1 = 1 THEN datetime('now', '+2 minutes')
                WHEN attempts + 1 = 2 THEN datetime('now', '+4 minutes')
                ELSE datetime('now', '+8 minutes')
            END",
        params![study_slug, JOB_TYPE_CHAT, payload_json, error],
    )?;

    let id: i64 = conn.query_row(
        "SELECT id FROM failed_llm_jobs
         WHERE study_slug = ?1 AND job_type = ?2 AND payload_json = ?3",
        params![study_slug, JOB_TYPE_CHAT, payload_json],
        |r| r.get(0),
    )?;

    Ok(id)
}

pub fn list_jobs(conn: &Connection, study_slug: Option<&str>) -> AppResult<Vec<FailedJob>> {
    let mut stmt = conn.prepare(
        "SELECT id, study_slug, job_type, payload_json, error, attempts, last_attempt, next_retry_at, created_at
         FROM failed_llm_jobs
         WHERE (?1 IS NULL OR study_slug = ?1)
         ORDER BY created_at DESC",
    )?;

    let rows = stmt.query_map(params![study_slug], |r| {
        let id: i64 = r.get(0)?;
        let slug: String = r.get(1)?;
        let job_type: String = r.get(2)?;
        let payload_json: String = r.get(3)?;
        let error: Option<String> = r.get(4)?;
        let attempts: i64 = r.get(5)?;
        let last_attempt: Option<String> = r.get(6)?;
        let next_retry_at: Option<String> = r.get(7)?;
        let created_at: String = r.get(8)?;

        let payload: ChatPayload = serde_json::from_str(&payload_json).unwrap_or(ChatPayload {
            query: "(corrupt payload)".to_string(),
            context_section_id: None,
        });

        Ok(FailedJob {
            id,
            study_slug: slug,
            job_type,
            query: payload.query,
            error,
            attempts,
            last_attempt,
            next_retry_at,
            created_at,
        })
    })?;

    let jobs: Result<Vec<_>, _> = rows.collect();
    Ok(jobs?)
}

/// 자동 워커 — `next_retry_at <= NOW`인 잡들을 반환.
pub fn list_due_jobs(conn: &Connection) -> AppResult<Vec<FailedJob>> {
    let mut stmt = conn.prepare(
        "SELECT id, study_slug, job_type, payload_json, error, attempts, last_attempt, next_retry_at, created_at
         FROM failed_llm_jobs
         WHERE next_retry_at IS NOT NULL AND next_retry_at <= datetime('now')
         ORDER BY next_retry_at ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        let payload_json: String = r.get(3)?;
        let payload: ChatPayload = serde_json::from_str(&payload_json).unwrap_or(ChatPayload {
            query: "(corrupt payload)".to_string(),
            context_section_id: None,
        });
        Ok(FailedJob {
            id: r.get(0)?,
            study_slug: r.get(1)?,
            job_type: r.get(2)?,
            query: payload.query,
            error: r.get(4)?,
            attempts: r.get(5)?,
            last_attempt: r.get(6)?,
            next_retry_at: r.get(7)?,
            created_at: r.get(8)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

/// retry_failed_job이 사용 — 재시도 시 잡의 study_slug를 *기록 그대로* 사용해
/// 재시도 결과 메시지가 원래 스터디에 영속되도록 한다.
pub fn fetch_study_slug(conn: &Connection, job_id: i64) -> AppResult<String> {
    conn.query_row(
        "SELECT study_slug FROM failed_llm_jobs WHERE id = ?1",
        params![job_id],
        |r| r.get::<_, String>(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
            message: format!("failed_llm_jobs id={job_id}"),
        },
        other => AppError::Db {
            message: other.to_string(),
        },
    })
}

pub fn fetch_payload(conn: &Connection, job_id: i64) -> AppResult<ChatPayload> {
    let payload_json: String = conn
        .query_row(
            "SELECT payload_json FROM failed_llm_jobs WHERE id = ?1",
            params![job_id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
                message: format!("failed_llm_jobs id={job_id}"),
            },
            other => AppError::Db {
                message: other.to_string(),
            },
        })?;

    serde_json::from_str(&payload_json).map_err(|e| AppError::Internal {
        message: format!("payload deserialize: {e}"),
    })
}

pub fn delete_job(conn: &Connection, job_id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM failed_llm_jobs WHERE id = ?1", params![job_id])?;
    Ok(())
}

/// 어떤 에러가 *큐에 적재할 가치*가 있는지 — 즉 재시도 가능 여부 판정.
/// 4xx 인증·입력 오류는 재시도 무의미라 큐 적재 X.
pub fn is_retryable_error(e: &AppError) -> bool {
    match e {
        AppError::NetworkUnavailable => true,
        AppError::LlmApi { message } => {
            // 5xx 또는 SSE 와이어 에러 (네트워크 일시 단절일 가능성).
            message.contains("HTTP 5") || message.contains("[SSE-WIRE]")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn make_conn() -> Connection {
        // v2부터 failed_llm_jobs는 studies(slug)를 FK로 참조한다.
        // 마이그 1·2를 모두 적용하고 테스트용 'default' 스터디를 미리 만든다.
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(include_str!("migrations/v1_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("migrations/v2_studies_and_chat.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO studies (slug, name, created_at) VALUES ('default','default',datetime('now'))",
            [],
        )
        .unwrap();
        conn
    }

    fn payload(q: &str) -> ChatPayload {
        ChatPayload {
            query: q.to_string(),
            context_section_id: None,
        }
    }

    #[test]
    fn enqueue_returns_positive_id() {
        let conn = make_conn();
        let id = enqueue_or_update(&conn, "default", &payload("hi"), "network").unwrap();
        assert!(id > 0);
    }

    #[test]
    fn enqueue_dedup_increments_attempts() {
        let conn = make_conn();
        let p = payload("same");
        let id1 = enqueue_or_update(&conn, "default", &p, "err1").unwrap();
        let id2 = enqueue_or_update(&conn, "default", &p, "err2").unwrap();
        assert_eq!(id1, id2, "same payload should reuse row");

        let attempts: i64 = conn
            .query_row("SELECT attempts FROM failed_llm_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(attempts, 1, "second enqueue should increment to 1");

        let error: String = conn
            .query_row("SELECT error FROM failed_llm_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(error, "err2", "error column should be updated");
    }

    #[test]
    fn list_jobs_returns_inserted_with_query() {
        let conn = make_conn();
        enqueue_or_update(&conn, "default", &payload("first"), "e1").unwrap();
        enqueue_or_update(&conn, "default", &payload("second"), "e2").unwrap();

        let jobs = list_jobs(&conn, Some("default")).unwrap();
        assert_eq!(jobs.len(), 2);
        // ORDER BY created_at DESC — 최신이 먼저
        let queries: Vec<&str> = jobs.iter().map(|j| j.query.as_str()).collect();
        assert!(queries.contains(&"first"));
        assert!(queries.contains(&"second"));
    }

    #[test]
    fn list_jobs_with_none_returns_all() {
        let conn = make_conn();
        enqueue_or_update(&conn, "default", &payload("q"), "e").unwrap();
        let jobs = list_jobs(&conn, None).unwrap();
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn fetch_payload_round_trips() {
        let conn = make_conn();
        let original = ChatPayload {
            query: "테스트 질문".to_string(),
            context_section_id: Some("Ch04/§State".to_string()),
        };
        let id = enqueue_or_update(&conn, "default", &original, "err").unwrap();
        let loaded = fetch_payload(&conn, id).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn fetch_payload_missing_id_returns_not_found() {
        let conn = make_conn();
        let err = fetch_payload(&conn, 9999).unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }));
    }

    #[test]
    fn delete_job_removes_row() {
        let conn = make_conn();
        let id = enqueue_or_update(&conn, "default", &payload("q"), "err").unwrap();
        delete_job(&conn, id).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM failed_llm_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn is_retryable_distinguishes_network_and_4xx() {
        assert!(is_retryable_error(&AppError::NetworkUnavailable));
        assert!(is_retryable_error(&AppError::LlmApi {
            message: "HTTP 503: server error".into()
        }));
        assert!(is_retryable_error(&AppError::LlmApi {
            message: "[SSE-WIRE] field line missing colon: ...".into()
        }));
        assert!(!is_retryable_error(&AppError::AuthRequired));
        assert!(!is_retryable_error(&AppError::LlmApi {
            message: "HTTP 400: bad request".into()
        }));
        assert!(!is_retryable_error(&AppError::InvalidInput {
            message: "x".into()
        }));
    }
}
