// F8 SRS — SuperMemo SM-2 알고리즘 + 카드 영속.
//
// SM-2 (1985 Wozniak, supermemo.com 공개) — 결정적·외부 의존 X.
// 평가 quality (0~5):
//   0~2: 실패. repetitions 리셋, interval_days = 1, e_factor 약간 감소
//   3:   힘들게 기억 (간신히 통과)
//   4:   기억함 (보통)
//   5:   완벽 (즉시 떠올림)
//
// 본 모듈은 *pure 함수* `sm2_step`와 *DB 헬퍼*를 분리해 결정적 코어 보존.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::info;

use crate::commands::pomodoro::days_to_ymd_pub; // PR 20에서 이미 짜놓은 헬퍼 — 재사용
use crate::error::{AppError, AppResult};
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrsCard {
    pub id: i64,
    pub study_slug: String,
    pub front: String,
    pub back: String,
    pub section_ref: Option<String>,
    pub page_ref: Option<i64>,
    pub e_factor: f64,
    pub interval_days: i64,
    pub repetitions: i64,
    pub due_date: String,
    pub last_reviewed: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Sm2Outcome {
    pub e_factor: f64,
    pub interval_days: i64,
    pub repetitions: i64,
}

/// SM-2 한 단계. quality는 0~5. 결과는 *다음 카드 상태*.
pub fn sm2_step(prev_ef: f64, prev_interval: i64, prev_reps: i64, quality: u8) -> Sm2Outcome {
    let q = quality.min(5) as f64;
    // e_factor 갱신 — Wozniak 공식.
    let mut ef = prev_ef + (0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02));
    if ef < 1.3 {
        ef = 1.3;
    }

    if quality < 3 {
        // 실패 — 리셋.
        return Sm2Outcome {
            e_factor: ef,
            interval_days: 1,
            repetitions: 0,
        };
    }
    let (reps, interval) = if prev_reps == 0 {
        (1, 1)
    } else if prev_reps == 1 {
        (2, 6)
    } else {
        let next = (prev_interval as f64 * ef).round() as i64;
        (prev_reps + 1, next.max(prev_interval + 1))
    };
    Sm2Outcome {
        e_factor: ef,
        interval_days: interval,
        repetitions: reps,
    }
}

/// 오늘 + N일 후 ISO 날짜 (YYYY-MM-DD, UTC 기준).
fn date_n_days_from_now(n: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let target_days = (secs / 86400) + n;
    let (y, m, d) = days_to_ymd_pub(target_days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn today() -> String {
    date_n_days_from_now(0)
}

#[derive(Debug, Deserialize)]
pub struct CardInput {
    pub front: String,
    pub back: String,
    pub section_ref: Option<String>,
    pub page_ref: Option<i64>,
}

#[tauri::command]
pub fn srs_add_card(
    state: State<'_, AppState>,
    study_slug: String,
    card: CardInput,
) -> AppResult<SrsCard> {
    if study_slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 슬러그가 비어 있습니다".into(),
        });
    }
    if card.front.trim().is_empty() || card.back.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "front·back은 필수입니다".into(),
        });
    }
    let due = today();
    let db = state.db.lock().expect("db mutex");
    db.conn().execute(
        "INSERT INTO srs_cards
         (study_slug, front, back, section_ref, page_ref, e_factor, interval_days, repetitions, due_date, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 2.5, 0, 0, ?6, datetime('now'))",
        params![
            study_slug,
            card.front,
            card.back,
            card.section_ref,
            card.page_ref,
            due
        ],
    )?;
    let id = db.conn().last_insert_rowid();
    let row = fetch_card(db.conn(), id)?.ok_or_else(|| AppError::Internal {
        message: "card row missing after insert".into(),
    })?;
    info!(target: "srs", slug = %study_slug, card_id = id, "srs_add_card");
    Ok(row)
}

