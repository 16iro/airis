// intervention.rs — v0.5 PR 3 (D-100).
//
// 메타인지 Level 1 알림 + backend 3지표 수집·발화.
//
// 3지표:
//   1. repeat_search    — search.rs에 이미 INSERT 로직 존재. 여기서는 조합 검출만.
//   2. progress_recall_gap — RAG citation_check 평균 vs 현재 읽기 진도 (청크 ord 비율) 격차.
//   3. self_report_low  — chat 발화 자기보고 정규식 hit + 직전 citation 평균 ≤ 0.4.
//
// 발화 정책 (D-031):
//   - 서로 다른 signal_type ≥ 2개 동시(5분 내) + cooldown 30분(같은 조합) → metacog:alert emit.
//   - memory_facts INSERT (kind='meta', source='metacog').
//   - 차단 X, toast 경고만.
//
// 주의:
//   - intervention_signals.fired_at: ISO 8601 텍스트 (datetime('now')).
//   - memory_facts.created_at: epoch ms (INTEGER).
//   - 두 단위가 다르므로 cooldown 체크 시 절대 섞지 않는다.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tracing::{info, warn};

use crate::commands::memory_facts::insert_fact;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::AppState;

// ---- 임계값 상수 -----------------------------------------------------------

/// citation_check 평균 vs 청크 진도 비율 격차 임계값.
pub const PROGRESS_RECALL_GAP_THRESHOLD: f64 = 0.3;

/// self_report_low: 직전 citation 평균 ≤ 이 값이면 발화.
pub const SELF_REPORT_CITATION_THRESHOLD: f64 = 0.4;

/// 최근 발화 검사 윈도우 (분).
const ALERT_WINDOW_MIN: i64 = 5;

/// cooldown 윈도우 (초) — 같은 지표 조합 30분 내 재발화 X.
const COOLDOWN_SECS: i64 = 30 * 60;

// ---- 공개 타입 ---------------------------------------------------------------

/// evaluate_metacog_signals 반환.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetacogEvaluation {
    pub inserted_signal_ids: Vec<i64>,
    pub alert_emitted: Option<MetacogAlert>,
}

/// metacog:alert 이벤트 payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetacogAlert {
    pub signal_ids: Vec<i64>,
    pub signal_types: Vec<String>,
    pub message: String,
    pub fired_at: String,
}

/// intervention_signals 행 — frontend 표시용.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterventionSignal {
    pub id: i64,
    pub study_slug: String,
    pub signal_type: String,
    pub severity: f64,
    pub metadata_json: Option<String>,
    pub fired_at: String,
    pub user_dismissed: bool,
}

// ---- ISO 8601 현재 시간 ---------------------------------------------------

fn now_iso() -> String {
    // SQLite datetime('now') 포맷과 호환.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    chrono_from_secs(now.as_secs())
}

/// epoch 초 → "YYYY-MM-DD HH:MM:SS" (SQLite datetime 포맷).
fn chrono_from_secs(secs: u64) -> String {
    // 간단한 수동 변환 — chrono 의존성 추가 없이.
    let s = secs as i64;
    // 2000-01-01 00:00:00 UTC = 946684800
    let epoch_2000: i64 = 946_684_800;
    if s < epoch_2000 {
        return "2000-01-01 00:00:00".to_string();
    }
    // 1일 = 86400초. 4년 주기 (leap year 단순 근사).
    let days_since_epoch = s / 86400;
    let time_of_day = s % 86400;

    // 야니 Tomohon 알고리즘: civil date from epoch days.
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let h = time_of_day / 3600;
    let min = (time_of_day % 3600) / 60;
    let sec = time_of_day % 60;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, m, d, h, min, sec
    )
}

// ---- self_report_low 정규식 패턴 ------------------------------------------

/// 자기과신 발화 패턴 (한국어 + 영문 보조).
/// 검출 결과: (hit: bool, matched_phrases: Vec<&str>).
fn detect_self_report(text: &str) -> (bool, Vec<String>) {
    let patterns: &[&str] = &[
        "잘 안다",
        "잘 알아",
        "이미 안다",
        "이미 알아",
        "쉽다",
        "쉬워",
        "이건 쉬워",
        "다 알아",
        "이해했다",
        "i know",
        "i got it",
        "easy",
    ];
    let lower = text.to_lowercase();
    let mut matched = Vec::new();
    for pat in patterns {
        if lower.contains(&pat.to_lowercase()) {
            matched.push(pat.to_string());
        }
    }
    (!matched.is_empty(), matched)
}

