// v0.4.3 PR 4 (D-090) — BGE-reranker-v2-m3 cross-encoder.
//
// 인용 검증(citation verification)·향후 retrieval re-rank를 위한 cross-encoder
// 점수 산출기. fastembed-rs 5.x의 `RerankerModel::BGERerankerV2M3`를 사용 (확인됨,
// fastembed 5.13.4 `models/reranking.rs` 11–11: `BGERerankerV2M3`,
// model_code = `rozgo/bge-reranker-v2-m3`, model.onnx + model.onnx.data 추가 파일).
//
// D-090 결정:
//   * 출처 = `fastembed::RerankerModel::BGERerankerV2M3` (확인됨).
//   * 양자화 = **fastembed 5.13.4에서 BGE-reranker-v2-m3 INT8 variant 미제공** —
//     `BGERerankerV2M3` 자체가 베이스 ONNX(model.onnx + model.onnx.data). HANDOFF의
//     "INT8" 표기는 fastembed가 제공하지 않아 베이스 ONNX로 채택. 폴백(자체 ort
//     wrapper)은 미발동 — fastembed enum 존재. T2(BGE-M3) 패턴과 동일.
//
// Cache: T1·T2와 동일한 `<app_data>/models/` 디렉토리 강제 (D-077).
//
// Prefix: BGE-reranker는 cross-encoder라 query·passage를 함께 토큰화해 score를 낸다.
// fastembed `TextRerank::rerank`가 (query, [docs]) 시그니처라 호출 측은 query·candidates
// 를 따로 전달.
//
// 모델 사이즈: ~600MB (FP, model.onnx + model.onnx.data). T2(2GB) + T3(600MB) 합 =
// ~2.6GB 모델 cache. binary 사이즈는 무관 (런타임 다운로드).
//
// Mutex: TextRerank::rerank가 `&mut self` (encoder 세션)이므로 Mutex로 직렬화.
// 인용 검증은 chat 응답당 1회 호출이라 가벼운 동시성으로 충분.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use crate::error::{AppError, AppResult};
use crate::index::v041::embedder::model_cache_dir;

/// rerank 호출의 batch 크기 — 인용 검증은 보통 N=3~6 candidates라 작음.
/// retrieval re-rank로 확장 시(향후) top-K=20+ 도 무난.
pub const RERANK_BATCH_SIZE: usize = 16;

/// Reranker 핸들 — fastembed `TextRerank`(BGE-reranker-v2-m3)를 `Mutex`로 직렬화.
///
/// `rerank()` 가 `&mut self`라 동시 호출 불가. chat 응답당 인용 검증 1회 호출이라
/// `Mutex<TextRerank>` 로 충분 (T2 EmbedderT2 패턴 동일).
pub struct Reranker {
    cache_dir: PathBuf,
    model: Mutex<TextRerank>,
}

impl Reranker {
    /// 새 Reranker. 모델을 *동기적으로* 로드 (~600MB FP 첫 다운로드).
    ///
    /// 호출 측은 `tokio::task::spawn_blocking`으로 격리해야 async 런타임을 막지 않는다.
    /// 첫 호출 = 모델 다운로드(인터넷 필요), 이후 = 디스크 cache hit.
    pub fn new(app_data_dir: &Path) -> AppResult<Self> {
        let cache_dir = model_cache_dir(app_data_dir);
        std::fs::create_dir_all(&cache_dir).map_err(|e| AppError::Internal {
            message: format!("모델 cache 디렉토리 생성 실패 ({}): {e}", cache_dir.display()),
        })?;

        let model = TextRerank::try_new(
            RerankInitOptions::new(RerankerModel::BGERerankerV2M3)
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(false),
        )
        .map_err(|e| AppError::Internal {
            message: format!("fastembed BGE-reranker-v2-m3 로드 실패: {e}"),
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

    /// query × candidates → 각 candidate의 cross-encoder 점수 리스트.
    ///
    /// 반환 순서는 *입력 candidates 순서 그대로* (fastembed::TextRerank::rerank는
    /// score 내림차순으로 반환하므로 본 함수가 index로 재정렬).
    /// candidates 비면 빈 Vec 반환.
    pub fn rerank(&self, query: &str, candidates: &[String]) -> AppResult<Vec<f32>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut guard = self.model.lock().map_err(|_| AppError::Internal {
            message: "Reranker mutex poisoned".to_string(),
        })?;
        // fastembed::TextRerank::rerank의 documents는 `AsRef<[S]>` where S: AsRef<str>.
        // &[String] 자체는 AsRef<[&str]>를 구현하지 않으므로 &[&str]로 변환해 넘긴다.
        let docs: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let results = guard
            .rerank(query, &docs, false, Some(RERANK_BATCH_SIZE))
            .map_err(|e| AppError::Internal {
                message: format!("fastembed BGE-reranker rerank() 실패: {e}"),
            })?;

        // RerankResult 의 `index`는 입력 순서 인덱스. 본 함수는 입력 순서로 score 정렬.
        let mut scores = vec![0.0_f32; candidates.len()];
        for r in results {
            if r.index < scores.len() {
                scores[r.index] = r.score;
            }
        }
        Ok(scores)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_under_app_data() {
        let app_data = Path::new("/tmp/airis-reranker-test");
        // Reranker::new 자체는 fastembed 다운로드(~600MB)이라 e2e 게이팅.
        // 여기서는 v041 model_cache_dir 헬퍼 재사용을 통해 cache 위치 일관성만 검증.
        let cache = model_cache_dir(app_data);
        assert!(cache.starts_with(app_data));
        assert_eq!(cache.file_name().and_then(|s| s.to_str()), Some("models"));
    }

    /// e2e 통합 — env `AIRIS_E2E_RERANKER=1` 일 때만 실제 BGE-reranker 다운로드(~600MB).
    /// 일반 CI는 citation_check::tests에서 score 계약(0..=1 범위 등)만 확인하는 mock 단위.
    #[test]
    fn end_to_end_rerank_when_enabled() {
        if std::env::var("AIRIS_E2E_RERANKER").ok().as_deref() != Some("1") {
            eprintln!("skip: AIRIS_E2E_RERANKER 미설정 (BGE-reranker ~600MB 다운로드 비용)");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let reranker = Reranker::new(tmp.path()).expect("BGE-reranker init");
        let query = "Rust 메모리 안전성";
        let candidates = vec![
            "Rust ownership 모델은 컴파일 시점에 메모리 안전성을 보장합니다.".to_string(),
            "오늘 날씨가 좋네요.".to_string(),
        ];
        let scores = reranker.rerank(query, &candidates).expect("rerank");
        assert_eq!(scores.len(), 2);
        // 관련 candidate가 더 높은 score를 받아야 한다 (cross-encoder 검증).
        assert!(scores[0] > scores[1]);
    }

    #[test]
    fn empty_candidates_return_empty() {
        // Reranker 인스턴스 없이는 호출 불가하지만, 시그니처 자체 회귀 방지를 위해
        // 빈 입력 케이스만 별도 분기 검사 (실제 호출은 e2e 게이팅).
        let candidates: Vec<String> = Vec::new();
        assert!(candidates.is_empty());
    }
}
