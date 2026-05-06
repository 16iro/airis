// active_index.txt — 책별 핫스왑 포인터 (D-085).
//
// 위치: `<app_data>/notebooks/<book_id>/active_index.txt` (책별 분리).
// 내용: 단일 줄 — 'v0_bm25' / 'v1_me5-small' / 'v2_bge-m3' 중 하나.
//
// 핫스왑 메커니즘:
//   1. T2 빌드 완료 → manifest_t2.status='ready' 기록 (manifest::write_manifest_atomic).
//   2. 같은 디렉토리에 temp 파일로 새 active 값 기록.
//   3. `std::fs::rename(temp → active_index.txt)` — POSIX/Windows 같은 fs 안 아토믹.
//   4. 다음 retrieval 진입 시 새 값을 읽음. 진행 중 retrieval은 그 사이클을 끝낸 모델로
//      일관 (호출 측이 진입 시 1회 읽기 + 결과 캐시).
//
// 디폴트:
//   * 파일 부재 = `V1Me5Small` — v0.4.1 호환. T1 인덱싱 완료 직후 기본 active.
//   * 모르는 값(잘못된 manifest)이면 InvalidInput 에러. 호출 측이 폴백 결정.
//
// 본 모듈은 *파일 I/O만*. T2 빌드 완료 시 호출은 v042 indexer_t2가 책임.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::index::v042::manifest::{book_dir, IndexKind};

/// `<app_data>/notebooks/<book_id>/active_index.txt` 절대경로.
pub fn active_index_path(app_data_dir: &Path, book_id: &str) -> PathBuf {
    book_dir(app_data_dir, book_id).join("active_index.txt")
}

/// active_index.txt 읽기. 파일 부재 시 `V1Me5Small` 디폴트(v0.4.1 호환).
///
/// 잘못된 값(=알 수 없는 dir_name)이면 `AppError::InvalidInput` — UI는 사용자에게 책
/// 폴더 손상 안내 후 reindex 유도.
pub fn read_active_index(app_data_dir: &Path, book_id: &str) -> AppResult<IndexKind> {
    let path = active_index_path(app_data_dir, book_id);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_index_kind(text.trim()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(IndexKind::V1Me5Small),
        Err(e) => Err(AppError::Fs {
            message: format!("active_index.txt 읽기 실패 ({}): {e}", path.display()),
        }),
    }
}

/// active_index.txt 아토믹 쓰기 — 같은 디렉토리에 temp 작성 후 `std::fs::rename`.
///
/// POSIX `rename(2)` = 같은 파일시스템 내 아토믹. cross-fs는 보장 X — 항상 같은
/// 부모 디렉토리에 temp 파일을 만든다.
pub fn write_active_index_atomic(
    app_data_dir: &Path,
    book_id: &str,
    kind: IndexKind,
) -> AppResult<()> {
    let path = active_index_path(app_data_dir, book_id);
    let parent = path.parent().ok_or_else(|| AppError::Internal {
        message: format!("active_index 경로 부모 없음: {}", path.display()),
    })?;
    std::fs::create_dir_all(parent).map_err(|e| AppError::Fs {
        message: format!(
            "active_index 부모 폴더 생성 실패 ({}): {e}",
            parent.display()
        ),
    })?;

    // 한 줄 + 개행. UI/디버그가 cat으로 직접 읽기 쉽게.
    let payload = format!("{}\n", kind.dir_name());

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_name = format!(".active_index.txt.tmp.{pid}.{nanos}");
    let temp_path = parent.join(&temp_name);

    std::fs::write(&temp_path, payload.as_bytes()).map_err(|e| AppError::Fs {
        message: format!(
            "active_index temp 쓰기 실패 ({}): {e}",
            temp_path.display()
        ),
    })?;

    std::fs::rename(&temp_path, &path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        AppError::Fs {
            message: format!(
                "active_index rename 실패 ({} → {}): {e}",
                temp_path.display(),
                path.display()
            ),
        }
    })?;
    Ok(())
}

fn parse_index_kind(s: &str) -> AppResult<IndexKind> {
    match s {
        "v0_bm25" => Ok(IndexKind::V0Bm25),
        "v1_me5-small" => Ok(IndexKind::V1Me5Small),
        "v2_bge-m3" => Ok(IndexKind::V2BgeM3),
        other => Err(AppError::InvalidInput {
            message: format!("active_index.txt 알 수 없는 값: {other:?}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_default_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let kind = read_active_index(dir.path(), "ghost").unwrap();
        assert_eq!(kind, IndexKind::V1Me5Small, "파일 부재 = v1 디폴트");
    }

    #[test]
    fn write_then_read_round_trip_v2() {
        let dir = tempfile::tempdir().unwrap();
        write_active_index_atomic(dir.path(), "b1", IndexKind::V2BgeM3).unwrap();
        let kind = read_active_index(dir.path(), "b1").unwrap();
        assert_eq!(kind, IndexKind::V2BgeM3);
    }

    #[test]
    fn write_then_read_round_trip_v1() {
        let dir = tempfile::tempdir().unwrap();
        write_active_index_atomic(dir.path(), "b2", IndexKind::V1Me5Small).unwrap();
        let kind = read_active_index(dir.path(), "b2").unwrap();
        assert_eq!(kind, IndexKind::V1Me5Small);
    }

    #[test]
    fn write_overwrites_previous_value() {
        let dir = tempfile::tempdir().unwrap();
        write_active_index_atomic(dir.path(), "b1", IndexKind::V1Me5Small).unwrap();
        write_active_index_atomic(dir.path(), "b1", IndexKind::V2BgeM3).unwrap();
        let kind = read_active_index(dir.path(), "b1").unwrap();
        assert_eq!(kind, IndexKind::V2BgeM3, "마지막 write가 active");
    }

    #[test]
    fn write_cleans_up_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        write_active_index_atomic(dir.path(), "b1", IndexKind::V2BgeM3).unwrap();
        let parent = active_index_path(dir.path(), "b1");
        let parent = parent.parent().unwrap();
        let leftover: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n.to_string_lossy().starts_with(".active_index.txt.tmp."))
            .collect();
        assert!(leftover.is_empty(), "temp 파일 잔존: {leftover:?}");
    }

    #[test]
    fn parse_unknown_value_returns_invalid_input() {
        let err = parse_index_kind("v9_bogus").unwrap_err();
        match err {
            AppError::InvalidInput { .. } => {}
            other => panic!("기대 InvalidInput, 받음: {other:?}"),
        }
    }

    #[test]
    fn read_invalid_file_content_returns_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = active_index_path(dir.path(), "b1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "garbage\n").unwrap();
        let err = read_active_index(dir.path(), "b1").unwrap_err();
        match err {
            AppError::InvalidInput { .. } => {}
            other => panic!("기대 InvalidInput, 받음: {other:?}"),
        }
    }

    #[test]
    fn read_trims_trailing_newline_and_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = active_index_path(dir.path(), "b1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // 사람 손편집/CRLF 환경 대비.
        std::fs::write(&path, "  v2_bge-m3\r\n  ").unwrap();
        let kind = read_active_index(dir.path(), "b1").unwrap();
        assert_eq!(kind, IndexKind::V2BgeM3);
    }
}
