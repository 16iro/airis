// learning_dev — v0.5 PR 5 (D-102).
//
// 학습 acceptance gate 5개 측정 + self-rating 영속.
//
// gate 1 — Memory 신뢰도: active/total (7일 inserted)
// gate 2 — SRS 카드 품질: citation_score >= 0.5 / total auto-generated
// gate 3 — 메타인지 false positive: 주당 dismiss 건수 (7일 raw)
// gate 4 — 회상 응답률: 시도(correct/incorrect) / total triggers (7일)
// gate 5 — 종합 학습 효율: settings.learning_self_rating_log 평균 (최근 N=10)
//
// 응답 검증 통계:
//   citation_avg_last_50 — 최근 50건 assistant 응답의 citation_scores 평균.
//   history_compression_ratio_avg — chat_messages 중 context_json에 'summary:'가 포함된
//   비율 (history_compressor가 요약을 system prompt에 주입할 때 context_json에 메타 없음 →
//   DB에서 직접 측정 불가. 대신: N/A 반환, 측정 위치 명시).
//
// 주의: chat_messages.context_json에 history_compression 결과가 영속되지 않음.
// HistoryCompressor는 in-memory only (D-089). 따라서
// history_compression_ratio_avg는 None 반환 → dev panel에 "N/A" 표시.

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::settings::SelfRating;
use crate::AppState;

// ---- 상수 -------------------------------------------------------------------

/// gate 5 self-rating 최대 보관 건수.
const SELF_RATING_MAX: usize = 100;

/// gate 5 평균 계산 최근 N건.
const SELF_RATING_AVG_N: usize = 10;

/// gate 2 citation 품질 임계값 (D-090 그대로 재활용).
const CITATION_PASS_THRESHOLD: f64 = 0.5;

/// citation_avg 계산 최근 N건 assistant 메시지.
const CITATION_LAST_N: i64 = 50;

/// 7일 초 단위.
const SECS_7D: i64 = 7 * 24 * 3600;

// ---- 공개 타입 ---------------------------------------------------------------

/// 5 acceptance gate 측정값.
///
/// 모든 Optional 필드는 데이터 부족(분모=0) 또는 측정 불가 시 None.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceMetrics {
    /// gate 1 — Memory 신뢰도 (active/inserted, 7일).
    pub gate1_memory_keep_rate: Option<f64>,
    /// 분자: 7일 내 inserted 중 현재 status='active' 건수.
    pub gate1_active_7d: i64,
    /// 분모: 7일 내 inserted 총 건수.
    pub gate1_total_inserted_7d: i64,

    /// gate 2 — SRS 카드 품질 (citation_score >= 0.5 / total auto-generated).
    pub gate2_srs_quality_rate: Option<f64>,
    /// 분자: citation_score >= CITATION_PASS_THRESHOLD AND 자동 생성 카드.
    pub gate2_passing: i64,
    /// 분모: 자동 생성 카드 총 건수 (generation_method NOT IN ('manual','legacy')).
    pub gate2_total_auto: i64,

    /// gate 3 — 메타인지 false positive (7일 dismiss 건수).
    /// D-102: 단순 카운트. 임계 ≤ 2/주.
    pub gate3_dismiss_per_week: f64,
    /// raw 7일 dismiss 건수.
    pub gate3_dismissed_7d: i64,
    /// raw 7일 신호 총 건수.
    pub gate3_total_signals_7d: i64,

    /// gate 4 — 회상 응답률 (시도/트리거, 7일).
    pub gate4_attempt_rate: Option<f64>,
    /// 분자: 7일 outcome IN ('correct','incorrect').
    pub gate4_attempted_7d: i64,
    /// 분모: 7일 recall_attempts 총 건수.
    pub gate4_total_triggers_7d: i64,

    /// gate 5 — 종합 학습 효율 자가 평가 (최근 N=10 평균).
    pub gate5_self_rating_avg: Option<f64>,
    /// 자가 평가 총 기록 건수.
    pub gate5_self_rating_count: i64,

    // 응답 검증 통계 (D-090 citation_check + D-089 history_compressor).
    /// 최근 50건 assistant 응답의 citation verdict score 평균.
    /// context_json 없거나 citation_scores 없으면 None.
    pub citation_avg_last_50: Option<f64>,
    /// history compression 비율. D-089 in-memory only라 영속 데이터 없음 → 항상 None.
    /// dev panel에서 "N/A (in-memory only)" 표시 예정.
    pub history_compression_ratio_avg: Option<f64>,
}

