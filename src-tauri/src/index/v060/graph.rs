// v0.6.x PR (D-111) — 경량 로컬 GraphRAG (Neo4j 없이 SQLite 동시출현 그래프).
//
// WeKnora의 GraphRAG는 멀티홉 질문("A가 C에 어떤 영향을 줘?")에 강하지만 Neo4j(그래프
// 전용 DB)를 요구해 airis의 "로컬 단일 바이너리" 철학과 충돌한다. 본 모듈은 *경량 버전*:
//   1. 인덱싱 후 청크별 핵심 엔티티(키워드)를 *로컬 규칙*으로 추출 → `chunk_entities` 테이블.
//   2. 검색 시 매칭된 청크의 엔티티를 공유하는 *다른 청크*를 1홉 확장 후보로 끌어온다.
//
// 모델 호출 X (D-103 LLM 강제 정책의 "결정적 보조" 범주 — 인덱싱 5분 예산 보호).
// 한계(SUGGESTION에서 명시): "동시출현"은 진짜 인과·관계가 아니라 "같이 등장"이라 거친
// 연결 → 확장 후보는 *낮은 점수*로만 추가하고, MMR(query↔chunk cosine)이 안전망으로
// 무관한 확장을 down-rank 한다.
//
// 한국어 엔티티: 형태소 분석기(무거움) 없이 *조사 suffix 휴리스틱 제거* + 불용어 필터로
// 명사구 근사. 영어/약어/코드 식별자는 그대로 보존.

#![allow(dead_code)]

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::index::v041::retrieval::RetrievedChunk;

/// 청크당 보존할 최대 엔티티 수 (TF 상위) — 그래프 밀도 제어.
const MAX_ENTITIES_PER_CHUNK: usize = 10;

/// 확장 seed에서 추릴 상위 엔티티 수.
const SEED_TOP_ENTITIES: usize = 12;

/// 엔티티 최소 길이(문자) — 1자 토큰(조사 잔해·약어 아님)은 노이즈.
const MIN_ENTITY_CHARS: usize = 2;

/// 한 청크가 가질 수 있는 엔티티 추출 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub term: String,
    pub weight: f64,
}

/// 텍스트에서 핵심 엔티티(키워드) 추출 — 로컬 규칙 기반.
///
/// 동작:
///   1. 토큰화 — 한글 음절 run / ASCII 식별자 run 단위. 그 외 문자는 경계.
///   2. 정규화 — ASCII는 lowercase, 한글은 조사 suffix 제거.
///   3. 필터 — 불용어 / 1자 토큰 / 순수 숫자 제거.
///   4. TF 카운트 → 상위 MAX_ENTITIES_PER_CHUNK 개.
pub fn extract_entities(text: &str) -> Vec<Entity> {
    let mut freq: HashMap<String, f64> = HashMap::new();
    for raw in tokenize(text) {
        let norm = normalize_token(&raw);
        if !is_valid_entity(&norm) {
            continue;
        }
        *freq.entry(norm).or_insert(0.0) += 1.0;
    }
    let mut entities: Vec<Entity> = freq
        .into_iter()
        .map(|(term, weight)| Entity { term, weight })
        .collect();
    // weight 내림차순, 동점이면 term 사전순(결정적).
    entities.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.term.cmp(&b.term))
    });
    entities.truncate(MAX_ENTITIES_PER_CHUNK);
    entities
}

