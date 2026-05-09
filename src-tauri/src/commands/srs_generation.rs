// v0.5 PR 2 (D-099/D-103) — SRS on-demand card generation.
//
// 아키텍처: &Db(rusqlite Connection — !Send)를 async 경계에 걸치지 않도록 분리.
//   * 모든 DB 접근 함수는 동기(fn) — Tauri 명령이 spawn_blocking으로 격리.
//   * LLM 호출만 async — DB 접근 없이 입력(Vec<ChunkRow>)만 받음.
//   * Tauri 명령: (1) spawn_blocking → DB 쿼리 + 결정적 카드 생성·INSERT,
//                 (2) async LLM 호출,
//                 (3) spawn_blocking → LLM 카드 INSERT.
//
// 카드 종류:
//   * 결정적 3종: cloze / match / order
//   * LLM 1종: llm_mc4 (fast_model 1회 호출, D-103 — LLM 필수 정책)
//
// citation_check: substring ≥ CITATION_MIN_OVERLAP → score 0.6 (통과), 아니면 0.0 (차단·INSERT X).
// Progress 이벤트: srs:generate:progress { current, total, kind }
//                  srs:generate:done    { total_inserted, total_skipped, skipped_reasons }

use std::sync::Arc;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::{AppError, AppResult};
use crate::llm::{ChatEvent, ChatRequest, LlmProvider, Message, Role};

// ---- 상수 -------------------------------------------------------------------

/// citation_check substring 최소 중복 문자 수 (D-090 임계 재활용).
pub const CITATION_MIN_OVERLAP: usize = 6;

/// citation_check 통과 점수.
pub const CITATION_PASS_SCORE: f32 = 0.6;

/// citation_check 차단 점수.
pub const CITATION_BLOCK_SCORE: f32 = 0.0;

/// cloze 카드당 최대 생성 수.
pub const MAX_CLOZE_PER_CHUNK: usize = 3;

/// order 카드 생성 최소 chunks 수.
pub const ORDER_MIN_CHUNKS: usize = 3;

/// order 카드 생성 최대 chunks 수.
pub const ORDER_MAX_CHUNKS: usize = 5;

/// match 카드 오답 수 (정답 1 + 오답 3 = 4지선다).
pub const MATCH_DISTRACTORS: usize = 3;

// ---- 타입 -------------------------------------------------------------------

/// 생성된 카드 후보. citation_check 통과 시 srs_cards에 INSERT.
#[derive(Debug, Clone)]
pub struct NewCard {
    pub study_slug: String,
    pub front: String,
    pub back: String,
    pub section_ref: Option<String>,
    pub source_chunk_id: i64,
    pub generation_method: String,
    pub citation_score: f32,
}

/// Tauri 명령 반환 타입.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrsGenerateResult {
    pub inserted: Vec<i64>,
    pub skipped: Vec<SkippedCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedCard {
    pub chunk_id: i64,
    pub reason: String,
}

/// 진행 이벤트 payload.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateProgressPayload {
    pub current: usize,
    pub total: usize,
    pub kind: String,
}

/// 완료 이벤트 payload.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateDonePayload {
    pub total_inserted: usize,
    pub total_skipped: usize,
    pub skipped_reasons: Vec<String>,
}

// ---- chunk 행 타입 ----------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub id: i64,
    pub text: String,
    pub section_path: Option<String>,
    pub ord: i64,
}

fn row_to_chunk(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChunkRow> {
    Ok(ChunkRow {
        id: r.get(0)?,
        text: r.get(1)?,
        section_path: r.get(2)?,
        ord: r.get(3)?,
    })
}

// ---- DB 조회 헬퍼 (동기) ----------------------------------------------------

/// 책 내 특정 섹션의 모든 chunks 조회 (ord 순).
pub fn chunks_for_section(
    conn: &Connection,
    book_id: &str,
    section_path: &str,
) -> AppResult<Vec<ChunkRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, text, section_path, ord \
         FROM chunks \
         WHERE book_id = ?1 AND section_path = ?2 \
         ORDER BY ord ASC",
    )?;
    let rows = stmt
        .query_map(params![book_id, section_path], row_to_chunk)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 단일 chunk 조회.
pub fn chunk_by_id(conn: &Connection, chunk_id: i64) -> AppResult<Option<ChunkRow>> {
    let result = conn.query_row(
        "SELECT id, text, section_path, ord FROM chunks WHERE id = ?1",
        params![chunk_id],
        row_to_chunk,
    );
    match result {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::from(e)),
    }
}

