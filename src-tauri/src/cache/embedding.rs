// 임베딩 영속 cache (D-084) — sha256(text + ':' + model) → BLOB(little-endian f32) 페어.
//
// invariant:
//   * text_hash PK = sha256(text + ':' + model). 같은 텍스트라도 모델이 다르면 다른 row.
//   * model 컬럼은 row 필터링·디버그·LRU eviction 우선순위 (모델별 별도 cap도 가능)에 사용.
//   * dim 컬럼은 BLOB 길이 검증 (model에서 자동 도출되지만 row 자체로 자기검증 가능하게 영속).
//   * created_at은 처음 INSERT 시각 (ms epoch). last_hit_at은 hit 시 갱신 — LRU eviction 키.
//
// 인메모리 핫셋 (HotLru<String, Vec<f32>>):
//   * key = text_hash (sha256). 작은 cap(=1024)에 충분 — 대용량 텍스트 자체를 보관 X.
//   * SQLite 회피 — 첫 lookup만 SQLite, 이후엔 인메모리.
//   * 핫셋 capacity 초과 시 oldest 1개 제거 (HotLru가 책임).
//
// LRU eviction (영속):
//   * `evict_lru(max_rows)` — 행 수가 max_rows를 초과하면 last_hit_at ASC LIMIT 차이만큼 DELETE.
//   * NULL last_hit_at은 가장 오래된 (= COALESCE(last_hit_at, 0)).
//
// thread-safety:
//   * Connection은 호출 측이 *매 메서드*에 인자로 전달 (호출 측의 db Mutex 안에서).
//   * HotLru와 hit/miss 카운터는 self의 Mutex/Atomic.

#![allow(dead_code)]

use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::cache::{f32_from_le_bytes, f32_to_le_bytes, sha256_hex, CacheStats, HotLru};
use crate::error::{AppError, AppResult};

/// 인메모리 핫셋 cap — HANDOFF §1.2 권장.
pub const HOT_CAP: usize = 1024;

/// 영속 LRU 임계 — HANDOFF §1.1 (`MAX_ROWS=10_000`).
pub const MAX_ROWS_DEFAULT: usize = 10_000;

/// 임베딩 cache. SQLite 영속 + 인메모리 핫셋 + 누적 hit/miss 카운터.
///
/// Connection은 호출 측이 보유 — 매 메서드 진입에 `&Connection` 또는 `&mut Connection`
/// 인자로 전달. 이유: AppState의 `db: Mutex<Db>` 와 호환되고, 캐시 자체가 별도 핸들을
/// 열지 않아도 된다.
pub struct EmbeddingCache {
    hot: Mutex<HotLru<String, Vec<f32>>>,
    hit_count: std::sync::atomic::AtomicU64,
    miss_count: std::sync::atomic::AtomicU64,
}

impl Default for EmbeddingCache {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingCache {
    pub fn new() -> Self {
        Self::with_capacity(HOT_CAP)
    }