/// 한글 음절 run 또는 ASCII 식별자 run을 토큰으로 분리.
///
/// ASCII 식별자: 영숫자 + `_` (코드 식별자 보존). `::`·`.`·`-`는 경계로 처리(분해).
/// 한글: 음절(가-힣) 연속.
fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut buf_kind = TokKind::None;

    for c in text.chars() {
        let kind = char_kind(c);
        match kind {
            TokKind::None => {
                if !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
                buf_kind = TokKind::None;
            }
            _ => {
                if kind != buf_kind && !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
                buf.push(c);
                buf_kind = kind;
            }
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[derive(PartialEq, Clone, Copy)]
enum TokKind {
    None,
    Ascii,
    Hangul,
}

fn char_kind(c: char) -> TokKind {
    if c.is_ascii_alphanumeric() || c == '_' {
        TokKind::Ascii
    } else if ('가'..='힣').contains(&c) {
        TokKind::Hangul
    } else {
        TokKind::None
    }
}

/// 토큰 정규화 — ASCII lowercase / 한글 조사 suffix 제거.
fn normalize_token(tok: &str) -> String {
    if tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return tok.to_ascii_lowercase();
    }
    strip_korean_particle(tok)
}

/// 한국어 조사 목록 (긴 것 우선 — 그리디 suffix 매칭용). suffix 제거 + 단독 조사 토큰
/// 배제 양쪽에 쓰인다.
const PARTICLES: &[&str] = &[
    "으로부터", "에서는", "에게서", "라고", "이라고", "에서", "에게", "으로", "처럼", "까지",
    "부터", "보다", "마다", "조차", "마저", "이나", "거나", "든지", "은", "는", "이", "가", "을",
    "를", "의", "에", "도", "와", "과", "만", "로", "랑",
];

/// 한국어 조사(은/는/이/가/을/를/의/에/...) suffix를 휴리스틱으로 제거해 명사 형태 근사.
///
/// 보수적 — 제거 후 길이가 MIN_ENTITY_CHARS 미만이 되면 *원형 유지*. 다음절 조사부터
/// 검사(긴 것 우선)해 "에서"를 "에"로 잘못 자르지 않게 한다.
fn strip_korean_particle(tok: &str) -> String {
    let chars: Vec<char> = tok.chars().collect();
    for p in PARTICLES {
        if tok.ends_with(p) {
            let p_len = p.chars().count();
            if chars.len().saturating_sub(p_len) >= MIN_ENTITY_CHARS {
                return chars[..chars.len() - p_len].iter().collect();
            }
        }
    }
    tok.to_string()
}

/// 엔티티로 채택 가능한지 — 불용어/짧음/순수 숫자/단독 조사 배제.
fn is_valid_entity(term: &str) -> bool {
    let len = term.chars().count();
    if len < MIN_ENTITY_CHARS {
        return false;
    }
    // 순수 숫자(연도·페이지 등)는 엔티티 가치 낮음.
    if term.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // 단독 조사 토큰("으로", "에서" 등)은 명사가 아님 — suffix 제거가 짧아져 원형 유지된 케이스.
    if PARTICLES.contains(&term) {
        return false;
    }
    !is_stopword(term)
}

/// 한국어/영어 흔한 불용어.
fn is_stopword(term: &str) -> bool {
    const STOP: &[&str] = &[
        // 한국어 — 대명사·접속·일반.
        "그리고", "그러나", "하지만", "그래서", "또한", "또는", "이것", "그것", "저것",
        "여기", "거기", "저기", "이런", "그런", "저런", "때문", "경우", "통해", "위해",
        "대해", "관해", "있다", "없다", "이다", "하다", "되다", "같다", "다음", "이전",
        // 영어 — 관사·전치사·be 동사.
        "the", "and", "but", "for", "with", "this", "that", "these", "those", "from",
        "into", "are", "was", "were", "has", "have", "had", "not", "you", "your",
        "can", "will", "한다", "한",
    ];
    STOP.contains(&term)
}

// =============================================================================
// 그래프 빌드 — chunk_entities 테이블 적재 (DB v22).
// =============================================================================

/// 책의 모든 청크에서 엔티티를 추출해 `chunk_entities`를 재구축(idempotent).
///
/// 기존 책 엔트리를 지우고 다시 채운다. T1 인덱싱 완료 후 *백그라운드 티어*로 호출.
/// 반환값 = 적재한 (chunk, entity) 행 수.
pub fn rebuild_book_entities(conn: &Connection, book_id: &str) -> AppResult<usize> {
    // 1) 책 청크 본문 로드.
    let rows: Vec<(i64, String)> = {
        let mut stmt =
            conn.prepare("SELECT id, text FROM chunks WHERE book_id = ?1 ORDER BY ord")?;
        let mapped = stmt
            .query_map(params![book_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        mapped
    };

    // 2) 기존 엔트리 삭제 (재빌드 idempotent).
    conn.execute(
        "DELETE FROM chunk_entities WHERE book_id = ?1",
        params![book_id],
    )?;

    // 3) 추출 + INSERT.
    let mut inserted = 0usize;
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO chunk_entities (chunk_id, book_id, entity, weight) \
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (chunk_id, text) in rows {
        for ent in extract_entities(&text) {
            stmt.execute(params![chunk_id, book_id, ent.term, ent.weight])?;
            inserted += 1;
        }
    }
    Ok(inserted)
}

/// 책에 엔티티 인덱스가 이미 적재돼 있는지 (1행이라도).
pub fn has_entity_index(conn: &Connection, book_id: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM chunk_entities WHERE book_id = ?1 LIMIT 1)",
        params![book_id],
        |r| r.get::<_, i64>(0),
    )
    .map(|x| x == 1)
    .unwrap_or(false)
}

// =============================================================================
// 쿼리 시 1홉 확장.
// =============================================================================

/// seed 청크들의 엔티티를 공유하는 *다른* 청크를 1홉 확장 후보로 가져온다.
///
/// 점수 스케일: `base_score`(=가장 약한 실제 검색 hit 점수 근처)를 상한으로, 공유 엔티티
/// 가중치에 비례해 그 *이하*로만 부여한다. 이렇게 해야 RRF/MMR 점수 스케일에서 확장
/// 후보가 진짜 hit를 밀어내지 않고 "보강"으로만 작동한다.
///
/// `seeds`: 실제 검색으로 매칭된 chunk_id (확장에서 제외).
/// `max_add`: 추가할 최대 후보 수 (보수적 — 작게).
/// 반환: RetrievedChunk(메타 포함, score=확장 점수). seeds·빈 인덱스면 빈 Vec.
pub fn expand(
    conn: &Connection,
    book_id: &str,
    seeds: &[i64],
    max_add: usize,
    base_score: f64,
) -> AppResult<Vec<RetrievedChunk>> {
    if seeds.is_empty() || max_add == 0 {
        return Ok(Vec::new());
    }

    // 1) seed 엔티티 상위 N.
    let seed_entities = top_seed_entities(conn, seeds, SEED_TOP_ENTITIES)?;
    if seed_entities.is_empty() {
        return Ok(Vec::new());
    }

    // 2) 같은 엔티티를 가진 다른 청크 → 공유 가중치 합.
    let neighbors = neighbor_chunks(conn, book_id, &seed_entities, seeds, max_add)?;
    if neighbors.is_empty() {
        return Ok(Vec::new());
    }

    // 3) 점수 정규화 — 최댓값을 base_score에 맞추고 비례 축소.
    let max_w = neighbors
        .iter()
        .map(|(_, w)| *w)
        .fold(0.0_f64, f64::max)
        .max(1e-9);

    // 4) 청크 메타 로드 → RetrievedChunk.
    let mut out: Vec<RetrievedChunk> = Vec::with_capacity(neighbors.len());
    let mut stmt = conn.prepare(
        "SELECT id, text, page, section_path, parent_id, prev_chunk_id, next_chunk_id, \
                token_count \
         FROM chunks WHERE id = ?1",
    )?;
    for (chunk_id, shared_w) in neighbors {
        let score = base_score * (shared_w / max_w);
        let rec = stmt.query_row(params![chunk_id], |r| {
            let section_path: Option<String> = r.get::<_, Option<String>>(3)?.and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            });
            Ok(RetrievedChunk {
                id: r.get(0)?,
                text: r.get(1)?,
                page: r.get(2)?,
                section_path,
                parent_id: r.get(4)?,
                prev_chunk_id: r.get(5)?,
                next_chunk_id: r.get(6)?,
                token_count: r.get(7)?,
                score,
            })
        });
        match rec {
            Ok(c) => out.push(c),
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// seed 청크들의 엔티티를 가중치 합 기준 상위 N개 추출.
fn top_seed_entities(
    conn: &Connection,
    seeds: &[i64],
    top: usize,
) -> AppResult<Vec<String>> {
    let placeholders = placeholders(seeds.len());
    let sql = format!(
        "SELECT entity, SUM(weight) AS w FROM chunk_entities \
         WHERE chunk_id IN ({placeholders}) \
         GROUP BY entity ORDER BY w DESC, entity ASC LIMIT ?{}",
        seeds.len() + 1
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut binds: Vec<rusqlite::types::Value> =
        seeds.iter().map(|id| (*id).into()).collect();
    binds.push((top as i64).into());
    let rows = stmt
        .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
            r.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 주어진 엔티티 집합을 공유하는 (seed 제외) 청크 → 공유 가중치 합 상위 max_add.
fn neighbor_chunks(
    conn: &Connection,
    book_id: &str,
    entities: &[String],
    seeds: &[i64],
    max_add: usize,
) -> AppResult<Vec<(i64, f64)>> {
    if entities.is_empty() {
        return Ok(Vec::new());
    }
    let ent_ph = placeholders(entities.len());
    let seed_ph = placeholders(seeds.len());
    // book_id + entity IN (...) + chunk_id NOT IN (seeds) → GROUP BY chunk_id.
    let sql = format!(
        "SELECT chunk_id, SUM(weight) AS w FROM chunk_entities \
         WHERE book_id = ?1 AND entity IN ({ent_ph}) AND chunk_id NOT IN ({seed_ph}) \
         GROUP BY chunk_id ORDER BY w DESC, chunk_id ASC LIMIT ?{}",
        2 + entities.len() + seeds.len()
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut binds: Vec<rusqlite::types::Value> = Vec::new();
    binds.push(book_id.to_string().into());
    for e in entities {
        binds.push(e.clone().into());
    }
    for s in seeds {
        binds.push((*s).into());
    }
    binds.push((max_add as i64).into());
    let rows = stmt
        .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `?,?,?` placeholder 문자열 (n개).
fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE chunks (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                book_id       TEXT NOT NULL,
                ord           INTEGER NOT NULL,
                text          TEXT NOT NULL,
                page          INTEGER,
                parent_id     INTEGER,
                prev_chunk_id INTEGER,
                next_chunk_id INTEGER,
                section_path  TEXT,
                token_count   INTEGER
             );
             CREATE TABLE chunk_entities (
                chunk_id INTEGER NOT NULL,
                book_id  TEXT NOT NULL,
                entity   TEXT NOT NULL,
                weight   REAL NOT NULL DEFAULT 1.0,
                PRIMARY KEY (chunk_id, entity)
             );",
        )
        .unwrap();
        conn
    }

    fn insert_chunk(conn: &Connection, book: &str, ord: i64, text: &str) -> i64 {
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, token_count) VALUES (?1,?2,?3,?4)",
            params![book, ord, text, text.chars().count() as i64],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn extract_entities_picks_repeated_terms() {
        let text = "프로세스 스케줄링은 프로세스 관리의 핵심이다. 스케줄러가 프로세스를 배분한다.";
        let ents = extract_entities(text);
        let terms: Vec<&str> = ents.iter().map(|e| e.term.as_str()).collect();
        // "프로세스"가 조사 제거 후 통합되어 최상위 (3회 등장).
        assert!(terms.contains(&"프로세스"), "추출: {terms:?}");
        assert_eq!(ents[0].term, "프로세스", "최빈 엔티티가 상위");
        assert!(ents[0].weight >= 3.0);
    }

    #[test]
    fn extract_entities_normalizes_korean_particles() {
        // "프로세스가", "프로세스를", "프로세스의" → 모두 "프로세스".
        let text = "프로세스가 프로세스를 프로세스의";
        let ents = extract_entities(text);
        assert_eq!(ents.len(), 1, "조사 정규화로 1개 엔티티: {ents:?}");
        assert_eq!(ents[0].term, "프로세스");
        assert!((ents[0].weight - 3.0).abs() < 1e-9);
    }

    #[test]
    fn extract_entities_preserves_ascii_identifiers_lowercased() {
        let text = "TCP handshake 와 UDP 비교. tcp 재전송.";
        let ents = extract_entities(text);
        let terms: Vec<&str> = ents.iter().map(|e| e.term.as_str()).collect();
        // "TCP"·"tcp" → "tcp"로 통합 (2회).
        assert!(terms.contains(&"tcp"));
        let tcp = ents.iter().find(|e| e.term == "tcp").unwrap();
        assert!(tcp.weight >= 2.0);
    }

    #[test]
    fn extract_entities_drops_stopwords_and_numbers() {
        let text = "그리고 the 2026 으로";
        let ents = extract_entities(text);
        assert!(ents.is_empty(), "불용어·숫자만 → 엔티티 0: {ents:?}");
    }

    #[test]
    fn strip_particle_keeps_short_tokens_intact() {
        // 제거 후 2자 미만이 되면 원형 유지.
        assert_eq!(strip_korean_particle("가"), "가");
        assert_eq!(strip_korean_particle("이가"), "이가"); // "이가"→"이"는 1자라 유지.
        assert_eq!(strip_korean_particle("스레드는"), "스레드");
        assert_eq!(strip_korean_particle("메모리에서"), "메모리");
    }

    #[test]
    fn rebuild_and_expand_finds_shared_entity_neighbors() {
        let conn = fresh_conn();
        // c1·c2는 "프로세스" 공유, c3는 무관.
        let c1 = insert_chunk(&conn, "b1", 0, "프로세스 스케줄링 기초. 프로세스 상태.");
        let c2 = insert_chunk(&conn, "b1", 1, "프로세스 컨텍스트 스위칭. 프로세스 우선순위.");
        let c3 = insert_chunk(&conn, "b1", 2, "네트워크 라우팅 프로토콜 설명.");

        let n = rebuild_book_entities(&conn, "b1").unwrap();
        assert!(n > 0, "엔티티 적재됨");
        assert!(has_entity_index(&conn, "b1"));

        // seed = c1. 확장하면 "프로세스" 공유하는 c2가 나와야, c3는 안 나와야.
        let expanded = expand(&conn, "b1", &[c1], 5, 0.02).unwrap();
        let ids: Vec<i64> = expanded.iter().map(|c| c.id).collect();
        assert!(ids.contains(&c2), "공유 엔티티 이웃 c2 포함: {ids:?}");
        assert!(!ids.contains(&c3), "무관 청크 c3 제외");
        assert!(!ids.contains(&c1), "seed 자신 제외");
        // 점수는 base_score 이하.
        for c in &expanded {
            assert!(c.score <= 0.02 + 1e-9, "확장 점수 ≤ base_score");
            assert!(c.score > 0.0);
        }
    }

    #[test]
    fn expand_empty_seeds_returns_empty() {
        let conn = fresh_conn();
        insert_chunk(&conn, "b1", 0, "프로세스");
        rebuild_book_entities(&conn, "b1").unwrap();
        assert!(expand(&conn, "b1", &[], 5, 0.02).unwrap().is_empty());
    }

    #[test]
    fn expand_with_no_index_returns_empty() {
        let conn = fresh_conn();
        let c1 = insert_chunk(&conn, "b1", 0, "프로세스");
        // rebuild 호출 안 함 → 인덱스 없음.
        assert!(!has_entity_index(&conn, "b1"));
        assert!(expand(&conn, "b1", &[c1], 5, 0.02).unwrap().is_empty());
    }

    #[test]
    fn rebuild_is_idempotent() {
        let conn = fresh_conn();
        insert_chunk(&conn, "b1", 0, "프로세스 스케줄링");
        let n1 = rebuild_book_entities(&conn, "b1").unwrap();
        let n2 = rebuild_book_entities(&conn, "b1").unwrap();
        assert_eq!(n1, n2, "재빌드는 같은 결과");
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunk_entities WHERE book_id='b1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(total as usize, n2, "중복 누적 없음");
    }

    #[test]
    fn expand_excludes_other_books() {
        let conn = fresh_conn();
        let c1 = insert_chunk(&conn, "b1", 0, "프로세스 스케줄링");
        let _c2 = insert_chunk(&conn, "b2", 0, "프로세스 관리"); // 다른 책.
        rebuild_book_entities(&conn, "b1").unwrap();
        rebuild_book_entities(&conn, "b2").unwrap();
        let expanded = expand(&conn, "b1", &[c1], 5, 0.02).unwrap();
        // b2 청크는 book_id 필터로 제외.
        assert!(expanded.iter().all(|c| c.id != _c2));
    }
}
