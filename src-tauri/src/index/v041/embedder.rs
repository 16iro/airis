// v0.4.1 임베더 — fastembed-rs 5.x + multilingual-e5-small INT8 (384d).
//
// 결정 근거 (D-073, D-075, D-076, D-077):
//   * mE5-small INT8 = 5분 약속(300페이지급 ≈ 1500 청크) 베이스라인 (PoC 18.1 청크/s 측정).
//   * 모델 cache = `with_cache_dir(<Tauri appdata>/models/)` *강제* — 사용자 HF 캐시 오염 방지.
//   * mE5 prefix = passage(corpus): "passage: ", query: "query: ". fastembed가 자동으로
//     붙여주지 않는다 → 호출 측에서 강제. PoC `lib.rs::passage_prefix` 패턴 그대로.
//   * 동시성 = `Mutex<TextEmbedding>` (D-076 직렬 큐 보강). fastembed 5.x `embed()`는
//     `&mut self`라 동시 호출 불가. 인덱서·검색 모두 같은 mutex 공유.
//
// PR 2 범위:
//   * `Embedder` 구조체에 `Mutex<TextEmbedding>` 필드 추가 (lazy init = `OnceLock` /
//     `OnceCell` 대신 *명시 init* 사용 — 다운로드 실패를 호출 시점에 노출).
//   * `embed_passages` / `embed_query` 시그니처 + 단위 테스트(prefix · DIM 검증).
//   * 실제 인덱싱 호출(=`indexer.rs`의 임베딩 자리)은 PR 3에서 fastembed init과
//     함께 채움. 본 PR의 메서드 본문은 *fastembed 호출까지* 작성하되 통합·smoke 테스트는
//     PR 3에서. CI에서 모델 다운로드 비용을 본 PR 단위 테스트가 발생시키지 않도록
//     테스트 게이팅(env `AIRIS_E2E_EMBED=1`).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::error::{AppError, AppResult};

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

/// 임베딩 배치 크기 — fastembed `embed(_, Some(BATCH))` 인자.
///
/// PoC d1_embed_throughput.rs에서 32로 측정 PASS. 더 큰 배치는 메모리 압박 대비 처리량
/// 한계 효용 적음.
pub const EMBED_BATCH: usize = 32;

/// 임베더 핸들 — fastembed `TextEmbedding`을 `Mutex`로 직렬화.
///
/// `embed()` 메서드가 `&mut self`라 동시 호출 불가. v0.4.1은 인덱서 1개 + 검색 1개의
/// 가벼운 동시성이라 `Mutex<TextEmbedding>`로 충분 (큐·풀은 v0.4.2+).
///
/// 모델 init은 *명시*. 다운로드/로딩 실패가 첫 호출 시점에 즉시 노출돼야 사용자에게
/// "모델 다운로드 중..." UI 진행률을 정확히 보여줄 수 있다 (lazy init은 첫 검색
/// latency가 비상식적으로 길어진다).
pub struct Embedder {
    cache_dir: PathBuf,
    /// fastembed `TextEmbedding` 인스턴스 — `&mut self` 호출 직렬화용 mutex.
    model: Mutex<TextEmbedding>,
}

impl Embedder {
    /// 임베딩 차원 — mE5-small = 384. 차원 strict 검증(vec0 매치)에 사용.
    pub const DIM: usize = 384;

    /// 새 임베더 인스턴스. 모델을 *동기적으로* 로드한다 (명시 init).
    ///
    /// fastembed가 cache_dir 안에 모델을 캐시. 첫 호출 = 다운로드 (~120MB),
    /// 이후 = 디스크 hit. 호출 측은 `tokio::task::spawn_blocking`으로 격리해야
    /// async 런타임을 막지 않는다.
    pub fn new(app_data_dir: &Path) -> AppResult<Self> {
        let cache_dir = model_cache_dir(app_data_dir);
        std::fs::create_dir_all(&cache_dir).map_err(|e| AppError::Internal {
            message: format!("모델 cache 디렉토리 생성 실패 ({}): {e}", cache_dir.display()),
        })?;

        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(false),
        )
        .map_err(|e| AppError::Internal {
            message: format!("fastembed 모델 로드 실패: {e}"),
        })?;

        Ok(Self {
            cache_dir,
            model: Mutex::new(model),
        })
    }

    /// 임베더의 캐시 디렉토리 — 디버그/검증 용 노출.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// 청크 본문 배열 → 임베딩 벡터 배열. mE5 prefix(`"passage: "`)는 *호출 측이 미리* 붙인다.
    ///
    /// 이유: prefix는 인덱싱 vs 검색에 따라 다르고, 호출 측이 명시적으로 통제하는 게
    /// 안전. 본 메서드는 prefix 누락을 검사하지 않음 — caller's responsibility.
    pub fn embed_passages(&self, prefixed_chunks: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let inputs: Vec<&str> = prefixed_chunks.iter().map(String::as_str).collect();
        self.embed_inner(inputs)
    }

    /// 단일 사용자 질의 → 임베딩 벡터. 호출 측이 `query_prefix` 적용한 입력 전달.
    pub fn embed_query(&self, prefixed_query: &str) -> AppResult<Vec<f32>> {
        let mut vecs = self.embed_inner(vec![prefixed_query])?;
        vecs.pop().ok_or_else(|| AppError::Internal {
            message: "embed_query: fastembed가 빈 결과 반환".to_string(),
        })
    }

    /// 내부 — fastembed `embed(_, Some(BATCH))` 호출. mutex로 `&mut self` 직렬화.
    fn embed_inner(&self, inputs: Vec<&str>) -> AppResult<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let mut guard = self.model.lock().map_err(|_| AppError::Internal {
            message: "Embedder mutex poisoned".to_string(),
        })?;
        let vecs = guard
            .embed(inputs, Some(EMBED_BATCH))
            .map_err(|e| AppError::Internal {
                message: format!("fastembed embed() 실패: {e}"),
            })?;

        // 차원 검증 — 모델 mismatch 시 vec0 INSERT가 실패하기 전에 일찍 검출.
        if let Some(first) = vecs.first() {
            if first.len() != Self::DIM {
                return Err(AppError::Internal {
                    message: format!(
                        "임베딩 차원 mismatch: 기대 {} / 실제 {}",
                        Self::DIM,
                        first.len()
                    ),
                });
            }
        }
        Ok(vecs)
    }
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

    /// 게이팅 통합 테스트 — env `AIRIS_E2E_EMBED=1` 일 때만 실제 fastembed 다운로드·로드.
    /// 본 PR 단위 테스트가 CI에서 ~120MB 모델을 받지 않도록 격리. PR 3에서 정식 통합.
    #[test]
    fn end_to_end_embed_when_enabled() {
        if std::env::var("AIRIS_E2E_EMBED").ok().as_deref() != Some("1") {
            eprintln!("skip: AIRIS_E2E_EMBED 미설정 (모델 다운로드 비용)");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let embedder = Embedder::new(tmp.path()).expect("embedder init");
        let prefixed = vec![passage_prefix("Rust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다.")];
        let vecs = embedder.embed_passages(&prefixed).expect("embed_passages");
        assert_eq!(vecs.len(), 1);
        assert_eq!(vecs[0].len(), Embedder::DIM);

        let q = embedder
            .embed_query(&query_prefix("Rust 메모리 안전성"))
            .expect("embed_query");
        assert_eq!(q.len(), Embedder::DIM);
    }
}