/// 책 내 모든 섹션 경로 목록 (중복 제거, NULL 제외, ord 순).
pub fn distinct_sections(conn: &Connection, book_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT section_path FROM chunks \
         WHERE book_id = ?1 AND section_path IS NOT NULL \
         GROUP BY section_path \
         ORDER BY MIN(ord) ASC",
    )?;
    let rows = stmt
        .query_map(params![book_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// memory_facts correction kind 청크를 약점 가중치 기준으로 정렬해 반환.
pub fn weak_priority_chunks(
    conn: &Connection,
    study_id: &str,
    limit: usize,
) -> AppResult<Vec<ChunkRow>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.text, c.section_path, c.ord \
         FROM chunks c \
         JOIN memory_fact_chunks mfc ON mfc.chunk_id = c.id \
         JOIN memory_facts mf ON mf.id = mfc.fact_id \
         WHERE mf.study_id = ?1 AND mf.kind = 'correction' AND mf.status = 'active' \
         GROUP BY c.id \
         ORDER BY MAX(mfc.similarity) DESC \
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![study_id, limit as i64], row_to_chunk)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 같은 책의 다른 섹션에서 오답 후보 chunks 조회 (랜덤).
fn distractor_chunks(
    conn: &Connection,
    book_id: &str,
    exclude_section: Option<&str>,
    limit: usize,
) -> AppResult<Vec<ChunkRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, text, section_path, ord \
         FROM chunks \
         WHERE book_id = ?1 \
           AND (section_path IS NULL OR section_path != COALESCE(?2, '')) \
         ORDER BY RANDOM() \
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![book_id, exclude_section, limit as i64], row_to_chunk)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---- 결정적 카드 생성 (동기 — &Connection 필요) -----------------------------

/// CJK·ASCII 토큰 추출 — 길이 ≥ 2인 단어 토큰 목록.
pub fn tokenize_maskable(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        let is_word_char = ch.is_alphabetic() || ch.is_numeric();
        if is_word_char {
            current.push(ch);
        } else {
            if current.chars().count() >= 2 {
                tokens.push(current.clone());
            }
            current.clear();
        }
    }
    if current.chars().count() >= 2 {
        tokens.push(current);
    }
    tokens
}

/// 단일 chunk에서 cloze 카드 생성 (최대 MAX_CLOZE_PER_CHUNK).
pub fn generate_cloze(
    study_slug: &str,
    chunk: &ChunkRow,
    section_ref: Option<&str>,
) -> Vec<NewCard> {
    let tokens = tokenize_maskable(&chunk.text);
    if tokens.is_empty() {
        return Vec::new();
    }

    // 균등 간격으로 최대 MAX_CLOZE_PER_CHUNK 토큰 선택.
    let step = (tokens.len() / MAX_CLOZE_PER_CHUNK).max(1);
    let selected: Vec<&String> = tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| i % step == 0)
        .take(MAX_CLOZE_PER_CHUNK)
        .map(|(_, t)| t)
        .collect();

    selected
        .into_iter()
        .filter_map(|token| {
            let masked_text = chunk.text.replacen(token.as_str(), "[___]", 1);
            let score = substring_citation_score(token, &chunk.text);
            if score < CITATION_PASS_SCORE {
                return None;
            }
            Some(NewCard {
                study_slug: study_slug.to_string(),
                front: masked_text,
                back: token.clone(),
                section_ref: section_ref.map(|s| s.to_string()),
                source_chunk_id: chunk.id,
                generation_method: "deterministic_cloze".to_string(),
                citation_score: score,
            })
        })
        .collect()
}

/// chunk pair로 match 카드 생성 (chunk A 첫 줄 → chunk B 소속 섹션 4지선다).
pub fn generate_match(
    study_slug: &str,
    chunk_a: &ChunkRow,
    chunk_b: &ChunkRow,
    distractors: &[ChunkRow],
    section_ref: Option<&str>,
) -> Option<NewCard> {
    let first_line: String = chunk_a
        .text
        .lines()
        .next()
        .unwrap_or(&chunk_a.text)
        .chars()
        .take(100)
        .collect();
    if first_line.trim().is_empty() {
        return None;
    }

    let answer_label = format!(
        "청크 #{} ({})",
        chunk_b.ord,
        chunk_b.section_path.as_deref().unwrap_or("unknown")
    );

    let distractor_labels: Vec<String> = distractors
        .iter()
        .take(MATCH_DISTRACTORS)
        .map(|d| {
            format!(
                "청크 #{} ({})",
                d.ord,
                d.section_path.as_deref().unwrap_or("unknown")
            )
        })
        .collect();

    let mut choices = vec![answer_label.clone()];
    choices.extend(distractor_labels);

    let options: String = choices
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{}. {}", (b'A' + i as u8) as char, c))
        .collect::<Vec<_>>()
        .join("\n");

    let front = format!("다음 첫 줄은 어느 단락에 속합니까?\n\n\"{first_line}\"");
    let back = format!("정답: {answer_label}\n\n{options}");

    let score = substring_citation_score(&first_line, &chunk_a.text);
    if score < CITATION_PASS_SCORE {
        return None;
    }

    Some(NewCard {
        study_slug: study_slug.to_string(),
        front,
        back,
        section_ref: section_ref.map(|s| s.to_string()),
        source_chunk_id: chunk_a.id,
        generation_method: "deterministic_match".to_string(),
        citation_score: score,
    })
}

