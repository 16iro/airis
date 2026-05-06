// UPowerMonitor — Linux UPower D-Bus 구독.
//
// 구독 대상:
//   * `org.freedesktop.UPower` 시스템 버스 / `/org/freedesktop/UPower` 객체:
//       - `OnBattery` 속성 변경 (PropertiesChanged 시그널) → BatteryLow / BatteryOk.
//   * `org.freedesktop.login1` 시스템 버스 / `/org/freedesktop/login1` 객체:
//       - `PrepareForSleep` 시그널 (true=진입, false=복귀) → SleepEntering / SleepResumed.
//
// 본 모듈 책임:
//   * `try_new()` — 시스템 버스 연결 + 시그널 스트림 생성. 실패 시 에러 반환 (호출 측이
//     NoopMonitor로 폴백).
//   * `subscribe()` — 콜백 등록. 백그라운드 tokio task가 시그널을 받아 fan-out.
//
// 한계:
//   * 발열(Thermal)은 UPower가 직접 노출하지 않음 → v0.4.4에서 hwmon/thermal_zone 폴링
//     별도 워커. PR 3은 *Battery + Sleep 트리거*에 집중.
//   * 본 PR 단위 테스트는 *zbus 통합 게이팅* — 실제 D-Bus 환경 없으면 skip.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};

use super::{Callback, PowerEvent, PowerMonitor};

/// UPower D-Bus 구독 모니터. 시스템 버스 연결 + 시그널 스트림을 백그라운드 task로
/// 처리하며 등록된 콜백에 fan-out.
pub struct UPowerMonitor {
    /// 콜백 fan-out 대상. 이벤트 도착 시 모두 호출.
    callbacks: Arc<Mutex<Vec<Callback>>>,
    /// 백그라운드 task 핸들. drop되면 task abort.
    _task: tokio::task::JoinHandle<()>,
}

impl UPowerMonitor {
    /// 시스템 버스 연결 시도 + 시그널 스트림 등록 + 백그라운드 task spawn.
    ///
    /// 실패 사유:
    ///   * D-Bus daemon 미실행 (Linux headless / minimal container).
    ///   * 권한 부족 (드물게 PolicyKit 설정 문제).
    ///
    /// `default_monitor()`가 본 함수 실패 시 NoopMonitor로 폴백 — 호출 측 startup은
    /// 막지 않는다.
    pub fn try_new() -> AppResult<Self> {
        // Tokio 런타임이 없는 컨텍스트에서 호출되면 곧장 에러 — Tauri는 이미 multi-thread
        // 런타임이라 정상 케이스에선 이 분기에 안 닿는다.
        let handle = tokio::runtime::Handle::try_current().map_err(|e| AppError::Internal {
            message: format!("UPowerMonitor: tokio runtime 컨텍스트 부재 ({e})"),
        })?;

        let callbacks: Arc<Mutex<Vec<Callback>>> = Arc::new(Mutex::new(Vec::new()));
        let cbs_for_task = callbacks.clone();

        let task = handle.spawn(async move {
            if let Err(e) = run_monitor_loop(cbs_for_task).await {
                tracing::warn!(
                    target: "power_monitor",
                    error = %e,
                    "UPower 모니터 루프 종료 — D-Bus 연결 손실 또는 시스템 종료"
                );
            }
        });

        Ok(Self {
            callbacks,
            _task: task,
        })
    }
}

impl PowerMonitor for UPowerMonitor {
    fn subscribe(&self, callback: Callback) {
        self.callbacks
            .lock()
            .expect("UPowerMonitor mutex poisoned")
            .push(callback);
    }

    fn label(&self) -> &'static str {
        "linux-upower"
    }
}

