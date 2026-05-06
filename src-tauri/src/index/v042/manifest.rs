// manifest.json — v0.4.2 cascade 인덱스 단계별 *상태 라벨*.
//
// architecture §5 명시: SQLite 테이블이 진실의 출처이고 manifest는 *카운터가 아닌
// 상태 라벨*. 즉 chunk_count·progress 같은 숫자는 디버그·UI 표시용이고, 인덱싱
// 진행 자체는 chunks·indexing_jobs 테이블이 책임.
//
// 폴더 layout (D-085):
//   <app_data>/notebooks/<book_id>/
//     active_index.txt          — 현재 active tier 한 줄
//     indexes/
//       v0_bm25/                — FTS5는 chunks.db 안에 있어 manifest 불필요 (placeholder)
//       v1_me5-small/manifest.json
//       v2_bge-m3/manifest.json
//
// 본 모듈 책임:
//   * `Manifest` 구조체(직렬화) + `ManifestStatus` enum.
//   * `book_index_dir(app_data, book_id)` 폴더 helper.
//   * `tier_dir(app_data, book_id, kind)` — `v1_me5-small` / `v2_bge-m3` 분기.
//   * `read_manifest(path)` / `write_manifest_atomic(path, m)` — 같은 디렉토리 temp +
//     `std::fs::rename` (POSIX/Windows 모두 같은 파일시스템 내 아토믹).
//
// PR 3가 *manifest.json progress 필드*를 UI 빌드 진행률 표시에 활용 (호출만, 본문 X).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// 인덱스 tier 종류 — manifest 폴더 이름 + active_index.txt 값 일치.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    /// v0 = FTS5(BM25). chunks.db 내장이라 manifest 폴더는 placeholder.
    V0Bm25,
    /// v1 = mE5-small (384d).
    V1Me5Small,
    /// v2 = BGE-M3 (1024d).
    V2BgeM3,
}

impl IndexKind {
    /// 폴더명·active_index.txt 값. architecture §5 그대로.
    pub fn dir_name(&self) -> &'static str {
        match self {
            Self::V0Bm25 => "v0_bm25",
            Self::V1Me5Small => "v1_me5-small",
            Self::V2BgeM3 => "v2_bge-m3",
        }
    }

    /// 모델 식별자(manifest.model 필드).
    pub fn model_id(&self) -> &'static str {
        match self {
            Self::V0Bm25 => "bm25",
            Self::V1Me5Small => "me5-small",
            Self::V2BgeM3 => "bge-m3",
        }
    }

    /// 차원 — vec0 가상 테이블·차원 검증 ground truth.
    /// V0(FTS5)은 차원 개념 없음 → 0 반환.
    pub fn dim(&self) -> usize {
        match self {
            Self::V0Bm25 => 0,
            Self::V1Me5Small => 384,
            Self::V2BgeM3 => 1024,
        }
    }
}

/// 인덱스 단계의 상태 라벨. SQLite 테이블이 진실이지만 UI/디버그가 빠르게 분기하기
/// 위해 manifest에도 보유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStatus {
    /// 빌드 진행 중.
    Building,
    /// 빌드 완료 — retrieval에 사용 가능.
    Ready,
    /// 빌드 실패 (재시도 후보).
    Failed,
}

/// 단계별 manifest.json 본문. architecture §5 그대로.
///
/// `progress`/`completed_chunks`는 *Optional*. SQLite 테이블이 진실의 출처이므로 본
/// 값들은 UI hint 수준. 누락돼도 retrieval 흐름에 영향 X.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// 모델 식별자 — 'me5-small' / 'bge-m3' 등. `IndexKind::model_id()` 그대로.
    pub model: String,
    /// 임베딩 차원. vec0 가상 테이블 차원과 일관성 검증.
    pub dim: usize,
    /// 'building' / 'ready' / 'failed'.
    pub status: ManifestStatus,
    /// 빌드 시작 epoch ms.
    pub started_at: i64,
    /// 빌드 완료 epoch ms (Ready/Failed에만 채워짐).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_at: Option<i64>,
    /// 빌드 대상 청크 총 개수 (선언적 — 진행 중 갱신).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<i64>,
    /// 0.0~1.0 진행률 (UI 표시용; SQLite indexing_jobs.progress_chunks/total가 진실).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// 누적 처리 청크 (UI 표시용).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_chunks: Option<i64>,
    /// 최근 에러 메시지(상태 = Failed일 때만 의미 있음).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Manifest {
    /// 빌드 시작 시 `Building` 상태 manifest 생성. epoch ms는 호출 시점.
    pub fn new_building(kind: IndexKind, started_at: i64, total_chunks: Option<i64>) -> Self {
        Self {
            model: kind.model_id().to_string(),
            dim: kind.dim(),
            status: ManifestStatus::Building,
            started_at,
            built_at: None,
            chunk_count: total_chunks,
            progress: Some(0.0),
            completed_chunks: Some(0),
            last_error: None,
        }
    }

    /// 빌드 완료 → Ready 전환. `built_at` 채움.
    pub fn mark_ready(&mut self, built_at: i64, total_chunks: i64) {
        self.status = ManifestStatus::Ready;
        self.built_at = Some(built_at);
        self.chunk_count = Some(total_chunks);
        self.progress = Some(1.0);
        self.completed_chunks = Some(total_chunks);
        self.last_error = None;
    }

    /// 진행 중 갱신 — UI 진행률 hint 용.
    pub fn update_progress(&mut self, completed: i64, total: i64) {
        if total > 0 {
            self.progress = Some((completed as f64) / (total as f64));
        }
        self.completed_chunks = Some(completed);
        self.chunk_count = Some(total);
    }

    /// 실패 마킹 — Failed + last_error.
    pub fn mark_failed(&mut self, finished_at: i64, error_message: impl Into<String>) {
        self.status = ManifestStatus::Failed;
        self.built_at = Some(finished_at);
        self.last_error = Some(error_message.into());
    }
}