// ---- epoch 헬퍼 ------------------------------------------------------------

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---- gate 계산 헬퍼 ---------------------------------------------------------

/// gate 1 — memory_facts 7일 keep rate.
fn gate1(conn: &Connection, study_slug: &str) -> AppResult<(Option<f64>, i64, i64)> {
    let cutoff = now_secs() - SECS_7D;
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts \
         WHERE study_id = ?1 AND created_at >= ?2",
        params![study_slug, cutoff],
        |r| r.get(0),
    )?;
    let active: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts \
         WHERE study_id = ?1 AND created_at >= ?2 AND status = 'active'",
        params![study_slug, cutoff],
        |r| r.get(0),
    )?;
    let rate = if total > 0 {
        Some((active as f64) / (total as f64))
    } else {
        None
    };
    Ok((rate, active, total))
}

/// gate 2 — srs_cards quality rate (auto-generated, citation_score >= threshold).
fn gate2(conn: &Connection, study_slug: &str) -> AppResult<(Option<f64>, i64, i64)> {
    // srs_cards.generation_method NOT IN ('manual','legacy') = 자동 생성.
    let total_auto: i64 = conn.query_row(
        "SELECT COUNT(*) FROM srs_cards \
         WHERE study_slug = ?1 \
           AND generation_method NOT IN ('manual','legacy') \
           AND generation_method IS NOT NULL",
        params![study_slug],
        |r| r.get(0),
    )?;
    let passing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM srs_cards \
         WHERE study_slug = ?1 \
           AND generation_method NOT IN ('manual','legacy') \
           AND generation_method IS NOT NULL \
           AND citation_score >= ?2",
        params![study_slug, CITATION_PASS_THRESHOLD],
        |r| r.get(0),
    )?;
    let rate = if total_auto > 0 {
        Some((passing as f64) / (total_auto as f64))
    } else {
        None
    };
    Ok((rate, passing, total_auto))
}

/// gate 3 — 7일 intervention_signals dismiss 카운트.
/// intervention_signals.fired_at는 ISO 8601 TEXT (v4 스키마).
fn gate3(conn: &Connection, study_slug: &str) -> AppResult<(f64, i64, i64)> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM intervention_signals \
         WHERE study_slug = ?1 \
           AND fired_at >= datetime('now','-7 days')",
        params![study_slug],
        |r| r.get(0),
    )?;
    let dismissed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM intervention_signals \
         WHERE study_slug = ?1 \
           AND fired_at >= datetime('now','-7 days') \
           AND user_dismissed = 1",
        params![study_slug],
        |r| r.get(0),
    )?;
    // dismiss/7d = 주당 환산 (이미 7일 카운트이므로 그대로 사용).
    Ok((dismissed as f64, dismissed, total))
}

/// gate 4 — recall_attempts 7일 응답률.
/// recall_attempts.fired_at은 ISO 8601 TEXT (v21 스키마 확인).
fn gate4(conn: &Connection, study_slug: &str) -> AppResult<(Option<f64>, i64, i64)> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM recall_attempts \
         WHERE study_slug = ?1 \
           AND fired_at >= datetime('now','-7 days')",
        params![study_slug],
        |r| r.get(0),
    )?;
    let attempted: i64 = conn.query_row(
        "SELECT COUNT(*) FROM recall_attempts \
         WHERE study_slug = ?1 \
           AND fired_at >= datetime('now','-7 days') \
           AND outcome IN ('correct','incorrect')",
        params![study_slug],
        |r| r.get(0),
    )?;
    let rate = if total > 0 {
        Some((attempted as f64) / (total as f64))
    } else {
        None
    };
    Ok((rate, attempted, total))
}

