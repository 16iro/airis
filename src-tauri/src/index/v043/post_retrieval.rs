// v0.4.3 PR 2 (D-088) — Sentence window 확장 / Auto-merging / MMR 중복 제거.
//
// 책임:
//   1. expand_sentence_window — hybrid_search 결과 청크 각각의 prev/next 텍스트를 chunks
//      테이블에서 lookup하고 *합본 텍스트*를 만들어 둔다 (architecture §4.7.2 Sentence Window
//      Retrieval). 검색 정밀도(작은 청크) ↔ 컨텍스트 풍부함(앞뒤 N문장) trade-off 균형.
//   2. merge_parents — 같은 parent_id의 청크가 2개 이상 매칭되고 *부모의 자식 토큰 합 < 800*
//      이면 부모 청크 하나로 *치환* (auto-merging). 1개만 매칭되거나 토큰 합이 800 이상이면
//      그대로 sentence window 결과만 유지.
//   3. mmr_dedupe — Maximal Marginal Relevance (Carbonell·Goldstein 1998). top-K 후보에서
//      relevance × diversity 균형(λ=0.5 default)으로 top-N을 추린다. 임베딩이 누락된
//      후보는 cosine similarity 페널티 없이 *relevance만으로* 평가 — 임베딩 갱신 race나
//      v041 hybrid retrieval에 포함되지 않은 청크에 대한 graceful 폴백.
//
// 통합 흐름 (commands::llm::build_v041_block):
//   hybrid_search → expand_sentence_window → merge_parents → mmr_dedupe → top-N
//   * SearchStrength::Fast → 후처리 skip (속도 우선, 원본 retrieval 결과 그대로).
//   * SearchStrength::Balanced (default) / Accurate → 후처리 ON.
//
// 의존성 추가 X — HashMap·자체 cosine 구현. 모델 호출 X (순수 후처리).

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use tracing::warn;

use crate::error::AppResult;
use crate::index::v041::chunker::token_count_heuristic;
use crate::index::v041::retrieval::RetrievedChunk;

/// auto-merging 토큰 가드 — parent의 자식 청크 토큰 합이 이 값 *미만*일 때만 merge 한다.
/// HANDOFF §9: "parent 청크가 너무 큰 경우 토큰 예산 폭발". 한국어 학습 시나리오에서
/// 한 섹션 800 토큰(=대략 ~3,200자) 정도가 LLM 컨텍스트 패킹에 무리 없는 상한.
pub const AUTO_MERGE_TOKEN_LIMIT: usize = 800;

/// auto-merging 임계 — 같은 parent의 청크가 *이 수 이상* 매칭돼야 merge 한다 (D-088).
pub const AUTO_MERGE_MIN_GROUP: usize = 2;

/// MMR λ default — relevance vs diversity 균형 (D-088).
pub const MMR_LAMBDA_DEFAULT: f32 = 0.5;

// =============================================================================
// 1) Sentence window 확장
// =============================================================================

/// 한 chunk + 그 prev/next 텍스트 — 합본 본문까지 사전 계산.
#[derive(Debug, Clone)]
pub struct RetrievedChunkExpanded {
    /// 원본 retrieval 결과 (점수·메타 그대로).
    pub core: RetrievedChunk,
    /// prev_chunk_id가 가리키는 청크의 본문 — 없으면 None.
    pub prev_text: Option<String>,
    /// next_chunk_id가 가리키는 청크의 본문 — 없으면 None.
    pub next_text: Option<String>,
    /// 합본 텍스트 — `prev_text\n\n core.text \n\n next_text` 형식.
    /// prev/next가 None이면 자연스레 그 부분이 없는 형태로 결합.
    pub expanded_text: String,
}

impl RetrievedChunkExpanded {
    /// 합본 본문의 토큰 휴리스틱.
    pub fn token_count(&self) -> usize {
        token_count_heuristic(&self.expanded_text)
    }
}

