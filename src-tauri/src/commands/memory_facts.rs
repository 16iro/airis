// memory_facts — v0.5 PR 1 (D-097/D-098).
//
// memory_facts 테이블 CRUD + 시스템 프롬프트 주입 어댑터.
// D-010 b "1회 확인" 부분 supersede — LLM extraction은 자동 INSERT, reports view가 사후 정정.
//
// 시스템 프롬프트 주입 필터: confidence >= 0.5 AND status = 'active'.
// l1 = preference + correction (~2000자), l2 = progress + meta + goal (~4000자).

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::AppState;

// ---- 공개 타입 ---------------------------------------------------------------

/// memory_facts 행 — frontend Fact 타입과 1:1 매핑.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: i64,
    pub study_id: String,
    pub kind: String,
    pub content: String,
    pub source: String,
    pub confidence: f64,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 시스템 프롬프트 주입 결과.
/// l1 = preference + correction, l2 = progress + meta + goal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryInjection {
    pub l1: String,
    pub l2: String,
    pub l1_chars: usize,
    pub l2_chars: usize,
}

// ---- char 한도 ---------------------------------------------------------------

const L1_CHAR_BUDGET: usize = 2_000;
const L2_CHAR_BUDGET: usize = 4_000;

// ---- 내부 DB 헬퍼 -----------------------------------------------------------

/// 현재 UNIX epoch (초).
pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 단일 행 조회 — id 기준.
fn get_fact_by_id(conn: &Connection, id: i64) -> AppResult<Fact> {
    conn.query_row(
        "SELECT id, study_id, kind, content, source, confidence, status, created_at, updated_at \
         FROM memory_facts WHERE id = ?1",
        params![id],
        row_to_fact,
    )
    .map_err(|e| AppError::Db {
        message: format!("memory_facts row not found id={id}: {e}"),
    })
}

