// 파일 열기 — v0.1 단일 파일 모드.
// JS 측이 plugin-dialog로 경로를 고른 뒤 file_open(path) 호출.
// 백엔드는 *경로 검증*(.md/.txt + 크기 한도) → 본문 읽기 → AppState.current_file 채움 → 메타 반환.

use std::fs;
use std::path::Path;

use serde::Serialize;
use tauri::State;
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::AppState;

/// 단일 파일 최대 크기 — 1MB. v0.1엔 LLM 컨텍스트로 통째 주입되므로 토큰 비용 고려한 안전선.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

const ALLOWED_EXTENSIONS: &[&str] = &["md", "txt", "markdown"];

#[derive(Debug, Serialize)]
pub struct FileMeta {
    pub name: String,
    pub path: String,
    pub char_count: usize,
}

#[tauri::command]
pub fn file_open(state: State<'_, AppState>, path: String) -> AppResult<FileMeta> {
    let p = Path::new(&path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some(e) if ALLOWED_EXTENSIONS.contains(&e) => {}
        _ => {
            return Err(AppError::InvalidInput {
                message: format!(
                    "지원하지 않는 파일 형식입니다 (.md / .txt만 가능): {}",
                    p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                ),
            });
        }
    }

    let metadata = fs::metadata(p)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(AppError::InvalidInput {
            message: format!(
                "파일이 너무 큽니다 (최대 {}MB): {} bytes",
                MAX_FILE_BYTES / (1024 * 1024),
                metadata.len()
            ),
        });
    }

    let bytes = fs::read(p)?;
    let text = String::from_utf8(bytes).map_err(|e| AppError::InvalidInput {
        message: format!("파일이 UTF-8이 아닙니다: {e}"),
    })?;

    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();

    let meta = FileMeta {
        name: name.clone(),
        path: p.to_string_lossy().to_string(),
        char_count: text.chars().count(),
    };

    *state.current_file.lock().expect("current_file mutex") = Some(text);

    info!(
        target: "file",
        file_name = %name,
        bytes = metadata.len(),
        char_count = meta.char_count,
        "file_open"
    );

    Ok(meta)
}

#[tauri::command]
pub fn file_close(state: State<'_, AppState>) -> AppResult<()> {
    *state.current_file.lock().expect("current_file mutex") = None;
    info!(target: "file", "file_close");
    Ok(())
}

/// 현재 열린 파일의 *본문*을 반환. v0.1엔 FileViewer가 화면 표시할 때만 사용.
/// 보안 측면: 파일 본문은 사용자가 직접 연 파일이므로 노출 가능.
#[tauri::command]
pub fn file_current_content(state: State<'_, AppState>) -> AppResult<Option<String>> {
    Ok(state
        .current_file
        .lock()
        .expect("current_file mutex")
        .clone())
}
