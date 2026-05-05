// v0.4.1 임베더 — fastembed-rs 5.x + multilingual-e5-small INT8 (384d).
//
// 결정 근거 (D-073, D-077):
//   * mE5-small INT8 = 5분 약속(300페이지급 ≈ 1500 청크) 베이스라인 (PoC 18.1 청크/s 측정).
//   * 모델 cache = `with_cache_dir(<Tauri appdata>/models/)` *강제* — 사용자 HF 캐시 오염 방지.
//   * mE5 prefix = passage(corpus): "passage: ", query: "query: ". fastembed가 자동으로
//     붙여주지 않는다 → 호출 측에서 강제. PoC `lib.rs::passage_prefix` 패턴 그대로.
//
// PR 1 범위:
//   * 공개 API stub만 (시그니처 + cache_dir resolver + prefix helper)
//   * 실제 fastembed 모델 로드 / `embed()` 호출 로직은 PR 2 또는 PR 3에서.
//   * 단위 테스트 = prefix helper 검증만 (모델 로드 X — PoC가 이미 검증, CI에서 모델
//     다운로드 비용 회피).
//
// 동시성 메모 (D-076 직렬 큐):
//   * fastembed-rs 5.x의 `embed()`는 `&mut self`. Mutex 또는 직렬 큐 필요.
//   * PR 2/3에서 `Mutex<TextEmbedding>` 또는 jobs 큐로 직렬화.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// 모델 cache 디렉토리 결정. `with_cache_dir(<app_data>/models/)`.
///
/// 호출 측에서 Tauri `app_data_dir`를 받아 넘긴다. 디렉토리가 없으면 *생성*만 하고
/// (이 함수에서) 권한·디스크 부족은 fastembed init에서 자연스레 노출된다.
pub fn model_cache_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("models")
}

/// passage(corpus) 측 prefix — 인덱싱(청크 → 임베딩)에 사용.
/// mE5 / E5 계열은 prefix가 *학습 시점*에 붙어 있어 inference에도 강제.
pub fn passage_prefix(chunk: &str) -> String {
    format!("passage: {chunk}")
}

/// query 측 prefix — 검색(사용자 질의 → 임베딩)에 사용.
pub fn query_prefix(q: &str) -> String {
    format!("query: {q}")
}

/// 임베더 핸들 stub. PR 2/3에서 fastembed `TextEmbedding` 인스턴스를 들고 있게 된다.
///
/// 시그니처만 미리 잡아 두는 이유: 호출 측(commands/index, retrieval) 모듈이
/// 컴파일 가능하려면 타입 이름이 필요. 실제 필드·메서드 본문은 PR 2/3.
pub struct Embedder {
    _cache_dir: PathBuf,
}

impl Embedder {
    /// 새 임베더 인스턴스. PR 1 stub — 호출되면 컴파일은 되지만 실제 사용은 PR 2/3.
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            _cache_dir: model_cache_dir(app_data_dir),
        }
    }

    /// 임베딩 차원 — mE5-small = 384. 차원 strict 검증(vec0 매치)에 사용.
    pub const DIM: usize = 384;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn passage_prefix_format() {
        assert_eq!(passage_prefix("hello"), "passage: hello");
        assert_eq!(passage_prefix("한국어 청크"), "passage: 한국어 청크");
    }

    #[test]
    fn query_prefix_format() {
        assert_eq!(query_prefix("Rust ownership"), "query: Rust ownership");
        assert_eq!(query_prefix("소유권 모델"), "query: 소유권 모델");
    }

    #[test]
    fn cache_dir_under_app_data() {
        let app_data = PathBuf::from("/tmp/airis-test-data");
        let cache = model_cache_dir(&app_data);
        assert!(cache.starts_with(&app_data));
        assert_eq!(cache.file_name().and_then(|s| s.to_str()), Some("models"));
    }

    #[test]
    fn dim_constant_is_384() {
        // mE5-small 차원 — vec0 가상 테이블 차원과 일관성 검증의 ground truth.
        assert_eq!(Embedder::DIM, 384);
    }
}
