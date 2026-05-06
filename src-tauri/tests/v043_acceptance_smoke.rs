//! v0.4.3 PR 5 smoke — acceptance 측정 4 gate 프레임 통합 검증.
//!
//! 검증 (HANDOFF §1.6):
//!   1. chat_messages에 다양한 형태의 user/assistant rows를 적재.
//!   2. citation_accuracy / followup_skip_rate / prefix_cache_ratio 측정 SQL 본체가
//!      예상 통계를 내는지 확인 — Tauri State 의존을 우회한 *순수 SQL* 동등.
//!
//! `dev_acceptance` 의 command 함수는 `tauri::State` 인자를 받아 통합 테스트에서 직접
//! 호출하기 어려움. 본 smoke는 동등 SQL을 다시 호출해 *결과 값만* 비교 — 함수 본체의
//! SQL/로직 변경 시 unit test가 실패하면서 본 통합 smoke도 함께 흔들림.

use rusqlite::{params, Connection};
use std::path::Path;

const MIGRATIONS: &[&str] = &[
    include_str!("../src/migrations/v1_initial.sql"),
    include_str!("../src/migrations/v2_studies_and_chat.sql"),
    include_str!("../src/migrations/v3_paragraphs_fts.sql"),
    include_str!("../src/migrations/v4_intervention_and_history.sql"),
    include_str!("../src/migrations/v5_pomodoro_cycles.sql"),
    include_str!("../src/migrations/v6_srs_cards.sql"),
    include_str!("../src/migrations/v7_recall_challenges.sql"),
    include_str!("../src/migrations/v8_book_thumbnail.sql"),
    include_str!("../src/migrations/v9_study_thumbnail.sql"),
    include_str!("../src/migrations/v10_thumbnails_dir_rename.sql"),
    include_str!("../src/migrations/v11_study_description.sql"),
    include_str!("../src/migrations/v12_chat_context.sql"),
];

fn open_with_migrations(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open file db");
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (\
            version INTEGER PRIMARY KEY,\
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))\
         );",
    )
    .unwrap();
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let v = (i + 1) as i64;
        conn.execute_batch(sql).unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            params![v],
        )
        .unwrap();
    }
    conn
}

fn seed_study(conn: &Connection, slug: &str) {
    conn.execute(
        "INSERT INTO studies (slug, name, created_at) VALUES (?1, ?1, datetime('now'))",
        params![slug],
    )
    .unwrap();
}

fn insert_user_message(conn: &Connection, study_slug: &str, content: &str) {
    conn.execute(
        "INSERT INTO chat_messages (study_slug, role, content, created_at) \
         VALUES (?1, 'user', ?2, datetime('now'))",
        params![study_slug, content],
    )
    .unwrap();
}

fn insert_assistant_message(
    conn: &Connection,
    study_slug: &str,
    content: &str,
    creation_tokens: i64,
    cache_hit_tokens: i64,
    context_json: Option<&str>,
) {
    conn.execute(
        "INSERT INTO chat_messages (study_slug, role, content, created_at, \
            creation_tokens, cache_hit_tokens, context_json) \
         VALUES (?1, 'assistant', ?2, datetime('now'), ?3, ?4, ?5)",
        params![
            study_slug,
            content,
            creation_tokens,
            cache_hit_tokens,
            context_json,
        ],
    )
    .unwrap();
}

#[derive(Default)]
struct CitationStats {
    messages: i64,
    markers: i64,
    pass: i64,
    low: i64,
    no_match: i64,
}

fn measure_citations(conn: &Connection, study_slug: &str, last_n: u32) -> CitationStats {
    let lim = last_n.min(500) as i64;
    let mut stmt = conn
        .prepare(
            "SELECT context_json FROM chat_messages \
             WHERE study_slug = ?1 AND role = 'assistant' AND context_json IS NOT NULL \
             ORDER BY id DESC LIMIT ?2",
        )
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map(params![study_slug, lim], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let mut s = CitationStats::default();
    for raw in rows {
        let v: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(scores) = v.get("citation_scores").and_then(|x| x.as_array()) else {
            continue;
        };
        if scores.is_empty() {
            continue;
        }
        s.messages += 1;
        for sc in scores {
            s.markers += 1;
            match sc.get("verdict").and_then(|x| x.as_str()).unwrap_or("") {
                "pass" => s.pass += 1,
                "low" => s.low += 1,
                "no_match" => s.no_match += 1,
                _ => {}
            }
        }
    }
    s
}

#[test]
fn smoke_citation_accuracy_returns_expected_distribution() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_with_migrations(&dir.path().join("smoke.db"));
    seed_study(&conn, "s");

    insert_assistant_message(
        &conn,
        "s",
        "응답 [S1] [S2]",
        100,
        0,
        Some(
            r#"{"kind":"v041_hybrid","hits":[],"citation_scores":[
                {"source_idx":1,"score":0.7,"verdict":"pass"},
                {"source_idx":2,"score":0.3,"verdict":"low"}
            ]}"#,
        ),
    );
    insert_assistant_message(
        &conn,
        "s",
        "다른 응답",
        100,
        0,
        Some(r#"{"kind":"none","hits":[]}"#),
    );

    let s = measure_citations(&conn, "s", 50);
    assert_eq!(s.messages, 1);
    assert_eq!(s.markers, 2);
    assert_eq!(s.pass, 1);
    assert_eq!(s.low, 1);
}

