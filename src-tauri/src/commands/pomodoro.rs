// F9 Pomodoro — 사용자 학습 사이클 관리.
//
// 정책 (PR 20 결정 — wall-clock 기반):
//   * tokio sleep 의존 X — OS sleep/wake 시 *경과 시간 어긋남* 회피
//   * AppState.pomodoro에 *시작 시각·duration_min·phase*만 저장
//   * 잔여 시간 = max(0, duration_min*60 - (now - started_at))
//   * 프론트가 1초 polling으로 진행률 표시
//   * Phase 종료(잔여=0)는 *프론트가 감지* + stop/transition 호출
//   * 호출 시 pomodoro_cycles에 종료 row INSERT (completed=1 또는 0)
//
// OS 네이티브 알림(`tauri-plugin-notification`)은 v0.3+. PR 20엔 인앱 토스트만.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PomodoroPhase {
    Focus,
    Break,
}

impl PomodoroPhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Break => "break",
        }
    }
}

/// 진행 중인 Pomodoro. AppState.pomodoro에 보관.
#[derive(Debug, Clone, Serialize)]
pub struct PomodoroSession {
    pub study_slug: String,
    pub phase: PomodoroPhase,
    pub duration_min: u32,
    /// epoch seconds.
    pub started_at: u64,
}

/// 프론트가 polling으로 받는 *현재 상태*. running=false면 idle.
#[derive(Debug, Clone, Serialize)]
pub struct PomodoroState {
    pub running: bool,
    pub session: Option<PomodoroSession>,
    /// 잔여 초 — running일 때만 의미. 끝나면 0.
    pub remaining_sec: i64,
}

const DEFAULT_FOCUS_MIN: u32 = 25;
const DEFAULT_BREAK_MIN: u32 = 5;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 새 Pomodoro 시작. focus=true → 집중 25분, false → 휴식 5분 (인자로 분 override 가능).
#[tauri::command]
pub fn start_pomodoro(
    state: State<'_, AppState>,
    study_slug: String,
    focus: bool,
    duration_min: Option<u32>,
) -> AppResult<PomodoroSession> {
    if study_slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 슬러그가 비어 있습니다".into(),
        });
    }
    let phase = if focus {
        PomodoroPhase::Focus
    } else {
        PomodoroPhase::Break
    };
    let dur = duration_min.unwrap_or(if focus {
        DEFAULT_FOCUS_MIN
    } else {
        DEFAULT_BREAK_MIN
    });
    let session = PomodoroSession {
        study_slug,
        phase,
        duration_min: dur,
        started_at: now_unix(),
    };
    *state.pomodoro.lock().expect("pomodoro mutex") = Some(session.clone());
    info!(
        target: "pomodoro",
        slug = %session.study_slug,
        phase = session.phase.as_str(),
        duration_min = dur,
        "start_pomodoro"
    );
    Ok(session)
}

/// 진행 중 Pomodoro 종료. completed=true(자연 만료) 또는 false(사용자 중단).
/// pomodoro_cycles에 row INSERT + AppState 비움.
#[tauri::command]
pub fn stop_pomodoro(
    state: State<'_, AppState>,
    completed: bool,
    interruption: Option<String>,
) -> AppResult<()> {
    let session = state.pomodoro.lock().expect("pomodoro mutex").take();
    let Some(session) = session else {
        return Ok(()); // 이미 idle — noop.
    };
    let db = state.db.lock().expect("db mutex");
    persist_cycle(db.conn(), &session, completed, interruption.as_deref())?;
    info!(
        target: "pomodoro",
        slug = %session.study_slug,
        completed,
        "stop_pomodoro"
    );
    Ok(())
}

#[tauri::command]
pub fn get_pomodoro_state(state: State<'_, AppState>) -> AppResult<PomodoroState> {
    let session = state.pomodoro.lock().expect("pomodoro mutex").clone();
    let Some(session) = session else {
        return Ok(PomodoroState {
            running: false,
            session: None,
            remaining_sec: 0,
        });
    };
    let elapsed = now_unix().saturating_sub(session.started_at) as i64;
    let total = (session.duration_min as i64) * 60;
    let remaining_sec = (total - elapsed).max(0);
    Ok(PomodoroState {
        running: true,
        session: Some(session),
        remaining_sec,
    })
}

fn persist_cycle(
    conn: &Connection,
    session: &PomodoroSession,
    completed: bool,
    interruption: Option<&str>,
) -> AppResult<()> {
    let started_iso = format_iso(session.started_at);
    conn.execute(
        "INSERT INTO pomodoro_cycles
         (study_slug, phase, duration_min, started_at, ended_at, completed, interruption)
         VALUES (?1, ?2, ?3, ?4, datetime('now'), ?5, ?6)",
        params![
            session.study_slug,
            session.phase.as_str(),
            session.duration_min,
            started_iso,
            if completed { 1 } else { 0 },
            interruption,
        ],
    )?;
    Ok(())
}

/// epoch seconds → SQLite 호환 ISO 8601 (`YYYY-MM-DD HH:MM:SS`, UTC).
fn format_iso(secs: u64) -> String {
    let days_since_epoch = (secs / 86400) as i64;
    let secs_in_day = (secs % 86400) as u32;
    let (y, m, d) = days_to_ymd(days_since_epoch);
    let h = secs_in_day / 3600;
    let mm = (secs_in_day % 3600) / 60;
    let s = secs_in_day % 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mm:02}:{s:02}")
}

pub fn days_to_ymd_pub(days: i64) -> (i32, u32, u32) {
    days_to_ymd(days)
}

fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    let mut year: i32 = 1970;
    loop {
        let leap = is_leap(year);
        let yd = if leap { 366 } else { 365 };
        if days < yd as i64 {
            break;
        }
        days -= yd as i64;
        year += 1;
    }
    let months: [u32; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0;
    for (i, dm) in months.iter().enumerate() {
        if days < *dm as i64 {
            m = i + 1;
            break;
        }
        days -= *dm as i64;
    }
    (year, m as u32, days as u32 + 1)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

/// AppState 필드 type — lib.rs에서 init.
pub type PomodoroSlot = Mutex<Option<PomodoroSession>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn seed_study(db: &Db, slug: &str) {
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES (?1, ?1, datetime('now'))",
                params![slug],
            )
            .unwrap();
    }

    #[test]
    fn persist_cycle_inserts_row() {
        let db = Db::open_in_memory_for_test();
        seed_study(&db, "s1");
        let session = PomodoroSession {
            study_slug: "s1".into(),
            phase: PomodoroPhase::Focus,
            duration_min: 25,
            started_at: 1_700_000_000,
        };
        persist_cycle(db.conn(), &session, true, None).unwrap();
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM pomodoro_cycles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let phase: String = db
            .conn()
            .query_row("SELECT phase FROM pomodoro_cycles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(phase, "focus");
    }

    #[test]
    fn format_iso_round_trip() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let s = format_iso(1_704_067_200);
        assert_eq!(s, "2024-01-01 00:00:00");
    }

    #[test]
    fn days_to_ymd_handles_leap_year() {
        // 2024 is a leap year. 2024-02-29 = day 31+29-1 since 2024-01-01 = day 59.
        // 2024-01-01 epoch = 1704067200. days since 1970 = 1704067200 / 86400 = 19723.
        let (y, m, d) = days_to_ymd(19723 + 59);
        assert_eq!((y, m, d), (2024, 2, 29));
    }
}