// ---- progress_recall_gap 검출 -------------------------------------------

/// (severity, metadata_json) — 격차 ≥ threshold 시 Some.
///
/// citation_avg: 최근 chat의 citation_check 평균 (0~1).
/// progress: 현재 읽기 진도 (0~1) — 청크 ord 비율.
pub fn detect_progress_recall_gap(
    citation_avg: Option<f64>,
    progress: Option<f64>,
) -> Option<(f64, String)> {
    let citation_avg = citation_avg?;
    let progress = progress?;
    let gap = (citation_avg - progress).abs();
    if gap >= PROGRESS_RECALL_GAP_THRESHOLD {
        // severity: 0.5 base + 격차 비례.
        let severity = (0.5 + (gap - PROGRESS_RECALL_GAP_THRESHOLD) / 0.7).clamp(0.0, 1.0);
        let metadata = serde_json::json!({
            "citation_avg": citation_avg,
            "progress": progress,
            "gap": gap,
        })
        .to_string();
        Some((severity, metadata))
    } else {
        None
    }
}

// ---- self_report_low 검출 ------------------------------------------------

/// (severity, metadata_json) — 정규식 hit + citation_avg ≤ threshold 시 Some.
pub fn detect_self_report_low(
    user_msg: &str,
    citation_avg: Option<f64>,
) -> Option<(f64, String)> {
    let (hit, matched) = detect_self_report(user_msg);
    if !hit {
        return None;
    }
    let citation_avg = citation_avg.unwrap_or(0.0);
    if citation_avg <= SELF_REPORT_CITATION_THRESHOLD {
        let severity = (0.5 + (SELF_REPORT_CITATION_THRESHOLD - citation_avg)).clamp(0.0, 1.0);
        let metadata = serde_json::json!({
            "matched_phrases": matched,
            "citation_avg": citation_avg,
        })
        .to_string();
        Some((severity, metadata))
    } else {
        None
    }
}

// ---- INSERT 헬퍼 ----------------------------------------------------------

/// intervention_signals에 단일 신호 INSERT. 삽입된 id 반환.
fn insert_signal(
    conn: &Connection,
    study_slug: &str,
    signal_type: &str,
    severity: f64,
    metadata: &str,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO intervention_signals \
            (study_slug, signal_type, severity, metadata_json, fired_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now'))",
        params![study_slug, signal_type, severity, metadata],
    )?;
    Ok(conn.last_insert_rowid())
}

// ---- 조합 임계 + cooldown 검사 -------------------------------------------

