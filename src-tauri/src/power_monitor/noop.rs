// NoopMonitor — 이벤트 없는 폴백. cross-platform.
//
// 사용처:
//   * unit/integration 테스트 (실제 D-Bus·IOKit 비의존).
//   * UPower 미설치 / Linux headless desktop 환경.
//   * macOS·Windows native impl 빌드 실패 시 startup 보장.
//
// 시간이 흘러도 callback이 호출되지 않음을 보장 — 검증은 본 모듈 단위 테스트.

#![allow(dead_code)]

use std::sync::Mutex;

use super::{Callback, PowerMonitor};

/// 이벤트를 발행하지 않는 모니터. 등록된 콜백은 보존만 하고 호출 X.
#[derive(Default)]
pub struct NoopMonitor {
    /// 보존만 — 디버그 카운트·테스트 검증.
    callbacks: Mutex<Vec<Callback>>,
}

impl NoopMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 등록된 콜백 수 — 단위 테스트 검증용.
    pub fn callback_count(&self) -> usize {
        self.callbacks
            .lock()
            .expect("NoopMonitor mutex poisoned")
            .len()
    }
}

impl PowerMonitor for NoopMonitor {
    fn subscribe(&self, callback: Callback) {
        self.callbacks
            .lock()
            .expect("NoopMonitor mutex poisoned")
            .push(callback);
    }

    fn label(&self) -> &'static str {
        "noop"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::power_monitor::PowerEvent;

    #[test]
    fn noop_monitor_does_not_invoke_callback() {
        let monitor = NoopMonitor::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_cb = counter.clone();
        let cb: Callback = Arc::new(move |_ev: PowerEvent| {
            counter_for_cb.fetch_add(1, Ordering::SeqCst);
        });
        monitor.subscribe(cb);
        // 등록은 됐지만 이벤트는 발행되지 않음.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert_eq!(monitor.callback_count(), 1);
    }

    #[test]
    fn noop_monitor_label_is_stable() {
        let monitor = NoopMonitor::new();
        assert_eq!(monitor.label(), "noop");
    }

    #[test]
    fn multiple_subscribers_all_held() {
        let monitor = NoopMonitor::new();
        for _ in 0..3 {
            let cb: Callback = Arc::new(|_| {});
            monitor.subscribe(cb);
        }
        assert_eq!(monitor.callback_count(), 3);
    }
}
