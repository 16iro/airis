// F1 — 스터디 단위 (CRUD + 활성 전환).
//
// 활성 스터디는 *DB의 studies.is_active*가 source of truth.
// AppState.active_study는 매 명령마다 DB를 두드리지 않으려는 *메모리 캐시*일 뿐이다.
// `studies` 테이블의 partial unique index가 "동시에 활성 2개"를 DB 단에서 차단한다.
//
// 슬러그 규칙 (URL-safe + 일관성):
//   * 영소문자·숫자·하이픈만, 1~64자
//   * 첫 글자는 영소문자 또는 숫자 (하이픈으로 시작 X)
//   * 예: "rust-deep-dive"·"algo-2025"·"default"

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;
use tracing::{info, warn};

use crate::commands::overview::{self, StudyOverview};
use crate::error::{AppError, AppResult};
use crate::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StudyMeta {
    pub slug: String,
    pub name: String,
    pub language: String,
    pub created_at: String,
    pub last_opened: Option<String>,
    pub is_active: bool,
    pub book_count: u32,
    pub session_count: u32,
}

const DEFAULT_STUDY_SLUG: &str = "default";
const DEFAULT_STUDY_NAME: &str = "기본 스터디";
const DEFAULT_LANGUAGE: &str = "ko";

const SELECT_STUDY_SQL: &str = "
    SELECT s.slug, s.name, s.language, s.created_at, s.last_opened, s.is_active,
           (SELECT COUNT(*) FROM books WHERE study_slug = s.slug)
    FROM studies s
    WHERE s.slug = ?1
";

const SELECT_ACTIVE_SQL: &str = "
    SELECT s.slug, s.name, s.language, s.created_at, s.last_opened, s.is_active,
           (SELECT COUNT(*) FROM books WHERE study_slug = s.slug)
    FROM studies s
    WHERE s.is_active = 1
";

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StudyMeta> {
    let book_count: i64 = r.get(6)?;
    Ok(StudyMeta {
        slug: r.get(0)?,
        name: r.get(1)?,
        language: r.get(2)?,
        created_at: r.get(3)?,
        last_opened: r.get(4)?,
        is_active: r.get::<_, i64>(5)? == 1,
        book_count: book_count.max(0) as u32,
        // session_count는 PR 19 (sessions 테이블) 도입 후 채움. 그전엔 0.
        session_count: 0,
    })
}

/// 슬러그 형식 검증. 잘못된 입력은 InvalidInput으로 즉시 반려.
fn validate_slug(slug: &str) -> AppResult<()> {
    let bytes = slug.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(AppError::InvalidInput {
            message: "슬러그는 1~64자여야 합니다".into(),
        });
    }
    let first = bytes[0] as char;
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(AppError::InvalidInput {
            message: "슬러그는 영소문자 또는 숫자로 시작해야 합니다".into(),
        });
    }
    for &b in bytes {
        let c = b as char;
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(AppError::InvalidInput {
                message: "슬러그는 영소문자/숫자/하이픈만 가능합니다".into(),
            });
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> AppResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 이름이 비어 있습니다".into(),
        });
    }
    if trimmed.chars().count() > 80 {
        return Err(AppError::InvalidInput {
            message: "스터디 이름은 최대 80자입니다".into(),
        });
    }
    Ok(())
}

fn fetch_one(conn: &Connection, slug: &str) -> AppResult<Option<StudyMeta>> {
    conn.query_row(SELECT_STUDY_SQL, params![slug], map_row)
        .optional()
        .map_err(AppError::from)
}

fn fetch_active_internal(conn: &Connection) -> AppResult<Option<StudyMeta>> {
    conn.query_row(SELECT_ACTIVE_SQL, [], map_row)
        .optional()
        .map_err(AppError::from)
}

