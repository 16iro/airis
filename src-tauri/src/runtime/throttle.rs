// v0.4.2 PR 5 (D-083) — 백그라운드 빌드 자원 제한.
//
// 결정 (decision-log D-083):
//   * OS-level priority + ONNX intra_op_threads 절반 (T2 빌드만 적용).
//   * Linux/macOS = `libc::setpriority(PRIO_PROCESS, 0, 10)` (nice 10).
//   * Windows = `windows::Win32::System::Threading::SetPriorityClass(IDLE_PRIORITY_CLASS)`.
//   * fastembed 5.13.4의 `InitOptions`는 thread 수 제어 메서드가 없음 — try_new에서
//     `available_parallelism()`로 항상 전체 코어를 잡는다 (`text_embedding/impl.rs:52`).
//     env `OMP_NUM_THREADS` / `ORT_NUM_INTRA_OP_THREADS`는 ONNX 런타임이 *모델 로드 전*
//     읽으면 honor — best-effort로 모델 로드 직전에 세팅. 효과 검증은 사용자 1주 dev
//     검증에서 응답 시간 ≤+50% gate 3 통과 여부로 본다.
//
// 한계:
//   * Linux nice 10은 *프로세스 전체*에 적용. T2 인덱싱 외 chat / UI / DB 모두 영향.
//     실제로는 chat 응답이 *진행 중 인덱싱과 동등 우선순위*가 되므로 OK — chat은
//     fastembed BGE-M3과 별개 thread + IO bound. 인덱싱이 우선순위 ↓되면 chat이 자연
//     선점.
//   * Windows IDLE_PRIORITY_CLASS는 더 강한 throttling. cooperative pause와 합치면
//     사실상 chat 응답이 즉시 우선.
//   * macOS Mach setpriority(libc 동등)는 nice 동작 OS에 따라 thread 단위가 아닐 수
//     있으나 BSD 호환층이 PRIO_PROCESS를 인식. v0.4.2 1주 검증에서 사용자 명시.
//
// 호출 패턴:
//   * `start_t2_build` 진입 시 `set_low_priority()` → 작업 끝나면 `restore_normal_priority()`.
//   * 실패는 *non-fatal* — priority 변경 실패해도 인덱싱 자체는 진행. log warn만.
//   * thread 절반 제어는 `apply_low_thread_hint()` — env 변수 시도. 이미 설정된 값
//     덮어쓰지 않음 (사용자 명시 OMP_NUM_THREADS 보존).

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::AppResult;

/// nice 값 — `setpriority(PRIO_PROCESS, 0, NICE_LOW)`. 클수록 낮은 우선순위.
/// 0=normal, 19=가장 낮은 사용자 우선순위. architecture §5는 10 권고.
const NICE_LOW: i32 = 10;
const NICE_NORMAL: i32 = 0;

/// 두 번째 set_low_priority 호출이 이미 적용된 상태에서 또 호출되었는지 감지.
/// idempotent 보장: 두 번째 호출도 실패하지 않지만 *불필요한 syscall* 회피.
static LOW_PRIORITY_APPLIED: AtomicBool = AtomicBool::new(false);

/// T2 빌드용 *낮은 priority* 적용. 실패는 log warn 후 Ok(()) — non-fatal.
///
/// 이미 적용된 상태(=LOW_PRIORITY_APPLIED true)면 syscall skip + Ok(()).
/// `restore_normal_priority`로 명시 해제 후 다시 호출 가능.
pub fn set_low_priority() -> AppResult<()> {
    if LOW_PRIORITY_APPLIED.swap(true, Ordering::SeqCst) {
        // 이미 적용 — 중복 호출 idempotent.
        return Ok(());
    }
    apply_low_priority_impl();
    Ok(())
}

