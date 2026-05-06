// T2 임베더 — fastembed-rs 5.x + BGE-M3 (1024d, BAAI/bge-m3 ONNX FP).
//
// D-082 결정:
//   * 출처 = `fastembed::EmbeddingModel::BGEM3` (확인됨, fastembed 5.13.4 기준
//     `text_embedding.rs` 49–50: `/// BAAI/bge-m3` `BGEM3,`).
//   * 양자화 = **fastembed 5.13.4에서 BGE-M3 INT8 variant 미제공** — `BGEM3` 자체가
//     베이스 ONNX(model.onnx). HANDOFF의 "INT8" 표기는 fastembed가 제공하지 않아
//     베이스 ONNX로 채택. 폴백 (BAAI/bge-m3 직접 + 자체 ONNX wrapper)는 *미발동* —
//     fastembed enum 존재. 보고서·decision-log에 명시.
//
// 차원: 1024 (`BGEM3` model_info.dim = 1024). v041 mE5-small 384d와 분리.
// Cache: D-077 정책 그대로 `with_cache_dir(<app_data>/models/)` 강제.
//
// Prefix: BGE-M3는 mE5와 다르게 prefix 강제 X (BGE-M3는 CLS pooling, prompt-free
// inference 학습). fastembed 자체도 prefix 자동 첨가 X (mE5와 동일 — 호출 측이
// 명시 통제). T2 호출 측은 raw chunk text를 그대로 전달.
//
// 모델 사이즈: ~2GB (FP). i5-8350U 16GB에서 동시 chat 응답에 메모리 압박 →
// cooperative pause는 PR 5에서.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::error::{AppError, AppResult};
use crate::index::v041::embedder::model_cache_dir;

/// fastembed `embed(_, Some(BATCH))` 인자 — v041 EMBED_BATCH=32와 일관.
/// BGE-M3는 mE5보다 무거워 OOM 위험 있으나, 32는 PoC 베이스라인. PR 5에서 자원
/// 제한 시 절반(=16) 분기 옵션.
pub const T2_EMBED_BATCH: usize = 32;

/// T2 임베더 핸들 — fastembed `TextEmbedding`(BGE-M3)을 `Mutex`로 직렬화.
///
/// `embed()` 메서드가 `&mut self`라 동시 호출 불가. T2는 백그라운드 빌드 1개 +
/// (PR 5 hotswap 후) 사용자 검색 1개의 가벼운 동시성이라 `Mutex<TextEmbedding>`로
/// 충분.
pub struct EmbedderT2 {
    cache_dir: PathBuf,
    model: Mutex<TextEmbedding>,
}

impl EmbedderT2 {
    /// BGE-M3 차원 — vec0 가상 테이블 차원과 일관성 검증의 ground truth.
    pub const DIM: usize = 1024;

    /// 새 T2 임베더. 모델을 *동기적으로* 로드 (~2GB FP 첫 다운로드).
    ///
    /// 호출 측은 `tokio::task::spawn_blocking`으로 격리해야 async 런타임을 막지 않는다.
    /// 첫 호출 = 모델 다운로드(인터넷 필요), 이후 = 디스크 cache hit.
    pub fn new(app_data_dir: &Path) -> AppResult<Self> {
        let cache_dir = model_cache_dir(app_data_dir);
        std::fs::create_dir_all(&cache_dir).map_err(|e| AppError::Internal {
            message: format!("모델 cache 디렉토리 생성 실패 ({}): {e}", cache_dir.display()),
        })?;

        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGEM3)
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(false),
        )
        .map_err(|e| AppError::Internal {
            message: format!("fastembed BGE-M3 로드 실패: {e}"),
        })?;

        Ok(Self {
            cache_dir,
            model: Mutex::new(model),
        })
    }

    /// 디버그/검증용 cache 경로 노출.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// 청크 본문 배열 → 1024d 임베딩 벡터 배열.
    ///
    /// BGE-M3는 prefix 강제 X — 호출 측이 raw text를 그대로 전달.
    /// 차원 검증은 본 함수가 책임 (호출 측 실수 일찍 검출).
    pub fn embed_passages(&self, chunks: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let inputs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        self.embed_inner(inputs)
    }

    /// 단일 사용자 질의 → 1024d 임베딩 벡터.
    pub fn embed_query(&self, query: &str) -> AppResult<Vec<f32>> {
        let mut vecs = self.embed_inner(vec![query])?;
        vecs.pop().ok_or_else(|| AppError::Internal {
            message: "EmbedderT2::embed_query: fastembed가 빈 결과 반환".to_string(),
        })
    }

    fn embed_inner(&self, inputs: Vec<&str>) -> AppResult<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let mut guard = self.model.lock().map_err(|_| AppError::Internal {
            message: "EmbedderT2 mutex poisoned".to_string(),
        })?;
        let vecs = guard
            .embed(inputs, Some(T2_EMBED_BATCH))
            .map_err(|e| AppError::Internal {
                message: format!("fastembed BGE-M3 embed() 실패: {e}"),
            })?;

        // 차원 검증 — vec0 INSERT가 실패하기 전에 일찍 차단.
        if let Some(first) = vecs.first() {
            if first.len() != Self::DIM {
                return Err(AppError::Internal {
                    message: format!(
                        "T2 임베딩 차원 mismatch: 기대 {} / 실제 {}",
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

    #[test]
    fn dim_constant_is_1024() {
        // BGE-M3 차원 — vec0 가상 테이블 차원과 일관성 검증의 ground truth.
        assert_eq!(EmbedderT2::DIM, 1024);
    }

    #[test]
    fn cache_dir_under_app_data() {
        let app_data = Path::new("/tmp/airis-t2-test");
        // EmbedderT2::new 자체는 fastembed 다운로드(~2GB)이라 e2e 게이팅.
        // 여기서는 v041 model_cache_dir 헬퍼 재사용을 통해 cache 위치 일관성만 검증.
        let cache = model_cache_dir(app_data);
        assert!(cache.starts_with(app_data));
        assert_eq!(cache.file_name().and_then(|s| s.to_str()), Some("models"));
    }

    /// e2e 통합 — env `AIRIS_E2E_T2=1` 일 때만 실제 BGE-M3 다운로드(~2GB).
    /// 일반 CI는 mock embedder 패턴을 indexer_t2 테스트에서 사용.
    #[test]
    fn end_to_end_embed_when_enabled() {
        if std::env::var("AIRIS_E2E_T2").ok().as_deref() != Some("1") {
            eprintln!("skip: AIRIS_E2E_T2 미설정 (BGE-M3 ~2GB 다운로드 비용)");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let embedder = EmbedderT2::new(tmp.path()).expect("BGE-M3 init");
        // BGE-M3는 prefix 강제 X — raw text 그대로.
        let inputs = vec!["Rust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다.".to_string()];
        let vecs = embedder.embed_passages(&inputs).expect("embed_passages");
        assert_eq!(vecs.len(), 1);
        assert_eq!(vecs[0].len(), EmbedderT2::DIM);

        let q = embedder.embed_query("Rust 메모리 안전성").expect("embed_query");
        assert_eq!(q.len(), EmbedderT2::DIM);
    }
}