#[tauri::command]
pub fn srs_list_due(state: State<'_, AppState>, study_slug: String) -> AppResult<Vec<SrsCard>> {
    if study_slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 슬러그가 비어 있습니다".into(),
        });
    }
    let today = today();
    let db = state.db.lock().expect("db mutex");
    let mut stmt = db.conn().prepare(
        "SELECT id, study_slug, front, back, section_ref, page_ref,
                e_factor, interval_days, repetitions, due_date, last_reviewed, created_at
         FROM srs_cards
         WHERE study_slug = ?1 AND due_date <= ?2
         ORDER BY due_date ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![study_slug, today], map_card_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

#[tauri::command]
pub fn srs_review_card(
    state: State<'_, AppState>,
    card_id: i64,
    quality: u8,
) -> AppResult<SrsCard> {
    if quality > 5 {
        return Err(AppError::InvalidInput {
            message: "quality는 0~5 사이여야 합니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    let card = fetch_card(db.conn(), card_id)?.ok_or_else(|| AppError::NotFound {
        message: format!("SRS 카드 id={card_id} 없음"),
    })?;
    let next = sm2_step(card.e_factor, card.interval_days, card.repetitions, quality);
    let due_date = date_n_days_from_now(next.interval_days);
    db.conn().execute(
        "UPDATE srs_cards
         SET e_factor = ?1, interval_days = ?2, repetitions = ?3, due_date = ?4,
             last_reviewed = datetime('now')
         WHERE id = ?5",
        params![
            next.e_factor,
            next.interval_days,
            next.repetitions,
            due_date,
            card_id
        ],
    )?;
    let updated = fetch_card(db.conn(), card_id)?.ok_or_else(|| AppError::Internal {
        message: "card row missing after update".into(),
    })?;
    Ok(updated)
}

#[tauri::command]
pub fn srs_delete_card(state: State<'_, AppState>, card_id: i64) -> AppResult<()> {
    let db = state.db.lock().expect("db mutex");
    db.conn()
        .execute("DELETE FROM srs_cards WHERE id = ?1", params![card_id])?;
    Ok(())
}

fn fetch_card(conn: &Connection, id: i64) -> AppResult<Option<SrsCard>> {
    conn.query_row(
        "SELECT id, study_slug, front, back, section_ref, page_ref,
                e_factor, interval_days, repetitions, due_date, last_reviewed, created_at
         FROM srs_cards WHERE id = ?1",
        params![id],
        map_card_row,
    )
    .optional()
    .map_err(AppError::from)
}

fn map_card_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SrsCard> {
    Ok(SrsCard {
        id: r.get(0)?,
        study_slug: r.get(1)?,
        front: r.get(2)?,
        back: r.get(3)?,
        section_ref: r.get(4)?,
        page_ref: r.get(5)?,
        e_factor: r.get(6)?,
        interval_days: r.get(7)?,
        repetitions: r.get(8)?,
        due_date: r.get(9)?,
        last_reviewed: r.get(10)?,
        created_at: r.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sm2_first_pass_sets_interval_one() {
        let out = sm2_step(2.5, 0, 0, 4);
        assert_eq!(out.repetitions, 1);
        assert_eq!(out.interval_days, 1);
    }

    #[test]
    fn sm2_second_pass_sets_interval_six() {
        let out = sm2_step(2.5, 1, 1, 4);
        assert_eq!(out.repetitions, 2);
        assert_eq!(out.interval_days, 6);
    }

    #[test]
    fn sm2_failure_resets_repetitions() {
        let out = sm2_step(2.5, 30, 5, 1);
        assert_eq!(out.repetitions, 0);
        assert_eq!(out.interval_days, 1);
        assert!(out.e_factor < 2.5, "e_factor should decrease on fail");
    }

    #[test]
    fn sm2_perfect_score_grows_interval_geometrically() {
        // 3rd review onwards: interval *= e_factor.
        let out = sm2_step(2.5, 6, 2, 5);
        assert_eq!(out.repetitions, 3);
        assert_eq!(out.interval_days, (6.0_f64 * 2.6).round() as i64);
    }

    #[test]
    fn sm2_e_factor_floor_is_one_three() {
        // 반복 실패에도 e_factor가 1.3 미만으로 안 떨어진다.
        let mut ef = 2.5;
        for _ in 0..20 {
            let out = sm2_step(ef, 1, 0, 0);
            ef = out.e_factor;
        }
        assert!(ef >= 1.3);
    }
}
