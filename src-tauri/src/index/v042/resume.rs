// 재개 로직 — 비정상 종료/사용자 일시정지 잡 발견 + 미완료 청크 plan 산출.
//
// 진입 시점: 앱 시작 직후 (commands::book 또는 lib.rs setup 단계). PR 1은 *함수
// 시그니처 + 단위 테스트*만. 호출 측 wiring은 PR 3가 일시정지 UI + UPower
// 트리거 통합 시 채운다 (HANDOFF §1.5).
//
// 정책:
//   * status='running' = 비정상 종료 = 재개 후보. log warn + plan 반환.
//   * status='paused'  = 의도적 일시정지 = 재개 후보. plan 반환.
//   * status='queued'  = 아직 시작 전 = 재개 후보 X (큐 자체가 책임).
//   * status='completed' / 'failed' = plan 비어 있어도 무방. 본 함수가 무시.
//   * pending 청크 = `embed_status_t{tier} IS NULL OR = 'failed'`.
//     단 'failed' (= attempts>=MAX) 청크는 worker가 다시 시도해도 즉시 skip된다는
//     약속이지만, *plan 자체*는 그대로 포함해 호출 측이 보고에 노출하게 한다.
//   * 무한 재개 방지는 worker의 attempts 카운터에서 책임지므로 본 함수는 단순 조회.

#![allow(dead_code)]

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};
use crate::index::v042::worker::Tier;

/// 재개해야 할 잡 + 그 잡이 다시 처리해야 할 청크 ID 목록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePlan {
    pub job_id: i64,
    pub book_id: String,
    pub tier: Tier,
    pub pending_chunk_ids: Vec<i64>,
    /// 마이그 직전 상태 — UI/로그가 "재개" vs "비정상 종료 회복"을 분기.
    pub status_was: ResumeStatusWas,
}

/// 잡 상태가 어디서 출발하는지. UI 알림에 활용 (PR 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeStatusWas {
    /// 'running' — 앱 비정상 종료 후 발견. 사용자 알림 후보.
    AbnormalRunning,
    /// 'paused' — 의도적 사용자/배터리 일시정지.
    UserPaused,
}

/// 재개 후보 잡들의 plan을 반환. 호출 측 (PR 3의 commands)이 worker를 띄우는
/// 패턴은 본 PR 범위 X.
///
/// `running` 상태인 잡은 비정상 종료로 간주 (architecture §5: graceful shutdown은
/// status='paused'로 전환). 무파괴 — *DB 변경 X*. plan만 산출.
pub fn resume_pending_jobs(conn: &Connection) -> AppResult<Vec<ResumePlan>> {
    let mut plans = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, book_id, tier, status FROM indexing_jobs \
         WHERE status IN ('running', 'paused') \
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for (job_id, book_id, tier_int, status) in rows {
        let tier = parse_tier(tier_int)?;
        let pending_chunk_ids = pending_chunk_ids_for(conn, &book_id, tier)?;
        let status_was = match status.as_str() {
            "running" => {
                tracing::warn!(
                    job_id,
                    book_id = %book_id,
                    "v0.4.2 재개: status='running'인 잡 발견 — 비정상 종료 회복"
                );
                ResumeStatusWas::AbnormalRunning
            }
            "paused" => ResumeStatusWas::UserPaused,
            other => {
                // 위 SQL 필터로 도달 불가 — defensive.
                return Err(AppError::Internal {
                    message: format!("resume_pending_jobs: 예상 못한 status='{other}'"),
                });
            }
        };
        plans.push(ResumePlan {
            job_id,
            book_id,
            tier,
            pending_chunk_ids,
            status_was,
        });
    }
    Ok(plans)
}

/// 잡 완료 마킹 — worker 루프가 모든 청크 처리 후 호출.
///
/// status='completed' + finished_at = now(). 트랜잭션 단위가 아닌 단일 UPDATE라
/// 호출 측에서 트랜잭션 잡을 필요 X.
pub fn mark_job_completed(conn: &Connection, job_id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE indexing_jobs SET \
            status = 'completed', \
            finished_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
            updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
            pause_reason = NULL \
         WHERE id = ?1",
        params![job_id],
    )?;
    Ok(())
}

/// 사용자/이벤트 트리거로 잡을 일시정지 마킹 — PR 3 UI가 호출.
///
/// 트랜잭션 단위 X — 단일 UPDATE.
pub fn mark_job_paused(
    conn: &Connection,
    job_id: i64,
    reason: crate::index::v042::worker::PauseReason,
) -> AppResult<()> {
    conn.execute(
        "UPDATE indexing_jobs SET \
            status = 'paused', \
            pause_reason = ?1, \
            updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
         WHERE id = ?2",
        params![reason.as_db_str(), job_id],
    )?;
    Ok(())
}