/// hybrid_search 결과를 받아 각 청크의 prev/next 텍스트를 chunks 테이블에서 lookup,
/// 합본 텍스트로 확장한다.
///
/// * prev/next id 셋을 한 번에 모아 하나의 SELECT IN 쿼리로 batched lookup → N+1 회피.
/// * lookup 실패 row(예: chunk 삭제 race)는 *그대로 None* — 합본 텍스트에서 자연 누락.
/// * 점수·순서는 입력 그대로 유지 (호출 측이 점수 내림차순으로 받으면 출력도 동일).
pub fn expand_sentence_window(
    conn: &Connection,
    retrieved: &[RetrievedChunk],
) -> AppResult<Vec<RetrievedChunkExpanded>> {
    if retrieved.is_empty() {
        return Ok(Vec::new());
    }

    // 1) 모든 prev/next id 수집 → batched lookup.
    let mut neighbor_ids: HashSet<i64> = HashSet::new();
    for c in retrieved {
        if let Some(p) = c.prev_chunk_id {
            neighbor_ids.insert(p);
        }
        if let Some(n) = c.next_chunk_id {
            neighbor_ids.insert(n);
        }
    }
    let neighbor_text_map = if neighbor_ids.is_empty() {
        HashMap::new()
    } else {
        fetch_text_by_ids(conn, &neighbor_ids)?
    };

    // 2) 각 청크별 합본 텍스트 만들기.
    let mut out: Vec<RetrievedChunkExpanded> = Vec::with_capacity(retrieved.len());
    for c in retrieved {
        let prev_text = c
            .prev_chunk_id
            .and_then(|id| neighbor_text_map.get(&id).cloned());
        let next_text = c
            .next_chunk_id
            .and_then(|id| neighbor_text_map.get(&id).cloned());

        let mut expanded = String::new();
        if let Some(p) = &prev_text {
            expanded.push_str(p);
            expanded.push_str("\n\n");
        }
        expanded.push_str(&c.text);
        if let Some(n) = &next_text {
            expanded.push_str("\n\n");
            expanded.push_str(n);
        }

        out.push(RetrievedChunkExpanded {
            core: c.clone(),
            prev_text,
            next_text,
            expanded_text: expanded,
        });
    }
    Ok(out)
}

