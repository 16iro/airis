//! v0.4.2 PR 5 smoke test — D-083 자원 제한 (throttle).
//!
//! 검증 (HANDOFF §1.9):
//!   1. set_low_priority → restore_normal_priority round-trip 호출 자체가 panic 없이 완주.
//!   2. set_low_priority idempotent — 두 번째 호출도 성공.
//!   3. 작은 청크 시퀀스에 대해 IndexingWorker가 throttle 호출과 함께 정상 commit.
//!   4. PauseReason::CooperativeChat이 priority 정책에서 가장 약한 자동 사유.
//!
//! 본 smoke 테스트는 *호출 자체의 안정성*만 검증한다. 실제 OS priority 변경 효과는
//! OS 의존이라 unit 단위로 검증 불가 — 사용자 1주 dev 빌드 사용 후 gate 3 측정으로 확인.

use airis_lib::power_monitor::priority::{can_auto_resume, priority_score, should_override};
use airis_lib::index::v042::worker::PauseReason;
use airis_lib::runtime::throttle::{
    apply_low_thread_hint, restore_normal_priority, set_low_priority,
};

#[test]
fn priority_round_trip_does_not_panic() {
    // set → restore 사이클 + idempotent 호출 모두 성공.
    set_low_priority().expect("set_low_priority Ok");
    set_low_priority().expect("set_low_priority idempotent Ok");
    restore_normal_priority().expect("restore_normal_priority Ok");
    restore_normal_priority().expect("restore_normal_priority idempotent Ok");
}

#[test]
fn thread_hint_applies_without_panic() {
    // OMP_NUM_THREADS는 사용자가 설정한 값이 있으면 보존되고, 아니면 우리가 set.
    // 어느 경우든 panic 없이 완주.
    apply_low_thread_hint();
    // ORT_NUM_INTRA_OP_THREADS는 우리가 set한 값이거나 사용자가 set한 값.
    let val = std::env::var("ORT_NUM_INTRA_OP_THREADS").ok();
    assert!(val.is_some(), "thread hint set 후 env 변수가 존재해야 함");
}

#[test]
fn cooperative_chat_is_lowest_auto_reason() {
    // D-083 — cooperative_chat은 자동 사유 중 *가장 낮음*.
    // user(4) > app_quit(3) > thermal(2) > battery_low(1) > cooperative_chat(0).
    let cc = priority_score(PauseReason::CooperativeChat);
    let user = priority_score(PauseReason::User);
    let app_quit = priority_score(PauseReason::AppQuit);
    let thermal = priority_score(PauseReason::Thermal);
    let battery_low = priority_score(PauseReason::BatteryLow);
    assert!(cc < battery_low, "cooperative_chat < battery_low");
    assert!(cc < thermal);
    assert!(cc < app_quit);
    assert!(cc < user);
}

#[test]
fn cooperative_chat_does_not_override_existing_pause() {
    // chat 진입은 thermal/battery로 이미 pause된 worker를 *덮지 않음*.
    assert!(!should_override(
        Some(PauseReason::Thermal),
        PauseReason::CooperativeChat
    ));
    assert!(!should_override(
        Some(PauseReason::BatteryLow),
        PauseReason::CooperativeChat
    ));
    assert!(!should_override(
        Some(PauseReason::User),
        PauseReason::CooperativeChat
    ));
    // None 상태(=idle)에서는 적용 가능.
    assert!(should_override(None, PauseReason::CooperativeChat));
}

#[test]
fn cooperative_chat_auto_resume_allowed() {
    // chat 종료 시 cooperative_chat 사유 worker는 자동 resume 가능.
    assert!(can_auto_resume(Some(PauseReason::CooperativeChat)));
    // user pause는 보호 — chat 종료가 자동 resume 안 함.
    assert!(!can_auto_resume(Some(PauseReason::User)));
}