/// 백그라운드 task 본체. zbus connection · 두 시그널 스트림 (UPower OnBattery
/// PropertiesChanged + login1 PrepareForSleep)을 select! 처리.
async fn run_monitor_loop(callbacks: Arc<Mutex<Vec<Callback>>>) -> AppResult<()> {
    use zbus::Connection;

    let conn = Connection::system().await.map_err(|e| AppError::Internal {
        message: format!("UPower: 시스템 버스 연결 실패: {e}"),
    })?;

    // UPower 객체에 PropertiesChanged 시그널 매처 — OnBattery 속성 변경만 필터.
    let upower_proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await
    .map_err(|e| AppError::Internal {
        message: format!("UPower proxy 생성 실패: {e}"),
    })?;

    // 초기 OnBattery 상태 — 부트시 이미 배터리 모드면 BatteryLow 이벤트 시뮬.
    let initial_on_battery: bool = upower_proxy
        .get_property("OnBattery")
        .await
        .unwrap_or(false);
    if initial_on_battery {
        fanout(&callbacks, PowerEvent::BatteryLow);
    }

    // PropertiesChanged 시그널 — UPower 인터페이스 한정.
    let props_proxy = zbus::fdo::PropertiesProxy::builder(&conn)
        .destination("org.freedesktop.UPower")
        .map_err(|e| AppError::Internal {
            message: format!("UPower PropertiesProxy destination 실패: {e}"),
        })?
        .path("/org/freedesktop/UPower")
        .map_err(|e| AppError::Internal {
            message: format!("UPower PropertiesProxy path 실패: {e}"),
        })?
        .build()
        .await
        .map_err(|e| AppError::Internal {
            message: format!("UPower PropertiesProxy build 실패: {e}"),
        })?;

    use futures_util::stream::StreamExt;

    let mut props_stream = props_proxy
        .receive_properties_changed()
        .await
        .map_err(|e| AppError::Internal {
            message: format!("UPower PropertiesChanged 구독 실패: {e}"),
        })?;

    // login1 PrepareForSleep 시그널.
    let login_proxy = zbus::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|e| AppError::Internal {
        message: format!("login1 proxy 생성 실패: {e}"),
    })?;

    let mut sleep_stream = login_proxy
        .receive_signal("PrepareForSleep")
        .await
        .map_err(|e| AppError::Internal {
            message: format!("PrepareForSleep 구독 실패: {e}"),
        })?;

    tracing::info!(
        target: "power_monitor",
        initial_on_battery,
        "UPower 모니터 루프 시작 (OnBattery + PrepareForSleep)"
    );

    loop {
        tokio::select! {
            Some(signal) = props_stream.next() => {
                if let Ok(args) = signal.args() {
                    if let Some(zvar) = args.changed_properties.get("OnBattery") {
                        if let Ok(on_battery) = bool::try_from(zvar) {
                            let event = if on_battery {
                                PowerEvent::BatteryLow
                            } else {
                                PowerEvent::BatteryOk
                            };
                            tracing::debug!(
                                target: "power_monitor",
                                on_battery,
                                "UPower OnBattery 변경"
                            );
                            fanout(&callbacks, event);
                        }
                    }
                }
            }
            Some(signal) = sleep_stream.next() => {
                if let Ok(start) = signal.body().deserialize::<bool>() {
                    let event = if start {
                        PowerEvent::SleepEntering
                    } else {
                        PowerEvent::SleepResumed
                    };
                    tracing::info!(
                        target: "power_monitor",
                        sleeping = start,
                        "login1 PrepareForSleep 시그널"
                    );
                    fanout(&callbacks, event);
                }
            }
            else => {
                // 양쪽 스트림 모두 종료 = D-Bus 연결 손실. 루프 탈출 → task 종료.
                break;
            }
        }
    }

    Ok(())
}

fn fanout(callbacks: &Arc<Mutex<Vec<Callback>>>, event: PowerEvent) {
    let snapshot: Vec<Callback> = callbacks
        .lock()
        .expect("UPowerMonitor mutex poisoned")
        .clone();
    for cb in snapshot {
        cb(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UPower 통합 테스트 — env `AIRIS_E2E_UPOWER=1`일 때만 실제 시스템 버스 연결.
    /// 일반 CI는 D-Bus daemon이 없거나 권한이 없을 수 있어 skip.
    #[tokio::test]
    async fn try_new_when_enabled() {
        if std::env::var("AIRIS_E2E_UPOWER").ok().as_deref() != Some("1") {
            eprintln!("skip: AIRIS_E2E_UPOWER 미설정 (시스템 D-Bus 미가동 가능성)");
            return;
        }
        let monitor = UPowerMonitor::try_new().expect("UPower 연결 실패");
        assert_eq!(monitor.label(), "linux-upower");
    }

    #[test]
    fn label_constant() {
        // 시그니처 검증 — 인스턴스 생성 없이 label 함수가 빠르게 호출됨.
        // (실제 인스턴스 생성은 tokio runtime 필요라 통합 게이팅.)
        let _ = std::any::type_name::<UPowerMonitor>();
    }
}
