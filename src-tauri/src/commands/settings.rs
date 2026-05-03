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

/// Anthropic 키 prefix. 다른 provider 추가 시 매칭 테이블로 확장.
const ANTHROPIC_PREFIX: &str = "sk-ant-";
/// Anthropic 키 최소 길이 — 실키는 ~100자. 32는 명백한 오타·짤림 방지선.
const ANTHROPIC_MIN_LEN: usize = 32;

/// API 키의 *형식* 검증 (인터넷 사용 X). 실제 키 유효성은 PR 4의 LlmProvider가 검증.
#[tauri::command]
pub fn api_key_check(provider: String, key: String) -> AppResult<()> {
    match provider.as_str() {
        "anthropic" => {
            if !key.starts_with(ANTHROPIC_PREFIX) {
                return Err(AppError::InvalidInput {
                    message: format!("키가 '{ANTHROPIC_PREFIX}'로 시작해야 합니다"),
                });
            }
            if key.len() < ANTHROPIC_MIN_LEN {
                return Err(AppError::InvalidInput {
                    message: format!("키 길이가 너무 짧습니다 (최소 {ANTHROPIC_MIN_LEN}자)"),
                });
            }
            Ok(())
        }
        other => Err(AppError::InvalidInput {
            message: format!("알 수 없는 provider: {other}"),
        }),
    }
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

/// Settings 갱신 — 메모리 캐시 업데이트 + 디스크 원자 쓰기.
#[tauri::command]
pub fn settings_write(state: State<'_, AppState>, settings: Settings) -> AppResult<()> {
    let path = state.settings_path.clone();
    settings::write(&path, &settings)?;
    *state.settings.lock().expect("settings mutex poisoned") = settings;
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
    fn check_unknown_provider_rejected() {
        let err = api_key_check("openai".into(), "sk-".to_string() + &"x".repeat(40)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_empty_key_rejected() {
        let err = api_key_check("anthropic".into(), String::new()).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }
}
