// v0.4.4 PR 4 (D-094) — 하드웨어 자동 감지 + 모델 티어링 추천 Tauri 명령.
//
// frontend `HardwareRecommendation.tsx`가 다음 두 명령을 호출:
//   * `dev_probe_hardware`            → HardwareInfo (CPU·RAM·OS·arch)
//   * `dev_get_model_recommendation`  → RecommendationDetail (등급 + 이유 + 모델 사이즈)
//
// 본 명령들은 *읽기 전용*. 결정은 frontend가 settings에 영속.
// 명령 이름 prefix `dev_*` — 다른 acceptance 측정 명령과 동일한 prefix 채택.

#![allow(dead_code)]

use crate::error::AppResult;
use crate::runtime::hardware_probe::{
    probe_hardware, recommend_tier, HardwareInfo, RecommendationDetail,
};

/// 사용자 머신 사양 1회 측정.
///
/// sysinfo `System::new_all` + `refresh_all` — 모든 정보 1회 수집. 본 명령은 매 호출마다
/// 새 측정 (캐싱 X) — 호출 빈도 낮음 (settings 진입 시 1회).
#[tauri::command]
pub fn dev_probe_hardware() -> AppResult<HardwareInfo> {
    Ok(probe_hardware())
}

/// 추천 등급 + 이유 + 모델 사이즈 합계.
///
/// `dev_probe_hardware`를 내부에서 1회 호출한 뒤 `recommend_tier`로 매트릭스 분기.
/// frontend가 이 결과를 카드에 그대로 표시.
#[tauri::command]
pub fn dev_get_model_recommendation() -> AppResult<RecommendationDetail> {
    let info = probe_hardware();
    Ok(recommend_tier(&info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::hardware_probe::RecommendedTier;

    #[test]
    fn dev_probe_hardware_returns_meaningful_values() {
        let info = dev_probe_hardware().expect("probe Ok");
        assert!(info.cpu_cores >= 1);
        assert!(info.total_ram_gb > 0.0);
        assert!(!info.os.is_empty());
        assert!(!info.arch.is_empty());
    }

    #[test]
    fn dev_get_model_recommendation_returns_consistent_tier() {
        // 실제 머신 등급은 OS 의존 — 본 테스트는 *반환 자체*가 자기 일관성을 갖는지만 확인.
        let r = dev_get_model_recommendation().expect("recommend Ok");
        // T1은 항상 활성.
        assert!(r.t1_enabled);
        // tier ↔ {t2,t3}_enabled 일관성.
        match r.tier {
            RecommendedTier::Conservative => {
                assert!(!r.t2_enabled);
                assert!(!r.t3_enabled);
            }
            RecommendedTier::Balanced => {
                assert!(r.t2_enabled);
                assert!(!r.t3_enabled);
            }
            RecommendedTier::Aggressive => {
                assert!(r.t2_enabled);
                assert!(r.t3_enabled);
            }
        }
        // total_model_size_mb는 활성 티어 합과 일치.
        assert_eq!(r.total_model_size_mb, r.tier.total_model_size_mb());
        assert!(!r.reason.is_empty());
    }
}