/// 일시정지 잡을 다시 'running'으로 돌림.
pub fn mark_job_running(conn: &Connection, job_id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE indexing_jobs SET \
            status = 'running', \
            pause_reason = NULL, \
            updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
         WHERE id = ?1",
        params![job_id],
    )?;
    Ok(())
}

/// `book_id` + `tier` 의 *pending* 청크 ID 시퀀스. 'NULL' or 'failed' (== retry candidate).
/// ord 오름차순 — 사용자가 책 앞쪽부터 보는 직관 보존.
fn pending_chunk_ids_for(conn: &Connection, book_id: &str, tier: Tier) -> AppResult<Vec<i64>> {
    let status_col = tier.embed_status_column();
    let sql = format!(
        "SELECT id FROM chunks \
         WHERE book_id = ?1 \
           AND ({status_col} IS NULL OR {status_col} = 'failed') \
         ORDER BY ord ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt
        .query_map(params![book_id], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

fn parse_tier(t: i64) -> AppResult<Tier> {
    match t {
        1 => Ok(Tier::T1Me5Small),
        2 => Ok(Tier::T2BgeM3),
        // tier=0(=future)·기타는 본 PR에서 처리 대상 X.
        other => Err(AppError::Internal {
            message: format!("resume_pending_jobs: 알 수 없는 tier {other}"),
        }),
    }
}

/// 잡 단건 조회 (디버그/테스트 용).
pub fn get_job_status(conn: &Connection, job_id: i64) -> AppResult<Option<String>> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM indexing_jobs WHERE id = ?1",
            params![job_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::v042::worker::PauseReason;
    use rusqlite::Connection;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("FK on");
        let migrations: &[&str] = &[
            include_str!("../../migrations/v1_initial.sql"),
            include_str!("../../migrations/v2_studies_and_chat.sql"),
            include_str!("../../migrations/v3_paragraphs_fts.sql"),
            include_str!("../../migrations/v4_intervention_and_history.sql"),
            include_str!("../../migrations/v5_pomodoro_cycles.sql"),
            include_str!("../../migrations/v6_srs_cards.sql"),
            include_str!("../../migrations/v7_recall_challenges.sql"),
            include_str!("../../migrations/v8_book_thumbnail.sql"),
            include_str!("../../migrations/v9_study_thumbnail.sql"),
            include_str!("../../migrations/v10_thumbnails_dir_rename.sql"),
            include_str!("../../migrations/v11_study_description.sql"),
            include_str!("../../migrations/v12_chat_context.sql"),
            include_str!("../../migrations/v13_chunks.sql"),
            include_str!("../../migrations/v14_ab_compare.sql"),
            include_str!("../../migrations/v15_robustness.sql"),
        ];
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (\
                version INTEGER PRIMARY KEY,\
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
             );",
        )
        .unwrap();
        for sql in migrations {
            conn.execute_batch(sql).unwrap();
        }
        conn
    }

    fn seed_book(conn: &Connection, book_id: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO studies (slug, name, created_at) VALUES ('s','S',datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (
                id, study_slug, role, title, source_path, file_format,
                file_size, file_hash, added_at
             ) VALUES (?1,'s','main','B','/x','md',0,'h',datetime('now'))",
            params![book_id],
        )
        .unwrap();
    }

    fn insert_chunk(
        conn: &Connection,
        book_id: &str,
        ord: usize,
        embed_status_t1: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, token_count, embed_status_t1) \
             VALUES (?1, ?2, ?3, 1, ?4)",
            params![book_id, ord as i64, format!("c{ord}"), embed_status_t1],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_job(conn: &Connection, book_id: &str, status: &str, tier: i64) -> i64 {
        conn.execute(
            "INSERT INTO indexing_jobs \
                (book_id, status, tier, progress_chunks, started_at) \
             VALUES (?1, ?2, ?3, 0, CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
            params![book_id, status, tier],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn resume_returns_only_running_and_paused_jobs() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        seed_book(&conn, "b2");
        seed_book(&conn, "b3");
        seed_book(&conn, "b4");

        let running = insert_job(&conn, "b1", "running", 1);
        let paused = insert_job(&conn, "b2", "paused", 1);
        let _completed = insert_job(&conn, "b3", "completed", 1);
        let _queued = insert_job(&conn, "b4", "queued", 1);

        let plans = resume_pending_jobs(&conn).unwrap();
        assert_eq!(plans.len(), 2, "running·paused 2건만 plan");
        let job_ids: Vec<i64> = plans.iter().map(|p| p.job_id).collect();
        assert!(job_ids.contains(&running));
        assert!(job_ids.contains(&paused));
    }

    #[test]
    fn resume_classifies_status_was() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        seed_book(&conn, "b2");
        let r = insert_job(&conn, "b1", "running", 1);
        let p = insert_job(&conn, "b2", "paused", 1);
        let plans = resume_pending_jobs(&conn).unwrap();
        let by_id = |id: i64| plans.iter().find(|x| x.job_id == id).unwrap().clone();
        assert_eq!(by_id(r).status_was, ResumeStatusWas::AbnormalRunning);
        assert_eq!(by_id(p).status_was, ResumeStatusWas::UserPaused);
    }

    #[test]
    fn resume_collects_pending_chunks_only_for_target_tier() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        // 청크 4개: 2개 done (vectors 적재됨), 1개 NULL, 1개 failed.
        insert_chunk(&conn, "b1", 0, Some("done"));
        insert_chunk(&conn, "b1", 1, Some("done"));
        let pending_id = insert_chunk(&conn, "b1", 2, None);
        let failed_id = insert_chunk(&conn, "b1", 3, Some("failed"));

        let job_id = insert_job(&conn, "b1", "running", 1);
        let plans = resume_pending_jobs(&conn).unwrap();
        assert_eq!(plans.len(), 1);
        let p = &plans[0];
        assert_eq!(p.job_id, job_id);
        assert_eq!(p.tier, Tier::T1Me5Small);
        assert_eq!(p.pending_chunk_ids, vec![pending_id, failed_id]);
    }

    #[test]
    fn resume_pending_for_t2_uses_t2_status_column() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        // T1은 다 done이지만 T2는 아직 NULL.
        insert_chunk(&conn, "b1", 0, Some("done"));
        insert_chunk(&conn, "b1", 1, Some("done"));

        let _ = insert_job(&conn, "b1", "running", 2); // T2 잡.
        let plans = resume_pending_jobs(&conn).unwrap();
        // t2 컬럼 기준이라 두 청크 모두 pending.
        let p = &plans[0];
        assert_eq!(p.tier, Tier::T2BgeM3);
        assert_eq!(p.pending_chunk_ids.len(), 2);
    }

    #[test]
    fn mark_job_completed_sets_status_and_finished_at() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        let job_id = insert_job(&conn, "b1", "running", 1);
        mark_job_completed(&conn, job_id).unwrap();
        let (status, finished_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT status, finished_at FROM indexing_jobs WHERE id = ?1",
                params![job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "completed");
        assert!(finished_at.is_some());
    }

    #[test]
    fn mark_job_paused_writes_pause_reason() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        let job_id = insert_job(&conn, "b1", "running", 1);
        mark_job_paused(&conn, job_id, PauseReason::User).unwrap();
        let (status, reason): (String, Option<String>) = conn
            .query_row(
                "SELECT status, pause_reason FROM indexing_jobs WHERE id = ?1",
                params![job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "paused");
        assert_eq!(reason.as_deref(), Some("user"));
    }

    #[test]
    fn mark_job_running_clears_pause_reason() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        let job_id = insert_job(&conn, "b1", "paused", 1);
        // 사전 상태에 reason 채워 두기.
        conn.execute(
            "UPDATE indexing_jobs SET pause_reason = 'battery_low' WHERE id = ?1",
            params![job_id],
        )
        .unwrap();
        mark_job_running(&conn, job_id).unwrap();
        let (status, reason): (String, Option<String>) = conn
            .query_row(
                "SELECT status, pause_reason FROM indexing_jobs WHERE id = ?1",
                params![job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "running");
        assert_eq!(reason, None);
    }

    #[test]
    fn parse_tier_rejects_unknown_value() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        let _ = insert_job(&conn, "b1", "running", 0); // tier=0 = future 자리.
        let err = resume_pending_jobs(&conn).unwrap_err();
        match err {
            AppError::Internal { .. } => {}
            other => panic!("기대 Internal, 받음: {other:?}"),
        }
    }

    #[test]
    fn resume_returns_empty_when_no_running_or_paused() {
        let conn = fresh_db();
        seed_book(&conn, "b1");
        let _ = insert_job(&conn, "b1", "completed", 1);
        let plans = resume_pending_jobs(&conn).unwrap();
        assert!(plans.is_empty());
    }
}
