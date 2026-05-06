// 응답 영속 cache (D-084) — sha256(book_id + rewritten_query +
//                                  sorted(retrieved_chunk_ids) + active_model) → response_text.
//
// invariant (architecture §4.11.1):
//   * key는 4개 입력의 안정적 직렬화의 sha256.
//   * sorted(retrieved_chunk_ids) — 정렬은 i64 오름차순. 같은 청크 셋에 대해 결정적.
//   * active_model — 모델 변경 시 자동 stale (다른 키).
//   * notebook_id 컬럼은 invalidate_book(book_id)의 lookup 용 (영속 row 단위).
//
// TTL:
//   * default 7일 — created_at + 7d < now() 이면 stale → miss로 처리 (코드 측 검사).
//
// Invalidation:
//   * `invalidate_book(book_id)` — 해당 notebook_id row 모두 DELETE (chunks INSERT/UPDATE/DELETE
//     트리거 시점에 호출 측이 명시 호출). architecture §4.11.1 + HANDOFF §1.3.
//
// 인메모리 핫셋 (HotLru<String, String>) cap=1024.
//
// thread-safety:
//   * Connection은 호출 측이 매 메서드 진입에 인자로 전달.
//   * HotLru와 hit/miss 카운터는 self의 Mutex/Atomic.

#![allow(dead_code)]

use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::cache::{sha256_hex, CacheStats, HotLru};
use crate::error::AppResult;

/// 인메모리 핫셋 cap — HANDOFF §1.2.
pub const HOT_CAP: usize = 1024;

/// 영속 LRU 임계 — HANDOFF §1.1.
pub const MAX_ROWS_DEFAULT: usize = 10_000;

/// TTL default — 7일.
pub const DEFAULT_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// 응답 cache. SQLite 영속 + 인메모리 핫셋 + TTL stale check.
pub struct ResponseCache {
    hot: Mutex<HotLru<String, String>>,
    ttl_ms: i64,
    hit_count: std::sync::atomic::AtomicU64,
    miss_count: std::sync::atomic::AtomicU64,
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 응답 cache 키 생성 — `(book_id, rewritten_query, sorted(chunk_ids), active_model)`.
///
/// 직렬화 컨벤션:
///   - 구분자 `\x1f` (Unit Separator).
///   - chunk_ids: 오름차순 정렬 후 `,` join.
pub fn make_response_cache_key(
    book_id: &str,
    rewritten_query: &str,
    retrieved_chunk_ids: &[i64],
    active_model: &str,
) -> String {
    let mut sorted = retrieved_chunk_ids.to_vec();
    sorted.sort_unstable();
    let ids_csv: String = sorted
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let combined = format!("{book_id}\x1f{rewritten_query}\x1f{ids_csv}\x1f{active_model}");
    sha256_hex(&combined)
}

impl ResponseCache {
    pub fn new() -> Self {
        Self::with_capacity(HOT_CAP, DEFAULT_TTL_MS)
    }