/// `<app_data>/notebooks/<book_id>/` — 책별 인덱스 루트.
pub fn book_dir(app_data_dir: &Path, book_id: &str) -> PathBuf {
    app_data_dir.join("notebooks").join(book_id)
}

/// `<app_data>/notebooks/<book_id>/indexes/` — tier별 manifest 컨테이너.
pub fn book_index_dir(app_data_dir: &Path, book_id: &str) -> PathBuf {
    book_dir(app_data_dir, book_id).join("indexes")
}

/// 한 tier의 폴더 — `<app_data>/notebooks/<book_id>/indexes/<dir_name>/`.
pub fn tier_dir(app_data_dir: &Path, book_id: &str, kind: IndexKind) -> PathBuf {
    book_index_dir(app_data_dir, book_id).join(kind.dir_name())
}

/// 한 tier의 manifest.json 절대경로.
pub fn manifest_path(app_data_dir: &Path, book_id: &str, kind: IndexKind) -> PathBuf {
    tier_dir(app_data_dir, book_id, kind).join("manifest.json")
}

/// manifest 폴더 + 부모 디렉토리 보장 (idempotent). write 전에 호출.
pub fn ensure_tier_dir(app_data_dir: &Path, book_id: &str, kind: IndexKind) -> AppResult<PathBuf> {
    let dir = tier_dir(app_data_dir, book_id, kind);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Fs {
        message: format!("manifest 폴더 생성 실패 ({}): {e}", dir.display()),
    })?;
    Ok(dir)
}

/// manifest.json 읽기. 파일이 없으면 None.
pub fn read_manifest(path: &Path) -> AppResult<Option<Manifest>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let m: Manifest = serde_json::from_str(&text).map_err(|e| AppError::Internal {
                message: format!("manifest.json 파싱 실패 ({}): {e}", path.display()),
            })?;
            Ok(Some(m))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::Fs {
            message: format!("manifest.json 읽기 실패 ({}): {e}", path.display()),
        }),
    }
}

