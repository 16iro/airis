// 전원·시스템 이벤트 모니터 — 인덱싱 워커가 4 트리거(배터리/발열/슬립/앱 종료)에
// 강건하게 반응하도록 OS별 구독을 추상화. architecture §5 「일시정지 트리거」 참조.
//
// 책임:
//   * `PowerMonitor` trait — 콜백 기반 구독 인터페이스. 호출 측은 구체 OS 구현을
//     알 필요 없음.
//   * `PowerEvent` enum — 5종 이벤트(배터리 부족·발열·슬립 진입·슬립 복귀·앱 종료).
//   * OS별 impl:
//     - Linux  = `linux::UPowerMonitor`(zbus + UPower D-Bus). v0.4.2 PR 3에서 *정확
//       구현*.
//     - macOS  = `macos::IokitMonitor` *stub*. PR 3은 시그니처만, 실제 native는
//       v0.4.4 슬라이스에서.
//     - Windows = `windows::SystemEventsMonitor` *stub*. PR 3은 시그니처만.
//   * `noop::NoopMonitor` — cross-platform 폴백. 이벤트 무발생. e2e 테스트·CI용.
//
// `default_monitor()` 헬퍼는 cfg에 따라 가능한 가장 풍부한 구현을 반환:
//   * Linux  → UPowerMonitor 시도 → 실패 시 NoopMonitor.
//   * macOS  → IokitMonitor (현재 stub = 사실상 NoopMonitor) → 실패 시 NoopMonitor.
//   * Windows → SystemEventsMonitor (stub) → 실패 시 NoopMonitor.
//   * 기타 → NoopMonitor.
//
// 본 모듈은 *이벤트 발행*만 책임. 이벤트 → IndexingWorker.pause/resume 매핑은
// `commands::book` 측에서 D-081 우선순위 정책에 따라 수행.

#![allow(dead_code)]

use std::sync::Arc;

pub mod noop;
pub mod priority;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

/// 5 트리거 이벤트. architecture §5 4 트리거 + SIGTERM/atexit를 합친 5종.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// 배터리 ≤ 20% + AC 미연결 — 사용자가 노트북 전원 분리 후 외출 시나리오.
    BatteryLow,
    /// 배터리 ≥ 21% 또는 AC 연결 — BatteryLow 자동 해제 신호.
    BatteryOk,
    /// CPU 발열 임계 — pause 사유. 일정 시간 후 자동 retry.
    Thermal,
    /// OS 슬립 진입(Linux: PrepareForSleep true / macOS: NSWorkspace.willSleep).
    SleepEntering,
    /// OS wake — 미커밋 배치 재시도 + resume.
    SleepResumed,
    /// SIGTERM/atexit — graceful shutdown.
    AppQuitRequested,
}

/// 콜백 시그니처 — `PowerEvent`를 받아 호출 측 정책 결정.
pub type Callback = Arc<dyn Fn(PowerEvent) + Send + Sync + 'static>;

/// PowerMonitor — 구체 OS 구현이 무엇이든 `subscribe` 진입만 노출.
///
/// 한 모니터에 여러 콜백 등록 가능 (각 OS impl이 fan-out 책임). v0.4.2 PR 3에선
/// 단일 콜백 사용 — 다중 구독은 선택 사양.
pub trait PowerMonitor: Send + Sync {
    /// 콜백 등록. 호출 즉시 반환. 모니터가 drop되면 구독 자동 해제.
    fn subscribe(&self, callback: Callback);

    /// 디버그 라벨 — 어떤 OS 구현이 활성인지 로그·UI 진단에 사용.
    fn label(&self) -> &'static str;
}

/// cfg에 맞춰 가장 풍부한 모니터를 만든다. 실패하면 `NoopMonitor`로 폴백 (앱 startup
/// 자체는 절대 막지 않는다 — 인덱싱 강건성은 best-effort).
pub fn default_monitor() -> Box<dyn PowerMonitor> {
    #[cfg(target_os = "linux")]
    {
        match linux::UPowerMonitor::try_new() {
            Ok(m) => return Box::new(m),
            Err(e) => {
                tracing::warn!(
                    target: "power_monitor",
                    error = %e,
                    "UPower 구독 실패 — NoopMonitor 폴백"
                );
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // PR 3은 stub — 실제 native는 v0.4.4. 현재는 NoopMonitor와 동일 동작.
        return Box::new(macos::IokitMonitor::new());
    }
    #[cfg(target_os = "windows")]
    {
        return Box::new(windows::SystemEventsMonitor::new());
    }
    Box::new(noop::NoopMonitor::new())
}