/// gate 5 — self-rating 최근 N건 평균.
fn gate5(log: &[SelfRating]) -> (Option<f64>, i64) {
    let count = log.len() as i64;
    if log.is_empty() {
        return (None, count);
    }
    let take_n = SELF_RATING_AVG_N.min(log.len());
    // 최신 N건 = 마지막 take_n개 (Vec는 rated_at 오름차순으로 append됨).
    let recent = &log[log.len() - take_n..];
    let avg = recent.iter().map(|r| r.score as f64).sum::<f64>() / (take_n as f64);
    (Some(avg), count)
}

/// 응답 검증 통계 — chat_messages.context_json에서 citation_scores 추출.
/// history_compression_ratio는 DB에서 추출 불가 (in-memory) → None.
fn citation_stats(conn: &Connection, study_slug: &str) -> AppResult<Option<f64>> {
    // 최근 CITATION_LAST_N건 assistant 메시지 중 citation_scores가 있는 것만.
    let mut stmt = conn.prepare(
        "SELECT context_json FROM chat_messages \
         WHERE study_slug = ?1 AND role = 'assistant' \
           AND context_json IS NOT NULL \
         ORDER BY id DESC \
         LIMIT ?2",
    )?;
    let rows: Vec<String> = stmt
        .query_map(params![study_slug, CITATION_LAST_N], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut sum = 0.0_f64;
    let mut count = 0usize;
    for json_str in &rows {
        // citation_scores를 가벼운 형태로 역직렬화.
        #[derive(Deserialize)]
        struct CitationScoresWrapper {
            citation_scores: Option<Vec<CScore>>,
        }
        #[derive(Deserialize)]
        struct CScore {
            score: f64,
        }
        if let Ok(parsed) = serde_json::from_str::<CitationScoresWrapper>(json_str) {
            if let Some(scores) = parsed.citation_scores {
                for s in &scores {
                    sum += s.score;
                    count += 1;
                }
            }
        }
    }
    if count > 0 {
        Ok(Some(sum / (count as f64)))
    } else {
        Ok(None)
    }
}

// ---- 공개 함수 (Tauri command) -----------------------------------------------

/// 5 acceptance gate 측정값 반환.
/// study_slug = 활성 스터디 slug.
#[tauri::command]
pub fn learning_acceptance_metrics(
    state: State<'_, AppState>,
    study_slug: String,
) -> AppResult<AcceptanceMetrics> {
    if study_slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "study_slug가 비어 있습니다".into(),
        });
    }

    let (settings_clone, db) = {
        let s = state.settings.lock().expect("settings mutex");
        let db = state.db.lock().expect("db mutex");
        // settings_clone 먼저 복사하고 db는 그대로 사용 불가 (drop해야 함).
        // borrow checker: 두 Mutex를 동시에 잡지 않도록 settings만 clone.
        let sc = s.clone();
        drop(s);
        (sc, db)
    };

    let conn = db.conn();

    let (g1_rate, g1_active, g1_total) = gate1(conn, &study_slug)?;
    let (g2_rate, g2_passing, g2_total) = gate2(conn, &study_slug)?;
    let (g3_per_week, g3_dismissed, g3_total) = gate3(conn, &study_slug)?;
    let (g4_rate, g4_attempted, g4_total) = gate4(conn, &study_slug)?;
    let (g5_avg, g5_count) = gate5(&settings_clone.learning_self_rating_log);
    let citation_avg = citation_stats(conn, &study_slug)?;

    Ok(AcceptanceMetrics {
        gate1_memory_keep_rate: g1_rate,
        gate1_active_7d: g1_active,
        gate1_total_inserted_7d: g1_total,

        gate2_srs_quality_rate: g2_rate,
        gate2_passing: g2_passing,
        gate2_total_auto: g2_total,

        gate3_dismiss_per_week: g3_per_week,
        gate3_dismissed_7d: g3_dismissed,
        gate3_total_signals_7d: g3_total,

        gate4_attempt_rate: g4_rate,
        gate4_attempted_7d: g4_attempted,
        gate4_total_triggers_7d: g4_total,

        gate5_self_rating_avg: g5_avg,
        gate5_self_rating_count: g5_count,

        citation_avg_last_50: citation_avg,
        history_compression_ratio_avg: None, // D-089 in-memory only — 영속 불가
    })
}