/// chunks 3~5개 → 순서 재정렬 카드.
pub fn generate_order(
    study_slug: &str,
    chunks: &[ChunkRow],
    section_ref: Option<&str>,
) -> Option<NewCard> {
    if chunks.len() < ORDER_MIN_CHUNKS {
        return None;
    }

    let first_lines: Vec<String> = chunks
        .iter()
        .map(|c| {
            c.text
                .lines()
                .next()
                .unwrap_or(&c.text)
                .chars()
                .take(80)
                .collect()
        })
        .collect();

    // 간단 deterministic 셔플 — 첫·끝 교환.
    let mut shuffled_indices: Vec<usize> = (0..chunks.len()).collect();
    if shuffled_indices.len() >= 2 {
        let last = shuffled_indices.len() - 1;
        shuffled_indices.swap(0, last);
    }

    let shuffled_display: String = shuffled_indices
        .iter()
        .enumerate()
        .map(|(display_pos, &orig_idx)| {
            format!("{}. {}", (b'A' + display_pos as u8) as char, first_lines[orig_idx])
        })
        .collect::<Vec<_>>()
        .join("\n");

    // 정답 = 원래 순서 chunk가 shuffled에서 어느 레이블인지.
    let answer_labels: Vec<String> = (0..chunks.len())
        .map(|orig_idx| {
            let display_pos = shuffled_indices
                .iter()
                .position(|&i| i == orig_idx)
                .unwrap_or(orig_idx);
            format!("{}", (b'A' + display_pos as u8) as char)
        })
        .collect();

    let front = format!("다음 단락들을 원래 순서대로 정렬하시오.\n\n{shuffled_display}");
    let back = format!(
        "정답 순서: {}\n\n(각 알파벳이 위 선택지의 단락)",
        answer_labels.join(" → ")
    );

    let score = substring_citation_score(&first_lines[0], &chunks[0].text);
    if score < CITATION_PASS_SCORE {
        return None;
    }

    Some(NewCard {
        study_slug: study_slug.to_string(),
        front,
        back,
        section_ref: section_ref.map(|s| s.to_string()),
        source_chunk_id: chunks[0].id,
        generation_method: "deterministic_order".to_string(),
        citation_score: score,
    })
}

/// 섹션 chunks 결정적 카드 3종 생성 (동기, &Connection).
pub fn generate_deterministic(
    conn: &Connection,
    study_slug: &str,
    book_id: &str,
    chunks: &[ChunkRow],
) -> AppResult<Vec<NewCard>> {
    let section_ref = chunks.first().and_then(|c| c.section_path.clone());
    let mut cards: Vec<NewCard> = Vec::new();

    for chunk in chunks {
        cards.extend(generate_cloze(study_slug, chunk, section_ref.as_deref()));
    }

    if chunks.len() >= 2 {
        let distractors = distractor_chunks(conn, book_id, section_ref.as_deref(), MATCH_DISTRACTORS)?;
        for pair in chunks.windows(2) {
            if let [a, b] = pair {
                if let Some(card) = generate_match(study_slug, a, b, &distractors, section_ref.as_deref()) {
                    cards.push(card);
                }
            }
        }
    }

    if chunks.len() >= ORDER_MIN_CHUNKS {
        let order_slice: &[ChunkRow] = if chunks.len() > ORDER_MAX_CHUNKS {
            &chunks[..ORDER_MAX_CHUNKS]
        } else {
            chunks
        };
        if let Some(card) = generate_order(study_slug, order_slice, section_ref.as_deref()) {
            cards.push(card);
        }
    }

    Ok(cards)
}

// ---- LLM 4지선다 (async — &Connection 없음) ---------------------------------