/// 최근 ALERT_WINDOW_MIN 분 내 signals 중 서로 다른 signal_type ≥ 2개이면
/// MetacogAlert 구성. cooldown 체크 포함.
///
/// cooldown 구현: memory_facts에서 같은 조합 키를 포함한 row가 30분 내 존재 시 skip.
/// (memory_facts.created_at은 epoch ms — COOLDOWN_SECS * 1000으로 변환)
fn check_alert_threshold(
    conn: &Connection,
    study_slug: &str,
) -> AppResult<Option<MetacogAlert>> {
    // 최근 ALERT_WINDOW_MIN 분 row (미dismiss).
    let window_expr = format!("-{ALERT_WINDOW_MIN} minutes");
    let mut stmt = conn.prepare(
        "SELECT id, signal_type FROM intervention_signals \
         WHERE study_slug = ?1 \
           AND fired_at >= datetime('now', ?2) \
           AND user_dismissed = 0 \
         ORDER BY fired_at DESC",
    )?;
    let recent: Vec<(i64, String)> = stmt
        .query_map(params![study_slug, window_expr], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // signal_type별 id 그룹핑.
    let mut by_type: HashMap<String, Vec<i64>> = HashMap::new();
    for (id, ty) in &recent {
        by_type.entry(ty.clone()).or_default().push(*id);
    }

    // 서로 다른 signal_type < 2 → alert X.
    if by_type.len() < 2 {
        return Ok(None);
    }

    // 조합 키 생성 (정렬된 signal_type 문자열).
    let mut combo: Vec<&str> = by_type.keys().map(|s| s.as_str()).collect();
    combo.sort_unstable();
    let combo_key = combo.join("+");

    // cooldown — memory_facts에서 같은 조합 키 30분 내 존재 시 skip.
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let cutoff_ms = now_ms - COOLDOWN_SECS * 1_000;

    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM memory_facts \
             WHERE study_id = ?1 \
               AND kind = 'meta' \
               AND source = 'metacog' \
               AND content LIKE '%' || ?2 || '%' \
               AND created_at >= ?3 \
             LIMIT 1",
            params![study_slug, &combo_key, cutoff_ms],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if exists {
        return Ok(None);
    }

    let signal_ids: Vec<i64> = by_type.values().flatten().copied().collect();
    let signal_types: Vec<String> = combo.iter().map(|s| s.to_string()).collect();
    let message = format_alert_message(&signal_types);

    Ok(Some(MetacogAlert {
        signal_ids,
        signal_types,
        message,
        fired_at: now_iso(),
    }))
}

/// signal_types → 한국어 알림 메시지.
fn format_alert_message(signal_types: &[String]) -> String {
    let labels: Vec<&str> = signal_types
        .iter()
        .map(|t| signal_type_label(t))
        .collect();
    format!(
        "능력 착각 신호: {} 동시 발화",
        labels.join(" + ")
    )
}

/// signal_type → 한국어 레이블.
fn signal_type_label(t: &str) -> &str {
    match t {
        "repeat_search" => "같은 검색 반복",
        "progress_recall_gap" => "진도-회상 격차",
        "self_report_low" => "자기보고-실제 격차",
        "short_dwell" => "짧은 체류",
        "forced_output_miss" => "응답 후 정주행 X",
        _ => t,
    }
}

// ---- 공개 핵심 함수 -------------------------------------------------------

/// 매 chat:done 후 background에서 호출.
///
/// 1) progress_recall_gap + self_report_low 검출 + INSERT (조건 만족 시).
/// 2) 최근 5분 signals → 서로 다른 type ≥ 2개 + cooldown 통과 → alert emit + memory_facts INSERT.
///
/// citation_avg: 이번 chat 응답의 citation_check 점수 평균 (없으면 None).
/// progress:     현재 활성 책의 읽기 진도 비율 0~1 (없으면 None).
pub fn evaluate_metacog_signals(
    app: &AppHandle,
    db: &Db,
    study_slug: &str,
    user_msg: &str,
    citation_avg: Option<f64>,
    progress: Option<f64>,
    metacog_enabled: bool,
) -> AppResult<MetacogEvaluation> {
    if !metacog_enabled {
        return Ok(MetacogEvaluation {
            inserted_signal_ids: Vec::new(),
            alert_emitted: None,
        });
    }

    let conn = db.conn();
    let mut inserted_ids: Vec<i64> = Vec::new();

    // 1a) progress_recall_gap 검출.
    if let Some((severity, metadata)) =
        detect_progress_recall_gap(citation_avg, progress)
    {
        match insert_signal(conn, study_slug, "progress_recall_gap", severity, &metadata) {
            Ok(id) => {
                inserted_ids.push(id);
                info!(
                    target: "intervention",
                    study = %study_slug,
                    signal = "progress_recall_gap",
                    severity = severity,
                    "signal inserted"
                );
            }
            Err(e) => warn!(target: "intervention", error = %e, "progress_recall_gap insert failed"),
        }
    }

    // 1b) self_report_low 검출.
    if let Some((severity, metadata)) = detect_self_report_low(user_msg, citation_avg) {
        match insert_signal(conn, study_slug, "self_report_low", severity, &metadata) {
            Ok(id) => {
                inserted_ids.push(id);
                info!(
                    target: "intervention",
                    study = %study_slug,
                    signal = "self_report_low",
                    severity = severity,
                    "signal inserted"
                );
            }
            Err(e) => warn!(target: "intervention", error = %e, "self_report_low insert failed"),
        }
    }

    // 2) 조합 임계 + cooldown 검사 → alert.
    let alert = match check_alert_threshold(conn, study_slug) {
        Ok(Some(alert)) => {
            // memory_facts INSERT — kind='meta', source='metacog'.
            let confidence = alert.signal_types.len() as f64 / 5.0;
            let content = format!(
                "능력 착각 신호: {} (서로 다른 신호 {}개 동시, {})",
                alert.signal_types.join("+"),
                alert.signal_types.len(),
                &alert.fired_at,
            );
            match insert_fact(conn, study_slug, "meta", &content, "metacog", confidence) {
                Ok(fact) => {
                    info!(
                        target: "intervention",
                        study = %study_slug,
                        fact_id = fact.id,
                        combo = %alert.signal_types.join("+"),
                        "metacog alert fact inserted"
                    );
                }
                Err(e) => warn!(target: "intervention", error = %e, "metacog alert fact insert failed"),
            }

            // metacog:alert 이벤트 emit.
            if let Err(e) = app.emit("metacog:alert", &alert) {
                warn!(target: "intervention", error = %e, "metacog:alert emit failed");
            }

            Some(alert)
        }
        Ok(None) => None,
        Err(e) => {
            warn!(target: "intervention", error = %e, "check_alert_threshold failed (non-fatal)");
            None
        }
    };

    Ok(MetacogEvaluation {
        inserted_signal_ids: inserted_ids,
        alert_emitted: alert,
    })
}

// ---- Tauri 명령 -----------------------------------------------------------

/// signal dismiss — user_dismissed = 1 마킹.
#[tauri::command]
pub fn intervention_signal_dismiss(
    state: State<'_, AppState>,
    signal_id: i64,
) -> AppResult<()> {
    let db = state.db.lock().expect("db mutex");
    let changed = db.conn().execute(
        "UPDATE intervention_signals SET user_dismissed = 1 WHERE id = ?1",
        params![signal_id],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound {
            message: format!("intervention_signals id={signal_id} not found"),
        });
    }
    info!(target: "intervention", signal_id = signal_id, "signal dismissed");
    Ok(())
}

