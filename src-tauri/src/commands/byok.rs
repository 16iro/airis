// v0.4.4 PR 5 (D-095) — BYOK 클라우드 임베딩 Tauri 명령.
//
// frontend `ByokSection.tsx`가 다음 명령을 호출:
//   * `byok_key_set`         → Voyage / Gemini 키 keyring 영속.
//   * `byok_key_present`     → 키 존재 여부 (값 자체는 절대 노출 X).
//   * `byok_key_delete`      → 키 삭제.
//   * `byok_estimate_cost`   → 청크 수·평균 토큰 → 예상 비용(USD) 계산.
//
// 본 명령들은 *settings 자체*는 건드리지 않는다 — settings.byok_embedding 갱신은 frontend가
// `settings_write` 한 번에 일괄 처리. 본 모듈은 *키체인 + 비용 안내*만 책임.

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::index::v044::byok_embedding::ByokProvider;
use crate::secrets;
use crate::AppState;

/// BYOK provider별 키 형식 — prefix + 최소 길이.
/// 출처:
///   * Voyage: `pa-` 접두사 (https://docs.voyageai.com/docs/api-key-and-installation).
///   * Gemini Embedding: 일반 Google API key 형식 (`AIza` + 35자).
const BYOK_KEY_FORMAT: &[(ByokProvider, &str, usize)] = &[
    (ByokProvider::Voyage, "pa-", 16),
    (ByokProvider::Gemini, "AIza", 30),
];

fn format_for(provider: ByokProvider) -> Option<(&'static str, usize)> {
    BYOK_KEY_FORMAT
        .iter()
        .find(|(p, _, _)| *p == provider)
        .map(|(_, prefix, min)| (*prefix, *min))
}

/// 키 *형식* 검증. 실제 호출 검증은 인덱싱 시점 어댑터가 401을 받으면 사용자 안내.
fn check_key_format(provider: ByokProvider, key: &str) -> AppResult<()> {
    let Some((prefix, min_len)) = format_for(provider) else {
        return Err(AppError::InvalidInput {
            message: "지원하지 않는 BYOK provider 입니다".to_string(),
        });
    };
    if !key.starts_with(prefix) {
        return Err(AppError::InvalidInput {
            message: format!("BYOK 키가 '{prefix}'로 시작해야 합니다"),
        });
    }
    if key.len() < min_len {
        return Err(AppError::InvalidInput {
            message: format!("BYOK 키 길이가 너무 짧습니다 (최소 {min_len}자)"),
        });
    }
    Ok(())
}

/// 형식 검증 → keyring 저장.
#[tauri::command]
pub fn byok_key_set(provider: ByokProvider, key: String) -> AppResult<()> {
    check_key_format(provider, &key)?;
    secrets::set_byok(provider.keyring_id(), &key)?;
    tracing::info!(
        target: "security",
        provider = provider.keyring_id(),
        "byok_key_set"
    );
    Ok(())
}

/// 키 존재 여부만 반환. 값 자체는 절대 노출 X.
#[tauri::command]
pub fn byok_key_present(provider: ByokProvider) -> AppResult<bool> {
    Ok(secrets::has_byok(provider.keyring_id()))
}

/// keyring에서 키 삭제. 없으면 noop.
#[tauri::command]
pub fn byok_key_delete(provider: ByokProvider) -> AppResult<()> {
    secrets::delete_byok(provider.keyring_id())?;
    tracing::info!(
        target: "security",
        provider = provider.keyring_id(),
        "byok_key_deleted"
    );
    Ok(())
}

/// 예상 비용 안내. 사용자가 *모르고* 큰 청구 받지 않도록 settings UI에 표시.
///
/// 분기 (handoff §9 — 정확하지 않아도 *대략*):
///   * Voyage `voyage-3-lite`: $0.02 / 1M input tokens (2026 시점, 변동 가능).
///   * Voyage `voyage-3`     : $0.06 / 1M input tokens.
///   * Gemini `text-embedding-004`: 무료 티어 (사용자 Gemini 구독 활용 가능).
///
/// chunks * avg_tokens 으로 총 토큰 추정 → 단가 곱.
#[derive(Debug, Clone, Serialize)]
pub struct ByokCostEstimate {
    pub provider: ByokProvider,
    pub model: String,
    pub chunks: u32,
    pub avg_tokens_per_chunk: u32,
    /// 예상 USD. 무료 티어면 0.0. 소수점 4자리.
    pub usd_estimate: f64,
    /// "$0.02 / 1M tokens" 같은 단가 표시.
    pub unit_price_label: String,
}

/// chunks * avg_tokens 기반 비용 추정. 모델별 단가는 모듈 상수.
#[tauri::command]
pub fn byok_estimate_cost(
    provider: ByokProvider,
    model: String,
    chunks: u32,
    avg_tokens_per_chunk: u32,
) -> AppResult<ByokCostEstimate> {
    let total_tokens = chunks as u64 * avg_tokens_per_chunk.max(1) as u64;
    let (usd_per_million, label) = unit_price(provider, &model);
    let usd = (total_tokens as f64 / 1_000_000.0) * usd_per_million;
    Ok(ByokCostEstimate {
        provider,
        model,
        chunks,
        avg_tokens_per_chunk,
        usd_estimate: round4(usd),
        unit_price_label: label.to_string(),
    })
}

