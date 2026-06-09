// 비-비밀 사용자 설정. {app_data_dir}/settings.json 평문 저장.
// API 키처럼 비밀로 다뤄야 할 값은 절대 여기 두지 않는다 (secrets.rs 사용).
//
// PR 13 v0.2b — D-005 부분 supersede: Anthropic + OpenAI + Gemini 3개 프로바이더 지원.
// 사용자가 active provider를 골라 사용. 각 프로바이더별 *기본 모델*을 따로 보관 — 전환 시 마지막 선택 유지.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commands::recall_v05::RecallStrength;
use crate::error::{AppError, AppResult};
use crate::runtime::hardware_probe::RecommendedTier;

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

/// v0.4.3 PR 1 (D-086) — 검색 강도 토글 (architecture §4.7.1).
///
/// 사용자가 채팅 응답의 *느림 vs 정확함*을 직접 고를 수 있도록 노출.
/// - `Fast`     : query rewriting · HyDE 모두 skip — 사용자 입력 그대로 검색.
/// - `Balanced` : (default) query rewriting ON · HyDE OFF — Haiku 1회 호출 비용으로 정밀도 ↑.
/// - `Accurate` : query rewriting + HyDE 모두 ON. HyDE 활성화 자체는 PR 3에서.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchStrength {
    Fast,
    #[default]
    Balanced,
    Accurate,
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
    /// v0.4.3 PR 1 (D-086) — 검색 강도 (빠름 / 균형 / 정확). 디폴트 `Balanced`.
    /// `Fast`는 query rewriting을 생략, `Balanced`는 rewriting ON, `Accurate`는 rewriting +
    /// HyDE ON (HyDE 자체는 PR 3에서 활성화).
    /// `#[serde(default)]`로 v0.4.2 이전 settings.json 무파괴 — 키 부재 시 `Balanced` 폴백.
    #[serde(default)]
    pub search_strength: SearchStrength,
    /// v0.4.4 PR 2 (D-092) — dev 전용 raw chat event 콘솔 로그 토글. 디폴트 OFF.
    /// ON이면 frontend `ChatPanel`/`AbComparePanel`이 `chat:*` 이벤트 도착마다
    /// `console.debug`로 payload + 카운터를 출력. BUG-002 같은 listener 누수
    /// 회귀를 디버깅할 때 사용. 사용자 빌드에서도 settings 모달 진단 그룹에서
    /// 직접 켤 수 있도록 frontend가 토글 노출.
    /// `#[serde(default)]`로 v0.4.3 이전 settings.json 무파괴 — 키 부재 시 false.
    #[serde(default)]
    pub dev_event_log: bool,
    /// v0.4.4 PR 4 (D-094) — 사용자 수동 등급 override. None이면 자동 추천을 따름.
    /// `Conservative` / `Balanced` / `Aggressive`. settings.json에서 lowercase 직렬화.
    /// `#[serde(default)]`로 v0.4.3 이전 settings.json 무파괴 — 키 부재 시 None.
    #[serde(default)]
    pub hardware_tier_override: Option<RecommendedTier>,
    /// v0.4.4 PR 4 (D-094) — 첫 추천 표시 시점 (epoch ms). None이면 추천 카드 미노출 상태 →
    /// frontend가 첫 진입 시 카드 자동 표시 + 사용자 응답 후 본 값 set.
    /// `#[serde(default)]`로 v0.4.3 이전 settings.json 무파괴.
    #[serde(default)]
    pub hardware_recommended_at: Option<i64>,
    /// v0.5 PR 3 (D-100) — 메타인지 Level 1 알림 활성화. 기본 true.
    /// 5지표 중 ≥2개 동시 발화 시 우상단 toast 알림. 차단 X (경고만).
    /// gate 3 false positive ≤ 2/주 미달 시 PR 5 폴백으로 default false 전환.
    /// `#[serde(default = "default_metacog_alerts_enabled")]`로 기존 settings.json 무파괴.
    #[serde(default = "default_metacog_alerts_enabled")]
    pub learning_metacog_alerts_enabled: bool,
    /// v0.5 PR 4 (D-101) — 회상 챌린지 강도. 기본 Weak (답 가리기만).
    /// weak = 답 가리기, medium = +4지선다, strong = +30초 시간 제한 (옵트인).
    /// `#[serde(default)]`로 기존 settings.json 무파괴.
    #[serde(default)]
    pub learning_recall_strength: RecallStrength,
    /// v0.5 PR 4 (D-101) — 회상 챌린지 자동 트리거 활성화. 기본 true.
    /// chat:done 후 citation confidence ≥ 0.5 청크 자동 트리거. 5분 쿨다운.
    /// `#[serde(default = "default_recall_auto_trigger")]`로 기존 settings.json 무파괴.
    #[serde(default = "default_recall_auto_trigger")]
    pub learning_recall_auto_trigger: bool,
    /// v0.5 PR 5 (D-102) — dev panel 표시 여부.
    /// None 이면 `import.meta.env.DEV` 기준 (dev 빌드만 ON).
    /// Some(true/false) 면 사용자 명시 값 사용.
    /// `#[serde(default)]`로 기존 settings.json 무파괴 — 키 부재 시 None.
    #[serde(default)]
    pub learning_dev_panel_enabled: Option<bool>,
    /// v0.5 PR 5 (D-102) — 학습 효율 자가 평가 기록.
    /// [{rated_at: epoch_ms, score: 1~10}] 형태. 최대 100건 (오래된 것부터 drop).
    /// `#[serde(default)]`로 기존 settings.json 무파괴 — 키 부재 시 빈 Vec.
    #[serde(default)]
    pub learning_self_rating_log: Vec<SelfRating>,
    /// v0.5 PR 5 (D-102) — 첫 실행 시각 (epoch ms).
    /// None 이면 최초 settings_read 시 자동 set.
    /// gate 5 self-rating 활성 조건 (7일 elapsed) 판단에 사용.
    /// `#[serde(default)]`로 기존 settings.json 무파괴.
    #[serde(default)]
    pub first_run_at: Option<i64>,
    /// v0.6.0 PR 2 (D-105) — PDF 뷰어 배율 모드.
    /// "auto" | "actual" | "fit-page" | "fit-width" | "percent". default = "auto".
    /// `#[serde(default)]`로 v0.5 이전 settings.json 무파괴 — 키 부재 시 "auto" 폴백.
    #[serde(default = "default_pdf_zoom_mode")]
    pub pdf_zoom_mode: String,
    /// v0.6.0 PR 2 (D-105) — pdf_zoom_mode == "percent" 일 때 배율값. 기본 100.
    /// 유효 범위 50~400, 10 단위 (clamping은 frontend 책임).
    /// `#[serde(default)]`로 v0.5 이전 settings.json 무파괴 — 키 부재 시 100 폴백.
    #[serde(default = "default_pdf_zoom_percent")]
    pub pdf_zoom_percent: u32,
    /// v0.6.x (D-110) — RAG 파이프라인 트레이스(관측성) dev 토글. 디폴트 OFF.
    /// ON이면 chat retrieval 경로가 단계별 시간·점수·버려진 source 수를 기록해
    /// `chat:context` 메타(`rag_trace`)로 노출 + tracing 로그. 평소(OFF)엔 비용 0.
    /// `#[serde(default)]`로 v0.6.0 이전 settings.json 무파괴 — 키 부재 시 false.
    #[serde(default)]
    pub dev_rag_trace: bool,
}