/// LLM 4지선다 응답 JSON 구조.
#[derive(Debug, Deserialize)]
pub(crate) struct Mc4Response {
    question: String,
    correct: String,
    distractors: Vec<String>,
}

/// chunks(소유) → LLM 4지선다 카드 생성. 실패 시 None — retry X.
/// 중요: &Connection 매개변수 없음 — await 포인트에서 &Db 비노출.
pub async fn generate_llm_mc4(
    provider: &Arc<dyn LlmProvider>,
    study_slug: &str,
    chunks: &[ChunkRow],
    section_ref: Option<&str>,
) -> Option<NewCard> {
    if chunks.is_empty() {
        return None;
    }

    let fast = provider.fast_model();
    if fast.is_empty() {
        warn!(target: "srs_generation", "fast_model() 미설정 — LLM MC4 skip");
        return None;
    }

    let content: String = chunks
        .iter()
        .take(3)
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join("\n---\n");

    let prompt = format!(
        "다음 텍스트의 핵심 개념 1개를 4지선다 문제로 만들어라. \
         반드시 JSON만 출력하라 (다른 텍스트 없음).\n\
         형식: {{\"question\":\"...\",\"correct\":\"...\",\"distractors\":[\"...\",\"...\",\"...\"]}}\n\n\
         텍스트:\n{content}"
    );

    let request = ChatRequest {
        model: fast.to_string(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: prompt,
        }],
        max_tokens: 512,
        cache_breakpoints: Vec::new(),
    };

    let response_text = match collect_stream(provider, request).await {
        Ok(t) => t,
        Err(e) => {
            warn!(target: "srs_generation", error = %e, "LLM MC4 stream 실패 — skip");
            return None;
        }
    };

    let parsed: Mc4Response = match parse_mc4_json(&response_text) {
        Some(p) => p,
        None => {
            warn!(
                target: "srs_generation",
                response = %response_text,
                "LLM MC4 JSON 파싱 실패 — skip"
            );
            return None;
        }
    };

    if parsed.question.trim().is_empty() || parsed.correct.trim().is_empty() {
        return None;
    }

    let mut options = vec![parsed.correct.clone()];
    options.extend(parsed.distractors.into_iter().take(3));
    let choices_text: String = options
        .iter()
        .enumerate()
        .map(|(i, opt)| format!("{}. {}", (b'A' + i as u8) as char, opt))
        .collect::<Vec<_>>()
        .join("\n");

    let front = parsed.question.clone();
    let back = format!("정답: {}\n\n{choices_text}", parsed.correct);

    // citation_check: 정답이 source chunks any-of에 substring 매칭.
    let score = chunks
        .iter()
        .map(|c| substring_citation_score(&parsed.correct, &c.text))
        .fold(CITATION_BLOCK_SCORE, f32::max);

    if score < CITATION_PASS_SCORE {
        warn!(
            target: "srs_generation",
            correct = %parsed.correct,
            score = score,
            "LLM MC4 citation_check 미달 — skip"
        );
        return None;
    }

    Some(NewCard {
        study_slug: study_slug.to_string(),
        front,
        back,
        section_ref: section_ref.map(|s| s.to_string()),
        source_chunk_id: chunks[0].id,
        generation_method: "llm_mc4".to_string(),
        citation_score: score,
    })
}

/// ChatStream 전체를 단일 문자열로 수집.
async fn collect_stream(
    provider: &Arc<dyn LlmProvider>,
    request: ChatRequest,
) -> AppResult<String> {
    use futures_util::StreamExt;

    let mut stream = provider.chat_stream(request).await?;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event? {
            ChatEvent::TextDelta { text: t } => text.push_str(&t),
            ChatEvent::Done { .. } => break,
        }
    }
    Ok(text)
}

/// JSON 파싱 시도 — ```json ``` 펜스 안에 있을 수 있으므로 중괄호 추출.
pub fn parse_mc4_json(text: &str) -> Option<Mc4Response> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

// ---- citation_check 헬퍼 (동기) --------------------------------------------

/// substring 폴백 citation score.
pub fn substring_citation_score(answer: &str, source_text: &str) -> f32 {
    let chars: Vec<char> = answer.chars().collect();
    if chars.len() < CITATION_MIN_OVERLAP {
        // 짧은 정답 — 전체 포함 확인.
        if source_text.contains(answer) {
            return CITATION_PASS_SCORE;
        }
        return CITATION_BLOCK_SCORE;
    }
    for win_start in 0..=chars.len().saturating_sub(CITATION_MIN_OVERLAP) {
        let window: String = chars[win_start..win_start + CITATION_MIN_OVERLAP]
            .iter()
            .collect();
        if source_text.contains(&window) {
            return CITATION_PASS_SCORE;
        }
    }
    CITATION_BLOCK_SCORE
}

