// SystemEventsMonitor (Windows) — *PR 3 stub*. 실제 native impl은 v0.4.4 슬라이스에서 본격 검증.
//
// 향후 작업:
//   * `SystemEvents.PowerModeChanged` 또는 `RegisterPowerSettingNotification`
//     — 배터리/AC + 슬립 진입/복귀.
//   * `WM_POWERBROADCAST` 메시지 — `PBT_APMRESUMESUSPEND` 등.
//   * `WM_QUERYENDSESSION` — 시스템 종료 신호 (AppQuitRequested 보강).
//
// 본 stub은 NoopMonitor와 동작이 동일. cfg-gating으로 빌드 보장만 책임.

#![allow(dead_code)]

use std::sync::Mutex;

use super::{Callback, PowerMonitor};

/// Windows 전원·시스템 이벤트 모니터 stub. 등록된 콜백은 보존만 하고 호출 X.
#[derive(Default)]
pub struct SystemEventsMonitor {
    callbacks: Mutex<Vec<Callback>>,
}

impl SystemEventsMonitor {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PowerMonitor for SystemEventsMonitor {
    fn subscribe(&self, callback: Callback) {
        self.callbacks
            .lock()
            .expect("SystemEventsMonitor mutex poisoned")
            .push(callback);
    }

    fn label(&self) -> &'static str {
        "windows-system-events-stub"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn windows_stub_does_not_invoke_callback() {
        let monitor = SystemEventsMonitor::new();
        let cb: Callback = Arc::new(|_| {
            panic!("stub은 콜백을 호출하지 않아야 함");
        });
        monitor.subscribe(cb);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