/// `id IN (...)` batched lookup — id → text 매핑.
fn fetch_text_by_ids(
    conn: &Connection,
    ids: &HashSet<i64>,
) -> AppResult<HashMap<i64, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    // rusqlite의 `?n` 바인딩은 슬라이스 IN 직접 지원 X — placeholder를 직접 만든다.
    // ids 개수가 보통 retrieved.len()*2 이내(=20~40)라 SQL 길이는 무리 없음.
    let mut placeholders = String::with_capacity(ids.len() * 2);
    for i in 0..ids.len() {
        if i > 0 {
            placeholders.push(',');
        }
        placeholders.push('?');
    }
    let sql = format!("SELECT id, text FROM chunks WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let id_vec: Vec<i64> = ids.iter().copied().collect();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(id_vec.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

// =============================================================================
// 2) Auto-merging
// =============================================================================

/// 후처리 결과의 단일 단위 — 부모 chunk로 병합됐거나, 단일 chunk(sentence window 확장본)
/// 그대로 유지된 것.
#[derive(Debug, Clone)]
pub struct MergedChunk {
    /// 병합 결과 chunks.id — 부모로 치환됐으면 parent_id, 그대로 유지면 원본 chunks.id.
    pub id: i64,
    /// 병합·확장 후 본문.
    pub text: String,
    /// 점수 — *참여한 source chunks의 점수 합*. 큰 값이 더 관련도 높음.
    pub score: f64,
    /// 페이지 (PDF 1-base, MD/HTML은 None) — 부모 row의 page를 우선, 없으면 첫 source 청크.
    pub page: Option<i64>,
    /// section_path — 부모 우선, 없으면 첫 source 청크.
    pub section_path: Option<String>,
    /// 토큰 카운트 (휴리스틱) — context 빌더가 토큰 예산 패킹에 사용.
    pub token_count: usize,
    /// 본 merged 단위에 참여한 *원본 chunks.id*들. 디버깅·UI 표시(병합 사실 명시) 용도.
    pub source_chunks: Vec<i64>,
}

/// 같은 parent_id 청크 ≥ AUTO_MERGE_MIN_GROUP 매칭 + 부모의 자식 토큰 합 < AUTO_MERGE_TOKEN_LIMIT
/// 일 때만 *부모 chunk*로 치환. 그 외엔 sentence-window 확장본을 그대로 single MergedChunk로
/// 유지.
///
/// 출력 순서는 입력 순서를 *기본*으로 보존 (점수 내림차순). 그룹 병합으로 사라지는 entry는
/// 첫 등장 위치를 기준으로 한 번만 결과에 push.
pub fn merge_parents(
    conn: &Connection,
    expanded: &[RetrievedChunkExpanded],
) -> AppResult<Vec<MergedChunk>> {
    if expanded.is_empty() {
        return Ok(Vec::new());
    }

    // 1) parent_id별 그룹 수집 + 첫 등장 인덱스 기록(출력 순서 보존).
    let mut groups: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut first_seen: HashMap<i64, usize> = HashMap::new();
    for (i, e) in expanded.iter().enumerate() {
        if let Some(p) = e.core.parent_id {
            groups.entry(p).or_default().push(i);
            first_seen.entry(p).or_insert(i);
        }
    }

    // 2) 그룹 ≥ AUTO_MERGE_MIN_GROUP 인 parent의 *자식 토큰 합*을 한 번에 lookup.
    let merge_candidate_parents: Vec<i64> = groups
        .iter()
        .filter(|(_, idxs)| idxs.len() >= AUTO_MERGE_MIN_GROUP)
        .map(|(p, _)| *p)
        .collect();

    let parent_total_tokens = if merge_candidate_parents.is_empty() {
        HashMap::new()
    } else {
        fetch_parent_token_sums(conn, &merge_candidate_parents)?
    };

    // 3) 실제로 merge할 parent 결정 — 토큰 합 < LIMIT.
    let mut merged_parents: HashMap<i64, ParentMergeRecord> = HashMap::new();
    for parent_id in &merge_candidate_parents {
        let total = parent_total_tokens.get(parent_id).copied().unwrap_or(0);
        if total >= AUTO_MERGE_TOKEN_LIMIT {
            continue;
        }
        match fetch_parent_record(conn, *parent_id) {
            Ok(Some(rec)) => {
                merged_parents.insert(*parent_id, rec);
            }
            Ok(None) => {
                // parent가 사라졌으면(예: 인덱싱 race) merge skip — sentence window만 유지.
                warn!(
                    target: "v043.post_retrieval",
                    parent_id,
                    "auto-merging skip — parent 청크 row 부재"
                );
            }
            Err(e) => {
                warn!(
                    target: "v043.post_retrieval",
                    parent_id,
                    error = %e,
                    "auto-merging skip — parent 조회 실패"
                );
            }
        }
    }

    // 4) 출력 시퀀스 빌드 — 입력 순서 보존, 첫 등장 시점에 결과 push.
    let mut out: Vec<MergedChunk> = Vec::with_capacity(expanded.len());
    let mut emitted_parents: HashSet<i64> = HashSet::new();

    for (i, e) in expanded.iter().enumerate() {
        // (a) 부모 병합 케이스 — 그룹 첫 등장 자리에 1번만 push.
        if let Some(parent_id) = e.core.parent_id {
            if let Some(parent_rec) = merged_parents.get(&parent_id) {
                if first_seen.get(&parent_id) == Some(&i)
                    && !emitted_parents.contains(&parent_id)
                {
                    let group_idxs = groups.get(&parent_id).cloned().unwrap_or_default();
                    let score_sum: f64 =
                        group_idxs.iter().map(|j| expanded[*j].core.score).sum();
                    let source_chunks: Vec<i64> =
                        group_idxs.iter().map(|j| expanded[*j].core.id).collect();
                    let token_count = parent_total_tokens
                        .get(&parent_id)
                        .copied()
                        .unwrap_or_else(|| token_count_heuristic(&parent_rec.text));
                    out.push(MergedChunk {
                        id: parent_id,
                        text: parent_rec.text.clone(),
                        score: score_sum,
                        page: parent_rec.page,
                        section_path: parent_rec.section_path.clone(),
                        token_count,
                        source_chunks,
                    });
                    emitted_parents.insert(parent_id);
                }
                continue; // group 멤버 두 번째 이후는 skip.
            }
        }

        // (b) 병합 안 된 케이스 — sentence-window 확장본 그대로 single MergedChunk.
        let token_count = e.token_count();
        out.push(MergedChunk {
            id: e.core.id,
            text: e.expanded_text.clone(),
            score: e.core.score,
            page: e.core.page,
            section_path: e.core.section_path.clone(),
            token_count,
            source_chunks: vec![e.core.id],
        });
    }

    Ok(out)
}

/// parent 청크 본문 + 메타.
struct ParentMergeRecord {
    text: String,
    page: Option<i64>,
    section_path: Option<String>,
}

/// parent_id가 같은 *모든* chunks의 token_count 합. token_count NULL은 본문 휴리스틱으로
/// 대체.
fn fetch_parent_token_sums(
    conn: &Connection,
    parents: &[i64],
) -> AppResult<HashMap<i64, usize>> {
    if parents.is_empty() {
        return Ok(HashMap::new());
    }
    // parent_id 별 토큰 합. token_count가 NULL인 row는 본문 길이 휴리스틱으로 fallback.
    // (chunker가 항상 token_count를 채우므로 NULL은 드물지만 안전 분기.)
    let mut placeholders = String::with_capacity(parents.len() * 2);
    for i in 0..parents.len() {
        if i > 0 {
            placeholders.push(',');
        }
        placeholders.push('?');
    }
    let sql = format!(
        "SELECT parent_id, token_count, text FROM chunks \
         WHERE parent_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(parents.iter()), |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut sum: HashMap<i64, usize> = HashMap::new();
    for (parent_opt, tc_opt, text) in rows {
        let Some(parent_id) = parent_opt else {
            continue;
        };
        let tc = tc_opt
            .map(|t| t.max(0) as usize)
            .unwrap_or_else(|| token_count_heuristic(&text));
        *sum.entry(parent_id).or_insert(0) += tc;
    }
    Ok(sum)
}

/// parent 청크 row 자체 lookup (text + page + section_path).
fn fetch_parent_record(
    conn: &Connection,
    parent_id: i64,
) -> AppResult<Option<ParentMergeRecord>> {
    let row = conn.query_row(
        "SELECT text, page, section_path FROM chunks WHERE id = ?1",
        params![parent_id],
        |r| {
            let section_path: Option<String> = r.get::<_, Option<String>>(2)?.and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            });
            Ok(ParentMergeRecord {
                text: r.get::<_, String>(0)?,
                page: r.get::<_, Option<i64>>(1)?,
                section_path,
            })
        },
    );
    match row {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// =============================================================================
// 3) MMR (Maximal Marginal Relevance)
// =============================================================================

/// MMR (Carbonell·Goldstein 1998) — top-N 후보를 다양성과 균형 있게 선택.
///
/// 알고리즘:
///   1. relevance(c, q) = cosine(c, q). 임베딩 누락이면 *score 자체*를 relevance로 사용.
///   2. 매 step:
///      s(c) = λ · relevance(c, q) − (1−λ) · max cosine(c, c') over selected.
///   3. argmax s(c) 하나 선택 → selected 추가. selected.len() == n까지 반복.
///
/// 입력 `embeddings`: id → 임베딩 벡터(L2 정규화 권장 — 호출 측 책임). 누락 시 cosine
/// 페널티 0으로 처리(=다양성 항 무력화)되므로 relevance만으로 평가.
///
/// `lambda`:
///   * 1.0 → relevance만 — MMR 효과 없음 (점수 내림차순과 동치).
///   * 0.0 → diversity 극대화 — relevance 영향 X.
///   * 0.5 → 균형 (default).
///
/// `n`이 후보 수보다 크거나 같으면 모든 후보 반환 (단, *MMR 순서로 재배열*).
pub fn mmr_dedupe(
    query_embedding: &[f32],
    candidates: &[MergedChunk],
    embeddings: &HashMap<i64, Vec<f32>>,
    lambda: f32,
    n: usize,
) -> Vec<MergedChunk> {
    if candidates.is_empty() || n == 0 {
        return Vec::new();
    }
    let lambda = lambda.clamp(0.0, 1.0);
    let target = n.min(candidates.len());

    // relevance 벡터 사전 계산 — id가 embeddings에 없거나 query_embedding이 비어있으면
    // *score 그대로* (단, 후보들 사이 비교가 가능해야 하므로 candidates의 score 스케일을
    // 그대로 사용).
    let relevance: Vec<f32> = candidates
        .iter()
        .map(|c| {
            if query_embedding.is_empty() {
                c.score as f32
            } else {
                match embeddings.get(&c.id) {
                    Some(emb) if !emb.is_empty() => cosine_similarity(query_embedding, emb),
                    _ => c.score as f32,
                }
            }
        })
        .collect();

    let mut selected: Vec<usize> = Vec::with_capacity(target);
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();

    while selected.len() < target && !remaining.is_empty() {
        let mut best_idx_in_remaining = 0_usize;
        let mut best_score = f32::NEG_INFINITY;

        for (rem_pos, cand_idx) in remaining.iter().enumerate() {
            let rel = relevance[*cand_idx];
            let div_penalty = if selected.is_empty() {
                0.0
            } else {
                // max cosine vs already selected.
                let cand_emb = embeddings.get(&candidates[*cand_idx].id);
                let mut max_sim = 0.0_f32;
                for sel in &selected {
                    let sel_emb = embeddings.get(&candidates[*sel].id);
                    let sim = match (cand_emb, sel_emb) {
                        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => {
                            cosine_similarity(a, b)
                        }
                        // 임베딩 누락이면 다양성 페널티 0 — graceful (relevance만 영향).
                        _ => 0.0,
                    };
                    if sim > max_sim {
                        max_sim = sim;
                    }
                }
                max_sim
            };
            let mmr_score = lambda * rel - (1.0 - lambda) * div_penalty;
            if mmr_score > best_score {
                best_score = mmr_score;
                best_idx_in_remaining = rem_pos;
            }
        }
        let chosen = remaining.swap_remove(best_idx_in_remaining);
        selected.push(chosen);
    }

    selected
        .into_iter()
        .map(|i| candidates[i].clone())
        .collect()
}

/// cosine similarity — 길이 다르면 짧은 쪽으로 잘라서 안전 계산. 0 벡터면 0.0.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..len {
        let ai = a[i];
        let bi = b[i];
        dot += ai * bi;
        na += ai * ai;
        nb += bi * bi;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // -------------------------------------------------------------------------
    // 헬퍼 — in-memory chunks 테이블 (마이그레이션 전체 적용 부담 회피, 최소 스키마).
    // -------------------------------------------------------------------------

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // post_retrieval은 chunks 테이블만 read 함 — FK·트리거·FTS 불필요. 최소 스키마.
        conn.execute_batch(
            "CREATE TABLE chunks (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                book_id         TEXT    NOT NULL,
                ord             INTEGER NOT NULL,
                text            TEXT    NOT NULL,
                page            INTEGER,
                parent_id       INTEGER,
                prev_chunk_id   INTEGER,
                next_chunk_id   INTEGER,
                section_path    TEXT,
                token_count     INTEGER
            );",
        )
        .unwrap();
        conn
    }

    /// chunks INSERT 헬퍼.
    #[allow(clippy::too_many_arguments)]
    fn insert_chunk(
        conn: &Connection,
        book_id: &str,
        ord: i64,
        text: &str,
        section: Option<&str>,
        page: Option<i64>,
        parent_id: Option<i64>,
        prev_id: Option<i64>,
        next_id: Option<i64>,
        token_count: Option<i64>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO chunks (book_id, ord, text, section_path, page, parent_id, \
                                  prev_chunk_id, next_chunk_id, token_count) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                book_id, ord, text, section, page, parent_id, prev_id, next_id, token_count
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn rc(
        id: i64,
        score: f64,
        text: &str,
        parent_id: Option<i64>,
        prev: Option<i64>,
        next: Option<i64>,
        token_count: Option<i64>,
    ) -> RetrievedChunk {
        RetrievedChunk {
            id,
            text: text.to_string(),
            page: None,
            section_path: None,
            parent_id,
            prev_chunk_id: prev,
            next_chunk_id: next,
            token_count,
            score,
        }
    }

    // -------------------------------------------------------------------------
    // expand_sentence_window
    // -------------------------------------------------------------------------

    #[test]
    fn expand_sentence_window_concatenates_prev_and_next() {
        let conn = fresh_conn();
        let id1 = insert_chunk(&conn, "b1", 0, "first chunk", None, None, None, None, None, None);
        let id2 = insert_chunk(
            &conn,
            "b1",
            1,
            "middle chunk",
            None,
            None,
            None,
            Some(id1),
            None,
            None,
        );
        let id3 = insert_chunk(
            &conn,
            "b1",
            2,
            "last chunk",
            None,
            None,
            None,
            Some(id2),
            None,
            None,
        );
        // id2 의 next_chunk_id = id3로 보정 (insert 순서 때문에 id3 INSERT 후 UPDATE).
        conn.execute(
            "UPDATE chunks SET next_chunk_id = ?1 WHERE id = ?2",
            params![id3, id2],
        )
        .unwrap();
        // id1 의 next_chunk_id = id2.
        conn.execute(
            "UPDATE chunks SET next_chunk_id = ?1 WHERE id = ?2",
            params![id2, id1],
        )
        .unwrap();

        // retrieved = [id2 단독 매칭]. prev=id1, next=id3가 합쳐져야 함.
        let retrieved =
            vec![rc(id2, 0.9, "middle chunk", None, Some(id1), Some(id3), None)];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].prev_text.as_deref(), Some("first chunk"));
        assert_eq!(expanded[0].next_text.as_deref(), Some("last chunk"));
        assert!(expanded[0].expanded_text.contains("first chunk"));
        assert!(expanded[0].expanded_text.contains("middle chunk"));
        assert!(expanded[0].expanded_text.contains("last chunk"));
        assert!(
            expanded[0].expanded_text.find("first chunk").unwrap()
                < expanded[0].expanded_text.find("middle chunk").unwrap()
        );
        assert!(
            expanded[0].expanded_text.find("middle chunk").unwrap()
                < expanded[0].expanded_text.find("last chunk").unwrap()
        );
    }

    #[test]
    fn expand_sentence_window_handles_boundary_chunks_with_null_neighbors() {
        let conn = fresh_conn();
        let id1 = insert_chunk(&conn, "b1", 0, "solo chunk", None, None, None, None, None, None);
        // prev/next 모두 NULL.
        let retrieved = vec![rc(id1, 0.5, "solo chunk", None, None, None, None)];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        assert_eq!(expanded.len(), 1);
        assert!(expanded[0].prev_text.is_none());
        assert!(expanded[0].next_text.is_none());
        assert_eq!(expanded[0].expanded_text, "solo chunk");
    }

    #[test]
    fn expand_sentence_window_empty_input_returns_empty() {
        let conn = fresh_conn();
        let expanded = expand_sentence_window(&conn, &[]).unwrap();
        assert!(expanded.is_empty());
    }

    #[test]
    fn expand_sentence_window_silently_drops_missing_neighbor_text() {
        let conn = fresh_conn();
        // retrieved 의 prev_chunk_id=999 (실재 X). lookup 실패 → prev_text=None, expanded엔
        // 그 부분 누락된 합본.
        let id1 = insert_chunk(&conn, "b1", 0, "core chunk", None, None, None, None, None, None);
        let retrieved = vec![rc(id1, 0.5, "core chunk", None, Some(999), None, None)];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        assert_eq!(expanded.len(), 1);
        assert!(expanded[0].prev_text.is_none(), "삭제된 이웃은 graceful skip");
        assert_eq!(expanded[0].expanded_text, "core chunk");
    }

    // -------------------------------------------------------------------------
    // merge_parents
    // -------------------------------------------------------------------------

    #[test]
    fn merge_parents_replaces_with_parent_when_two_children_match_and_under_token_limit() {
        let conn = fresh_conn();
        // parent (id=1, text=parent body) + 자식 2 (id=2,3).
        let parent_id =
            insert_chunk(&conn, "b1", 0, "parent body", Some("Ch01"), Some(10), None, None, None, Some(50));
        let child1 = insert_chunk(
            &conn,
            "b1",
            1,
            "child A",
            None,
            None,
            Some(parent_id),
            None,
            None,
            Some(60),
        );
        let child2 = insert_chunk(
            &conn,
            "b1",
            2,
            "child B",
            None,
            None,
            Some(parent_id),
            Some(child1),
            None,
            Some(70),
        );
        // child1·child2 둘 다 retrieved → token_sum=130 < 800 → merge.
        let retrieved = vec![
            rc(child1, 0.9, "child A", Some(parent_id), None, Some(child2), Some(60)),
            rc(child2, 0.7, "child B", Some(parent_id), Some(child1), None, Some(70)),
        ];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        let merged = merge_parents(&conn, &expanded).unwrap();
        assert_eq!(merged.len(), 1, "두 자식 → 부모 1개로 병합");
        assert_eq!(merged[0].id, parent_id);
        assert_eq!(merged[0].text, "parent body");
        assert!((merged[0].score - 1.6).abs() < 1e-9, "score 합산");
        assert_eq!(merged[0].section_path.as_deref(), Some("Ch01"));
        assert_eq!(merged[0].page, Some(10));
        let mut srcs = merged[0].source_chunks.clone();
        srcs.sort();
        let mut expected = vec![child1, child2];
        expected.sort();
        assert_eq!(srcs, expected);
    }

    #[test]
    fn merge_parents_keeps_single_match_unmerged() {
        let conn = fresh_conn();
        let parent_id =
            insert_chunk(&conn, "b1", 0, "parent body", None, None, None, None, None, Some(50));
        let child1 = insert_chunk(
            &conn,
            "b1",
            1,
            "child A",
            Some("Ch01"),
            None,
            Some(parent_id),
            None,
            None,
            Some(60),
        );
        // 한 자식만 retrieved → 그대로 유지.
        let retrieved = vec![rc(
            child1,
            0.8,
            "child A",
            Some(parent_id),
            None,
            None,
            Some(60),
        )];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        let merged = merge_parents(&conn, &expanded).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, child1, "부모로 치환되지 않음");
        assert!((merged[0].score - 0.8).abs() < 1e-9);
        assert_eq!(merged[0].source_chunks, vec![child1]);
    }

    #[test]
    fn merge_parents_skips_merge_when_token_sum_exceeds_limit() {
        let conn = fresh_conn();
        let parent_id =
            insert_chunk(&conn, "b1", 0, "huge parent", None, None, None, None, None, Some(900));
        // 자식 2개 token_count 합 = 900 ≥ 800 → merge skip.
        let child1 = insert_chunk(
            &conn,
            "b1",
            1,
            "child A",
            None,
            None,
            Some(parent_id),
            None,
            None,
            Some(450),
        );
        let child2 = insert_chunk(
            &conn,
            "b1",
            2,
            "child B",
            None,
            None,
            Some(parent_id),
            Some(child1),
            None,
            Some(450),
        );
        let retrieved = vec![
            rc(child1, 0.9, "child A", Some(parent_id), None, Some(child2), Some(450)),
            rc(child2, 0.7, "child B", Some(parent_id), Some(child1), None, Some(450)),
        ];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        let merged = merge_parents(&conn, &expanded).unwrap();
        assert_eq!(merged.len(), 2, "토큰 한도 초과 → merge skip, sentence window만");
        assert_eq!(merged[0].id, child1);
        assert_eq!(merged[1].id, child2);
    }

    #[test]
    fn merge_parents_does_not_cross_different_parents() {
        let conn = fresh_conn();
        let p1 = insert_chunk(&conn, "b1", 0, "P1", None, None, None, None, None, Some(20));
        let p2 = insert_chunk(&conn, "b1", 1, "P2", None, None, None, None, None, Some(20));
        let c1a = insert_chunk(&conn, "b1", 2, "c1a", None, None, Some(p1), None, None, Some(30));
        let c2a = insert_chunk(&conn, "b1", 3, "c2a", None, None, Some(p2), None, None, Some(30));

        let retrieved = vec![
            rc(c1a, 0.9, "c1a", Some(p1), None, None, Some(30)),
            rc(c2a, 0.7, "c2a", Some(p2), None, None, Some(30)),
        ];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        let merged = merge_parents(&conn, &expanded).unwrap();
        assert_eq!(merged.len(), 2, "다른 parent끼리는 병합 X");
    }

    #[test]
    fn merge_parents_preserves_input_order_with_partial_merge() {
        let conn = fresh_conn();
        let p = insert_chunk(&conn, "b1", 0, "P", None, None, None, None, None, Some(20));
        let c1 = insert_chunk(&conn, "b1", 1, "c1", None, None, Some(p), None, None, Some(30));
        let c2 = insert_chunk(&conn, "b1", 2, "c2", None, None, Some(p), None, None, Some(30));
        // sibling 없이 단독 매칭 — 별도 청크.
        let solo = insert_chunk(&conn, "b1", 3, "solo", Some("Other"), None, None, None, None, Some(40));

        // 입력 순서: [c1, solo, c2]. 첫 등장이 c1이라 병합 결과는 c1 자리에 부모 entry 1개.
        let retrieved = vec![
            rc(c1, 0.9, "c1", Some(p), None, None, Some(30)),
            rc(solo, 0.5, "solo", None, None, None, Some(40)),
            rc(c2, 0.4, "c2", Some(p), None, None, Some(30)),
        ];
        let expanded = expand_sentence_window(&conn, &retrieved).unwrap();
        let merged = merge_parents(&conn, &expanded).unwrap();
        assert_eq!(merged.len(), 2, "[parent, solo] — c1·c2는 부모 1개로 병합");
        assert_eq!(merged[0].id, p, "첫 entry = parent (c1 자리)");
        assert_eq!(merged[1].id, solo, "solo는 그대로");
    }

    #[test]
    fn merge_parents_empty_input_returns_empty() {
        let conn = fresh_conn();
        let merged = merge_parents(&conn, &[]).unwrap();
        assert!(merged.is_empty());
    }

    // -------------------------------------------------------------------------
    // mmr_dedupe
    // -------------------------------------------------------------------------

    fn mc(id: i64, score: f64, text: &str) -> MergedChunk {
        MergedChunk {
            id,
            text: text.to_string(),
            score,
            page: None,
            section_path: None,
            token_count: text.chars().count(),
            source_chunks: vec![id],
        }
    }

    fn unit(v: Vec<f32>) -> Vec<f32> {
        // 정규화 안 해도 cosine은 자체 분모로 처리 — 그대로 반환.
        v
    }

    #[test]
    fn mmr_dedupe_lambda_one_acts_like_relevance_only() {
        // λ=1.0이면 diversity 항 제거 → relevance 내림차순과 같다.
        let candidates = vec![mc(1, 0.9, "A"), mc(2, 0.5, "B"), mc(3, 0.1, "C")];
        let mut emb = HashMap::new();
        // query embedding과 candidates 임베딩 — relevance를 score와 동일하게 만들기 위해
        // *같은 방향*의 길이 다른 벡터로 cosine ≈ 1로 동등하게 하는 대신, embeddings 누락 →
        // relevance = score 폴백을 활용.
        let _ = emb.insert(0_i64, vec![1.0]); // dummy
        let q = unit(vec![1.0, 0.0]);
        let out = mmr_dedupe(&q, &candidates, &emb, 1.0, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, 1);
        assert_eq!(out[1].id, 2);
        assert_eq!(out[2].id, 3);
    }

    #[test]
    fn mmr_dedupe_lambda_zero_maximizes_diversity() {
        // 후보 3개. id=1과 id=2가 임베딩 같음(=거의 중복). λ=0이면 첫 선택 후 중복 청크가
        // 제일 페널티 많아 *덜 비슷한* id=3이 다음으로 선택된다.
        let candidates = vec![mc(1, 0.9, "A"), mc(2, 0.85, "B"), mc(3, 0.1, "C")];
        let mut emb: HashMap<i64, Vec<f32>> = HashMap::new();
        emb.insert(1, unit(vec![1.0, 0.0]));
        emb.insert(2, unit(vec![1.0, 0.001])); // id=1과 거의 동일.
        emb.insert(3, unit(vec![0.0, 1.0])); // 직교 — 가장 다양.

        let q = unit(vec![1.0, 0.0]);
        // λ=0이면 첫 선택은 모두 relevance 0이라 *후보 순서대로 첫 번째*가 선택될 수 있음 —
        // tie-break에 의존. 본 테스트는 λ=0에서 *2번째 선택은 id=3* (직교)임을 검증.
        let out = mmr_dedupe(&q, &candidates, &emb, 0.0, 3);
        assert_eq!(out.len(), 3);
        // 첫 선택은 tie라 구체 id 단정 X. 다만 *id=3은 두 번째 안에 등장*해야 함 —
        // 첫 선택이 1이든 2이든 그 다음 후보로 가장 다른 게 3이므로.
        let first_two: Vec<i64> = out.iter().take(2).map(|c| c.id).collect();
        assert!(
            first_two.contains(&3),
            "λ=0에서 두 번째 선택은 가장 다양한 청크 (id=3)"
        );
    }

    #[test]
    fn mmr_dedupe_balanced_lambda_diversifies_similar_high_relevance_chunks() {
        // id=1·2가 둘 다 query에 매우 유사·서로도 매우 유사. id=3은 query 유사도 중간이지만
        // 1·2와 직교. λ=0.5이면 1·2 둘 다 뽑기보다 1과 3을 뽑는 쪽이 선호됨.
        let candidates = vec![mc(1, 0.9, "A"), mc(2, 0.88, "B"), mc(3, 0.6, "C")];
        let mut emb: HashMap<i64, Vec<f32>> = HashMap::new();
        emb.insert(1, unit(vec![1.0, 0.0]));
        emb.insert(2, unit(vec![1.0, 0.01])); // id=1과 거의 동일.
        emb.insert(3, unit(vec![0.5, 0.866])); // query 유사도 0.5, id=1과는 0.5.

        let q = unit(vec![1.0, 0.0]);
        let out = mmr_dedupe(&q, &candidates, &emb, 0.5, 2);
        assert_eq!(out.len(), 2);
        // 첫 선택 = id=1 (relevance 최대).
        assert_eq!(out[0].id, 1);
        // 두 번째 선택은 *id=3* — id=2는 id=1과 거의 동일이라 페널티가 큼.
        assert_eq!(out[1].id, 3, "λ=0.5에서 다양성으로 id=3 우선");
    }

    #[test]
    fn mmr_dedupe_falls_back_to_score_when_embedding_missing() {
        // 모든 후보의 임베딩 누락. cosine 계산 불가 → relevance = score, diversity penalty=0.
        // 결과는 score 내림차순 (= λ에 무관, 분모 페널티 항이 0).
        let candidates = vec![mc(10, 0.4, "X"), mc(20, 0.9, "Y"), mc(30, 0.1, "Z")];
        let emb: HashMap<i64, Vec<f32>> = HashMap::new();
        let q = unit(vec![1.0, 0.0]);
        let out = mmr_dedupe(&q, &candidates, &emb, 0.5, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, 20);
        assert_eq!(out[1].id, 10);
        assert_eq!(out[2].id, 30);
    }

    #[test]
    fn mmr_dedupe_n_zero_returns_empty() {
        let candidates = vec![mc(1, 0.9, "A")];
        let emb: HashMap<i64, Vec<f32>> = HashMap::new();
        let out = mmr_dedupe(&[], &candidates, &emb, 0.5, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn mmr_dedupe_n_larger_than_candidates_returns_all() {
        let candidates = vec![mc(1, 0.9, "A"), mc(2, 0.5, "B")];
        let emb: HashMap<i64, Vec<f32>> = HashMap::new();
        let out = mmr_dedupe(&[], &candidates, &emb, 0.5, 10);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_identical_is_one() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vector_is_zero() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-9);
    }

    #[test]
    fn cosine_similarity_handles_length_mismatch() {
        // 짧은 쪽으로 잘라 안전 계산.
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }
}
