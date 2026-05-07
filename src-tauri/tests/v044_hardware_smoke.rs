//! v0.4.4 PR 4 smoke test — D-094 하드웨어 자동 감지 + 모델 티어링.
//!
//! 검증 (HANDOFF PR 4):
//!   1. probe_hardware 호출 자체 panic 없이 완주.
//!   2. 반환된 cpu_cores·total_ram_gb는 0보다 큰 *의미 있는 값*.
//!   3. recommend_tier 분기 — 임계값 매트릭스 (cores<4|ram<8 / ram<16 / ram<32 / else).
//!   4. tier ↔ {t1,t2,t3}_enabled 자기 일관성.
//!
//! 본 smoke 테스트는 *cross-platform 동작 안정성* 확인. 실제 머신별 등급은 OS 의존이라
//! 사용자 1주 dev 빌드에서 gate 4 (사용자 머신에 맞는 추천 표시)로 검증.

use airis_lib::runtime::hardware_probe::{
    probe_hardware, recommend_tier, HardwareInfo, RecommendedTier, T1_MODEL_SIZE_MB,
    T2_MODEL_SIZE_MB, T3_MODEL_SIZE_MB,
};

#[test]
fn probe_hardware_returns_meaningful_values() {
    // sysinfo가 panic 없이 완주 + 0 아닌 값.
    let info = probe_hardware();
    assert!(info.cpu_cores >= 1, "cpu_cores ≥ 1 (폴백 포함)");
    assert!(
        info.total_ram_gb > 0.0,
        "total_ram_gb > 0 — 0이면 sysinfo total_memory 실패"
    );
    assert!(
        info.available_ram_gb >= 0.0,
        "available_ram_gb는 음수 X (sysinfo 0이면 0.0)"
    );
    assert!(
        info.available_ram_gb <= info.total_ram_gb + 0.5,
        "available <= total (반올림 오차 0.5 허용)"
    );
    assert!(!info.os.is_empty(), "OS 식별자 존재");
    assert!(!info.arch.is_empty(), "arch 식별자 존재");
}

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
fn recommend_tier_matrix_below_minimum() {
    // 2코어 / 4GB — 최소 사양 미만. Conservative + below_minimum=true.
    let r = recommend_tier(&fake(2, 4.0));
    assert_eq!(r.tier, RecommendedTier::Conservative);
    assert!(r.below_minimum);
    assert!(r.t1_enabled);
    assert!(!r.t2_enabled);
    assert!(!r.t3_enabled);
    assert_eq!(r.total_model_size_mb, T1_MODEL_SIZE_MB);
}

#[test]
fn recommend_tier_matrix_conservative_no_warning() {
    // 4코어 / 8GB — 최소 만족, 16 미만이라 Conservative.
    let r = recommend_tier(&fake(4, 8.0));
    assert_eq!(r.tier, RecommendedTier::Conservative);
    assert!(!r.below_minimum);
    assert_eq!(r.total_model_size_mb, T1_MODEL_SIZE_MB);
}

#[test]
fn recommend_tier_matrix_balanced() {
    // 4코어 / 16GB — Balanced 경계 (=16).
    let r = recommend_tier(&fake(4, 16.0));
    assert_eq!(r.tier, RecommendedTier::Balanced);
    assert!(r.t1_enabled);
    assert!(r.t2_enabled);
    assert!(!r.t3_enabled);
    assert_eq!(r.total_model_size_mb, T1_MODEL_SIZE_MB + T2_MODEL_SIZE_MB);
}

#[test]
fn recommend_tier_matrix_aggressive() {
    // 8코어 / 32GB — Aggressive 경계 (=32).
    let r = recommend_tier(&fake(8, 32.0));
    assert_eq!(r.tier, RecommendedTier::Aggressive);
    assert!(r.t1_enabled);
    assert!(r.t2_enabled);
    assert!(r.t3_enabled);
    assert_eq!(
        r.total_model_size_mb,
        T1_MODEL_SIZE_MB + T2_MODEL_SIZE_MB + T3_MODEL_SIZE_MB
    );

    // 64GB도 Aggressive 유지.
    let r = recommend_tier(&fake(16, 64.0));
    assert_eq!(r.tier, RecommendedTier::Aggressive);
}

#[test]
fn recommend_tier_threshold_just_below_balanced() {
    // 8코어 / 15.99GB — Balanced 미진입.
    let r = recommend_tier(&fake(8, 15.99));
    assert_eq!(r.tier, RecommendedTier::Conservative);
    assert!(!r.below_minimum);
}

#[test]
fn recommend_tier_threshold_just_below_aggressive() {
    // 8코어 / 31.99GB — Aggressive 미진입.
    let r = recommend_tier(&fake(8, 31.99));
    assert_eq!(r.tier, RecommendedTier::Balanced);
}

#[test]
fn recommend_tier_reason_is_korean_haptae() {
    // 합니다체 끝맺음 검증 (한국어 합니다체 — "다." 또는 "니다." 끝).
    let r = recommend_tier(&fake(8, 32.0));
    let trimmed = r.reason.trim_end_matches('.');
    assert!(
        trimmed.ends_with("니다") || trimmed.ends_with("있습니다") || r.reason.contains("권장합니다"),
        "reason은 합니다체 한국어이어야 함: {}",
        r.reason
    );
}