fn unit_price(provider: ByokProvider, model: &str) -> (f64, &'static str) {
    match (provider, model) {
        (ByokProvider::Voyage, "voyage-3") => (0.06, "$0.06 / 1M tokens"),
        (ByokProvider::Voyage, "voyage-3-lite") => (0.02, "$0.02 / 1M tokens"),
        (ByokProvider::Voyage, _) => (0.02, "$0.02 / 1M tokens (default voyage-3-lite)"),
        (ByokProvider::Gemini, _) => (0.0, "무료 (Gemini 무료 티어)"),
    }
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// gate 5 (BYOK 라우팅) 측정 결과 — settings.byok_embedding 활성 시 어댑터 라우팅이
/// 잡히는지 *시뮬*. 실제 호출은 사용자 키가 있어야 가능 — 본 명령은 *상태 검증*만.
#[derive(Debug, Clone, Serialize)]
pub struct ByokRoutingResult {
    /// settings.byok_embedding이 Some 인지.
    pub byok_active: bool,
    /// 활성 provider — None이면 비활성.
    pub provider: Option<ByokProvider>,
    /// 활성 모델 — None이면 비활성.
    pub model: Option<String>,
    /// keyring에 키가 보관 중인지. byok_active=true 인데 false면 사용자에게 "키 입력 필요".
    pub key_present: bool,
    /// "fastembed (mE5-small)" | "cloud (voyage-3-lite)" — UI 표시용 레이블.
    pub routed_to: String,
}

/// gate 5 측정 dev 명령. settings + keyring 상태를 한 묶음으로 반환.
#[tauri::command]
pub fn dev_byok_routing_check(state: State<'_, AppState>) -> AppResult<ByokRoutingResult> {
    let cfg = {
        let g = state.settings.lock().expect("settings mutex");
        g.byok_embedding.clone()
    };
    let (byok_active, provider, model, key_present, routed_to) = match cfg {
        Some(c) => {
            let key_present = secrets::has_byok(c.provider.keyring_id());
            let routed = if key_present {
                format!("cloud ({})", c.model)
            } else {
                "fastembed (mE5-small) — BYOK 키 없음, 폴백".to_string()
            };
            (true, Some(c.provider), Some(c.model), key_present, routed)
        }
        None => (
            false,
            None,
            None,
            false,
            "fastembed (mE5-small)".to_string(),
        ),
    };
    Ok(ByokRoutingResult {
        byok_active,
        provider,
        model,
        key_present,
        routed_to,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_voyage_prefix_required() {
        let err = check_key_format(ByokProvider::Voyage, "sk-foo-1234567890123456").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_voyage_min_length_required() {
        let err = check_key_format(ByokProvider::Voyage, "pa-tooshort").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_voyage_valid_passes() {
        let key = "pa-".to_string() + &"a".repeat(40);
        check_key_format(ByokProvider::Voyage, &key).expect("should pass");
    }

    #[test]
    fn check_gemini_prefix_required() {
        let err = check_key_format(ByokProvider::Gemini, "pa-".to_string().as_str()).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }

    #[test]
    fn check_gemini_valid_passes() {
        let key = "AIza".to_string() + &"a".repeat(35);
        check_key_format(ByokProvider::Gemini, &key).expect("should pass");
    }

    #[test]
    fn estimate_voyage_lite_cost() {
        // 1500 청크 * 200 토큰 = 300_000 토큰. $0.02/1M → 0.006 USD.
        let r = byok_estimate_cost(
            ByokProvider::Voyage,
            "voyage-3-lite".into(),
            1500,
            200,
        )
        .unwrap();
        assert_eq!(r.chunks, 1500);
        assert_eq!(r.avg_tokens_per_chunk, 200);
        assert!((r.usd_estimate - 0.006).abs() < 1e-6);
        assert!(r.unit_price_label.contains("0.02"));
    }

    #[test]
    fn estimate_voyage_3_higher_cost() {
        // voyage-3은 lite 대비 3배 비싸다.
        let lite = byok_estimate_cost(
            ByokProvider::Voyage,
            "voyage-3-lite".into(),
            1000,
            500,
        )
        .unwrap();
        let v3 = byok_estimate_cost(
            ByokProvider::Voyage,
            "voyage-3".into(),
            1000,
            500,
        )
        .unwrap();
        assert!((v3.usd_estimate - lite.usd_estimate * 3.0).abs() < 1e-4);
    }

    #[test]
    fn estimate_gemini_is_free() {
        let r = byok_estimate_cost(
            ByokProvider::Gemini,
            "text-embedding-004".into(),
            10_000,
            1000,
        )
        .unwrap();
        assert_eq!(r.usd_estimate, 0.0);
        assert!(r.unit_price_label.contains("무료"));
    }

    #[test]
    fn estimate_handles_zero_tokens_safely() {
        // avg_tokens_per_chunk=0 → max(1) 폴백, 0 청크 = 0 USD.
        let r = byok_estimate_cost(
            ByokProvider::Voyage,
            "voyage-3-lite".into(),
            0,
            0,
        )
        .unwrap();
        assert_eq!(r.usd_estimate, 0.0);
    }

    #[test]
    fn round4_preserves_four_decimals() {
        assert_eq!(round4(0.123456), 0.1235);
        assert_eq!(round4(0.0), 0.0);
        assert_eq!(round4(1.99995), 2.0);
    }
}