/// 정상 priority로 복귀. T2 빌드 완료/취소 시 호출.
pub fn restore_normal_priority() -> AppResult<()> {
    if !LOW_PRIORITY_APPLIED.swap(false, Ordering::SeqCst) {
        // 이미 정상 — 중복 호출 idempotent.
        return Ok(());
    }
    apply_normal_priority_impl();
    Ok(())
}

/// fastembed `try_new`가 모델 로드 시 호출하는 ort `Session::with_intra_threads`는
/// `std::thread::available_parallelism()` 결과를 그대로 사용 (5.13.4 fixed). 직접
/// 옵션이 없으니 OS env로 best-effort hint:
///   * `OMP_NUM_THREADS` — OpenMP/ONNX 런타임이 인식.
///   * `ORT_NUM_INTRA_OP_THREADS` — onnxruntime 1.17+ 인식.
///
/// 이미 환경에 설정돼 있으면 *덮어쓰지 않음*. 사용자가 명시한 값을 우선.
/// 본 함수는 `EmbedderT2::new` 직전에 호출 — 이후 process 수명 내 영향.
pub fn apply_low_thread_hint() {
    let cores = num_cpus::get().max(1);
    // 절반(올림 X — 단순 / 2). 최소 1.
    let target = (cores / 2).max(1);
    let target_str = target.to_string();
    for var in ["OMP_NUM_THREADS", "ORT_NUM_INTRA_OP_THREADS"] {
        if std::env::var_os(var).is_none() {
            // SAFETY: process-level env mutation은 multithread 환경에서 race 가능.
            // 현재 호출 순간은 모델 로드 *전* — fastembed가 ONNX 런타임을 init 하기
            // 전이라 안전. 한 번 set 후 재호출 X (apply_normal_thread_hint도 unset).
            std::env::set_var(var, &target_str);
        }
    }
    tracing::info!(
        target: "throttle",
        cores,
        target,
        "ONNX intra_op_threads hint set (best-effort env vars)"
    );
}

/// 평소 thread hint로 복귀. 본 모듈이 명시 set한 OMP_NUM_THREADS만 unset.
/// 사용자가 자체 set한 값은 *건드리지 않음* (apply_low_thread_hint 패턴).
pub fn clear_low_thread_hint() {
    // env 단방향 — set한 게 *우리*인지 사용자인지 구분 불가능. v0.4.2는 단순화 위해
    // unset *하지 않음*. 다음 T2 빌드에 동일 값 재set은 no-op (var_os 존재). 사용자가
    // 명시 변경하려면 process restart.
    tracing::debug!(
        target: "throttle",
        "thread hint는 process 수명 내 유지 (env 단방향 단순화)"
    );
}

// ---- OS별 구현 ------------------------------------------------------------

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn apply_low_priority_impl() {
    // libc::setpriority(PRIO_PROCESS, 0=current, NICE_LOW).
    // 반환값 0=성공, -1=실패. errno로 진단 가능하지만 v0.4.2는 단순화 — log only.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, NICE_LOW) };
    if rc != 0 {
        tracing::warn!(
            target: "throttle",
            errno = std::io::Error::last_os_error().to_string(),
            "setpriority(NICE_LOW) 실패 — non-fatal"
        );
    } else {
        tracing::info!(
            target: "throttle",
            nice = NICE_LOW,
            "백그라운드 priority 적용 (Linux/macOS nice)"
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn apply_normal_priority_impl() {
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, NICE_NORMAL) };
    if rc != 0 {
        tracing::warn!(
            target: "throttle",
            errno = std::io::Error::last_os_error().to_string(),
            "setpriority(NICE_NORMAL) 실패 — non-fatal"
        );
    } else {
        tracing::info!(
            target: "throttle",
            nice = NICE_NORMAL,
            "정상 priority 복귀 (Linux/macOS nice)"
        );
    }
}

#[cfg(target_os = "windows")]
fn apply_low_priority_impl() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, IDLE_PRIORITY_CLASS,
    };
    let handle = unsafe { GetCurrentProcess() };
    // SAFETY: GetCurrentProcess는 pseudo handle 반환 — close 불필요. SetPriorityClass는
    // 본 process에 idle priority 적용. 실패 시 log warn — non-fatal.
    let r = unsafe { SetPriorityClass(handle, IDLE_PRIORITY_CLASS) };
    if r.is_err() {
        tracing::warn!(
            target: "throttle",
            error = ?r.err(),
            "SetPriorityClass(IDLE_PRIORITY_CLASS) 실패 — non-fatal"
        );
    } else {
        tracing::info!(
            target: "throttle",
            "백그라운드 priority 적용 (Windows IDLE_PRIORITY_CLASS)"
        );
    }
}

