// IokitMonitor (macOS) — *PR 3 stub*. 실제 native impl은 v0.4.4 슬라이스에서 본격 검증.
//
// 향후 작업:
//   * `IOPSNotificationCreateRunLoopSource` — 배터리/AC 변화.
//   * `NSWorkspace.willSleepNotification` / `didWakeNotification` — 슬립 진입/복귀.
//   * `NSProcessInfo.thermalStateDidChange` — 발열 상태(Thermal).
//
// 본 stub은 NoopMonitor와 동작이 동일. cfg-gating으로 빌드 보장만 책임.
//
// Linux는 zbus를 통해 PR 3에서 *정확* 구현 — Linux를 베이스로 매뉴얼 검증.

#![allow(dead_code)]

use std::sync::Mutex;

use super::{Callback, PowerMonitor};

/// macOS 전원·시스템 이벤트 모니터 stub. 등록된 콜백은 보존만 하고 호출 X.
#[derive(Default)]
pub struct IokitMonitor {
    callbacks: Mutex<Vec<Callback>>,
}

impl IokitMonitor {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PowerMonitor for IokitMonitor {
    fn subscribe(&self, callback: Callback) {
        self.callbacks
            .lock()
            .expect("IokitMonitor mutex poisoned")
            .push(callback);
    }

    fn label(&self) -> &'static str {
        "macos-iokit-stub"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn macos_stub_does_not_invoke_callback() {
        let monitor = IokitMonitor::new();
        let cb: Callback = Arc::new(|_| {
            panic!("stub은 콜백을 호출하지 않아야 함");
        });
        monitor.subscribe(cb);
        // 시간이 흘러도 panic이 안 일어나야 한다.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