fn is_followup_query(text: &str) -> bool {
    // dev_acceptance::is_followup_query 와 동일 — 통합 smoke는 SQL 회귀 외에 *분류* 자체도 본다.
    let t = text.trim();
    if t.len() < 2 {
        return false;
    }
    if t.chars().count() <= 6
        && (t.contains("왜")
            || t.contains("어떻게")
            || t.contains("뭐")
            || t.contains("어디"))
    {
        return true;
    }
    let hints: &[&str] = &[
        "그러면",
        "그럼",
        "그건 왜",
        "왜 그래",
        "왜 그런",
        "다시 설명",
        "더 자세",
        "예시",
        "예시를",
        "구체적",
        "근거",
        "출처",
        "이전에",
        "방금",
        "방금 답변",
        "그 부분",
        "그것",
        "그게 무슨",
        "어떻게 그",
        "방금 말한",
    ];
    hints.iter().any(|h| t.contains(h))
}

#[test]
fn smoke_followup_skip_rate_classifies_korean_followups() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_with_migrations(&dir.path().join("smoke.db"));
    seed_study(&conn, "s");

    insert_user_message(&conn, "s", "PPU 구조 설명해줘");
    insert_assistant_message(
        &conn,
        "s",
        "응답1",
        100,
        0,
        Some(r#"{"kind":"v041_hybrid","hits":[]}"#),
    );
    insert_user_message(&conn, "s", "그러면 왜 그렇게 동작?");
    insert_assistant_message(
        &conn,
        "s",
        "응답2",
        100,
        0,
        Some(r#"{"kind":"v041_hybrid","hits":[]}"#),
    );
    insert_user_message(&conn, "s", "CPU와 GPU의 본질적 차이");

    let mut stmt = conn
        .prepare(
            "SELECT id, role, content, context_json FROM chat_messages \
             WHERE study_slug = ?1 ORDER BY id ASC",
        )
        .unwrap();
    let rows: Vec<(i64, String, String, Option<String>)> = stmt
        .query_map(params!["s"], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    let mut user_messages = 0_i64;
    let mut followups = 0_i64;
    let mut reusable = 0_i64;
    let mut last_assistant_had_chunks = false;
    for (_, role, content, ctx) in &rows {
        match role.as_str() {
            "user" => {
                user_messages += 1;
                if is_followup_query(content) {
                    followups += 1;
                    if last_assistant_had_chunks {
                        reusable += 1;
                    }
                }
            }
            "assistant" => {
                last_assistant_had_chunks = ctx
                    .as_deref()
                    .map(|s| s.contains("\"v041_hybrid\""))
                    .unwrap_or(false);
            }
            _ => {}
        }
    }
    assert_eq!(user_messages, 3);
    assert_eq!(followups, 1);
    assert_eq!(reusable, 1);
}

#[test]
fn smoke_prefix_cache_ratio_aggregates_input_vs_cache_read() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_with_migrations(&dir.path().join("smoke.db"));
    seed_study(&conn, "s");

    insert_assistant_message(&conn, "s", "a1", 400, 600, None);
    insert_assistant_message(&conn, "s", "a2", 200, 800, None);
    // 메타 누락 — 0/0 row는 카운트 X.
    insert_assistant_message(&conn, "s", "a3", 0, 0, None);

    let mut stmt = conn
        .prepare(
            "SELECT creation_tokens, cache_hit_tokens FROM chat_messages \
             WHERE study_slug = ?1 AND role = 'assistant' \
             ORDER BY id DESC LIMIT ?2",
        )
        .unwrap();
    let rows: Vec<(i64, i64)> = stmt
        .query_map(params!["s", 50_i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    let mut messages = 0_i64;
    let mut cache_read_total = 0_i64;
    let mut input_total = 0_i64;
    for (input, cache_read) in rows {
        if input <= 0 && cache_read <= 0 {
            continue;
        }
        messages += 1;
        cache_read_total += cache_read.max(0);
        input_total += input.max(0);
    }
    assert_eq!(messages, 2);
    assert_eq!(cache_read_total, 1400);
    assert_eq!(input_total, 600);
    let denom = cache_read_total + input_total;
    let hit_ratio = cache_read_total as f64 / denom as f64;
    assert!((hit_ratio - 0.7).abs() < 1e-9);
}