/// v0.5 PR 5 (D-102) — 학습 효율 자가 평가 단일 항목.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfRating {
    /// 평가 시각 (epoch ms).
    pub rated_at: i64,
    /// 점수 1~10.
    pub score: u8,
}

fn default_metacog_alerts_enabled() -> bool {
    true
}

fn default_recall_auto_trigger() -> bool {
    true
}

fn default_pdf_zoom_mode() -> String {
    "auto".to_string()
}

fn default_pdf_zoom_percent() -> u32 {
    100
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
            search_strength: SearchStrength::Balanced,
            dev_event_log: false,
            hardware_tier_override: None,
            hardware_recommended_at: None,
            learning_metacog_alerts_enabled: true,
            learning_recall_strength: RecallStrength::Weak,
            learning_recall_auto_trigger: true,
            learning_dev_panel_enabled: None,
            learning_self_rating_log: Vec::new(),
            first_run_at: None,
            pdf_zoom_mode: default_pdf_zoom_mode(),
            pdf_zoom_percent: default_pdf_zoom_percent(),
            dev_rag_trace: false,
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
        assert_eq!(
            s.search_strength,
            SearchStrength::Balanced,
            "v0.4.3 PR 1 D-086 — 검색 강도 default = Balanced"
        );
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
    fn legacy_settings_json_without_search_strength_defaults_balanced() {
        // v0.4.2 이전 settings.json은 search_strength 키 없음 — Balanced 폴백 검증 (D-086).
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"active_provider":"anthropic","models":{},"model":"x","language":"ko","theme":"dark","welcome_seen":true,"intervention_level":"confirm","auth_mode":"cli","cli_versions":{},"dev_ab_compare":false}"#,
        )
        .unwrap();
        let s = read(&path).unwrap();
        assert_eq!(s.search_strength, SearchStrength::Balanced);
    }

    #[test]
    fn search_strength_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        let original = Settings {
            search_strength: SearchStrength::Accurate,
            ..Settings::default()
        };
        write(&path, &original).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.search_strength, SearchStrength::Accurate);

        // serde lowercase 직렬화 확인.
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"search_strength\":\"accurate\""));

        // Fast 케이스도 검증.
        let fast = Settings {
            search_strength: SearchStrength::Fast,
            ..Settings::default()
        };
        let json = serde_json::to_string(&fast).unwrap();
        assert!(json.contains("\"search_strength\":\"fast\""));
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

    #[test]
    fn dev_event_log_default_is_off() {
        // v0.4.4 PR 2 (D-092) — BUG-002 listener 누수 디버깅용. 디폴트 OFF.
        let s = Settings::default();
        assert!(!s.dev_event_log);
    }

    #[test]
    fn legacy_settings_json_without_dev_event_log_defaults_off() {
        // v0.4.3 이전 settings.json은 dev_event_log 키 없음 — false 폴백 검증.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"active_provider":"anthropic","models":{},"model":"x","language":"ko","theme":"dark","welcome_seen":true,"intervention_level":"confirm","auth_mode":"cli","cli_versions":{},"dev_ab_compare":false,"search_strength":"balanced"}"#,
        )
        .unwrap();
        let s = read(&path).unwrap();
        assert!(!s.dev_event_log);
        assert_eq!(s.search_strength, SearchStrength::Balanced);
    }

    #[test]
    fn dev_event_log_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        let original = Settings {
            dev_event_log: true,
            ..Settings::default()
        };
        write(&path, &original).unwrap();
        let loaded = read(&path).unwrap();
        assert!(loaded.dev_event_log);

        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"dev_event_log\":true"));
    }

    #[test]
    fn hardware_fields_default_to_none() {
        // v0.4.4 PR 4 (D-094) — 자동 추천 따름 (None) + 추천 시점 미기록 (None).
        let s = Settings::default();
        assert!(s.hardware_tier_override.is_none());
        assert!(s.hardware_recommended_at.is_none());
    }

    #[test]
    fn legacy_settings_json_without_hardware_fields_defaults_none() {
        // v0.4.4 PR 3 이전 settings.json은 hardware_* 키 없음 — None 폴백 검증.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"active_provider":"anthropic","models":{},"model":"x","language":"ko","theme":"dark","welcome_seen":true,"intervention_level":"confirm","auth_mode":"cli","cli_versions":{},"dev_ab_compare":false,"search_strength":"balanced","dev_event_log":false}"#,
        )
        .unwrap();
        let s = read(&path).unwrap();
        assert!(s.hardware_tier_override.is_none());
        assert!(s.hardware_recommended_at.is_none());
    }

    #[test]
    fn legacy_settings_json_with_byok_field_is_ignored() {
        // D-096′ (2026-05-09): BYOK 어댑터 제거 후, 기존 사용자 settings.json 에 남아 있는
        // `byok_embedding` 필드는 *무시*되어야 한다 — 앱 시작 panic X. serde가 기본적으로
        // unknown field 를 무시(deny_unknown_fields 미지정)하는 동작에 의존.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"active_provider":"anthropic","models":{},"model":"x","language":"ko","theme":"dark","welcome_seen":true,"intervention_level":"confirm","auth_mode":"cli","cli_versions":{},"dev_ab_compare":false,"search_strength":"balanced","dev_event_log":false,"byok_embedding":{"provider":"voyage","model":"voyage-3-lite"}}"#,
        )
        .unwrap();
        let s = read(&path).unwrap();
        // 다른 필드가 정상 deserialize 되었는지 확인 — 무파괴.
        assert_eq!(s.theme, "dark");
        assert_eq!(s.search_strength, SearchStrength::Balanced);
    }

    #[test]
    fn metacog_alerts_default_is_true() {
        // v0.5 PR 3 (D-100) — 기본 ON (gate 3 미달 시 폴백으로 default false).
        let s = Settings::default();
        assert!(
            s.learning_metacog_alerts_enabled,
            "learning_metacog_alerts_enabled default must be true"
        );
    }

    #[test]
    fn legacy_settings_json_without_metacog_alerts_defaults_true() {
        // v0.4.x settings.json에 learning_metacog_alerts_enabled 없으면 true 폴백.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"active_provider":"anthropic","models":{},"model":"x","language":"ko","theme":"dark","welcome_seen":true,"intervention_level":"confirm","auth_mode":"cli","cli_versions":{},"dev_ab_compare":false,"search_strength":"balanced","dev_event_log":false}"#,
        )
        .unwrap();
        let s = read(&path).unwrap();
        assert!(
            s.learning_metacog_alerts_enabled,
            "missing key should default to true"
        );
    }

    #[test]
    fn metacog_alerts_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        let original = Settings {
            learning_metacog_alerts_enabled: false,
            ..Settings::default()
        };
        write(&path, &original).unwrap();
        let loaded = read(&path).unwrap();
        assert!(!loaded.learning_metacog_alerts_enabled);

        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"learning_metacog_alerts_enabled\":false"));
    }

    #[test]
    fn hardware_tier_override_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        let original = Settings {
            hardware_tier_override: Some(RecommendedTier::Balanced),
            hardware_recommended_at: Some(1_730_000_000_000),
            ..Settings::default()
        };
        write(&path, &original).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(
            loaded.hardware_tier_override,
            Some(RecommendedTier::Balanced)
        );
        assert_eq!(loaded.hardware_recommended_at, Some(1_730_000_000_000));

        // serde lowercase 검증.
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"hardware_tier_override\":\"balanced\""));
    }

    #[test]
    fn settings_default_has_auto_pdf_zoom() {
        // v0.6.0 PR 2 (D-105) — pdf_zoom_mode default = "auto", pdf_zoom_percent default = 100.
        let s = Settings::default();
        assert_eq!(
            s.pdf_zoom_mode, "auto",
            "pdf_zoom_mode default must be \"auto\""
        );
        assert_eq!(
            s.pdf_zoom_percent, 100,
            "pdf_zoom_percent default must be 100"
        );
    }

    #[test]
    fn settings_round_trip_preserves_zoom_mode() {
        // pdf_zoom_mode·pdf_zoom_percent 직렬화 → 역직렬화 왕복 검증.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        let original = Settings {
            pdf_zoom_mode: "fit-width".to_string(),
            pdf_zoom_percent: 150,
            ..Settings::default()
        };
        write(&path, &original).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.pdf_zoom_mode, "fit-width");
        assert_eq!(loaded.pdf_zoom_percent, 150);

        // JSON 키명 확인 (kebab-case 그대로).
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"pdf_zoom_mode\":\"fit-width\""));
        assert!(json.contains("\"pdf_zoom_percent\":150"));
    }

    #[test]
    fn settings_legacy_json_without_zoom_keys_loads_with_auto_default() {
        // v0.5 이전 settings.json에 pdf_zoom_mode·pdf_zoom_percent 없어도 default 폴백 — 무파괴.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            br#"{"active_provider":"anthropic","models":{},"model":"x","language":"ko","theme":"dark","welcome_seen":true,"intervention_level":"confirm","auth_mode":"cli","cli_versions":{},"dev_ab_compare":false,"search_strength":"balanced","dev_event_log":false}"#,
        )
        .unwrap();
        let s = read(&path).unwrap();
        assert_eq!(
            s.pdf_zoom_mode, "auto",
            "missing key must fall back to \"auto\""
        );
        assert_eq!(s.pdf_zoom_percent, 100, "missing key must fall back to 100");
    }
}