/// 최근 N일 signals 조회.
#[tauri::command]
pub fn intervention_signal_recent(
    state: State<'_, AppState>,
    study_slug: String,
    days: u32,
) -> AppResult<Vec<InterventionSignal>> {
    if study_slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "study_slug가 비어 있습니다".into(),
        });
    }
    let db = state.db.lock().expect("db mutex");
    let window_expr = format!("-{} days", days);
    let mut stmt = db.conn().prepare(
        "SELECT id, study_slug, signal_type, severity, metadata_json, fired_at, user_dismissed \
         FROM intervention_signals \
         WHERE study_slug = ?1 AND fired_at >= datetime('now', ?2) \
         ORDER BY fired_at DESC",
    )?;
    let rows = stmt
        .query_map(params![study_slug, window_expr], |r| {
            Ok(InterventionSignal {
                id: r.get(0)?,
                study_slug: r.get(1)?,
                signal_type: r.get(2)?,
                severity: r.get(3)?,
                metadata_json: r.get(4)?,
                fired_at: r.get(5)?,
                user_dismissed: r.get::<_, i64>(6)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---- 읽기 진도 헬퍼 -------------------------------------------------------

/// 활성 스터디의 책 읽기 진도 비율 (0.0 ~ 1.0) 추출.
///
/// 구현: study의 책 중 T1 인덱싱 완료된 책에서 active_section 또는 최근 인용 청크의
/// ord 비율을 진도로 간주.
///
/// books 테이블에 progress 컬럼이 없으므로:
///   - active_section.section_path와 일치하는 chunk의 ord / max(ord) 로 계산.
///   - active_section 없으면 None → progress_recall_gap skip.
pub fn compute_progress(
    conn: &Connection,
    study_slug: &str,
    active_section_path: Option<&str>,
) -> Option<f64> {
    let section_path = active_section_path?;

    // 해당 section_path를 가진 chunk의 최소 ord (섹션 첫 청크).
    // MIN(c.ord)이 NULL일 수 있으므로 Option<i64>로 받음.
    let section_ord: Option<i64> = conn
        .query_row(
            "SELECT MIN(c.ord) FROM chunks c \
             JOIN books b ON b.id = c.book_id \
             WHERE b.study_slug = ?1 AND c.section_path = ?2",
            params![study_slug, section_path],
            |r| r.get(0),
        )
        .ok()?;
    let section_ord = section_ord?;

    // 해당 study의 총 청크 최대 ord (T1 인덱싱 완료 책 한정).
    let total_ord: Option<i64> = conn
        .query_row(
            "SELECT MAX(c.ord) FROM chunks c \
             JOIN books b ON b.id = c.book_id \
             WHERE b.study_slug = ?1",
            params![study_slug],
            |r| r.get(0),
        )
        .ok()?;
    let total_ord = total_ord?;

    if total_ord == 0 {
        return None;
    }

    Some((section_ord as f64 / total_ord as f64).clamp(0.0, 1.0))
}

/// CitationVerdict 목록에서 score 평균 계산.
/// score 필드 = cross-encoder raw score.
pub fn citation_scores_avg(scores: &[crate::index::v043::citation_check::CitationVerdict]) -> Option<f64> {
    if scores.is_empty() {
        return None;
    }
    let sum: f64 = scores.iter().map(|v| v.score as f64).sum();
    Some(sum / scores.len() as f64)
}

// ---- 단위 테스트 -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    /// in-memory DB + studies 행 삽입 (intervention_signals FK 충족).
    fn setup() -> Db {
        let db = Db::open_in_memory_for_test();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, language, created_at, is_active) \
                 VALUES ('s1', 'test', 'ko', datetime('now'), 0)",
                [],
            )
            .expect("insert study for test");
        db
    }

    // ---- detect_progress_recall_gap ----------------------------------------

    #[test]
    fn gap_below_threshold_is_none() {
        // gap = |0.6 - 0.5| = 0.1 < 0.3 → None
        let result = detect_progress_recall_gap(Some(0.6), Some(0.5));
        assert!(result.is_none(), "gap 0.1 should not fire");
    }

    #[test]
    fn gap_at_threshold_fires() {
        // gap = |0.9 - 0.5| = 0.4 > 0.3 → Some. 부동소수점 회피용 마진 포함.
        let result = detect_progress_recall_gap(Some(0.9), Some(0.5));
        assert!(result.is_some(), "gap 0.4 (above threshold 0.3) should fire");
        let (severity, metadata) = result.unwrap();
        assert!(severity >= 0.5, "severity should be >= 0.5 base");
        assert!(metadata.contains("gap"), "metadata should contain gap key");
    }

    #[test]
    fn gap_above_threshold_has_higher_severity() {
        // low_gap: |0.8 - 0.45| = 0.35 > 0.3 → fires. high_gap: |0.9 - 0.2| = 0.7 → higher severity.
        let low_gap = detect_progress_recall_gap(Some(0.8), Some(0.45)).unwrap().0;
        let high_gap = detect_progress_recall_gap(Some(0.9), Some(0.2)).unwrap().0;
        assert!(high_gap > low_gap, "higher gap should produce higher severity");
    }

    #[test]
    fn gap_none_citation_returns_none() {
        let result = detect_progress_recall_gap(None, Some(0.5));
        assert!(result.is_none(), "no citation_avg → None");
    }

    #[test]
    fn gap_none_progress_returns_none() {
        let result = detect_progress_recall_gap(Some(0.8), None);
        assert!(result.is_none(), "no progress → None");
    }

    // ---- detect_self_report_low -------------------------------------------

    #[test]
    fn self_report_regex_hit_with_low_citation_fires() {
        // "쉽다" hit + citation_avg=0.3 ≤ 0.4 → Some
        let result = detect_self_report_low("이건 쉽다", Some(0.3));
        assert!(result.is_some(), "regex hit + low citation should fire");
        let (severity, metadata) = result.unwrap();
        assert!(severity >= 0.5);
        assert!(metadata.contains("쉽다") || metadata.contains("matched_phrases"));
    }

    #[test]
    fn self_report_regex_hit_with_high_citation_skips() {
        // "쉽다" hit + citation_avg=0.5 > 0.4 → None
        let result = detect_self_report_low("이건 쉽다", Some(0.5));
        assert!(result.is_none(), "regex hit + high citation should NOT fire");
    }

    #[test]
    fn self_report_no_regex_hit_skips() {
        // no match → None regardless of citation
        let result = detect_self_report_low("Rust의 소유권에 대해 설명해줘", Some(0.1));
        assert!(result.is_none(), "no regex hit → None");
    }

    #[test]
    fn self_report_all_phrases_matched() {
        let phrases = [
            "잘 안다", "잘 알아", "이미 안다", "이미 알아",
            "쉽다", "쉬워", "이건 쉬워", "다 알아", "이해했다",
        ];
        for p in &phrases {
            let result = detect_self_report_low(p, Some(0.0));
            assert!(result.is_some(), "phrase '{}' should match", p);
        }
    }

    // ---- check_alert_threshold --------------------------------------------

    #[test]
    fn single_signal_does_not_fire_alert() {
        let db = setup();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'repeat_search', 0.5, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        let alert = check_alert_threshold(conn, "s1").unwrap();
        assert!(alert.is_none(), "single signal type should not fire alert");
    }

    #[test]
    fn two_different_signals_fire_alert() {
        let db = setup();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'repeat_search', 0.5, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'self_report_low', 0.6, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        let alert = check_alert_threshold(conn, "s1").unwrap();
        assert!(alert.is_some(), "two different signal types should fire alert");
        let alert = alert.unwrap();
        assert_eq!(alert.signal_types.len(), 2);
    }

    #[test]
    fn same_combo_within_cooldown_skips() {
        let db = setup();
        let conn = db.conn();
        // 같은 조합으로 이미 memory_facts 행이 있음 (최근).
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'repeat_search', 0.5, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'self_report_low', 0.6, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        // memory_facts에 같은 조합 키로 최근 행 존재 → cooldown.
        conn.execute(
            "INSERT INTO memory_facts \
                (study_id, kind, content, source, confidence, status, created_at, updated_at) \
             VALUES ('s1', 'meta', '능력 착각 신호: repeat_search+self_report_low (서로 다른 신호 2개 동시, 2026-01-01 00:00:00)', 'metacog', 0.4, 'active', ?1, ?1)",
            params![now_ms],
        )
        .unwrap();
        let alert = check_alert_threshold(conn, "s1").unwrap();
        assert!(alert.is_none(), "same combo within cooldown should skip alert");
    }

    #[test]
    fn different_combo_within_cooldown_fires() {
        let db = setup();
        let conn = db.conn();
        // 다른 조합 = progress_recall_gap 포함. 기존 cooldown은 repeat_search+self_report_low.
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'repeat_search', 0.5, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'progress_recall_gap', 0.7, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // 기존 cooldown은 완전히 다른 조합.
        conn.execute(
            "INSERT INTO memory_facts \
                (study_id, kind, content, source, confidence, status, created_at, updated_at) \
             VALUES ('s1', 'meta', '능력 착각 신호: self_report_low+progress_recall_gap+X (서로 다른 신호 3개 동시, ...)', 'metacog', 0.6, 'active', ?1, ?1)",
            params![now_ms],
        )
        .unwrap();
        let alert = check_alert_threshold(conn, "s1").unwrap();
        assert!(alert.is_some(), "different combo should fire even within cooldown window");
    }

    // ---- dismiss ----------------------------------------------------------

    #[test]
    fn dismiss_marks_user_dismissed() {
        let db = setup();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO intervention_signals \
                (study_slug, signal_type, severity, metadata_json, fired_at) \
             VALUES ('s1', 'repeat_search', 0.5, NULL, datetime('now'))",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE intervention_signals SET user_dismissed = 1 WHERE id = ?1",
            params![id],
        )
        .unwrap();
        let dismissed: i64 = conn
            .query_row(
                "SELECT user_dismissed FROM intervention_signals WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dismissed, 1, "user_dismissed should be 1 after dismiss");
    }

    // ---- format_alert_message -------------------------------------------

    #[test]
    fn alert_message_contains_korean_labels() {
        let msg = format_alert_message(&[
            "repeat_search".to_string(),
            "self_report_low".to_string(),
        ]);
        assert!(msg.contains("같은 검색 반복"), "message should contain Korean label");
        assert!(msg.contains("자기보고-실제 격차"));
    }

    // ---- now_iso --------------------------------------------------------

    #[test]
    fn now_iso_has_correct_format() {
        let s = now_iso();
        // "YYYY-MM-DD HH:MM:SS"
        assert_eq!(s.len(), 19, "ISO format should be 19 chars");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
    }
}