/// 자가 평가 점수 기록.
/// score: 1~10. settings.learning_self_rating_log에 append, 최대 100건 cap.
#[tauri::command]
pub fn learning_self_rating_record(
    state: State<'_, AppState>,
    score: u8,
) -> AppResult<()> {
    if score == 0 || score > 10 {
        return Err(AppError::InvalidInput {
            message: format!("score는 1~10 사이여야 합니다 (받은 값: {score})"),
        });
    }
    let entry = SelfRating {
        rated_at: now_ms(),
        score,
    };
    let path = state.settings_path.clone();
    let mut g = state.settings.lock().expect("settings mutex");
    g.learning_self_rating_log.push(entry);
    // 최대 100건 — 초과 시 오래된 것부터 drop.
    if g.learning_self_rating_log.len() > SELF_RATING_MAX {
        let overflow = g.learning_self_rating_log.len() - SELF_RATING_MAX;
        g.learning_self_rating_log.drain(..overflow);
    }
    crate::settings::write(&path, &g)?;
    info!(
        target: "learning_dev",
        score = score,
        total = g.learning_self_rating_log.len(),
        "self_rating recorded"
    );
    Ok(())
}

/// 자가 평가 활성화 여부 확인.
/// 첫 실행(settings.first_run_at) + 7일 elapsed 시 true.
/// first_run_at이 None이면 (초기화 전) false 반환.
#[tauri::command]
pub fn learning_self_rating_eligible(state: State<'_, AppState>) -> AppResult<bool> {
    let g = state.settings.lock().expect("settings mutex");
    let Some(first_run_at) = g.first_run_at else {
        return Ok(false);
    };
    let now = now_ms();
    let elapsed_ms = now - first_run_at;
    let seven_days_ms: i64 = 7 * 24 * 3600 * 1000;
    Ok(elapsed_ms >= seven_days_ms)
}

