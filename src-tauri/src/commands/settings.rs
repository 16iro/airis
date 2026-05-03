// F13 Settings — API 키 보관 + 사용자 설정.
// v0.1 PR 3: keyring 기반 키 보관 + 형식 검증만 (실제 LLM 호출 검증은 PR 4).
//
// 보안 원칙 (security.md L114~):
//   - api_key_get은 *command로 노출 X*. JS에 키 흘리지 않는다.
//   - 외부에는 api_key_present(boolean)만 노출.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::secrets;
use crate::settings::{self, Settings};
use crate::AppState;

/// 프로바이더별 키 형식 — prefix + 최소 길이.
/// 출처: 각 프로바이더 공식 문서·실키 패턴 관찰.
const KEY_FORMAT: &[(&str, &str, usize)] = &[
    ("anthropic", "sk-ant-", 32),
    ("openai", "sk-", 32),
    ("gemini", "AIza", 30), // Google API key (39자 표준, 30 안전선)
];

/// API 키의 *형식* 검증 (인터넷 사용 X). 실제 키 유효성은 chat_send 첫 호출 시 검증.
#[tauri::command]
pub fn api_key_check(provider: String, key: String) -> AppResult<()> {
    let Some(&(_, prefix, min_len)) = KEY_FORMAT.iter().find(|(p, _, _)| *p == provider) else {
        return Err(AppError::InvalidInput {
            message: format!("지원하지 않는 provider: {provider}"),
        });
    };
    if !key.starts_with(prefix) {
        return Err(AppError::InvalidInput {
            message: format!("{provider} 키가 '{prefix}'로 시작해야 합니다"),
        });
    }
    if key.len() < min_len {
        return Err(AppError::InvalidInput {
            message: format!("키 길이가 너무 짧습니다 (최소 {min_len}자)"),
        });
    }
    Ok(())
}

/// 형식 검증 → 키체인 저장. 기존 키가 있으면 덮어쓴다.
#[tauri::command]
pub fn api_key_set(provider: String, key: String) -> AppResult<()> {
    api_key_check(provider.clone(), key.clone())?;
    secrets::set(&provider, &key)?;
    tracing::info!(target: "security", provider = %provider, "api_key_set");
    Ok(())
}

/// 키체인에서 키 삭제. 없으면 noop.
#[tauri::command]
pub fn api_key_delete(provider: String) -> AppResult<()> {
    secrets::delete(&provider)?;
    tracing::info!(target: "security", provider = %provider, "api_key_deleted");
    Ok(())
}

/// 키 존재 여부만 반환. 키 *값*은 절대 반환하지 않는다.
#[tauri::command]
pub fn api_key_present(provider: String) -> AppResult<bool> {
    Ok(secrets::has(&provider))
}

/// 현재 Settings 반환. 메모리 캐시(AppState)에서 즉시 응답.
#[tauri::command]
pub fn settings_read(state: State<'_, AppState>) -> AppResult<Settings> {
    let guard = state.settings.lock().expect("settings mutex poisoned");
    Ok(guard.clone())
}

/// Settings 갱신 — 메모리 캐시 업데이트 + 디스크 원자 쓰기 + LLM 프로바이더 rebuild.
/// active_provider OR auth_mode 변경 시 새 instance로 교체. 진행 중 chat_send는 자기 Arc 살아있어 영향 X.
#[tauri::command]
pub fn settings_write(state: State<'_, AppState>, settings: Settings) -> AppResult<()> {
    let path = state.settings_path.clone();
    settings::write(&path, &settings)?;

    let (prev_provider, prev_auth) = {
        let g = state.settings.lock().expect("settings mutex poisoned");
        (g.active_provider, g.auth_mode)
    };
    let next_provider = settings.active_provider;
    let next_auth = settings.auth_mode;
    let data_dir = state.data_dir.clone();
    *state.settings.lock().expect("settings mutex poisoned") = settings;

    if prev_provider != next_provider || prev_auth != next_auth {
        let new_llm = crate::build_provider(next_provider, next_auth, &data_dir)?;
        *state.llm.lock().expect("llm mutex poisoned") = new_llm;
        tracing::info!(
            target: "llm",
            from_provider = prev_provider.as_str(),
            to_provider = next_provider.as_str(),
            from_auth = ?prev_auth,
            to_auth = ?next_auth,
            "provider/auth switched"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_anthropic_prefix_required() {
        let err =
            api_key_check("anthropic".into(), "sk-foo-".to_string() + &"x".repeat(40)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_anthropic_min_length_required() {
        let err = api_key_check("anthropic".into(), "sk-ant-short".into()).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_anthropic_valid_passes() {
        let key = "sk-ant-".to_string() + &"a".repeat(40);
        assert!(api_key_check("anthropic".into(), key).is_ok());
    }

    #[test]
    fn check_openai_prefix_required() {
        let err = api_key_check("openai".into(), "AIza".to_string() + &"x".repeat(40)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_openai_valid_passes() {
        let key = "sk-".to_string() + &"a".repeat(40);
        assert!(api_key_check("openai".into(), key).is_ok());
    }

    #[test]
    fn check_gemini_prefix_required() {
        let err = api_key_check("gemini".into(), "sk-".to_string() + &"x".repeat(40)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_gemini_valid_passes() {
        let key = "AIza".to_string() + &"a".repeat(35);
        assert!(api_key_check("gemini".into(), key).is_ok());
    }

    #[test]
    fn check_unknown_provider_rejected() {
        let err = api_key_check("local".into(), "sk-".to_string() + &"x".repeat(40)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_empty_key_rejected() {
        let err = api_key_check("anthropic".into(), String::new()).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }
}
