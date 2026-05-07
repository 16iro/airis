// v0.4.4 PR 4 (D-094) — 하드웨어 자동 감지 + 임베딩 모델 티어링 추천.
//
// 결정 (decision-log D-094 — 본 PR에서 락인):
//   * `sysinfo` crate 단일 의존 — Linux/macOS/Windows 동일 API. cross-platform 동작
//     검증. 0.32.x 채택 (rust-version 1.74 호환, 0.39은 1.95 요구).
//   * 추천 매트릭스 (단순 우선, 사용자 override 항상 우선):
//       cores < 4  OR ram < 8  → Conservative + 경고 ("최소 사양 미만")
//       ram   < 16            → Conservative (T1만)
//       ram   < 32            → Balanced     (T1 + T2)
//       else                  → Aggressive   (T1 + T2 + T3 reranker)
//   * GPU 감지는 v0.5+ — ONNX CUDA / Metal 검토. 본 PR은 CPU·RAM·OS·arch만.
//   * 모델 사이즈 안내값 (UI 표시 + total 계산):
//       T1 mE5-small INT8     ~120MB
//       T2 BGE-M3 FP          ~2_200MB (= 2.2GB)
//       T3 BGE Reranker FP    ~600MB
//
// 본 모듈은 *프레임*. 실제 settings 통합 / commands 노출 / frontend 표시는 별도 위치:
//   * `commands/hardware.rs` — Tauri 명령 노출.
//   * `settings.rs::Settings::hardware_tier_override` / `hardware_recommended_at`.
//   * `src/components/HardwareRecommendation.tsx` — 사용자 머신 사양 + 추천 카드.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sysinfo::System;

/// 1MiB 단위 — `total_memory()`는 byte. UI 표시는 GB로 환산하므로 1024^3.
const BYTES_PER_GB: f64 = 1_073_741_824.0;

/// 모델 사이즈 안내값 (MB). architecture §4.4 기준.
pub const T1_MODEL_SIZE_MB: u64 = 120; // mE5-small INT8 (384d)
pub const T2_MODEL_SIZE_MB: u64 = 2_200; // BGE-M3 FP (1024d)
pub const T3_MODEL_SIZE_MB: u64 = 600; // BGE Reranker FP

/// 추천 임계값 — D-094 락인. 변경 시 본 모듈 + frontend 동시 수정.
pub const MIN_CORES: usize = 4;
pub const MIN_RAM_GB: f64 = 8.0;
pub const BALANCED_RAM_GB: f64 = 16.0;
pub const AGGRESSIVE_RAM_GB: f64 = 32.0;

/// 사용자 머신 사양. UI에 노출.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareInfo {
    /// 논리 코어 수. 0 반환은 sysinfo 실패 시 fallback (1로 폴백).
    pub cpu_cores: usize,
    /// 총 RAM (GB, 소수점 2자리).
    pub total_ram_gb: f64,
    /// 사용 가능한 RAM (GB, 소수점 2자리). 추천에 직접 쓰진 않지만 UI 표시.
    pub available_ram_gb: f64,
    /// "linux" | "macos" | "windows" | OS 식별자.
    pub os: String,
    /// "x86_64" | "aarch64" | ...
    pub arch: String,
}

/// 추천 등급. settings.json 영속 (`hardware_tier_override`).
///
/// `serde(rename_all = "lowercase")` — TypeScript 쪽 union literal과 직접 매칭.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecommendedTier {
    /// T1만 (mE5-small INT8). 최소 사양 또는 RAM 16GB 미만.
    Conservative,
    /// T1 + T2 (mE5-small + BGE-M3). RAM 16~32GB.
    Balanced,
    /// T1 + T2 + T3 (mE5-small + BGE-M3 + Reranker). RAM 32GB+.
    Aggressive,
}

impl RecommendedTier {
    pub fn t1_enabled(&self) -> bool {
        true
    }

    pub fn t2_enabled(&self) -> bool {
        matches!(self, Self::Balanced | Self::Aggressive)
    }

    pub fn t3_enabled(&self) -> bool {
        matches!(self, Self::Aggressive)
    }

    pub fn total_model_size_mb(&self) -> u64 {
        let mut total = T1_MODEL_SIZE_MB;
        if self.t2_enabled() {
            total += T2_MODEL_SIZE_MB;
        }
        if self.t3_enabled() {
            total += T3_MODEL_SIZE_MB;
        }
        total
    }
}