    pub fn with_capacity(hot_cap: usize) -> Self {
        Self {
            hot: Mutex::new(HotLru::new(hot_cap)),
            hit_count: std::sync::atomic::AtomicU64::new(0),
            miss_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// 키 생성 — sha256(text + ':' + model). v15 마이그 주석 컨벤션.
    pub fn make_key(text: &str, model: &str) -> String {
        let mut combined = String::with_capacity(text.len() + 1 + model.len());
        combined.push_str(text);
        combined.push(':');
        combined.push_str(model);
        sha256_hex(&combined)
    }

    /// 단일 텍스트 lookup. 핫셋 hit이면 SQLite 회피. 아니면 SQLite lookup.
    /// hit 시 last_hit_at 갱신 + 핫셋 promote.
    pub fn get(
        &self,
        conn: &Connection,
        text: &str,
        model: &str,
    ) -> AppResult<Option<Vec<f32>>> {
        let key = Self::make_key(text, model);

        // 1) 핫셋.
        {
            let mut hot = self.hot.lock().expect("embedding cache hot poisoned");
            if let Some(v) = hot.get(&key) {
                self.hit_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(Some(v));
            }
        }

        // 2) SQLite.
        let row: Option<Vec<u8>> = conn
            .query_row(
                "SELECT embedding FROM embedding_cache WHERE text_hash = ?1 AND model = ?2",
                params![key, model],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .ok();

        match row {
            Some(bytes) => {
                let vec = f32_from_le_bytes(&bytes).map_err(|e| AppError::Internal {
                    message: format!("embedding_cache BLOB 파싱 실패: {e}"),
                })?;
                conn.execute(
                    "UPDATE embedding_cache SET last_hit_at = \
                        CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
                     WHERE text_hash = ?1",
                    params![key],
                )?;
                {
                    let mut hot = self.hot.lock().expect("embedding cache hot poisoned");
                    hot.put(key, vec.clone());
                }
                self.hit_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Some(vec))
            }
            None => {
                self.miss_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(None)
            }
        }
    }

    /// 배치 lookup — `(text, model)` 시퀀스에 대해 hit 결과만 채우고 miss는 None.
    /// 출력 순서 = 입력 순서. SQLite IN(...) 1회로 라운드트립 절감.
    pub fn get_batch(
        &self,
        conn: &Connection,
        items: &[(String, String)], // (text, model)
    ) -> AppResult<Vec<Option<Vec<f32>>>> {
        let mut results: Vec<Option<Vec<f32>>> = vec![None; items.len()];
        if items.is_empty() {
            return Ok(results);
        }

        // 1) 핫셋 1차 (입력 순서대로) — 핫셋 hit은 그 자리에 채우고, miss만 SQLite로.
        let mut sqlite_indices: Vec<usize> = Vec::with_capacity(items.len());
        let mut sqlite_keys: Vec<String> = Vec::with_capacity(items.len());
        {
            let mut hot = self.hot.lock().expect("embedding cache hot poisoned");
            for (i, (text, model)) in items.iter().enumerate() {
                let key = Self::make_key(text, model);
                if let Some(v) = hot.get(&key) {
                    results[i] = Some(v);
                    self.hit_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    sqlite_indices.push(i);
                    sqlite_keys.push(key);
                }
            }
        }

        if sqlite_keys.is_empty() {
            return Ok(results);
        }

        // 2) SQLite IN(...) — `?1, ?2, ...` 동적 placeholder.
        let placeholders: Vec<String> = (1..=sqlite_keys.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT text_hash, embedding FROM embedding_cache WHERE text_hash IN ({})",
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = sqlite_keys
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
        let mut by_hash: std::collections::HashMap<String, Vec<f32>> =
            std::collections::HashMap::new();
        while let Some(row) = rows.next()? {
            let hash: String = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            let vec = f32_from_le_bytes(&bytes).map_err(|e| AppError::Internal {
                message: format!("embedding_cache BLOB 파싱 실패: {e}"),
            })?;
            by_hash.insert(hash, vec);
        }
        drop(rows);
        drop(stmt);

        // 3) 결과 채움 + last_hit_at 갱신 + 핫셋 등재.
        let mut hits_to_touch: Vec<String> = Vec::new();
        {
            let mut hot = self.hot.lock().expect("embedding cache hot poisoned");
            for (idx_in_sqlite, original_idx) in sqlite_indices.iter().enumerate() {
                let key = &sqlite_keys[idx_in_sqlite];
                if let Some(vec) = by_hash.remove(key) {
                    hot.put(key.clone(), vec.clone());
                    results[*original_idx] = Some(vec);
                    hits_to_touch.push(key.clone());
                    self.hit_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    self.miss_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        // last_hit_at 일괄 갱신.
        if !hits_to_touch.is_empty() {
            let placeholders: Vec<String> =
                (1..=hits_to_touch.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "UPDATE embedding_cache SET \
                    last_hit_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
                 WHERE text_hash IN ({})",
                placeholders.join(",")
            );
            let params: Vec<&dyn rusqlite::ToSql> = hits_to_touch
                .iter()
                .map(|s| s as &dyn rusqlite::ToSql)
                .collect();
            conn.execute(&sql, rusqlite::params_from_iter(params))?;
        }

        Ok(results)
    }

    /// 영속 + 핫셋 put. INSERT OR REPLACE.
    pub fn put(
        &self,
        conn: &Connection,
        text: &str,
        model: &str,
        dim: usize,
        embedding: &[f32],
    ) -> AppResult<()> {
        if embedding.len() != dim {
            return Err(AppError::Internal {
                message: format!(
                    "embedding cache put: dim mismatch (선언 {} ≠ 실제 {})",
                    dim,
                    embedding.len()
                ),
            });
        }
        let key = Self::make_key(text, model);
        let bytes = f32_to_le_bytes(embedding);
        conn.execute(
            "INSERT INTO embedding_cache \
                (text_hash, embedding, model, dim, created_at, last_hit_at) \
             VALUES (?1, ?2, ?3, ?4, \
                     CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
                     CAST(strftime('%s', 'now') AS INTEGER) * 1000) \
             ON CONFLICT(text_hash) DO UPDATE SET \
                 embedding = excluded.embedding, \
                 model = excluded.model, \
                 dim = excluded.dim, \
                 last_hit_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000",
            params![key, bytes, model, dim as i64],
        )?;
        {
            let mut hot = self.hot.lock().expect("embedding cache hot poisoned");
            hot.put(key, embedding.to_vec());
        }
        Ok(())
    }

    /// LRU eviction — 행 수 max_rows 초과 시 last_hit_at ASC LIMIT 차이만큼 DELETE.
    /// 반환 = 실제 삭제된 row 수.
    pub fn evict_lru(&self, conn: &Connection, max_rows: usize) -> AppResult<usize> {
        let total: i64 =
            conn.query_row("SELECT COUNT(*) FROM embedding_cache", [], |r| r.get(0))?;
        if (total as usize) <= max_rows {
            return Ok(0);
        }
        let to_delete = (total as usize) - max_rows;
        let removed = conn.execute(
            "DELETE FROM embedding_cache WHERE text_hash IN ( \
                 SELECT text_hash FROM embedding_cache \
                  ORDER BY COALESCE(last_hit_at, 0) ASC \
                  LIMIT ?1)",
            params![to_delete as i64],
        )?;
        Ok(removed)
    }

    /// 통계 — rows + 누적 hit/miss.
    pub fn stats(&self, conn: &Connection) -> AppResult<CacheStats> {
        let rows: i64 =
            conn.query_row("SELECT COUNT(*) FROM embedding_cache", [], |r| r.get(0))?;
        Ok(CacheStats {
            rows,
            hit_count: self.hit_count.load(std::sync::atomic::Ordering::Relaxed),
            miss_count: self.miss_count.load(std::sync::atomic::Ordering::Relaxed),
        })
    }

    /// 인메모리 핫셋만 비움 — 디버그/테스트.
    pub fn clear_hot(&self) {
        let mut hot = self.hot.lock().expect("embedding cache hot poisoned");
        hot.clear();
    }

    /// 누적 hit/miss 0으로 리셋.
    pub fn reset_counters(&self) {
        self.hit_count
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.miss_count
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE embedding_cache ( \
                text_hash    TEXT    PRIMARY KEY, \
                embedding    BLOB    NOT NULL, \
                model        TEXT    NOT NULL, \
                dim          INTEGER NOT NULL, \
                created_at   INTEGER NOT NULL, \
                last_hit_at  INTEGER \
             ); \
             CREATE INDEX idx_embedding_cache_model    ON embedding_cache(model); \
             CREATE INDEX idx_embedding_cache_last_hit ON embedding_cache(last_hit_at);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn put_then_get_returns_same_vector() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        let v = vec![1.0_f32, 2.0, 3.0, 4.0];
        cache.put(&conn, "hello", "me5-small", 4, &v).unwrap();
        let got = cache.get(&conn, "hello", "me5-small").unwrap();
        assert_eq!(got, Some(v));
    }

    #[test]
    fn different_model_creates_separate_row() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        let v1 = vec![1.0_f32; 4];
        let v2 = vec![2.0_f32; 4];
        cache.put(&conn, "hello", "me5-small", 4, &v1).unwrap();
        cache.put(&conn, "hello", "bge-m3", 4, &v2).unwrap();

        let got1 = cache.get(&conn, "hello", "me5-small").unwrap();
        let got2 = cache.get(&conn, "hello", "bge-m3").unwrap();
        assert_eq!(got1, Some(v1));
        assert_eq!(got2, Some(v2));

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn miss_returns_none_and_increments_counter() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        assert!(cache.get(&conn, "absent", "me5-small").unwrap().is_none());
        let stats = cache.stats(&conn).unwrap();
        assert_eq!(stats.hit_count, 0);
        assert_eq!(stats.miss_count, 1);
    }

    #[test]
    fn evict_lru_removes_oldest_rows_above_threshold() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        for i in 0..5 {
            let v = vec![i as f32; 2];
            cache.put(&conn, &format!("text-{i}"), "m", 2, &v).unwrap();
            // last_hit_at 단조 증가 보장 — i=0이 가장 오래된.
            conn.execute(
                "UPDATE embedding_cache SET last_hit_at = ?1 WHERE text_hash = ?2",
                params![
                    (i + 1) as i64,
                    EmbeddingCache::make_key(&format!("text-{i}"), "m")
                ],
            )
            .unwrap();
        }
        let removed = cache.evict_lru(&conn, 3).unwrap();
        assert_eq!(removed, 2);

        for i in 0..5 {
            let key = EmbeddingCache::make_key(&format!("text-{i}"), "m");
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM embedding_cache WHERE text_hash = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .unwrap();
            if i < 2 {
                assert_eq!(exists, 0, "오래된 i={i} 는 삭제되어야");
            } else {
                assert_eq!(exists, 1, "최신 i={i} 는 살아있어야");
            }
        }
    }

    #[test]
    fn evict_lru_no_op_when_below_threshold() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        cache.put(&conn, "a", "m", 2, &[1.0, 2.0]).unwrap();
        cache.put(&conn, "b", "m", 2, &[3.0, 4.0]).unwrap();
        let removed = cache.evict_lru(&conn, 10).unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn hot_lru_avoids_sqlite_on_repeat_get() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        cache.put(&conn, "hot", "m", 2, &[7.0, 8.0]).unwrap();
        // 첫 get — 핫셋(put 시 등재) hit.
        let _ = cache.get(&conn, "hot", "m").unwrap();
        // SQLite 데이터 *지움* — 핫셋만으로 hit 확인.
        conn.execute("DELETE FROM embedding_cache", []).unwrap();
        let got = cache.get(&conn, "hot", "m").unwrap();
        assert_eq!(got, Some(vec![7.0, 8.0]));
    }

    #[test]
    fn put_with_wrong_dim_errors() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        let r = cache.put(&conn, "x", "m", 5, &[1.0, 2.0, 3.0]);
        assert!(r.is_err(), "dim 5 선언과 실제 3 mismatch는 에러");
    }

    #[test]
    fn stats_tracks_hit_and_miss() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        cache.put(&conn, "a", "m", 2, &[1.0, 2.0]).unwrap();
        let _ = cache.get(&conn, "a", "m").unwrap(); // hit (핫셋).
        let _ = cache.get(&conn, "absent", "m").unwrap();
        let s = cache.stats(&conn).unwrap();
        assert_eq!(s.hit_count, 1);
        assert_eq!(s.miss_count, 1);
        assert_eq!(s.rows, 1);
        assert!((s.hit_ratio() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn put_replace_updates_embedding() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        cache.put(&conn, "k", "m", 2, &[1.0, 2.0]).unwrap();
        cache.put(&conn, "k", "m", 2, &[9.0, 8.0]).unwrap(); // REPLACE.
        let got = cache.get(&conn, "k", "m").unwrap();
        assert_eq!(got, Some(vec![9.0, 8.0]));
    }

    #[test]
    fn get_batch_returns_aligned_results() {
        let conn = fresh_db();
        let cache = EmbeddingCache::new();
        cache.put(&conn, "a", "m", 2, &[1.0, 1.0]).unwrap();
        cache.put(&conn, "c", "m", 2, &[3.0, 3.0]).unwrap();
        // 핫셋 비움 → SQLite IN 경로 검증.
        cache.clear_hot();

        let items = vec![
            ("a".to_string(), "m".to_string()),
            ("b".to_string(), "m".to_string()),
            ("c".to_string(), "m".to_string()),
        ];
        let r = cache.get_batch(&conn, &items).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], Some(vec![1.0, 1.0]));
        assert!(r[1].is_none());
        assert_eq!(r[2], Some(vec![3.0, 3.0]));

        let s = cache.stats(&conn).unwrap();
        assert_eq!(s.hit_count, 2);
        assert_eq!(s.miss_count, 1);
    }
}