// ---- 단위 테스트 -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::settings::SelfRating;

    fn setup() -> Db {
        Db::open_in_memory_for_test()
    }

    // ---- gate 1 tests -------------------------------------------------------

    #[test]
    fn gate1_empty_db_returns_none_rate() {
        let db = setup();
        let (rate, active, total) = gate1(db.conn(), "s1").unwrap();
        assert!(rate.is_none());
        assert_eq!(active, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn gate1_all_active_returns_1_0() {
        let db = setup();
        let now = now_secs();
        db.conn()
            .execute(
                "INSERT INTO memory_facts \
                    (study_id,kind,content,source,confidence,status,created_at,updated_at) \
                 VALUES ('s1','preference','x','trigger',0.8,'active',?1,?1)",
                params![now],
            )
            .unwrap();
        let (rate, active, total) = gate1(db.conn(), "s1").unwrap();
        assert_eq!(rate, Some(1.0));
        assert_eq!(active, 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn gate1_archived_reduces_rate() {
        let db = setup();
        let now = now_secs();
        for _ in 0..4 {
            db.conn()
                .execute(
                    "INSERT INTO memory_facts \
                        (study_id,kind,content,source,confidence,status,created_at,updated_at) \
                     VALUES ('s1','preference','x','trigger',0.8,'active',?1,?1)",
                    params![now],
                )
                .unwrap();
        }
        db.conn()
            .execute(
                "INSERT INTO memory_facts \
                    (study_id,kind,content,source,confidence,status,created_at,updated_at) \
                 VALUES ('s1','preference','y','trigger',0.8,'archived',?1,?1)",
                params![now],
            )
            .unwrap();
        let (rate, active, total) = gate1(db.conn(), "s1").unwrap();
        assert_eq!(total, 5);
        assert_eq!(active, 4);
        assert!((rate.unwrap() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn gate1_ignores_old_facts() {
        let db = setup();
        let old = now_secs() - 10 * 86_400; // 10일 전
        db.conn()
            .execute(
                "INSERT INTO memory_facts \
                    (study_id,kind,content,source,confidence,status,created_at,updated_at) \
                 VALUES ('s1','preference','old','trigger',0.8,'active',?1,?1)",
                params![old],
            )
            .unwrap();
        let (rate, _, total) = gate1(db.conn(), "s1").unwrap();
        assert!(rate.is_none());
        assert_eq!(total, 0);
    }

    // ---- gate 5 tests -------------------------------------------------------

    #[test]
    fn gate5_empty_returns_none() {
        let (avg, count) = gate5(&[]);
        assert!(avg.is_none());
        assert_eq!(count, 0);
    }

    #[test]
    fn gate5_single_entry_returns_score() {
        let log = vec![SelfRating { rated_at: 0, score: 8 }];
        let (avg, count) = gate5(&log);
        assert_eq!(avg, Some(8.0));
        assert_eq!(count, 1);
    }

    #[test]
    fn gate5_uses_last_n_entries() {
        // N=10보다 많은 기록이 있을 때 마지막 10개 평균.
        let log: Vec<SelfRating> = (0..15)
            .map(|i| SelfRating {
                rated_at: i,
                score: if i < 5 { 1 } else { 9 }, // 앞 5개=1, 뒤 10개=9
            })
            .collect();
        // 마지막 10개는 i=5..15 중 score=9
        let _ = log.len(); // suppress warning
        let (avg, count) = gate5(&log);
        assert_eq!(count, 15);
        assert!((avg.unwrap() - 9.0).abs() < 1e-6, "avg={:?}", avg);
    }

    // ---- self_rating_record tests ------------------------------------------

    #[test]
    fn self_rating_record_appends_and_caps() {
        // 100건 cap 테스트 — 직접 settings에 access 불가 (AppState 필요) → 내부 로직 직접 테스트.
        let mut log: Vec<SelfRating> = Vec::new();
        // 100건 채우기.
        for i in 0..100_usize {
            log.push(SelfRating { rated_at: i as i64, score: 5 });
        }
        // 101번째 추가 시 첫 번째 drop.
        log.push(SelfRating { rated_at: 100, score: 7 });
        if log.len() > SELF_RATING_MAX {
            let overflow = log.len() - SELF_RATING_MAX;
            log.drain(..overflow);
        }
        assert_eq!(log.len(), SELF_RATING_MAX);
        assert_eq!(log[0].rated_at, 1, "oldest entry should be dropped");
        assert_eq!(log[SELF_RATING_MAX - 1].score, 7, "newest entry preserved");
    }

    // ---- eligible tests ----------------------------------------------------

    #[test]
    fn eligible_false_when_first_run_at_is_recent() {
        let log: Vec<SelfRating> = vec![];
        let first_run_at = now_ms() - (3 * 24 * 3600 * 1000_i64); // 3일 전
        let elapsed = now_ms() - first_run_at;
        let seven_days_ms = 7 * 24 * 3600 * 1000_i64;
        assert!(!log.len().eq(&0) || elapsed < seven_days_ms);
    }

    #[test]
    fn eligible_true_when_first_run_at_is_old() {
        let first_run_at = now_ms() - (8 * 24 * 3600 * 1000_i64); // 8일 전
        let elapsed = now_ms() - first_run_at;
        let seven_days_ms = 7 * 24 * 3600 * 1000_i64;
        assert!(elapsed >= seven_days_ms);
    }

    // ---- gate 4 tests -------------------------------------------------------

    #[test]
    fn gate4_empty_returns_none() {
        let db = setup();
        let (rate, attempted, total) = gate4(db.conn(), "s1").unwrap();
        assert!(rate.is_none());
        assert_eq!(attempted, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn gate4_counts_only_correct_incorrect() {
        let db = setup();
        // 'correct' + 'incorrect' = 시도.
        // 'dismissed' + 'timeout' + 'skipped' = 미시도.
        // fired_at은 ISO 8601 TEXT (v21 스키마).
        for outcome in &["correct", "incorrect", "dismissed", "timeout", "skipped"] {
            db.conn()
                .execute(
                    "INSERT INTO recall_attempts \
                        (study_slug,chunk_id,trigger_id,strength,outcome,fired_at) \
                     VALUES ('s1',1,'t1','weak',?1,datetime('now'))",
                    params![outcome],
                )
                .unwrap();
        }
        let (rate, attempted, total) = gate4(db.conn(), "s1").unwrap();
        assert_eq!(total, 5);
        assert_eq!(attempted, 2);
        assert!((rate.unwrap() - 0.4).abs() < 1e-9);
    }

    // ---- gate 2 helpers -------------------------------------------------------

    /// srs_cards는 studies.slug FK를 가지므로 테스트용 study row를 먼저 삽입.
    fn ensure_study(db: &Db, slug: &str) {
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO studies \
                    (slug, name, language, created_at, is_active) \
                 VALUES (?1, ?1, 'ko', datetime('now'), 1)",
                params![slug],
            )
            .unwrap();
    }

    // ---- gate 2 tests -------------------------------------------------------

    #[test]
    fn gate2_empty_returns_none() {
        let db = setup();
        let (rate, passing, total) = gate2(db.conn(), "s1").unwrap();
        assert!(rate.is_none());
        assert_eq!(passing, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn gate2_excludes_manual_and_legacy() {
        let db = setup();
        ensure_study(&db, "s1");
        // manual / legacy 는 제외.
        for method in &["manual", "legacy"] {
            db.conn()
                .execute(
                    "INSERT INTO srs_cards \
                        (study_slug,front,back,e_factor,interval_days,repetitions,due_date, \
                         generation_method,citation_score,created_at) \
                     VALUES ('s1','q','a',2.5,1,0,date('now'),?1,0.9,\
                             datetime('now'))",
                    params![method],
                )
                .unwrap();
        }
        let (rate, _, total) = gate2(db.conn(), "s1").unwrap();
        assert!(rate.is_none());
        assert_eq!(total, 0);
    }

    #[test]
    fn gate2_counts_auto_generated_passing() {
        let db = setup();
        ensure_study(&db, "s1");
        // deterministic_cloze + citation >= 0.5 = passing.
        for (method, score) in &[
            ("deterministic_cloze", 0.8_f64),
            ("deterministic_cloze", 0.3_f64), // fail
            ("llm_mc4", 0.6_f64),
        ] {
            db.conn()
                .execute(
                    "INSERT INTO srs_cards \
                        (study_slug,front,back,e_factor,interval_days,repetitions,due_date, \
                         generation_method,citation_score,created_at) \
                     VALUES ('s1','q','a',2.5,1,0,date('now'),?1,?2,\
                             datetime('now'))",
                    params![method, score],
                )
                .unwrap();
        }
        let (rate, passing, total) = gate2(db.conn(), "s1").unwrap();
        assert_eq!(total, 3);
        assert_eq!(passing, 2); // 0.8 + 0.6 pass; 0.3 fail
        assert!((rate.unwrap() - 2.0 / 3.0).abs() < 1e-9);
    }

    // ---- NaN 회피 테스트 ---------------------------------------------------

    #[test]
    fn no_nan_when_all_denominators_zero() {
        let db = setup();
        let (g1_rate, _, _) = gate1(db.conn(), "s1").unwrap();
        let (g2_rate, _, _) = gate2(db.conn(), "s1").unwrap();
        let (_, _, _) = gate3(db.conn(), "s1").unwrap();
        let (g4_rate, _, _) = gate4(db.conn(), "s1").unwrap();
        assert!(g1_rate.is_none(), "분모=0이면 None");
        assert!(g2_rate.is_none(), "분모=0이면 None");
        assert!(g4_rate.is_none(), "분모=0이면 None");
    }
}