/// 추천 + 그 이유. UI 카드에 표시.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationDetail {
    pub tier: RecommendedTier,
    /// 한국어 합니다체 한 줄 — UI에 그대로 노출.
    pub reason: String,
    pub t1_enabled: bool,
    pub t2_enabled: bool,
    pub t3_enabled: bool,
    pub total_model_size_mb: u64,
    /// 최소 사양 미만 경고. true면 UI에서 추가 알림 표시.
    pub below_minimum: bool,
}

/// 시스템 정보 수집. sysinfo가 OS 콜 실패해도 panic 없이 폴백 값 반환.
///
/// 동작:
///   * `System::new_all()` + `refresh_all()` — 모든 정보 1회 수집.
///   * `cpus().len()` — 논리 코어 수 (hyper-threading 포함).
///   * `total_memory()` / `available_memory()` — byte 단위 → GB 환산.
///
/// 폴백:
///   * cpu_cores 0 반환 시 1로 보정 (산술 안전).
///   * total_ram_gb 0.0 시 그대로 — `recommend_tier`가 below_minimum 분기.
pub fn probe_hardware() -> HardwareInfo {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_cores = sys.cpus().len().max(1);
    let total_ram_gb = round2(sys.total_memory() as f64 / BYTES_PER_GB);
    let available_ram_gb = round2(sys.available_memory() as f64 / BYTES_PER_GB);

    HardwareInfo {
        cpu_cores,
        total_ram_gb,
        available_ram_gb,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

/// 임계값 매트릭스로 등급 결정. 사용자 override 우선이라 본 함수는 *순수 추천*.
///
/// 분기:
///   * cores < MIN_CORES OR ram < MIN_RAM_GB → Conservative + below_minimum=true
///   * ram < BALANCED_RAM_GB                  → Conservative
///   * ram < AGGRESSIVE_RAM_GB                → Balanced
///   * else                                   → Aggressive
pub fn recommend_tier(info: &HardwareInfo) -> RecommendationDetail {
    let cores = info.cpu_cores;
    let ram = info.total_ram_gb;

    let (tier, reason, below_minimum) = if cores < MIN_CORES || ram < MIN_RAM_GB {
        (
            RecommendedTier::Conservative,
            format!(
                "최소 권장 사양({MIN_CORES}코어 / {min_ram}GB) 미만입니다. 가장 가벼운 mE5-small 임베딩만 활성화하기를 권장합니다.",
                min_ram = MIN_RAM_GB as u64,
            ),
            true,
        )
    } else if ram < BALANCED_RAM_GB {
        (
            RecommendedTier::Conservative,
            format!(
                "RAM이 {balanced}GB 미만이라 mE5-small 임베딩만 안정적으로 동작합니다. BGE-M3은 {balanced}GB 이상에서 권장합니다.",
                balanced = BALANCED_RAM_GB as u64,
            ),
            false,
        )
    } else if ram < AGGRESSIVE_RAM_GB {
        (
            RecommendedTier::Balanced,
            format!(
                "RAM {ram:.1}GB 환경에 적합합니다. mE5-small + BGE-M3 두 단계 임베딩으로 정확도를 높일 수 있습니다. Reranker는 {aggr}GB 이상에서 권장합니다.",
                ram = ram,
                aggr = AGGRESSIVE_RAM_GB as u64,
            ),
            false,
        )
    } else {
        (
            RecommendedTier::Aggressive,
            format!(
                "RAM {ram:.1}GB 환경입니다. mE5-small + BGE-M3 + Reranker 모두 활성화해 최고 정확도를 낼 수 있습니다.",
                ram = ram,
            ),
            false,
        )
    };

    RecommendationDetail {
        tier,
        reason,
        t1_enabled: tier.t1_enabled(),
        t2_enabled: tier.t2_enabled(),
        t3_enabled: tier.t3_enabled(),
        total_model_size_mb: tier.total_model_size_mb(),
        below_minimum,
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(cores: usize, ram_gb: f64) -> HardwareInfo {
        HardwareInfo {
            cpu_cores: cores,
            total_ram_gb: ram_gb,
            available_ram_gb: ram_gb,
            os: "linux".into(),
            arch: "x86_64".into(),
        }
    }

    #[test]
    fn probe_returns_nonzero_values() {
        // 실제 OS 의존 — 호출 자체가 panic 없이 완주하는지 + 의미 있는 값인지만 검증.
        let info = probe_hardware();
        assert!(info.cpu_cores >= 1, "cpu_cores >= 1 (폴백 포함)");
        assert!(info.total_ram_gb > 0.0, "total_ram_gb > 0 — 0이면 sysinfo 실패");
        assert!(!info.os.is_empty(), "OS 식별자 존재");
        assert!(!info.arch.is_empty(), "arch 식별자 존재");
    }

    #[test]
    fn under_min_cores_or_ram_is_conservative_with_warning() {
        // 2코어 / 4GB — 최소 사양 미만.
        let r = recommend_tier(&fake(2, 4.0));
        assert_eq!(r.tier, RecommendedTier::Conservative);
        assert!(r.below_minimum, "below_minimum=true");
        assert!(r.t1_enabled);
        assert!(!r.t2_enabled);
        assert!(!r.t3_enabled);
        assert!(r.reason.contains("최소"));
    }

    #[test]
    fn min_cores_with_8gb_is_conservative_no_warning() {
        // 4코어 / 8GB — 최소 사양 만족, 16GB 미만이라 Conservative.
        let r = recommend_tier(&fake(4, 8.0));
        assert_eq!(r.tier, RecommendedTier::Conservative);
        assert!(!r.below_minimum);
        assert!(r.t1_enabled);
        assert!(!r.t2_enabled);
    }

    #[test]
    fn ram_16gb_is_balanced() {
        // 4코어 / 16GB — Balanced 경계.
        let r = recommend_tier(&fake(4, 16.0));
        assert_eq!(r.tier, RecommendedTier::Balanced);
        assert!(!r.below_minimum);
        assert!(r.t1_enabled);
        assert!(r.t2_enabled);
        assert!(!r.t3_enabled);
    }

    #[test]
    fn ram_32gb_is_aggressive() {
        // 8코어 / 32GB — Aggressive 경계.
        let r = recommend_tier(&fake(8, 32.0));
        assert_eq!(r.tier, RecommendedTier::Aggressive);
        assert!(r.t1_enabled);
        assert!(r.t2_enabled);
        assert!(r.t3_enabled);
    }

    #[test]
    fn ram_64gb_is_aggressive() {
        let r = recommend_tier(&fake(8, 64.0));
        assert_eq!(r.tier, RecommendedTier::Aggressive);
    }

    #[test]
    fn total_model_size_matches_tier() {
        assert_eq!(
            RecommendedTier::Conservative.total_model_size_mb(),
            T1_MODEL_SIZE_MB
        );
        assert_eq!(
            RecommendedTier::Balanced.total_model_size_mb(),
            T1_MODEL_SIZE_MB + T2_MODEL_SIZE_MB
        );
        assert_eq!(
            RecommendedTier::Aggressive.total_model_size_mb(),
            T1_MODEL_SIZE_MB + T2_MODEL_SIZE_MB + T3_MODEL_SIZE_MB
        );
    }

    #[test]
    fn just_under_balanced_threshold_stays_conservative() {
        // 8코어 / 15GB — 16GB 미만이라 Balanced 진입 X.
        let r = recommend_tier(&fake(8, 15.99));
        assert_eq!(r.tier, RecommendedTier::Conservative);
        assert!(!r.below_minimum);
    }

    #[test]
    fn just_under_aggressive_threshold_stays_balanced() {
        // 8코어 / 31.99GB — 32GB 미만이라 Aggressive 진입 X.
        let r = recommend_tier(&fake(8, 31.99));
        assert_eq!(r.tier, RecommendedTier::Balanced);
    }

    #[test]
    fn serialized_tier_is_lowercase() {
        // frontend TS literal과 직접 매칭하는지 검증.
        let s = serde_json::to_string(&RecommendedTier::Conservative).unwrap();
        assert_eq!(s, "\"conservative\"");
        let s = serde_json::to_string(&RecommendedTier::Balanced).unwrap();
        assert_eq!(s, "\"balanced\"");
        let s = serde_json::to_string(&RecommendedTier::Aggressive).unwrap();
        assert_eq!(s, "\"aggressive\"");
    }

    #[test]
    fn round2_preserves_two_decimals() {
        assert_eq!(round2(7.123456), 7.12);
        assert_eq!(round2(15.999), 16.0);
        assert_eq!(round2(0.0), 0.0);
    }
}