fn row_to_fact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Fact> {
    Ok(Fact {
        id: row.get(0)?,
        study_id: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
        source: row.get(4)?,
        confidence: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

// ---- 공개 DB 함수 (non-Tauri, 다른 모듈에서 직접 호출 가능) -----------------

/// fact INSERT. id·created_at·updated_at을 채워 반환.
pub fn insert_fact(
    conn: &Connection,
    study_id: &str,
    kind: &str,
    content: &str,
    source: &str,
    confidence: f64,
) -> AppResult<Fact> {
    validate_kind(kind)?;
    validate_source(source)?;
    let now = now_secs();
    conn.execute(
        "INSERT INTO memory_facts \
            (study_id, kind, content, source, confidence, status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)",
        params![study_id, kind, content, source, confidence, now],
    )?;
    let id = conn.last_insert_rowid();
    get_fact_by_id(conn, id)
}

/// chunk 연관 INSERT.
pub fn insert_fact_chunk(
    conn: &Connection,
    fact_id: i64,
    chunk_id: i64,
    similarity: f64,
) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO memory_fact_chunks (fact_id, chunk_id, similarity) \
         VALUES (?1, ?2, ?3)",
        params![fact_id, chunk_id, similarity],
    )?;
    Ok(())
}

/// 목록 조회 — kind·status 필터 옵션.
pub fn list_facts(
    conn: &Connection,
    study_id: &str,
    kind: Option<&str>,
    status: Option<&str>,
) -> AppResult<Vec<Fact>> {
    // 동적 WHERE 절을 안전하게 조립.
    let mut sql = String::from(
        "SELECT id, study_id, kind, content, source, confidence, status, created_at, updated_at \
         FROM memory_facts WHERE study_id = ?1",
    );
    // rusqlite params_from_iter 대신 런타임 파라미터 바인딩을 위해 Vec<Box<dyn ToSql>> 사용.
    // 단순하게 optional 필터를 SQL 리터럴로 대신 처리 (injection 위험 없음 — CHECK constraint 값).
    if let Some(k) = kind {
        // kind가 CHECK 제약 안에 있는 값인지 검증 후 리터럴 삽입.
        validate_kind(k)?;
        sql.push_str(&format!(" AND kind = '{k}'"));
    }
    if let Some(s) = status {
        validate_status(s)?;
        sql.push_str(&format!(" AND status = '{s}'"));
    }
    sql.push_str(" ORDER BY created_at DESC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![study_id], row_to_fact)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 최근 N일 추가된 facts.
pub fn recent_facts(conn: &Connection, study_id: &str, days: u32) -> AppResult<Vec<Fact>> {
    let cutoff = now_secs() - (days as i64) * 86_400;
    let mut stmt = conn.prepare(
        "SELECT id, study_id, kind, content, source, confidence, status, created_at, updated_at \
         FROM memory_facts \
         WHERE study_id = ?1 AND created_at >= ?2 \
         ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map(params![study_id, cutoff], row_to_fact)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// status 갱신.
pub fn update_fact_status(conn: &Connection, id: i64, status: &str) -> AppResult<()> {
    validate_status(status)?;
    let now = now_secs();
    let changed = conn.execute(
        "UPDATE memory_facts SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now, id],
    )?;
    if changed == 0 {
        return Err(AppError::Db {
            message: format!("memory_facts id={id} not found"),
        });
    }
    Ok(())
}

/// 행 삭제 (memory_fact_chunks는 ON DELETE CASCADE로 자동 정리).
pub fn delete_fact(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM memory_facts WHERE id = ?1", params![id])?;
    Ok(())
}

/// content 수정 — updated_at = now.
pub fn update_fact_content(conn: &Connection, id: i64, content: &str) -> AppResult<()> {
    let now = now_secs();
    let changed = conn.execute(
        "UPDATE memory_facts SET content = ?1, updated_at = ?2 WHERE id = ?3",
        params![content, now, id],
    )?;
    if changed == 0 {
        return Err(AppError::Db {
            message: format!("memory_facts id={id} not found"),
        });
    }
    Ok(())
}

/// 일괄 status 변경. 반환: 갱신된 row 수.
pub fn bulk_update_status(conn: &Connection, ids: &[i64], status: &str) -> AppResult<usize> {
    validate_status(status)?;
    let now = now_secs();
    let mut count = 0usize;
    for &id in ids {
        let changed = conn.execute(
            "UPDATE memory_facts SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now, id],
        )?;
        count += changed;
    }
    Ok(count)
}

/// 시스템 프롬프트 주입용 facts 조회 + l1/l2 포매팅.
/// 필터: confidence >= 0.5 AND status = 'active'.
/// l1 = preference + correction, l2 = progress + meta + goal.
pub fn build_injection(conn: &Connection, study_id: &str) -> AppResult<MemoryInjection> {
    let mut stmt = conn.prepare(
        "SELECT id, study_id, kind, content, source, confidence, status, created_at, updated_at \
         FROM memory_facts \
         WHERE study_id = ?1 AND status = 'active' AND confidence >= 0.5 \
         ORDER BY kind, created_at ASC",
    )?;
    let facts = stmt
        .query_map(params![study_id], row_to_fact)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut l1 = String::new();
    let mut l2 = String::new();

    for fact in &facts {
        let target = match fact.kind.as_str() {
            "preference" | "correction" => &mut l1,
            "progress" | "meta" | "goal" => &mut l2,
            _ => continue,
        };
        target.push_str("- ");
        target.push_str(&fact.content);
        target.push('\n');
    }

    let l1 = truncate_keep_lines(&l1, L1_CHAR_BUDGET);
    let l2 = truncate_keep_lines(&l2, L2_CHAR_BUDGET);
    let l1_chars = l1.chars().count();
    let l2_chars = l2.chars().count();

    Ok(MemoryInjection {
        l1,
        l2,
        l1_chars,
        l2_chars,
    })
}

/// 한도 초과 시 끝에서부터 \n 단위 trim — memory::compress와 동일 정책.
fn truncate_keep_lines(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut buf: String = s.chars().take(limit).collect();
    if let Some(idx) = buf.rfind('\n') {
        buf.truncate(idx);
    }
    buf.push_str("\n…\n");
    buf
}

// ---- 유효성 검사 -----------------------------------------------------------

fn validate_kind(kind: &str) -> AppResult<()> {
    match kind {
        "preference" | "correction" | "progress" | "meta" | "goal" => Ok(()),
        other => Err(AppError::InvalidInput {
            message: format!("알 수 없는 kind: {other}"),
        }),
    }
}

fn validate_status(status: &str) -> AppResult<()> {
    match status {
        "active" | "archived" | "expired" => Ok(()),
        other => Err(AppError::InvalidInput {
            message: format!("알 수 없는 status: {other}"),
        }),
    }
}

fn validate_source(source: &str) -> AppResult<()> {
    match source {
        "user" | "trigger" | "srs" | "metacog" | "recall" | "citation" | "manual" => Ok(()),
        other => Err(AppError::InvalidInput {
            message: format!("알 수 없는 source: {other}"),
        }),
    }
}

// ---- Tauri 명령 -----------------------------------------------------------

/// 5섹션 reports 뷰용 — kind/status 필터 옵션.
#[tauri::command]
pub fn memory_facts_list(
    state: State<'_, AppState>,
    study_id: String,
    kind: Option<String>,
    status: Option<String>,
) -> AppResult<Vec<Fact>> {
    if study_id.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "study_id가 비어 있습니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    list_facts(db.conn(), &study_id, kind.as_deref(), status.as_deref())
}

/// 상단 "최근 N일 추가" 섹션용.
#[tauri::command]
pub fn memory_facts_recent(
    state: State<'_, AppState>,
    study_id: String,
    days: u32,
) -> AppResult<Vec<Fact>> {
    if study_id.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "study_id가 비어 있습니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    recent_facts(db.conn(), &study_id, days)
}

/// 내부/테스트용 단일 fact INSERT.
#[tauri::command]
pub fn memory_facts_insert(
    state: State<'_, AppState>,
    study_id: String,
    kind: String,
    content: String,
    source: String,
    confidence: f64,
) -> AppResult<Fact> {
    if study_id.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "study_id가 비어 있습니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    let fact = insert_fact(db.conn(), &study_id, &kind, &content, &source, confidence)?;
    info!(
        target: "memory_facts",
        study_id = %study_id,
        kind = %fact.kind,
        id = fact.id,
        "fact inserted"
    );
    Ok(fact)
}

/// PR 5에서 활성화 — status 갱신 (현재 노출).
#[tauri::command]
pub fn memory_facts_update_status(
    state: State<'_, AppState>,
    id: i64,
    status: String,
) -> AppResult<()> {
    let db = state.db.lock().expect("db mutex");
    update_fact_status(db.conn(), id, &status)?;
    info!(target: "memory_facts", id = id, status = %status, "fact status updated");
    Ok(())
}

/// PR 5에서 활성화 — 행 삭제 (현재 노출).
#[tauri::command]
pub fn memory_facts_delete(
    state: State<'_, AppState>,
    id: i64,
) -> AppResult<()> {
    let db = state.db.lock().expect("db mutex");
    delete_fact(db.conn(), id)?;
    info!(target: "memory_facts", id = id, "fact deleted");
    Ok(())
}

/// PR 5 신규 — content 수정. updated_at = now.
#[tauri::command]
pub fn memory_facts_update_content(
    state: State<'_, AppState>,
    id: i64,
    content: String,
) -> AppResult<()> {
    if content.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "content가 비어 있습니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    update_fact_content(db.conn(), id, &content)?;
    info!(target: "memory_facts", id = id, "fact content updated");
    Ok(())
}

/// PR 5 신규 — 일괄 status 변경. ids 목록의 status를 일괄 갱신.
/// 반환: 갱신된 row 수.
#[tauri::command]
pub fn memory_facts_bulk_status(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    status: String,
) -> AppResult<usize> {
    validate_status(&status)?;
    if ids.is_empty() {
        return Ok(0);
    }
    let db = state.db.lock().expect("db mutex");
    let count = bulk_update_status(db.conn(), &ids, &status)?;
    info!(
        target: "memory_facts",
        count = count,
        status = %status,
        "bulk status updated"
    );
    Ok(count)
}

/// 시스템 프롬프트 주입 — confidence >= 0.5 AND status='active' facts만.
/// 기존 memory::compress의 l1/l2 분리 패턴 재활용.
#[tauri::command]
pub fn memory_facts_inject(
    state: State<'_, AppState>,
    study_id: String,
) -> AppResult<MemoryInjection> {
    if study_id.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "study_id가 비어 있습니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    build_injection(db.conn(), &study_id)
}

// ---- 단위 테스트 -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn setup() -> Db {
        Db::open_in_memory_for_test()
    }

    #[test]
    fn insert_and_list_facts() {
        let db = setup();
        insert_fact(db.conn(), "s1", "preference", "빠른 결과 우선", "trigger", 0.7).unwrap();
        insert_fact(db.conn(), "s1", "correction", "영어 그대로 둘 것", "trigger", 0.8).unwrap();
        insert_fact(db.conn(), "s1", "progress", "Ch04 진행 중", "trigger", 0.9).unwrap();

        let all = list_facts(db.conn(), "s1", None, None).unwrap();
        assert_eq!(all.len(), 3);

        let prefs = list_facts(db.conn(), "s1", Some("preference"), None).unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].content, "빠른 결과 우선");
    }

    #[test]
    fn list_facts_status_filter() {
        let db = setup();
        insert_fact(db.conn(), "s1", "preference", "active fact", "trigger", 0.8).unwrap();
        let id = db.conn().last_insert_rowid();
        update_fact_status(db.conn(), id, "archived").unwrap();

        let active = list_facts(db.conn(), "s1", None, Some("active")).unwrap();
        assert_eq!(active.len(), 0);
        let archived = list_facts(db.conn(), "s1", None, Some("archived")).unwrap();
        assert_eq!(archived.len(), 1);
    }

    #[test]
    fn inject_filters_by_confidence_and_status() {
        let db = setup();
        // confidence >= 0.5 AND active → 주입됨
        insert_fact(db.conn(), "s1", "preference", "active high", "trigger", 0.9).unwrap();
        // confidence < 0.5 → 주입 X
        insert_fact(db.conn(), "s1", "correction", "low confidence", "trigger", 0.3).unwrap();
        // archived → 주입 X
        insert_fact(db.conn(), "s1", "progress", "archived", "trigger", 0.8).unwrap();
        let last_id = db.conn().last_insert_rowid();
        update_fact_status(db.conn(), last_id, "archived").unwrap();

        let injection = build_injection(db.conn(), "s1").unwrap();
        assert!(injection.l1.contains("active high"), "l1 should contain high-confidence active fact");
        assert!(!injection.l1.contains("low confidence"), "l1 must not contain low confidence fact");
        assert!(!injection.l2.contains("archived"), "l2 must not contain archived fact");
    }

    #[test]
    fn inject_l1_l2_split() {
        let db = setup();
        insert_fact(db.conn(), "s1", "preference", "선호 항목", "trigger", 1.0).unwrap();
        insert_fact(db.conn(), "s1", "correction", "교정 항목", "trigger", 1.0).unwrap();
        insert_fact(db.conn(), "s1", "progress", "진도 항목", "trigger", 1.0).unwrap();
        insert_fact(db.conn(), "s1", "meta", "메타 항목", "trigger", 1.0).unwrap();
        insert_fact(db.conn(), "s1", "goal", "목표 항목", "trigger", 1.0).unwrap();

        let inj = build_injection(db.conn(), "s1").unwrap();
        assert!(inj.l1.contains("선호 항목"));
        assert!(inj.l1.contains("교정 항목"));
        assert!(inj.l2.contains("진도 항목"));
        assert!(inj.l2.contains("메타 항목"));
        assert!(inj.l2.contains("목표 항목"));
    }

    #[test]
    fn inject_truncates_l1_when_over_budget() {
        let db = setup();
        // L1_CHAR_BUDGET = 2000. 각 항목이 ~100자 × 30 = 3000자 이상이 되도록 여러 건 INSERT.
        for i in 0..30_usize {
            let content = format!("항목 {} {}", i, "긴 내용 데이터 ".repeat(10));
            insert_fact(db.conn(), "s1", "preference", &content, "trigger", 1.0).unwrap();
        }
        let inj = build_injection(db.conn(), "s1").unwrap();
        assert!(inj.l1_chars <= L1_CHAR_BUDGET + 10, "l1 should be truncated");
        assert!(inj.l1.ends_with("…\n"), "truncated l1 must end with ellipsis");
    }

    #[test]
    fn delete_fact_removes_row() {
        let db = setup();
        insert_fact(db.conn(), "s1", "preference", "to delete", "trigger", 0.5).unwrap();
        let id = db.conn().last_insert_rowid();
        delete_fact(db.conn(), id).unwrap();
        let cnt: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM memory_facts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 0);
    }

    #[test]
    fn recent_facts_filters_by_time() {
        let db = setup();
        let old_time = now_secs() - 10 * 86_400; // 10일 전
        db.conn()
            .execute(
                "INSERT INTO memory_facts \
                    (study_id, kind, content, source, confidence, status, created_at, updated_at) \
                 VALUES ('s1', 'preference', 'old fact', 'trigger', 0.7, 'active', ?1, ?1)",
                params![old_time],
            )
            .unwrap();
        insert_fact(db.conn(), "s1", "preference", "recent fact", "trigger", 0.7).unwrap();

        let recent = recent_facts(db.conn(), "s1", 7).unwrap();
        // 최근 7일 내 = recent fact만
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].content, "recent fact");
    }

    #[test]
    fn validate_kind_rejects_unknown() {
        assert!(validate_kind("preference").is_ok());
        assert!(validate_kind("correction").is_ok());
        assert!(validate_kind("progress").is_ok());
        assert!(validate_kind("meta").is_ok());
        assert!(validate_kind("goal").is_ok());
        assert!(validate_kind("unknown").is_err());
    }

    #[test]
    fn validate_status_rejects_unknown() {
        assert!(validate_status("active").is_ok());
        assert!(validate_status("archived").is_ok());
        assert!(validate_status("expired").is_ok());
        assert!(validate_status("deleted").is_err());
    }

    // ---- PR 5 신규 명령 테스트 -----------------------------------------------

    #[test]
    fn update_content_changes_text() {
        let db = setup();
        insert_fact(db.conn(), "s1", "preference", "original", "trigger", 0.8).unwrap();
        let id = db.conn().last_insert_rowid();
        update_fact_content(db.conn(), id, "updated content").unwrap();
        let fact = get_fact_by_id(db.conn(), id).unwrap();
        assert_eq!(fact.content, "updated content");
        assert!(fact.updated_at >= fact.created_at);
    }

    #[test]
    fn update_content_empty_content_is_valid_at_db_level() {
        // 내부 함수는 빈 문자열을 허용 — 검증은 Tauri command 레이어에서만.
        // 이 테스트는 whitespace만 있어도 DB에는 저장됨을 확인 (정책 분리 명시).
        let db = setup();
        insert_fact(db.conn(), "s1", "preference", "original", "trigger", 0.8).unwrap();
        let id = db.conn().last_insert_rowid();
        // 내부 함수 직접 호출은 OK — Tauri command에서만 empty 거부.
        assert!(update_fact_content(db.conn(), id, "   ").is_ok());
    }

    #[test]
    fn update_content_returns_error_for_missing_id() {
        let db = setup();
        let result = update_fact_content(db.conn(), 9999, "x");
        assert!(result.is_err());
    }

    #[test]
    fn bulk_status_updates_multiple_rows() {
        let db = setup();
        insert_fact(db.conn(), "s1", "preference", "a", "trigger", 0.8).unwrap();
        let id1 = db.conn().last_insert_rowid();
        insert_fact(db.conn(), "s1", "correction", "b", "trigger", 0.8).unwrap();
        let id2 = db.conn().last_insert_rowid();
        insert_fact(db.conn(), "s1", "progress", "c", "trigger", 0.8).unwrap();
        let id3 = db.conn().last_insert_rowid();

        let count = bulk_update_status(db.conn(), &[id1, id2], "archived").unwrap();
        assert_eq!(count, 2);

        let f3 = get_fact_by_id(db.conn(), id3).unwrap();
        assert_eq!(f3.status, "active", "id3 should remain active");
        let f1 = get_fact_by_id(db.conn(), id1).unwrap();
        assert_eq!(f1.status, "archived");
    }

    #[test]
    fn bulk_status_rejects_invalid_status() {
        let db = setup();
        let result = bulk_update_status(db.conn(), &[1, 2], "deleted");
        assert!(result.is_err());
    }

    #[test]
    fn bulk_status_empty_ids_returns_zero() {
        let db = setup();
        let count = bulk_update_status(db.conn(), &[], "archived").unwrap();
        assert_eq!(count, 0);
    }
}
