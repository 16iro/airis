// D-081 일시정지 트리거 우선순위 — 자동 resume 정책의 단일 소스.
//
// 결정 (decision-log D-081):
//   * 우선순위: **user > app_quit > thermal > battery_low**.
//   * 사용자 명시 의지 최우선 — 자동 resume이 사용자 pause를 덮어쓰면 안 됨.
//   * 동시 트리거 시 가장 강한(=숫자 큰) 사유가 현 pause 상태의 진실.
//
// 본 모듈은 *우선순위 비교 + auto-resume 가능 여부* 결정 로직만. 실제 worker
// pause/resume 호출은 commands::book가 책임 (DB 영속도 함께).

#![allow(dead_code)]

use crate::index::v042::worker::PauseReason;

/// 우선순위 점수 — 큰 값일수록 강한 사유. D-081 그대로.
///   * user(4) > app_quit(3) > thermal(2) > battery_low(1).
///
/// 값 자체에 의존하지 말고 비교 결과만 신뢰. 향후 결정 변동 시 본 함수만 수정.
pub fn priority_score(reason: PauseReason) -> u8 {
    match reason {
        PauseReason::User => 4,
        PauseReason::AppQuit => 3,
        PauseReason::Thermal => 2,
        PauseReason::BatteryLow => 1,
    }
}

/// 새 트리거가 현재 사유를 *덮어쓸 수* 있는지. 더 강한 사유일 때만 덮어쓴다.
///
/// 같은 우선순위(=같은 사유)는 idempotent — 덮어써도 무해하나 *변경* 신호로 보지 않음.
pub fn should_override(current: Option<PauseReason>, incoming: PauseReason) -> bool {
    match current {
        None => true,
        Some(cur) => priority_score(incoming) > priority_score(cur),
    }
}

/// 자동 resume 가능 여부 — 현재 사유가 *user가 아닌 경우*에만 자동 resume 허용.
///
/// D-081 핵심 invariant: **user pause는 절대 자동 resume X**. 사용자 명시 resume
/// 명령(`resume_indexing_job`)만 클리어.
///
/// `incoming_clear_event`는 "이 사유를 자동 해제할 외부 이벤트가 도착했다"는 뜻.
/// 예) BatteryOk → BatteryLow 해제 후보, AC 연결 → BatteryLow 해제 후보.
pub fn can_auto_resume(current: Option<PauseReason>) -> bool {
    !matches!(current, Some(PauseReason::User))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_order_matches_d081() {
        // user > app_quit > thermal > battery_low.
        assert!(priority_score(PauseReason::User) > priority_score(PauseReason::AppQuit));
        assert!(priority_score(PauseReason::AppQuit) > priority_score(PauseReason::Thermal));
        assert!(priority_score(PauseReason::Thermal) > priority_score(PauseReason::BatteryLow));
    }

    #[test]
    fn override_only_when_stronger() {
        // None → 모든 incoming 가능.
        assert!(should_override(None, PauseReason::BatteryLow));
        assert!(should_override(None, PauseReason::User));

        // BatteryLow는 약한 사유 → Thermal·AppQuit·User에 덮임.
        assert!(should_override(
            Some(PauseReason::BatteryLow),
            PauseReason::Thermal
        ));
        assert!(should_override(
            Some(PauseReason::BatteryLow),
            PauseReason::User
        ));

        // User는 가장 강한 사유 → 어떤 incoming도 덮어쓰지 못함.
        assert!(!should_override(
            Some(PauseReason::User),
            PauseReason::BatteryLow
        ));
        assert!(!should_override(
            Some(PauseReason::User),
            PauseReason::Thermal
        ));
        assert!(!should_override(
            Some(PauseReason::User),
            PauseReason::AppQuit
        ));

        // 같은 사유는 덮어쓰지 *않음* — idempotent 신호 처리.
        assert!(!should_override(
            Some(PauseReason::Thermal),
            PauseReason::Thermal
        ));
    }

    #[test]
    fn user_pause_blocks_auto_resume() {
        // user pause는 auto resume 불가 — 사용자 명시 명령만 클리어.
        assert!(!can_auto_resume(Some(PauseReason::User)));

        // 자동 사유들은 모두 auto resume 가능.
        assert!(can_auto_resume(Some(PauseReason::BatteryLow)));
        assert!(can_auto_resume(Some(PauseReason::Thermal)));
        assert!(can_auto_resume(Some(PauseReason::AppQuit)));

        // 현재 pause 사유가 없으면 자동 resume은 의미 없지만 true (=무해).
        assert!(can_auto_resume(None));
    }
}
