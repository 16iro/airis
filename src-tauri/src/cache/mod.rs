// v0.4.2 PR 4 — Response cache + Embedding cache + prefix hooks (D-084).
//
// 목적 (HANDOFF §1.1·§1.2):
//   * embedding_cache·response_cache 둘 다 SQLite 테이블 (chunks.db 안, v15에서 정의).
//   * LRU eviction `MAX_ROWS=10_000` per cache. 메모리 only는 핫셋 1024 LRU만.
//   * Embedding cache key = sha256(text + ':' + model) — text와 model이 합쳐진
//     해시(같은 텍스트 다른 모델 row 공존). v15 마이그 주석이 명시한 컨벤션.
//   * Response cache key = sha256(book_id + rewritten_query +
//                                  sorted(retrieved_chunk_ids) + active_model).
//   * TTL: embedding cache 영구. response cache 7일 default.
//   * Invalidation: chunks INSERT/UPDATE/DELETE 시 invalidate_book(book_id) 명시 호출.
//
// 본 모듈은 *backend 영속/조회*만. UI 표시·prefix hint 같은 외부 wiring은 호출 측이.

#![allow(dead_code)]

pub mod embedding;
pub mod response;

use serde::Serialize;

/// 캐시 통계 — dev panel · cache hit ratio 가시화 용.
///
/// `rows`는 SQLite row 수, `hit_count`/`miss_count`는 *프로세스 lifetime* 누적.
/// 영속하지 않는다 — 앱 재시작 시 hit/miss 카운터는 0으로 시작.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct CacheStats {
    pub rows: i64,
    pub hit_count: u64,
    pub miss_count: u64,
}

impl CacheStats {
    /// 0~1 사이 hit ratio. total=0이면 0.0.
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hit_count + self.miss_count;
        if total == 0 {
            0.0
        } else {
            self.hit_count as f64 / total as f64
        }
    }
}

/// 인메모리 LRU 핫셋 — `lru` 크레이트 의존 회피, 자체 구현(HashMap + VecDeque).
///
/// 작은 사이즈(=1024) 가정이라 O(N) 탐색이 허용 — 핵심 hot path는 SQLite를 회피하는
/// 네트워크/파일 I/O 한 번 절감이지, 인메모리 자료구조의 미시 perf가 아님.
///
/// thread-safe하지 않음 — 호출 측이 `Mutex<HotLru<K, V>>`로 감싼다.
#[derive(Debug)]
pub struct HotLru<K, V>
where
    K: Eq + std::hash::Hash + Clone,
    V: Clone,
{
    cap: usize,
    map: std::collections::HashMap<K, V>,
    /// LRU 순서 — front=most recent, back=least recent (V0.4.2 규약).
    order: std::collections::VecDeque<K>,
}

