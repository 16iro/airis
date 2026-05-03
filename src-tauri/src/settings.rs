// 비-비밀 사용자 설정. {app_data_dir}/settings.json 평문 저장.
// API 키처럼 비밀로 다뤄야 할 값은 절대 여기 두지 않는다 (secrets.rs 사용).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// LLM 모델 식별자 (예: claude-opus-4-7).
    pub model: String,
    /// UI 언어. v0.1엔 "ko" 단일.
    pub language: String,
    /// 다크/라이트. "system" | "light" | "dark".
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: "claude-opus-4-7".to_string(),
            language: "ko".to_string(),
            theme: "system".to_string(),
        }
    }
}

/// 디스크에서 Settings 읽기. 파일 없거나 깨진 JSON이면 default 반환.
/// (사용자가 settings.json 직접 망가뜨려도 앱이 안 죽도록 fallback)
pub fn read(path: &Path) -> AppResult<Settings> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes).unwrap_or_default())
}

/// 원자적 쓰기 — 임시 파일에 쓰고 rename으로 원자 교체.
/// (앱 충돌 시 부분 쓰여진 파일이 남는 걸 방지)
pub fn write(path: &Path, settings: &Settings) -> AppResult<()> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| AppError::Internal {
        message: format!("settings serialize: {e}"),
    })?;
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_has_korean_and_opus() {
        let s = Settings::default();
        assert_eq!(s.language, "ko");
        assert_eq!(s.model, "claude-opus-4-7");
        assert_eq!(s.theme, "system");
    }

    #[test]
    fn read_nonexistent_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        let s = read(&path).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        let original = Settings {
            model: "claude-sonnet-4-6".into(),
            language: "ko".into(),
            theme: "dark".into(),
        };
        write(&path, &original).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn read_corrupt_json_falls_back_to_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{not valid json").unwrap();
        let s = read(&path).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn partial_json_fills_missing_with_default() {
        // v0.2에서 새 필드 추가됐을 때 v0.1 사용자 settings.json도 읽혀야 함.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, br#"{"model":"claude-opus-4-7"}"#).unwrap();
        let s = read(&path).unwrap();
        assert_eq!(s.model, "claude-opus-4-7");
        assert_eq!(s.language, "ko"); // default
        assert_eq!(s.theme, "system"); // default
    }
}