fn list_all(conn: &Connection) -> AppResult<Vec<StudyMeta>> {
    let mut stmt = conn.prepare(
        "SELECT s.slug, s.name, s.language, s.created_at, s.last_opened, s.is_active,
                (SELECT COUNT(*) FROM books WHERE study_slug = s.slug)
         FROM studies s
         ORDER BY (s.is_active = 1) DESC, COALESCE(s.last_opened, s.created_at) DESC",
    )?;
    let rows = stmt.query_map([], map_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 새 스터디 INSERT. 첫 스터디는 자동 활성, 그 외엔 비활성.
fn insert_study(conn: &Connection, slug: &str, name: &str, language: &str) -> AppResult<StudyMeta> {
    let already_active: i64 = conn.query_row(
        "SELECT COUNT(*) FROM studies WHERE is_active = 1",
        [],
        |r| r.get(0),
    )?;
    let is_active = if already_active == 0 { 1 } else { 0 };

    conn.execute(
        "INSERT INTO studies (slug, name, language, created_at, is_active)
         VALUES (?1, ?2, ?3, datetime('now'), ?4)",
        params![slug, name, language, is_active],
    )?;

    fetch_one(conn, slug)?.ok_or_else(|| AppError::Internal {
        message: "insert_study: row not found after INSERT".into(),
    })
}

/// `slug`를 활성으로, 나머지를 비활성으로. last_opened 갱신.
/// 트랜잭션 안에서 단일 UPDATE로 처리해 race를 차단한다.
fn activate(conn: &mut Connection, slug: &str) -> AppResult<()> {
    let exists = fetch_one(conn, slug)?.is_some();
    if !exists {
        return Err(AppError::NotFound {
            message: format!("스터디 '{slug}'를 찾을 수 없습니다"),
        });
    }

    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE studies
         SET is_active = CASE WHEN slug = ?1 THEN 1 ELSE 0 END,
             last_opened = CASE WHEN slug = ?1 THEN datetime('now') ELSE last_opened END",
        params![slug],
    )?;
    tx.commit()?;
    Ok(())
}

/// 부팅 시 호출 — 활성 스터디가 없으면 'default'를 자동으로 만들어 활성화.
/// v0.1 사용자 + 신규 사용자 모두 *끊김 없이* 챗 가능하게.
pub fn ensure_active_or_bootstrap_default(conn: &mut Connection) -> AppResult<StudyMeta> {
    if let Some(active) = fetch_active_internal(conn)? {
        return Ok(active);
    }

    // 활성이 없으면 'default' 행이 있는지 확인 (v1→v2 마이그가 자동 생성한 경우 포함).
    if fetch_one(conn, DEFAULT_STUDY_SLUG)?.is_none() {
        conn.execute(
            "INSERT INTO studies (slug, name, language, created_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
            params![DEFAULT_STUDY_SLUG, DEFAULT_STUDY_NAME, DEFAULT_LANGUAGE],
        )?;
    }
    activate(conn, DEFAULT_STUDY_SLUG)?;
    fetch_one(conn, DEFAULT_STUDY_SLUG)?.ok_or_else(|| AppError::Internal {
        message: "default study not found after bootstrap".into(),
    })
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub fn list_studies(state: State<'_, AppState>) -> AppResult<Vec<StudyMeta>> {
    let db = state.db.lock().expect("db mutex");
    list_all(db.conn())
}

#[tauri::command]
pub fn create_study(
    state: State<'_, AppState>,
    slug: String,
    name: String,
    language: Option<String>,
) -> AppResult<StudyMeta> {
    validate_slug(&slug)?;
    validate_name(&name)?;
    let lang = language.unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());

    let db = state.db.lock().expect("db mutex");
    let conn = db.conn();
    if fetch_one(conn, &slug)?.is_some() {
        return Err(AppError::InvalidInput {
            message: format!("'{slug}' 슬러그는 이미 사용 중입니다"),
        });
    }
    let study = insert_study(conn, &slug, name.trim(), &lang)?;
    drop(db);

    // Overview.md 템플릿 자동 생성. 실패해도 스터디 자체는 살아있게 둔다 —
    // read는 default 반환, 사용자는 외부 편집 또는 마법사에서 다시 시도 가능.
    if let Err(e) = overview::create_default(&state.data_dir, &slug, &lang, &study.created_at) {
        warn!(target: "study", slug = %slug, error = %e, "Overview.md create failed (non-fatal)");
    }

    if study.is_active {
        *state.active_study.lock().expect("active_study mutex") = Some(study.clone());
    }
    info!(target: "study", slug = %study.slug, active = study.is_active, "create_study");
    Ok(study)
}

#[tauri::command]
pub fn study_overview_read(state: State<'_, AppState>, slug: String) -> AppResult<StudyOverview> {
    validate_slug(&slug)?;
    overview::read(&state.data_dir, &slug)
}

/// 마법사 + Settings의 *목표/마감* 입력을 Overview.md에 반영.
/// body는 사용자 자유 영역이라 *덮어쓰지 않는다*.
#[tauri::command]
pub fn study_overview_write_meta(
    state: State<'_, AppState>,
    slug: String,
    stated_goal_chapter: String,
    deadline: String,
) -> AppResult<StudyOverview> {
    validate_slug(&slug)?;
    overview::patch_meta(&state.data_dir, &slug, &stated_goal_chapter, &deadline)
}

#[tauri::command]
pub fn select_study(state: State<'_, AppState>, slug: String) -> AppResult<()> {
    validate_slug(&slug)?;
    let next = {
        let mut db = state.db.lock().expect("db mutex");
        activate(db.conn_mut(), &slug)?;
        fetch_one(db.conn(), &slug)?
    };

    if let Some(meta) = next {
        *state.active_study.lock().expect("active_study mutex") = Some(meta);
    }
    info!(target: "study", slug = %slug, "select_study");
    Ok(())
}

#[tauri::command]
pub fn delete_study(state: State<'_, AppState>, slug: String, confirm: bool) -> AppResult<()> {
    if !confirm {
        return Err(AppError::InvalidInput {
            message: "delete_study는 confirm=true가 필요합니다".into(),
        });
    }
    validate_slug(&slug)?;

    let mut db = state.db.lock().expect("db mutex");
    let existed = fetch_one(db.conn(), &slug)?.is_some();
    if !existed {
        return Err(AppError::NotFound {
            message: format!("스터디 '{slug}'를 찾을 수 없습니다"),
        });
    }
    db.conn()
        .execute("DELETE FROM studies WHERE slug = ?1", params![&slug])?;

    // 활성 스터디를 지웠다면 다른 스터디(가장 최근 last_opened)를 자동 활성.
    // 없으면 'default'를 다시 만들어 활성. 사용자가 챗을 못 하는 상태로 두지 않는다.
    let next_active = ensure_active_or_bootstrap_default(db.conn_mut())?;
    *state.active_study.lock().expect("active_study mutex") = Some(next_active);

    info!(target: "study", slug = %slug, "delete_study");
    Ok(())
}

#[tauri::command]
pub fn get_active_study(state: State<'_, AppState>) -> AppResult<Option<StudyMeta>> {
    // 캐시 → 비어 있으면 DB 조회 (다른 경로로 선택됐을 가능성).
    {
        let cache = state.active_study.lock().expect("active_study mutex");
        if let Some(meta) = cache.as_ref() {
            return Ok(Some(meta.clone()));
        }
    }
    let db = state.db.lock().expect("db mutex");
    fetch_active_internal(db.conn())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn validate_slug_accepts_simple() {
        assert!(validate_slug("rust-deep-dive").is_ok());
        assert!(validate_slug("algo-2025").is_ok());
        assert!(validate_slug("default").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("0a").is_ok());
    }

    #[test]
    fn validate_slug_rejects_invalid() {
        assert!(validate_slug("").is_err());
        assert!(validate_slug("-leading-hyphen").is_err());
        assert!(validate_slug("Has-Upper").is_err());
        assert!(validate_slug("space here").is_err());
        assert!(validate_slug("dot.in.middle").is_err());
        assert!(validate_slug(&"a".repeat(65)).is_err());
    }

    #[test]
    fn validate_name_rejects_empty_and_too_long() {
        assert!(validate_name("OK").is_ok());
        assert!(validate_name("  ").is_err());
        assert!(validate_name(&"가".repeat(81)).is_err());
    }

    fn fresh_db() -> Db {
        Db::open_in_memory_for_test()
    }

    #[test]
    fn first_study_becomes_active() {
        let mut db = fresh_db();
        let s = insert_study(db.conn_mut(), "first", "First", "ko").unwrap();
        assert!(s.is_active);
    }

    #[test]
    fn second_study_does_not_steal_active() {
        let mut db = fresh_db();
        insert_study(db.conn_mut(), "first", "First", "ko").unwrap();
        let s2 = insert_study(db.conn_mut(), "second", "Second", "ko").unwrap();
        assert!(!s2.is_active);
    }

    #[test]
    fn activate_switches_active_atomically() {
        let mut db = fresh_db();
        insert_study(db.conn_mut(), "a", "A", "ko").unwrap();
        insert_study(db.conn_mut(), "b", "B", "ko").unwrap();
        activate(db.conn_mut(), "b").unwrap();
        let active = fetch_active_internal(db.conn()).unwrap().unwrap();
        assert_eq!(active.slug, "b");
    }

    #[test]
    fn activate_unknown_returns_not_found() {
        let mut db = fresh_db();
        let err = activate(db.conn_mut(), "ghost").unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }));
    }

    #[test]
    fn bootstrap_creates_default_when_empty() {
        let mut db = fresh_db();
        let active = ensure_active_or_bootstrap_default(db.conn_mut()).unwrap();
        assert_eq!(active.slug, DEFAULT_STUDY_SLUG);
        assert!(active.is_active);
    }

    #[test]
    fn bootstrap_keeps_existing_active() {
        let mut db = fresh_db();
        insert_study(db.conn_mut(), "existing", "Ex", "ko").unwrap();
        let active = ensure_active_or_bootstrap_default(db.conn_mut()).unwrap();
        assert_eq!(active.slug, "existing");
    }

    #[test]
    fn list_orders_active_first_then_recent() {
        let mut db = fresh_db();
        insert_study(db.conn_mut(), "a", "A", "ko").unwrap();
        insert_study(db.conn_mut(), "b", "B", "ko").unwrap();
        activate(db.conn_mut(), "b").unwrap();
        let items = list_all(db.conn()).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].slug, "b", "active study should be first");
    }
}
