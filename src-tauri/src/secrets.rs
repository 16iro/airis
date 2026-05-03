// OS 키체인 래퍼.
// macOS: Security.framework Keychain
// Windows: Credential Manager
// Linux: Secret Service (gnome-keyring·KWallet via DBus)
//
// 모든 키는 service="airis" + user="<provider>-api-key" 조합으로 분리 저장한다.
// 디스크 평문 저장 금지 (security.md L114~).

use keyring::Entry;
use zeroize::Zeroize;

use crate::error::{AppError, AppResult};

const SERVICE: &str = "airis";

fn entry_for(provider: &str) -> AppResult<Entry> {
    let user = format!("{provider}-api-key");
    Entry::new(SERVICE, &user).map_err(map_err)
}

fn map_err(e: keyring::Error) -> AppError {
    AppError::Internal {
        message: format!("keychain: {e}"),
    }
}

/// 키 저장. 기존 키가 있으면 덮어쓴다.
pub fn set(provider: &str, key: &str) -> AppResult<()> {
    let entry = entry_for(provider)?;
    entry.set_password(key).map_err(map_err)?;
    Ok(())
}

/// 키 존재 여부만 반환. 키 *값* 자체는 절대 JS로 흐르지 않는다 (security.md L116).
pub fn has(provider: &str) -> bool {
    match entry_for(provider) {
        Ok(entry) => match entry.get_password() {
            Ok(mut k) => {
                k.zeroize();
                true
            }
            Err(keyring::Error::NoEntry) => false,
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// 키 삭제. 존재하지 않으면 NoEntry를 무시하고 Ok 반환.
pub fn delete(provider: &str) -> AppResult<()> {
    let entry = entry_for(provider)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(map_err(e)),
    }
}

/// Rust 측 호출(LLM 어댑터)이 키를 꺼낼 때 사용.
/// 반환된 String은 호출자가 사용 후 zeroize 해야 한다 (현재는 reqwest가 헤더로 박은 직후 drop됨).
pub(crate) fn get(provider: &str) -> AppResult<String> {
    let entry = entry_for(provider)?;
    entry.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => AppError::AuthRequired,
        other => map_err(other),
    })
}
