// F1 — 스터디 단위 (CRUD + 활성 전환).
//
// 활성 스터디는 *DB의 studies.is_active*가 source of truth.
// AppState.active_study는 매 명령마다 DB를 두드리지 않으려는 *메모리 캐시*일 뿐이다.
// `studies` 테이블의 partial unique index가 "동시에 활성 2개"를 DB 단에서 차단한다.
//
// 슬러그 규칙 (v0.3 트랙 B 이후 — 디렉토리 직결, 한국어 허용):
//   * 디렉토리 이름으로 안전: OS 금지문자(`/ \ : * ? " < > |` + control)는 거부
//   * 시작/끝 공백 또는 점은 거부 (Windows 호환)
//   * Windows 예약어(CON, PRN, ...)는 거부
//   * 길이 1~200 byte (UTF-8 한글 약 66자)
//   * v0.2 시절 ascii 슬러그 ("rust-deep-dive", "default")도 그대로 통과
//   * 사용자가 슬러그를 직접 입력하지 않음 — 이름에서 sanitize_to_slug로 자동 도출

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
    /// PR 62: 라이브러리 카드 cover 이미지 경로. NULL이면 hue gradient + 첫 글자 placeholder.
    pub thumbnail_path: Option<String>,
    /// PR 68: 사용자가 남기는 자유 메모/설명. NULL이면 비어 있음.
    pub description: Option<String>,
}

const DEFAULT_STUDY_SLUG: &str = "default";
const DEFAULT_STUDY_NAME: &str = "기본 스터디";
const DEFAULT_LANGUAGE: &str = "ko";

const FORBIDDEN_SLUG_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const RESERVED_SLUG_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const MAX_SLUG_BYTES: usize = 200;
const SLUG_FALLBACK: &str = "스터디";

const SELECT_STUDY_SQL: &str = "
    SELECT s.slug, s.name, s.language, s.created_at, s.last_opened, s.is_active,
           (SELECT COUNT(*) FROM books WHERE study_slug = s.slug),
           s.thumbnail_path, s.description
    FROM studies s
    WHERE s.slug = ?1
";

const SELECT_ACTIVE_SQL: &str = "
    SELECT s.slug, s.name, s.language, s.created_at, s.last_opened, s.is_active,
           (SELECT COUNT(*) FROM books WHERE study_slug = s.slug),
           s.thumbnail_path, s.description
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
        thumbnail_path: r.get(7)?,
        description: r.get(8)?,
    })
}

/// 슬러그 형식 검증. 디렉토리 안전성 보장.
/// 한국어/숫자/영문 등 일반 텍스트는 통과. OS 금지문자/예약어/control char는 거부.
fn validate_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 이름이 비어 있습니다".into(),
        });
    }
    if slug.len() > MAX_SLUG_BYTES {
        return Err(AppError::InvalidInput {
            message: format!("스터디 이름이 너무 깁니다 (최대 {MAX_SLUG_BYTES} 바이트)"),
        });
    }
    let first = slug.chars().next().expect("non-empty");
    let last = slug.chars().next_back().expect("non-empty");
    if first.is_whitespace() || first == '.' {
        return Err(AppError::InvalidInput {
            message: "스터디 이름은 공백이나 점으로 시작할 수 없습니다".into(),
        });
    }
    if last.is_whitespace() || last == '.' {
        return Err(AppError::InvalidInput {
            message: "스터디 이름은 공백이나 점으로 끝날 수 없습니다".into(),
        });
    }
    for c in slug.chars() {
        if FORBIDDEN_SLUG_CHARS.contains(&c) || (c as u32) < 0x20 {
            return Err(AppError::InvalidInput {
                message: format!("'{c}'는 스터디 이름에 사용할 수 없습니다"),
            });
        }
    }
    let upper = slug.to_uppercase();
    let stem = upper.split('.').next().unwrap_or(&upper);
    if RESERVED_SLUG_NAMES.contains(&stem) {
        return Err(AppError::InvalidInput {
            message: format!("'{slug}'는 시스템 예약어라 사용할 수 없습니다"),
        });
    }
    Ok(())
}

/// 사용자 입력 이름 → 디렉토리 안전 슬러그.
/// trim → 금지문자/control 제거 → 끝부분 공백·점 제거 → 길이 제한 → 빈 결과면 fallback.
pub fn sanitize_to_slug(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !FORBIDDEN_SLUG_CHARS.contains(c) && (*c as u32) >= 0x20)
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c.is_whitespace() || c == '.');
    let mut s = String::with_capacity(trimmed.len().min(MAX_SLUG_BYTES));
    for c in trimmed.chars() {
        if s.len() + c.len_utf8() > MAX_SLUG_BYTES {
            break;
        }
        s.push(c);
    }
    let s = s
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string();
    if s.is_empty() {
        return SLUG_FALLBACK.to_string();
    }
    let upper = s.to_uppercase();
    let stem = upper.split('.').next().unwrap_or(&upper);
    if RESERVED_SLUG_NAMES.contains(&stem) {
        return format!("{s} ({SLUG_FALLBACK})");
    }
    s
}