/// manifest.json 아토믹 쓰기 — 같은 디렉토리에 temp 작성 후 `std::fs::rename`.
///
/// POSIX `rename(2)`은 같은 파일시스템 내 아토믹 보장. cross-fs는 보장 X — 부모
/// 디렉토리(`tier_dir`)에 temp를 만들고 rename. tempfile crate 미사용 (의존성 절감).
pub fn write_manifest_atomic(path: &Path, manifest: &Manifest) -> AppResult<()> {
    let parent = path.parent().ok_or_else(|| AppError::Internal {
        message: format!("manifest 경로 부모 없음: {}", path.display()),
    })?;
    std::fs::create_dir_all(parent).map_err(|e| AppError::Fs {
        message: format!("manifest 부모 폴더 생성 실패 ({}): {e}", parent.display()),
    })?;

    let serialized = serde_json::to_vec_pretty(manifest).map_err(|e| AppError::Internal {
        message: format!("manifest 직렬화 실패: {e}"),
    })?;

    // temp 파일은 *같은 디렉토리 안* — cross-fs rename 회피.
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("manifest.json");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_name = format!(".{file_name}.tmp.{pid}.{nanos}");
    let temp_path = parent.join(&temp_name);

    // temp write — 실패 시 본 manifest는 그대로 보존.
    std::fs::write(&temp_path, &serialized).map_err(|e| AppError::Fs {
        message: format!("manifest temp 쓰기 실패 ({}): {e}", temp_path.display()),
    })?;

    // rename = atomic on POSIX/Windows 같은 fs 안.
    std::fs::rename(&temp_path, path).map_err(|e| {
        // 실패 시 temp 정리 시도(best effort).
        let _ = std::fs::remove_file(&temp_path);
        AppError::Fs {
            message: format!(
                "manifest rename 실패 ({} → {}): {e}",
                temp_path.display(),
                path.display()
            ),
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_kind_dir_names_match_architecture() {
        assert_eq!(IndexKind::V0Bm25.dir_name(), "v0_bm25");
        assert_eq!(IndexKind::V1Me5Small.dir_name(), "v1_me5-small");
        assert_eq!(IndexKind::V2BgeM3.dir_name(), "v2_bge-m3");
    }

    #[test]
    fn index_kind_dimensions() {
        assert_eq!(IndexKind::V0Bm25.dim(), 0);
        assert_eq!(IndexKind::V1Me5Small.dim(), 384);
        assert_eq!(IndexKind::V2BgeM3.dim(), 1024);
    }

    #[test]
    fn book_dir_layout_matches_handoff() {
        let app_data = Path::new("/tmp/airis");
        let dir = book_dir(app_data, "book-123");
        assert_eq!(dir, PathBuf::from("/tmp/airis/notebooks/book-123"));
        let idx = book_index_dir(app_data, "book-123");
        assert_eq!(idx, PathBuf::from("/tmp/airis/notebooks/book-123/indexes"));
        let tier = tier_dir(app_data, "book-123", IndexKind::V2BgeM3);
        assert_eq!(
            tier,
            PathBuf::from("/tmp/airis/notebooks/book-123/indexes/v2_bge-m3")
        );
    }

    #[test]
    fn manifest_round_trip_serde() {
        let m = Manifest::new_building(IndexKind::V2BgeM3, 1_700_000_000_000, Some(1500));
        let json = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.status, ManifestStatus::Building);
        assert_eq!(back.dim, 1024);
    }

    #[test]
    fn manifest_status_serialized_as_snake_case() {
        let m = Manifest::new_building(IndexKind::V1Me5Small, 0, Some(0));
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"status\":\"building\""), "json: {json}");
    }

    #[test]
    fn write_manifest_atomic_creates_file_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = manifest_path(dir.path(), "b1", IndexKind::V2BgeM3);
        let m = Manifest::new_building(IndexKind::V2BgeM3, 1_700_000_000_000, Some(2000));
        write_manifest_atomic(&path, &m).unwrap();
        assert!(path.exists());
        let loaded = read_manifest(&path).unwrap().unwrap();
        assert_eq!(loaded, m);
    }

    #[test]
    fn write_manifest_atomic_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = manifest_path(dir.path(), "b1", IndexKind::V2BgeM3);
        let mut m = Manifest::new_building(IndexKind::V2BgeM3, 1_000, Some(100));
        write_manifest_atomic(&path, &m).unwrap();

        m.mark_ready(2_000, 100);
        write_manifest_atomic(&path, &m).unwrap();
        let loaded = read_manifest(&path).unwrap().unwrap();
        assert_eq!(loaded.status, ManifestStatus::Ready);
        assert_eq!(loaded.built_at, Some(2_000));
        assert_eq!(loaded.completed_chunks, Some(100));
    }

    #[test]
    fn write_manifest_atomic_cleans_up_temp_files_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = manifest_path(dir.path(), "b1", IndexKind::V1Me5Small);
        let m = Manifest::new_building(IndexKind::V1Me5Small, 0, Some(10));
        write_manifest_atomic(&path, &m).unwrap();
        // 부모 디렉토리에 ".manifest.json.tmp.*" 가 남아있지 않아야 한다.
        let parent = path.parent().unwrap();
        let leftover: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n.to_string_lossy().starts_with(".manifest.json.tmp."))
            .collect();
        assert!(leftover.is_empty(), "temp 파일 잔존: {leftover:?}");
    }

    #[test]
    fn read_manifest_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = manifest_path(dir.path(), "ghost", IndexKind::V2BgeM3);
        let m = read_manifest(&path).unwrap();
        assert!(m.is_none());
    }

    #[test]
    fn manifest_progress_update_computes_ratio() {
        let mut m = Manifest::new_building(IndexKind::V2BgeM3, 0, Some(200));
        m.update_progress(50, 200);
        assert!((m.progress.unwrap() - 0.25).abs() < 1e-9);
        assert_eq!(m.completed_chunks, Some(50));
        assert_eq!(m.chunk_count, Some(200));
    }

    #[test]
    fn manifest_mark_failed_records_error() {
        let mut m = Manifest::new_building(IndexKind::V1Me5Small, 0, Some(10));
        m.mark_failed(123, "tokenizer panic");
        assert_eq!(m.status, ManifestStatus::Failed);
        assert_eq!(m.last_error.as_deref(), Some("tokenizer panic"));
        assert_eq!(m.built_at, Some(123));
    }
}