impl<K, V> HotLru<K, V>
where
    K: Eq + std::hash::Hash + Clone,
    V: Clone,
{
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            map: std::collections::HashMap::with_capacity(cap.max(1)),
            order: std::collections::VecDeque::with_capacity(cap.max(1)),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 최근 사용으로 끌어올리며 lookup. 없으면 None.
    pub fn get(&mut self, k: &K) -> Option<V> {
        let v = self.map.get(k).cloned()?;
        self.touch(k);
        Some(v)
    }

    /// 삽입. 용량 초과 시 가장 오래된 1개 제거. 같은 key는 값 갱신 + most recent.
    pub fn put(&mut self, k: K, v: V) {
        if self.map.contains_key(&k) {
            self.map.insert(k.clone(), v);
            self.touch(&k);
            return;
        }
        if self.map.len() >= self.cap {
            // back = 가장 오래된 — 제거.
            if let Some(oldest) = self.order.pop_back() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_front(k.clone());
        self.map.insert(k, v);
    }

    pub fn remove(&mut self, k: &K) -> Option<V> {
        let v = self.map.remove(k)?;
        // O(N) 탐색이지만 cap=1024 가정으로 허용.
        if let Some(pos) = self.order.iter().position(|x| x == k) {
            self.order.remove(pos);
        }
        Some(v)
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    fn touch(&mut self, k: &K) {
        if let Some(pos) = self.order.iter().position(|x| x == k) {
            // VecDeque::remove는 O(N)이지만 cap=1024 가정.
            self.order.remove(pos);
        }
        self.order.push_front(k.clone());
    }
}

/// SHA-256 hex digest — embedding/response cache 키 정규화.
///
/// 16진 소문자 64자리. SQLite TEXT 컬럼 PK에 적합.
pub fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// f32 little-endian 바이트로 직렬화 (SQLite BLOB 컬럼).
/// vec0의 BLOB 컨벤션과 일치 — `f32_bytes`(v041 vector_store)와 동일 invariant.
pub fn f32_to_le_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// f32 little-endian 바이트 → Vec<f32>. 차원이 4의 배수가 아니면 에러.
pub fn f32_from_le_bytes(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "임베딩 BLOB 길이 {} 가 4의 배수가 아닙니다",
            bytes.len()
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().map_err(|_| "내부 오류".to_string())?;
        out.push(f32::from_le_bytes(arr));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_is_64_lowercase() {
        let s = sha256_hex("hello");
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn sha256_hex_known_vector() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn f32_round_trip_le() {
        let v = vec![0.0_f32, 1.0, -1.5, 2.5];
        let bytes = f32_to_le_bytes(&v);
        let back = f32_from_le_bytes(&bytes).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn f32_from_le_rejects_non_multiple_of_4() {
        assert!(f32_from_le_bytes(&[0u8, 1, 2]).is_err());
    }

    #[test]
    fn hot_lru_evicts_oldest_when_full() {
        let mut lru: HotLru<String, i32> = HotLru::new(2);
        lru.put("a".into(), 1);
        lru.put("b".into(), 2);
        lru.put("c".into(), 3); // 'a' eviction.
        assert!(lru.get(&"a".into()).is_none());
        assert_eq!(lru.get(&"b".into()), Some(2));
        assert_eq!(lru.get(&"c".into()), Some(3));
    }

    #[test]
    fn hot_lru_get_promotes_to_recent() {
        let mut lru: HotLru<String, i32> = HotLru::new(2);
        lru.put("a".into(), 1);
        lru.put("b".into(), 2);
        // 'a' touch — 가장 최신.
        let _ = lru.get(&"a".into());
        // 'c' 추가 → 'b' eviction.
        lru.put("c".into(), 3);
        assert_eq!(lru.get(&"a".into()), Some(1));
        assert!(lru.get(&"b".into()).is_none());
        assert_eq!(lru.get(&"c".into()), Some(3));
    }

    #[test]
    fn hot_lru_put_existing_updates_and_promotes() {
        let mut lru: HotLru<String, i32> = HotLru::new(2);
        lru.put("a".into(), 1);
        lru.put("b".into(), 2);
        lru.put("a".into(), 11); // 갱신 + 최신.
        lru.put("c".into(), 3); // 'b' eviction.
        assert!(lru.get(&"b".into()).is_none());
        assert_eq!(lru.get(&"a".into()), Some(11));
        assert_eq!(lru.get(&"c".into()), Some(3));
    }

    #[test]
    fn hot_lru_remove_works() {
        let mut lru: HotLru<String, i32> = HotLru::new(2);
        lru.put("a".into(), 1);
        assert_eq!(lru.remove(&"a".into()), Some(1));
        assert!(lru.get(&"a".into()).is_none());
    }

    #[test]
    fn cache_stats_hit_ratio() {
        let s = CacheStats {
            rows: 10,
            hit_count: 7,
            miss_count: 3,
        };
        assert!((s.hit_ratio() - 0.7).abs() < 1e-9);

        let zero = CacheStats::default();
        assert!((zero.hit_ratio() - 0.0).abs() < 1e-9);
    }
}