    pub fn with_capacity(hot_cap: usize, ttl_ms: i64) -> Self {
        Self {
            hot: Mutex::new(HotLru::new(hot_cap)),
            ttl_ms,
            hit_count: std::sync::atomic::AtomicU64::new(0),
            miss_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn ttl_ms(&self) -> i64 {
        self.ttl_ms
    }

    /// 명시 키 lookup.
    pub fn get_by_key(&self, conn: &Connection, key: &str) -> AppResult<Option<String>> {
        // 1) 핫셋.
        {
            let mut hot = self.hot.lock().expect("response cache hot poisoned");
            if let Some(v) = hot.get(&key.to_string()) {
                self.hit_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(Some(v));
            }
        }

        // 2) SQLite — TTL stale check.
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT response_text, created_at FROM response_cache WHERE key = ?1",
                params![key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .ok();

        match row {
            Some((text, created_at)) => {
                let now = now_ms();
                if now - created_at > self.ttl_ms {
                    self.miss_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Ok(None);
                }
                conn.execute(
                    "UPDATE response_cache SET \
                        last_hit_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
                        hit_count = hit_count + 1 \
                     WHERE key = ?1",
                    params![key],
                )?;
                {
                    let mut hot = self.hot.lock().expect("response cache hot poisoned");
                    hot.put(key.to_string(), text.clone());
                }
                self.hit_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Some(text))
            }
            None => {
                self.miss_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(None)
            }
        }
    }

    /// 입력 컴포넌트로 lookup.
    pub fn get(
        &self,
        conn: &Connection,
        book_id: &str,
        rewritten_query: &str,
        retrieved_chunk_ids: &[i64],
        active_model: &str,
    ) -> AppResult<Option<String>> {
        let key = make_response_cache_key(
            book_id,
            rewritten_query,
            retrieved_chunk_ids,
            active_model,
        );
        self.get_by_key(conn, &key)
    }

    /// 입력 컴포넌트 + 응답 텍스트로 INSERT OR REPLACE. 반환 = 사용된 키.
    pub fn put(
        &self,
        conn: &Connection,
        book_id: &str,
        rewritten_query: &str,
        retrieved_chunk_ids: &[i64],
        active_model: &str,
        response_text: &str,
    ) -> AppResult<String> {
        let key = make_response_cache_key(
            book_id,
            rewritten_query,
            retrieved_chunk_ids,
            active_model,
        );
        self.put_by_key(conn, &key, book_id, active_model, response_text)?;
        Ok(key)
    }

    /// 명시 키 PUT. created_at은 *지금* 으로 갱신.
    pub fn put_by_key(
        &self,
        conn: &Connection,
        key: &str,
        book_id: &str,
        active_model: &str,
        response_text: &str,
    ) -> AppResult<()> {
        conn.execute(
            "INSERT INTO response_cache \
                (key, notebook_id, response_text, model, created_at, last_hit_at, hit_count) \
             VALUES (?1, ?2, ?3, ?4, \
                     CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
                     CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
                     0) \
             ON CONFLICT(key) DO UPDATE SET \
                 response_text = excluded.response_text, \
                 model = excluded.model, \
                 created_at = excluded.created_at, \
                 last_hit_at = excluded.last_hit_at, \
                 hit_count = 0",
            params![key, book_id, response_text, active_model],
        )?;
        {
            let mut hot = self.hot.lock().expect("response cache hot poisoned");
            hot.put(key.to_string(), response_text.to_string());
        }
        Ok(())
    }

    /// 책 단위 invalidate — 해당 notebook_id row 모두 DELETE.
    /// 핫셋은 안전하게 *전체 clear* (key→book_id 역색인을 들고 있지 않음).
    /// 반환 = 삭제된 SQLite row 수.
    pub fn invalidate_book(&self, conn: &Connection, book_id: &str) -> AppResult<usize> {
        let removed = conn.execute(
            "DELETE FROM response_cache WHERE notebook_id = ?1",
            params![book_id],
        )?;
        if removed > 0 {
            let mut hot = self.hot.lock().expect("response cache hot poisoned");
            hot.clear();
        }
        Ok(removed)
    }

    /// LRU eviction.
    pub fn evict_lru(&self, conn: &Connection, max_rows: usize) -> AppResult<usize> {
        let total: i64 =
            conn.query_row("SELECT COUNT(*) FROM response_cache", [], |r| r.get(0))?;
        if (total as usize) <= max_rows {
            return Ok(0);
        }
        let to_delete = (total as usize) - max_rows;
        let removed = conn.execute(
            "DELETE FROM response_cache WHERE key IN ( \
                 SELECT key FROM response_cache \
                  ORDER BY COALESCE(last_hit_at, 0) ASC \
                  LIMIT ?1)",
            params![to_delete as i64],
        )?;
        Ok(removed)
    }

    /// 통계.
    pub fn stats(&self, conn: &Connection) -> AppResult<CacheStats> {
        let rows: i64 =
            conn.query_row("SELECT COUNT(*) FROM response_cache", [], |r| r.get(0))?;
        Ok(CacheStats {
            rows,
            hit_count: self.hit_count.load(std::sync::atomic::Ordering::Relaxed),
            miss_count: self.miss_count.load(std::sync::atomic::Ordering::Relaxed),
        })
    }

    pub fn clear_hot(&self) {
        let mut hot = self.hot.lock().expect("response cache hot poisoned");
        hot.clear();
    }

    pub fn reset_counters(&self) {
        self.hit_count
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.miss_count
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE response_cache ( \
                key            TEXT    PRIMARY KEY, \
                notebook_id    TEXT    NOT NULL, \
                response_text  TEXT    NOT NULL, \
                model          TEXT    NOT NULL, \
                created_at     INTEGER NOT NULL, \
                last_hit_at    INTEGER, \
                hit_count      INTEGER NOT NULL DEFAULT 0 \
             ); \
             CREATE INDEX idx_response_cache_book     ON response_cache(notebook_id); \
             CREATE INDEX idx_response_cache_last_hit ON response_cache(last_hit_at);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn put_then_get_returns_same_response() {
        let conn = fresh_db();
        let cache = ResponseCache::new();
        let _key = cache
            .put(&conn, "b1", "Rust ownership", &[10, 20], "claude-opus-4-7", "ANSWER")
            .unwrap();
        let got = cache
            .get(&conn, "b1", "Rust ownership", &[10, 20], "claude-opus-4-7")
            .unwrap();
        assert_eq!(got.as_deref(), Some("ANSWER"));
    }

    #[test]
    fn key_is_chunk_id_order_independent() {
        let k1 = make_response_cache_key("b", "q", &[3, 1, 2], "m");
        let k2 = make_response_cache_key("b", "q", &[1, 2, 3], "m");
        assert_eq!(k1, k2);
    }

    #[test]
    fn different_model_produces_different_key() {
        let k1 = make_response_cache_key("b", "q", &[1], "m1");
        let k2 = make_response_cache_key("b", "q", &[1], "m2");
        assert_ne!(k1, k2);
    }

    #[test]
    fn different_chunk_ids_produce_different_key() {
        let k1 = make_response_cache_key("b", "q", &[1], "m");
        let k2 = make_response_cache_key("b", "q", &[2], "m");
        assert_ne!(k1, k2);
    }

    #[test]
    fn invalidate_book_removes_all_book_rows() {
        let conn = fresh_db();
        let cache = ResponseCache::new();
        cache.put(&conn, "b1", "q1", &[1], "m", "A1").unwrap();
        cache.put(&conn, "b1", "q2", &[2], "m", "A2").unwrap();
        cache.put(&conn, "b2", "q3", &[3], "m", "A3").unwrap();

        let removed = cache.invalidate_book(&conn, "b1").unwrap();
        assert_eq!(removed, 2);

        let b1_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM response_cache WHERE notebook_id = 'b1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let b2_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM response_cache WHERE notebook_id = 'b2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(b1_rows, 0);
        assert_eq!(b2_rows, 1);

        let miss = cache.get(&conn, "b1", "q1", &[1], "m").unwrap();
        assert!(miss.is_none());
    }

    #[test]
    fn ttl_stale_returns_miss() {
        let conn = fresh_db();
        let cache = ResponseCache::with_capacity(HOT_CAP, 1000); // ttl 1초.
        let key = cache.put(&conn, "b", "q", &[1], "m", "A").unwrap();
        let eight_days_ago = now_ms() - 8 * 24 * 60 * 60 * 1000;
        conn.execute(
            "UPDATE response_cache SET created_at = ?1 WHERE key = ?2",
            params![eight_days_ago, key],
        )
        .unwrap();
        cache.clear_hot();

        let got = cache.get(&conn, "b", "q", &[1], "m").unwrap();
        assert!(got.is_none(), "TTL 초과 row는 miss로 처리");
    }

    #[test]
    fn evict_lru_trims_to_max_rows() {
        let conn = fresh_db();
        let cache = ResponseCache::new();
        for i in 0..5 {
            cache
                .put(&conn, "b", &format!("q{i}"), &[i], "m", &format!("A{i}"))
                .unwrap();
            conn.execute(
                "UPDATE response_cache SET last_hit_at = ?1 WHERE key = ( \
                     SELECT key FROM response_cache ORDER BY created_at DESC LIMIT 1)",
                params![i + 1],
            )
            .unwrap();
        }
        let removed = cache.evict_lru(&conn, 3).unwrap();
        assert_eq!(removed, 2);

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM response_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 3);
    }

    #[test]
    fn stats_tracks_counters() {
        let conn = fresh_db();
        let cache = ResponseCache::new();
        cache.put(&conn, "b", "q", &[1], "m", "A").unwrap();
        let _ = cache.get(&conn, "b", "q", &[1], "m").unwrap(); // hit (핫셋).
        let _ = cache.get(&conn, "b", "missing", &[2], "m").unwrap(); // miss.
        let s = cache.stats(&conn).unwrap();
        assert_eq!(s.hit_count, 1);
        assert_eq!(s.miss_count, 1);
        assert_eq!(s.rows, 1);
    }

    #[test]
    fn put_replaces_existing_response_text() {
        let conn = fresh_db();
        let cache = ResponseCache::new();
        let _ = cache.put(&conn, "b", "q", &[1], "m", "first").unwrap();
        let _ = cache.put(&conn, "b", "q", &[1], "m", "second").unwrap();
        let got = cache.get(&conn, "b", "q", &[1], "m").unwrap();
        assert_eq!(got.as_deref(), Some("second"));
    }

    #[test]
    fn hot_lru_avoids_sqlite_on_repeat() {
        let conn = fresh_db();
        let cache = ResponseCache::new();
        let _ = cache.put(&conn, "b", "q", &[1], "m", "ANS").unwrap();
        let _ = cache.get(&conn, "b", "q", &[1], "m").unwrap();
        conn.execute("DELETE FROM response_cache", []).unwrap();
        let got = cache.get(&conn, "b", "q", &[1], "m").unwrap();
        assert_eq!(got.as_deref(), Some("ANS"));
    }
}