#[cfg(target_os = "windows")]
fn apply_normal_priority_impl() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, NORMAL_PRIORITY_CLASS,
    };
    let handle = unsafe { GetCurrentProcess() };
    let r = unsafe { SetPriorityClass(handle, NORMAL_PRIORITY_CLASS) };
    if r.is_err() {
        tracing::warn!(
            target: "throttle",
            error = ?r.err(),
            "SetPriorityClass(NORMAL_PRIORITY_CLASS) 실패 — non-fatal"
        );
    } else {
        tracing::info!(
            target: "throttle",
            "정상 priority 복귀 (Windows NORMAL_PRIORITY_CLASS)"
        );
    }
}

// 미지원 OS: 호출은 성공하지만 noop. v0.4.2 영역 외 (다양화는 v0.4.4).
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn apply_low_priority_impl() {
    tracing::warn!(
        target: "throttle",
        os = std::env::consts::OS,
        "throttle 미지원 OS — set_low_priority noop"
    );
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn apply_normal_priority_impl() {
    tracing::debug!(
        target: "throttle",
        os = std::env::consts::OS,
        "throttle 미지원 OS — restore_normal_priority noop"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip — set_low_priority 후 restore_normal_priority가 idempotent에 성공.
    /// 실제 nice 값은 OS 의존이라 호출 자체가 panic 없이 완주하는지만 검증.
    #[test]
    fn priority_round_trip_completes_without_panic() {
        // 직전 테스트가 LOW_PRIORITY_APPLIED를 남겨놨을 수도 있어 명시 reset.
        let _ = restore_normal_priority();

        set_low_priority().expect("set_low_priority Ok");
        // 두 번째 호출 idempotent.
        set_low_priority().expect("set_low_priority idempotent Ok");
        assert!(LOW_PRIORITY_APPLIED.load(Ordering::SeqCst));

        restore_normal_priority().expect("restore_normal_priority Ok");
        // 두 번째 호출 idempotent.
        restore_normal_priority().expect("restore_normal_priority idempotent Ok");
        assert!(!LOW_PRIORITY_APPLIED.load(Ordering::SeqCst));
    }

    #[test]
    fn apply_low_thread_hint_does_not_overwrite_user_value() {
        // 사용자가 OMP_NUM_THREADS=8을 명시했다고 가정.
        std::env::set_var("OMP_NUM_THREADS", "8");
        apply_low_thread_hint();
        assert_eq!(std::env::var("OMP_NUM_THREADS").unwrap(), "8");
        // ORT_NUM_INTRA_OP_THREADS는 우리가 set한 값(=cores/2)이어야 한다면.
        // 다만 다른 테스트가 먼저 set 했을 수도 있으니 *존재*만 검증.
        assert!(std::env::var_os("ORT_NUM_INTRA_OP_THREADS").is_some());
        // 정리 — 다른 테스트에 영향 X.
        std::env::remove_var("OMP_NUM_THREADS");
    }

    #[test]
    fn nice_low_is_positive_per_arch_section_5() {
        // architecture §5 권고는 nice 10. 변경 시 본 테스트가 가드.
        assert_eq!(NICE_LOW, 10);
        assert_eq!(NICE_NORMAL, 0);
    }
}