// ---- DB INSERT 헬퍼 (동기) -------------------------------------------------

/// NewCard → srs_cards INSERT. 성공 시 새 row id 반환.
pub fn insert_card(conn: &Connection, card: &NewCard) -> AppResult<i64> {
    let due_today = today_iso();
    conn.execute(
        "INSERT INTO srs_cards \
            (study_slug, front, back, section_ref, e_factor, interval_days, repetitions, \
             due_date, created_at, source_chunk_id, generation_method, citation_score) \
         VALUES (?1, ?2, ?3, ?4, 2.5, 0, 0, ?5, datetime('now'), ?6, ?7, ?8)",
        params![
            card.study_slug,
            card.front,
            card.back,
            card.section_ref,
            due_today,
            card.source_chunk_id,
            card.generation_method,
            card.citation_score as f64,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn today_iso() -> String {
    use std::time::SystemTime;
    use crate::commands::pomodoro::days_to_ymd_pub;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs / 86400;
    let (y, m, d) = days_to_ymd_pub(days);
    format!("{y:04}-{m:02}-{d:02}")
}

// ---- 공개 생성 함수 (동기 — Tauri 명령의 spawn_blocking 내부에서 호출) ------

/// 결정적 카드 생성 + INSERT. 반환값: (inserted_ids, skipped).
pub fn generate_and_insert_deterministic(
    conn: &Connection,
    study_slug: &str,
    book_id: &str,
    chunks: &[ChunkRow],
) -> AppResult<(Vec<i64>, Vec<SkippedCard>)> {
    let det_cards = generate_deterministic(conn, study_slug, book_id, chunks)?;
    let mut inserted = Vec::new();
    let mut skipped = Vec::new();
    for card in det_cards {
        let cid = card.source_chunk_id;
        if card.citation_score >= CITATION_PASS_SCORE {
            match insert_card(conn, &card) {
                Ok(id) => inserted.push(id),
                Err(e) => skipped.push(SkippedCard {
                    chunk_id: cid,
                    reason: format!("insert error: {e}"),
                }),
            }
        } else {
            skipped.push(SkippedCard {
                chunk_id: cid,
                reason: "citation_check 미달".to_string(),
            });
        }
    }
    Ok((inserted, skipped))
}

/// cloze 카드 생성 + INSERT (단일 chunk 전용 — chat 진입점).
pub fn generate_and_insert_cloze(
    conn: &Connection,
    study_slug: &str,
    chunk: &ChunkRow,
) -> (Vec<i64>, Vec<SkippedCard>) {
    let section_ref = chunk.section_path.clone();
    let cards = generate_cloze(study_slug, chunk, section_ref.as_deref());
    let mut inserted = Vec::new();
    let mut skipped = Vec::new();
    for card in cards {
        let cid = card.source_chunk_id;
        if card.citation_score >= CITATION_PASS_SCORE {
            match insert_card(conn, &card) {
                Ok(id) => inserted.push(id),
                Err(e) => skipped.push(SkippedCard {
                    chunk_id: cid,
                    reason: format!("insert error: {e}"),
                }),
            }
        } else {
            skipped.push(SkippedCard {
                chunk_id: cid,
                reason: "citation_check 미달".to_string(),
            });
        }
    }
    (inserted, skipped)
}

// ---- 단위 테스트 ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::sync::Arc;

    fn setup() -> Db {
        Db::open_in_memory_for_test()
    }

    fn insert_study_book_chunks(
        db: &Db,
        study_slug: &str,
        book_id: &str,
        section_path: &str,
        texts: &[&str],
    ) {
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO studies (slug, name, created_at) VALUES (?1, ?1, datetime('now'))",
                params![study_slug],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO books \
                    (id, study_slug, role, title, source_path, file_format, file_size, file_hash, added_at) \
                 VALUES (?1, ?2, 'main', 'Test', '/tmp/t', 'md', 0, 'h', datetime('now'))",
                params![book_id, study_slug],
            )
            .unwrap();
        for (i, text) in texts.iter().enumerate() {
            db.conn()
                .execute(
                    "INSERT INTO chunks (book_id, ord, text, section_path, token_count) \
                     VALUES (?1, ?2, ?3, ?4, 10)",
                    params![book_id, i as i64, text, section_path],
                )
                .unwrap();
        }
    }

    // ---- tokenize_maskable --------------------------------------------------

    #[test]
    fn tokenize_maskable_extracts_words() {
        let tokens = tokenize_maskable("Rust 소유권은 메모리 안전성을 보장합니다.");
        assert!(!tokens.is_empty());
        assert!(tokens.iter().any(|t| t == "Rust"));
    }

    #[test]
    fn tokenize_maskable_skips_single_char_tokens() {
        let tokens = tokenize_maskable("a b c 안 녕");
        assert!(tokens.iter().all(|t| t.chars().count() >= 2));
    }

    #[test]
    fn tokenize_maskable_empty_returns_empty() {
        assert!(tokenize_maskable("").is_empty());
    }

    // ---- substring_citation_score -------------------------------------------

    #[test]
    fn citation_score_passes_on_overlap() {
        let answer = "GameBoy PPU";
        let source = "GameBoy PPU는 그래픽 처리를 담당합니다.";
        assert_eq!(substring_citation_score(answer, source), CITATION_PASS_SCORE);
    }

    #[test]
    fn citation_score_blocks_on_no_overlap() {
        let answer = "Rustlangxx";
        let source = "JavaScript는 동적 타입 언어입니다.";
        assert_eq!(substring_citation_score(answer, source), CITATION_BLOCK_SCORE);
    }

    #[test]
    fn citation_score_passes_short_answer_in_source() {
        let answer = "CPU";
        let source = "CPU는 명령어를 실행합니다.";
        assert_eq!(substring_citation_score(answer, source), CITATION_PASS_SCORE);
    }

    // ---- generate_cloze -----------------------------------------------------

    #[test]
    fn generate_cloze_produces_cards_for_maskable_chunk() {
        let chunk = ChunkRow {
            id: 1,
            text: "Rust 소유권 시스템은 메모리 안전성을 컴파일 시점에 보장합니다. \
                   이를 통해 런타임 오류 없이 효율적인 코드를 작성할 수 있습니다."
                .to_string(),
            section_path: Some("Ch01".to_string()),
            ord: 0,
        };
        let cards = generate_cloze("s1", &chunk, Some("Ch01"));
        assert!(!cards.is_empty(), "cloze 카드 최소 1장이어야");
        for card in &cards {
            assert_eq!(card.generation_method, "deterministic_cloze");
            assert!(card.front.contains("[___]"));
            assert!(!card.back.is_empty());
            assert!(card.citation_score >= CITATION_PASS_SCORE);
        }
    }

    #[test]
    fn generate_cloze_max_cards_limit() {
        let chunk = ChunkRow {
            id: 1,
            text: "Apple Banana Cherry Date Elderberry Fig Grape Honeydew Iced Jam Kiwi Lemon".to_string(),
            section_path: None,
            ord: 0,
        };
        let cards = generate_cloze("s1", &chunk, None);
        assert!(cards.len() <= MAX_CLOZE_PER_CHUNK);
    }

    // ---- generate_match -----------------------------------------------------

    #[test]
    fn generate_match_produces_card_for_two_chunks() {
        let a = ChunkRow {
            id: 1,
            text: "Rust 소유권은 메모리 안전성의 핵심입니다.".to_string(),
            section_path: Some("Ch01".to_string()),
            ord: 0,
        };
        let b = ChunkRow {
            id: 2,
            text: "빌림(Borrow) 검사기는 컴파일 타임에 검증합니다.".to_string(),
            section_path: Some("Ch01".to_string()),
            ord: 1,
        };
        let card = generate_match("s1", &a, &b, &[], Some("Ch01"));
        assert!(card.is_some());
        let c = card.unwrap();
        assert_eq!(c.generation_method, "deterministic_match");
        assert!(c.front.contains("Rust 소유권은"));
    }

    // ---- generate_order -----------------------------------------------------

    #[test]
    fn generate_order_requires_at_least_3_chunks() {
        let two: Vec<ChunkRow> = (0..2)
            .map(|i| ChunkRow {
                id: i as i64 + 1,
                text: format!("단락 {i} 내용입니다."),
                section_path: Some("Ch01".to_string()),
                ord: i as i64,
            })
            .collect();
        assert!(generate_order("s1", &two, None).is_none());
    }

    #[test]
    fn generate_order_produces_card_for_3_chunks() {
        let chunks: Vec<ChunkRow> = (0..3)
            .map(|i| ChunkRow {
                id: i as i64 + 1,
                text: format!("이것은 단락 {i}의 내용입니다."),
                section_path: Some("Ch01".to_string()),
                ord: i as i64,
            })
            .collect();
        let card = generate_order("s1", &chunks, Some("Ch01"));
        assert!(card.is_some());
        let c = card.unwrap();
        assert_eq!(c.generation_method, "deterministic_order");
        assert!(c.front.contains("순서대로 정렬"));
    }

    // ---- generate_deterministic DB 통합 -------------------------------------

    #[test]
    fn generate_deterministic_single_chunk_only_cloze() {
        let db = setup();
        insert_study_book_chunks(
            &db, "s1", "b1", "Ch01",
            &["Rust 소유권은 컴파일 타임에 메모리 안전성을 보장합니다."],
        );
        let chunks = chunks_for_section(db.conn(), "b1", "Ch01").unwrap();
        let cards = generate_deterministic(db.conn(), "s1", "b1", &chunks).unwrap();
        assert!(cards.iter().all(|c| c.generation_method == "deterministic_cloze"));
    }

    #[test]
    fn generate_deterministic_two_chunks_cloze_and_match() {
        let db = setup();
        insert_study_book_chunks(
            &db, "s1", "b1", "Ch01",
            &[
                "Rust 소유권은 메모리 안전성의 핵심입니다.",
                "빌림(Borrow) 검사기는 컴파일 타임에 작동합니다.",
            ],
        );
        let chunks = chunks_for_section(db.conn(), "b1", "Ch01").unwrap();
        let cards = generate_deterministic(db.conn(), "s1", "b1", &chunks).unwrap();
        let methods: Vec<&str> = cards.iter().map(|c| c.generation_method.as_str()).collect();
        assert!(methods.contains(&"deterministic_cloze"));
        assert!(methods.contains(&"deterministic_match"));
        assert!(!methods.contains(&"deterministic_order"));
    }

    #[test]
    fn generate_deterministic_three_chunks_all_types() {
        let db = setup();
        insert_study_book_chunks(
            &db, "s1", "b1", "Ch01",
            &[
                "이것은 첫 번째 단락 내용입니다.",
                "이것은 두 번째 단락 내용입니다.",
                "이것은 세 번째 단락 내용입니다.",
            ],
        );
        let chunks = chunks_for_section(db.conn(), "b1", "Ch01").unwrap();
        let cards = generate_deterministic(db.conn(), "s1", "b1", &chunks).unwrap();
        let methods: Vec<&str> = cards.iter().map(|c| c.generation_method.as_str()).collect();
        assert!(methods.contains(&"deterministic_cloze"));
        assert!(methods.contains(&"deterministic_match"));
        assert!(methods.contains(&"deterministic_order"));
    }

    // ---- insert_card --------------------------------------------------------

    #[test]
    fn insert_card_stores_all_generation_fields() {
        let db = setup();
        db.conn()
            .execute(
                "INSERT INTO studies (slug, name, created_at) VALUES ('s1','S',datetime('now'))",
                [],
            )
            .unwrap();
        let card = NewCard {
            study_slug: "s1".to_string(),
            front: "질문".to_string(),
            back: "정답".to_string(),
            section_ref: Some("Ch01".to_string()),
            source_chunk_id: 42,
            generation_method: "deterministic_cloze".to_string(),
            citation_score: 0.6,
        };
        let id = insert_card(db.conn(), &card).unwrap();
        assert!(id > 0);

        let (method, chunk_id, score): (String, i64, f64) = db
            .conn()
            .query_row(
                "SELECT generation_method, source_chunk_id, citation_score \
                 FROM srs_cards WHERE id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(method, "deterministic_cloze");
        assert_eq!(chunk_id, 42);
        assert!((score - 0.6).abs() < 1e-6);
    }

    // ---- parse_mc4_json -----------------------------------------------------

    #[test]
    fn parse_mc4_json_valid() {
        let json = r#"{"question":"Q","correct":"A","distractors":["B","C","D"]}"#;
        let parsed = parse_mc4_json(json);
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.question, "Q");
        assert_eq!(p.correct, "A");
        assert_eq!(p.distractors.len(), 3);
    }

    #[test]
    fn parse_mc4_json_with_markdown_fence() {
        let text = "```json\n{\"question\":\"Q\",\"correct\":\"A\",\"distractors\":[\"B\",\"C\",\"D\"]}\n```";
        let parsed = parse_mc4_json(text);
        assert!(parsed.is_some());
    }

    #[test]
    fn parse_mc4_json_malformed_returns_none() {
        assert!(parse_mc4_json("not json at all").is_none());
        assert!(parse_mc4_json("").is_none());
        assert!(parse_mc4_json("{bad}").is_none());
    }

    // ---- weak_priority_chunks -----------------------------------------------

    #[test]
    fn weak_priority_chunks_joins_correction_facts() {
        let db = setup();
        insert_study_book_chunks(&db, "s1", "b1", "Ch01", &["Rust 소유권 단락입니다."]);
        let chunk_id: i64 = db
            .conn()
            .query_row("SELECT id FROM chunks WHERE book_id='b1'", [], |r| r.get(0))
            .unwrap();

        db.conn()
            .execute(
                "INSERT INTO memory_facts \
                    (study_id, kind, content, source, confidence, status, created_at, updated_at) \
                 VALUES ('s1','correction','영어 원문 보존','trigger',0.8,'active',1000,1000)",
                [],
            )
            .unwrap();
        let fact_id = db.conn().last_insert_rowid();
        db.conn()
            .execute(
                "INSERT INTO memory_fact_chunks (fact_id, chunk_id, similarity) VALUES (?1, ?2, 0.9)",
                params![fact_id, chunk_id],
            )
            .unwrap();

        let weak = weak_priority_chunks(db.conn(), "s1", 10).unwrap();
        assert_eq!(weak.len(), 1);
        assert_eq!(weak[0].id, chunk_id);
    }

    // ---- LLM mock 테스트 ----------------------------------------------------

    #[tokio::test]
    async fn generate_llm_mc4_with_valid_json_mock() {
        use crate::llm::mock::MockProvider;

        let json_response = r#"{"question":"Rust의 소유권 규칙은?","correct":"각 값은 소유자가 있다","distractors":["소유자는 여럿이다","값은 공유된다","메모리는 GC가 관리"]}"#;
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![
            ChatEvent::TextDelta {
                text: json_response.to_string(),
            },
            ChatEvent::Done {
                usage: Default::default(),
            },
        ]).with_model("mock-fast"));

        let chunk = ChunkRow {
            id: 1,
            text: "각 값은 소유자가 있다. Rust의 소유권 규칙입니다.".to_string(),
            section_path: Some("Ch01".to_string()),
            ord: 0,
        };

        let card = generate_llm_mc4(&provider, "s1", &[chunk], Some("Ch01")).await;
        assert!(card.is_some(), "유효한 JSON이면 카드 생성이어야");
        let c = card.unwrap();
        assert_eq!(c.generation_method, "llm_mc4");
        assert!(c.citation_score >= CITATION_PASS_SCORE);
    }

    #[tokio::test]
    async fn generate_llm_mc4_with_malformed_json_returns_none() {
        use crate::llm::mock::MockProvider;

        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![
            ChatEvent::TextDelta {
                text: "이건 JSON이 아닙니다".to_string(),
            },
            ChatEvent::Done {
                usage: Default::default(),
            },
        ]).with_model("mock-fast"));

        let chunk = ChunkRow {
            id: 1,
            text: "테스트 청크".to_string(),
            section_path: None,
            ord: 0,
        };

        let card = generate_llm_mc4(&provider, "s1", &[chunk], None).await;
        assert!(card.is_none());
    }

    #[tokio::test]
    async fn generate_llm_mc4_with_low_citation_score_returns_none() {
        use crate::llm::mock::MockProvider;

        let json_response = r#"{"question":"무관한 질문?","correct":"ZZZZZZZZZZZZ 전혀 무관한 정답","distractors":["X","Y","Z"]}"#;
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![
            ChatEvent::TextDelta {
                text: json_response.to_string(),
            },
            ChatEvent::Done {
                usage: Default::default(),
            },
        ]).with_model("mock-fast"));

        let chunk = ChunkRow {
            id: 1,
            text: "Rust 소유권".to_string(),
            section_path: None,
            ord: 0,
        };

        let card = generate_llm_mc4(&provider, "s1", &[chunk], None).await;
        assert!(card.is_none(), "citation_check 미달이면 None이어야");
    }

    #[tokio::test]
    async fn generate_llm_mc4_fast_model_empty_returns_none() {
        use crate::llm::mock::MockProvider;

        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![
            ChatEvent::Done {
                usage: Default::default(),
            },
        ]));
        // MockProvider.fast_model() = "" → 즉시 None.
        let chunk = ChunkRow {
            id: 1,
            text: "test chunk text".to_string(),
            section_path: None,
            ord: 0,
        };
        let card = generate_llm_mc4(&provider, "s1", &[chunk], None).await;
        assert!(card.is_none());
    }
}
