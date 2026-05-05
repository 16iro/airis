// 비-비밀 사용자 설정. {app_data_dir}/settings.json 평문 저장.
// API 키처럼 비밀로 다뤄야 할 값은 절대 여기 두지 않는다 (secrets.rs 사용).
//
// PR 13 v0.2b — D-005 부분 supersede: Anthropic + OpenAI + Gemini 3개 프로바이더 지원.
// 사용자가 active provider를 골라 사용. 각 프로바이더별 *기본 모델*을 따로 보관 — 전환 시 마지막 선택 유지.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Anthropic,
    Openai,
    Gemini,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Anthropic => "claude-opus-4-7",
            Self::Openai => "gpt-4.1",
            Self::Gemini => "gemini-2.5-pro",
        }
    }
}

/// PR 24 (D-066) — 인증 경로.
/// `Cli`가 v0.2.1 메인. `ApiKey`는 v0.2 동작 유지를 위한 Advanced 백업 경로.
///
/// 기본값을 `ApiKey`로 둔 이유:
/// - 기존 v0.2 사용자의 settings.json에 auth_mode 필드가 없어도 깨지지 않게 (v0.2 챗 흐름 그대로 유지).
/// - 신규 사용자에게는 PR 27의 Welcome 화면이 명시적으로 `Cli`를 권장하도록.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    ApiKey,
    Cli,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InterventionLevel {
    /// 트리거 감지 시 1회 확인 다이얼로그 (사용자 OK 후 추가). 기본값.
    #[default]
    Confirm,
    /// 자동 적용 (다이얼로그 X). 적극적 누적.
    Auto,
    /// 트리거 감지 비활성. Memory 자동 갱신 X.
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 활성 LLM 프로바이더. chat_send가 dispatch에 사용.
    pub active_provider: Provider,
    /// 프로바이더별 마지막 선택 모델 — 전환 시 유지.
    /// key = Provider::as_str() ("anthropic"·"openai"·"gemini").
    pub models: HashMap<String, String>,
    /// (deprecated, v0.1 호환) — settings_write 시 active_provider의 models 항목으로 자동 마이그.
    /// 신규 호출자는 `Settings::active_model()` 사용.
    #[serde(default)]
    pub model: String,
    /// UI 언어. v0.1엔 "ko" 단일.
    pub language: String,
    /// 다크/라이트. "system" | "light" | "dark".
    pub theme: String,
    /// 환영 화면을 한 번 봤는지. false면 앱 시작 시 Welcome 표시.
    pub welcome_seen: bool,
    /// F10·F13.6 — 트리거 감지·갱신 다이얼로그 강도. 기본 Confirm.
    pub intervention_level: InterventionLevel,
    /// PR 24 (D-066) — 인증 경로. 기본 ApiKey (v0.2 호환). Cli는 Settings·Welcome에서 전환.
    pub auth_mode: AuthMode,
    /// PR 24 — 마지막으로 감지/설치한 CLI 버전. update 결정·UI 표시용.
    /// key = Provider::as_str(). 없으면 미설치 상태.
    pub cli_versions: HashMap<String, String>,
    /// v0.4.1 PR 5 — A/B 비교 dev 토글. 디폴트 OFF.
    /// ON이면 settings 모달의 *진단* 그룹에 dev panel이 보이고, ChatPanel에 "A/B 비교" 진입
    /// 버튼이 노출. handoff §1.3 acceptance gate 5.
    #[serde(default)]
    pub dev_ab_compare: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let mut models = HashMap::new();
        for p in [Provider::Anthropic, Provider::Openai, Provider::Gemini] {
            models.insert(p.as_str().to_string(), p.default_model().to_string());
        }
        Self {
            active_provider: Provider::Anthropic,
            models,
            model: Provider::Anthropic.default_model().to_string(),
            language: "ko".to_string(),
            theme: "system".to_string(),
            welcome_seen: false,
            intervention_level: InterventionLevel::Confirm,
            auth_mode: AuthMode::ApiKey,
            cli_versions: HashMap::new(),
            dev_ab_compare: false,
        }
    }
}

impl Settings {
    /// 활성 프로바이더의 모델. models HashMap이 비어있으면 default_model로 폴백.
    pub fn active_model(&self) -> String {
        self.models
            .get(self.active_provider.as_str())
            .cloned()
            .unwrap_or_else(|| self.active_provider.default_model().to_string())
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
    fn default_has_korean_and_anthropic_active() {
        let s = Settings::default();
        assert_eq!(s.language, "ko");
        assert_eq!(s.active_provider, Provider::Anthropic);
        assert_eq!(s.active_model(), "claude-opus-4-7");
        assert_eq!(s.models.get("openai").map(String::as_str), Some("gpt-4.1"));
        assert_eq!(
            s.models.get("gemini").map(String::as_str),
            Some("gemini-2.5-pro")
        );
        assert_eq!(s.theme, "system");
        assert!(!s.welcome_seen);
        assert!(!s.dev_ab_compare, "v0.4.1 PR 5 dev 토글은 디폴트 OFF");
    }

    #[test]
    fn legacy_settings_json_without_dev_ab_compare_defaults_off() {
        // v0.4.0 이전 settings.json은 dev_ab_compare 키가 없다 — 폴백 false 검증.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"active_provider":"anthropic","models":{},"model":"x","language":"ko","theme":"dark","welcome_seen":true,"intervention_level":"confirm","auth_mode":"cli","cli_versions":{}}"#,
        )
        .unwrap();
        let s = read(&path).unwrap();
        assert!(!s.dev_ab_compare);
        assert_eq!(s.theme, "dark");
    }

    #[test]
    fn active_model_switches_with_provider() {
        let s = Settings {
            active_provider: Provider::Openai,
            ..Settings::default()
        };
        assert_eq!(s.active_model(), "gpt-4.1");
        let s = Settings {
            active_provider: Provider::Gemini,
            ..Settings::default()
        };
        assert_eq!(s.active_model(), "gemini-2.5-pro");
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
        let mut models = Settings::default().models;
        models.insert("openai".to_string(), "gpt-4.1-mini".to_string());
        let original = Settings {
            theme: "dark".into(),
            welcome_seen: true,
            active_provider: Provider::Openai,
            models,
            ..Settings::default()
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
        // v0.1 사용자가 active_provider 없는 settings.json을 가지고 있어도 안전 폴백.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, br#"{"model":"claude-opus-4-7"}"#).unwrap();
        let s = read(&path).unwrap();
        assert_eq!(s.model, "claude-opus-4-7");
        assert_eq!(s.active_provider, Provider::Anthropic); // default
        assert_eq!(s.language, "ko"); // default
        assert_eq!(s.theme, "system"); // default
    }
}