/// 충돌 시 `이름 (2)`, `이름 (3)` 형태로 unique 슬러그 보장.
fn ensure_unique_slug(conn: &Connection, base: &str) -> AppResult<String> {
    if fetch_one(conn, base)?.is_none() {
        return Ok(base.to_string());
    }
    for n in 2..=999 {
        let candidate = format!("{base} ({n})");
        if candidate.len() > MAX_SLUG_BYTES {
            return Err(AppError::Internal {
                message: "충돌 처리 중 길이 초과 — 더 짧은 이름을 시도하세요".into(),
            });
        }
        if fetch_one(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(AppError::Internal {
        message: "고유 스터디 이름 생성 실패 (1000회 시도)".into(),
    })
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
                (SELECT COUNT(*) FROM books WHERE study_slug = s.slug),
                s.thumbnail_path, s.description
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

    // SQLite의 partial unique index `WHERE is_active = 1`은 deferred 미지원이라
    // *단일 UPDATE의 row 처리 도중*에도 즉시 검사된다. CASE WHEN으로 모든 row를
    // 한 번에 갱신하면 *대상 row가 1로 변경*되는 순간에 *기존 active row도 아직 1*이라
    // UNIQUE 위반 발생. 두 단계 UPDATE로 분리한다.
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE studies SET is_active = 0 WHERE is_active = 1 AND slug != ?1",
        params![slug],
    )?;
    tx.execute(
        "UPDATE studies SET is_active = 1, last_opened = datetime('now') WHERE slug = ?1",
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
    name: String,
    language: Option<String>,
) -> AppResult<StudyMeta> {
    validate_name(&name)?;
    let lang = language.unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());

    let trimmed_name = name.trim();
    let base_slug = sanitize_to_slug(trimmed_name);
    validate_slug(&base_slug)?;

    let db = state.db.lock().expect("db mutex");
    let conn = db.conn();
    let slug = ensure_unique_slug(conn, &base_slug)?;
    let study = insert_study(conn, &slug, trimmed_name, &lang)?;
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

const ALLOWED_THUMBNAIL_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

/// 매 호출마다 unique 파일명을 생성한다 (PR 64).
/// `cover.<ext>` 고정 이름이면 webview가 이전 이미지를 캐시에서 그대로 보여주는 버그가 있어,
/// 타임스탬프 suffix를 붙여 URL이 매번 바뀌도록 한다. 이전 파일은 호출자가 별도로 정리.
///
/// PR 65: `.thumbnails` → `thumbnails`. Tauri asset:// 스코프(`$HOME/**`, `$APPDATA/**`)의
/// glob 매칭이 점(`.`) 시작 디렉토리를 거부하면서 webview가 이미지를 로드하지 못하는 문제 해결.
fn study_thumbnail_target(
    data_dir: &std::path::Path,
    slug: &str,
    ext: &str,
) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    data_dir
        .join("studies")
        .join(slug)
        .join("thumbnails")
        .join(format!("cover-{stamp}.{ext}"))
}

#[tauri::command]
pub fn set_study_thumbnail(
    state: State<'_, AppState>,
    slug: String,
    src_path: String,
) -> AppResult<StudyMeta> {
    validate_slug(&slug)?;
    let src = std::path::Path::new(&src_path);
    if !src.exists() {
        return Err(AppError::InvalidInput {
            message: "이미지 파일을 찾을 수 없습니다".into(),
        });
    }
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if !ALLOWED_THUMBNAIL_EXTS.contains(&ext.as_str()) {
        return Err(AppError::InvalidInput {
            message: format!("지원하지 않는 이미지 형식: .{ext}"),
        });
    }

    // 이전 썸네일 파일 정리(실패 무시) — 확장자가 달라질 수 있어 직접 조회.
    let prev = {
        let db = state.db.lock().expect("db mutex");
        db.conn()
            .query_row(
                "SELECT thumbnail_path FROM studies WHERE slug = ?1",
                params![slug],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(AppError::from)?
            .flatten()
    };
    if let Some(old) = prev {
        let _ = std::fs::remove_file(old);
    }

    let dest = study_thumbnail_target(&state.data_dir, &slug, &ext);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let copied_bytes = std::fs::copy(src, &dest)?;
    tracing::info!(
        target: "study",
        slug = %slug,
        dest = %dest.display(),
        bytes = copied_bytes,
        "set_study_thumbnail: file copied"
    );

    {
        let mut db = state.db.lock().expect("db mutex");
        db.conn_mut().execute(
            "UPDATE studies SET thumbnail_path = ?1 WHERE slug = ?2",
            params![dest.to_string_lossy(), slug],
        )?;
    }

    let updated = {
        let db = state.db.lock().expect("db mutex");
        fetch_one(db.conn(), &slug)?.ok_or_else(|| AppError::NotFound {
            message: format!("스터디 '{slug}'를 찾을 수 없습니다"),
        })?
    };
    // 활성 스터디 캐시 업데이트.
    if updated.is_active {
        *state.active_study.lock().expect("active_study mutex") = Some(updated.clone());
    }
    info!(target: "study", slug = %slug, "set_study_thumbnail");
    Ok(updated)
}

#[tauri::command]
pub fn clear_study_thumbnail(
    state: State<'_, AppState>,
    slug: String,
) -> AppResult<StudyMeta> {
    validate_slug(&slug)?;
    let prev = {
        let db = state.db.lock().expect("db mutex");
        db.conn()
            .query_row(
                "SELECT thumbnail_path FROM studies WHERE slug = ?1",
                params![slug],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(AppError::from)?
            .flatten()
    };
    if let Some(old) = prev {
        let _ = std::fs::remove_file(old);
    }
    {
        let mut db = state.db.lock().expect("db mutex");
        db.conn_mut().execute(
            "UPDATE studies SET thumbnail_path = NULL WHERE slug = ?1",
            params![slug],
        )?;
    }
    let updated = {
        let db = state.db.lock().expect("db mutex");
        fetch_one(db.conn(), &slug)?.ok_or_else(|| AppError::NotFound {
            message: format!("스터디 '{slug}'를 찾을 수 없습니다"),
        })?
    };
    if updated.is_active {
        *state.active_study.lock().expect("active_study mutex") = Some(updated.clone());
    }
    info!(target: "study", slug = %slug, "clear_study_thumbnail");
    Ok(updated)
}

/// PR 68 — 스터디 정보(이름·자유 메모) 편집. 디렉토리 슬러그는 그대로 둔다.
///
/// `name`은 표시용 이름. 비어 있으면 거부.
/// `description`은 자유 메모. `None` 또는 빈 문자열이면 NULL로 클리어.
#[tauri::command]
pub fn update_study_info(
    state: State<'_, AppState>,
    slug: String,
    name: String,
    description: Option<String>,
) -> AppResult<StudyMeta> {
    validate_slug(&slug)?;
    validate_name(&name)?;
    let trimmed_name = name.trim().to_string();
    let trimmed_desc = description
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    {
        let mut db = state.db.lock().expect("db mutex");
        let affected = db.conn_mut().execute(
            "UPDATE studies SET name = ?1, description = ?2 WHERE slug = ?3",
            params![trimmed_name, trimmed_desc, slug],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound {
                message: format!("스터디 '{slug}'를 찾을 수 없습니다"),
            });
        }
    }

    let updated = {
        let db = state.db.lock().expect("db mutex");
        fetch_one(db.conn(), &slug)?.ok_or_else(|| AppError::NotFound {
            message: format!("스터디 '{slug}'를 찾을 수 없습니다"),
        })?
    };
    if updated.is_active {
        *state.active_study.lock().expect("active_study mutex") = Some(updated.clone());
    }
    info!(target: "study", slug = %slug, "update_study_info");
    Ok(updated)
}

/// PR 68 — OS 파일 매니저로 스터디 데이터 디렉토리 열기.
/// `<data_dir>/studies/<slug>` 경로를 OS 기본 파일 매니저(macOS Finder/Linux 파일/Windows 탐색기)로 노출.
#[tauri::command]
pub fn open_study_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    slug: String,
) -> AppResult<()> {
    use tauri_plugin_opener::OpenerExt;
    validate_slug(&slug)?;
    let path = state.data_dir.join("studies").join(&slug);
    if !path.is_dir() {
        return Err(AppError::NotFound {
            message: format!("스터디 폴더가 존재하지 않습니다: {}", path.display()),
        });
    }
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| AppError::Internal {
            message: format!("파일 매니저 열기 실패: {e}"),
        })?;
    info!(target: "study", slug = %slug, path = %path.display(), "open_study_folder");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn validate_slug_accepts_legacy_ascii() {
        // v0.2 시절 슬러그도 그대로 통과해야 한다 — 기존 DB와 호환.
        assert!(validate_slug("rust-deep-dive").is_ok());
        assert!(validate_slug("algo-2025").is_ok());
        assert!(validate_slug("default").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("Has-Upper").is_ok());
        assert!(validate_slug("dot.in.middle").is_ok());
    }

    #[test]
    fn validate_slug_accepts_korean_and_spaces() {
        assert!(validate_slug("러스트").is_ok());
        assert!(validate_slug("러스트 깊게 보기").is_ok());
        assert!(validate_slug("스터디 (2)").is_ok());
        assert!(validate_slug("머신러닝 입문 — 2주차").is_ok());
    }

    #[test]
    fn validate_slug_rejects_forbidden_chars() {
        assert!(validate_slug("").is_err());
        assert!(validate_slug("a/b").is_err());
        assert!(validate_slug("a\\b").is_err());
        assert!(validate_slug("name?").is_err());
        assert!(validate_slug("a:b").is_err());
        assert!(validate_slug("with*star").is_err());
        assert!(validate_slug("quote\"in").is_err());
        assert!(validate_slug("with<bracket").is_err());
        assert!(validate_slug("pipe|here").is_err());
    }

    #[test]
    fn validate_slug_rejects_edge_cases() {
        assert!(validate_slug("  leading space").is_err());
        assert!(validate_slug("trailing space ").is_err());
        assert!(validate_slug(".hidden").is_err());
        assert!(validate_slug("trailing.").is_err());
        assert!(validate_slug("CON").is_err());
        assert!(validate_slug("PRN").is_err());
        assert!(validate_slug("nul").is_err());
        assert!(validate_slug(&"가".repeat(70)).is_err()); // > 200 byte
    }

    #[test]
    fn sanitize_strips_forbidden_chars() {
        assert_eq!(sanitize_to_slug("러스트/깊게"), "러스트깊게");
        assert_eq!(sanitize_to_slug("name?with*bad:chars"), "namewithbadchars");
        assert_eq!(sanitize_to_slug("  spaced  "), "spaced");
        assert_eq!(sanitize_to_slug(".hidden."), "hidden");
    }

    #[test]
    fn sanitize_falls_back_for_empty_or_reserved() {
        assert_eq!(sanitize_to_slug(""), SLUG_FALLBACK);
        assert_eq!(sanitize_to_slug("///"), SLUG_FALLBACK);
        assert_eq!(sanitize_to_slug("..."), SLUG_FALLBACK);
        assert_eq!(sanitize_to_slug("CON"), format!("CON ({SLUG_FALLBACK})"));
    }

    #[test]
    fn sanitize_truncates_long_input() {
        let long = "가".repeat(100); // 300 bytes
        let out = sanitize_to_slug(&long);
        assert!(out.len() <= MAX_SLUG_BYTES);
        assert!(validate_slug(&out).is_ok());
    }

    #[test]
    fn ensure_unique_appends_counter() {
        let db = fresh_db();
        let conn = db.conn();
        // base 슬러그가 비어 있으면 그대로
        assert_eq!(ensure_unique_slug(conn, "러스트").unwrap(), "러스트");
        // 충돌 시 (2) suffix
        let mut db = db;
        insert_study(db.conn_mut(), "러스트", "러스트", "ko").unwrap();
        assert_eq!(
            ensure_unique_slug(db.conn(), "러스트").unwrap(),
            "러스트 (2)"
        );
        insert_study(db.conn_mut(), "러스트 (2)", "러스트", "ko").unwrap();
        assert_eq!(
            ensure_unique_slug(db.conn(), "러스트").unwrap(),
            "러스트 (3)"
        );
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
    fn activate_with_existing_active_does_not_violate_unique() {
        // partial unique index `WHERE is_active = 1` 위반 회귀 방지.
        // 첫 스터디는 자동 활성. 다른 slug로 select_study 호출 시 두 단계 UPDATE로 처리되어야 한다.
        let mut db = fresh_db();
        let a = insert_study(db.conn_mut(), "alpha", "Alpha", "ko").unwrap();
        assert!(a.is_active);
        insert_study(db.conn_mut(), "beta", "Beta", "ko").unwrap();
        // 핵심: UNIQUE constraint 위반 없이 활성 전환.
        activate(db.conn_mut(), "beta").unwrap();
        let active = fetch_active_internal(db.conn()).unwrap().unwrap();
        assert_eq!(active.slug, "beta");
        let alpha = fetch_one(db.conn(), "alpha").unwrap().unwrap();
        assert!(!alpha.is_active);
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
