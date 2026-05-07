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

// ---- v0.4.4 PR 5 (D-095) — BYOK 임베딩 키 -----------------------------------
//
// LLM 키(`anthropic`/`openai`/`gemini`)와 *분리*해 별도 entry로 영속한다 — provider 전환과
// 무관하게 사용자가 BYOK 임베딩 키만 따로 관리할 수 있도록. keyring user 식별자는
// `ByokProvider::keyring_id()` (`voyage-byok-embedding` 등)을 그대로 user에 박는다.
//
// 외부에 키 *값*은 절대 흐르지 않는다 — UI는 has/set/delete만 호출 (security.md L116).

/// BYOK 임베딩 키 저장. 기존 키가 있으면 덮어쓴다.
pub fn set_byok(keyring_id: &str, key: &str) -> AppResult<()> {
    let entry = entry_for(keyring_id)?;
    entry.set_password(key).map_err(map_err)?;
    Ok(())
}

/// BYOK 키 존재 여부.
pub fn has_byok(keyring_id: &str) -> bool {
    has(keyring_id)
}

/// BYOK 키 삭제. 존재하지 않으면 NoEntry를 무시하고 Ok 반환.
pub fn delete_byok(keyring_id: &str) -> AppResult<()> {
    delete(keyring_id)
}

/// Rust 측 어댑터(`VoyageEmbedder` 등)가 키를 꺼낼 때 사용.
/// PR 5는 *어댑터 추상*까지만 — 실제 라우팅 호출은 후속 슬라이스에서 들어가며 그 시점에
/// dead_code 경고가 자동 해소된다.
#[allow(dead_code)]
pub(crate) fn get_byok(keyring_id: &str) -> AppResult<String> {
    get(keyring_id)
}
