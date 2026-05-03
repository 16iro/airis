// tracing subscriber 초기화 + 민감 정보 마스킹 함수.
// design/architecture/logging.md "Rust 측" 절을 그대로 따른다.
//
// init() 호출 1회만. 반환된 WorkerGuard는 호출자가 보관해야 한다 —
// drop 되는 순간 비동기 writer가 종료되며 버퍼가 flush 되기 때문.

use std::fs;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// 일별 회전 파일 + (debug 빌드 한정) stderr 라이브 출력.
/// 보관: 14일치. 위치: `{data_dir}/logs/airis.YYYY-MM-DD.log`.
pub fn init(data_dir: &Path) -> std::io::Result<WorkerGuard> {
    let log_dir = data_dir.join("logs");
    fs::create_dir_all(&log_dir)?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .max_log_files(14)
        .filename_prefix("airis")
        .filename_suffix("log")
        .build(&log_dir)
        .expect("rolling file appender build (validated arguments)");

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // 환경변수 RUST_LOG가 우선. 없으면 info 기본 + 자체 크레이트만 debug.
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,airis=debug"));

    let file_layer = fmt::layer().with_writer(non_blocking).with_ansi(false);

    // dev 빌드에서만 stderr로도 흘려서 `pnpm tauri dev` 콘솔에서 즉시 확인.
    let stderr_layer = if cfg!(debug_assertions) {
        Some(fmt::layer().with_writer(std::io::stderr))
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(stderr_layer)
        .init();

    Ok(guard)
}

/// API 키를 `prefix***last4` 형태로 마스킹.
/// 8자 미만 입력은 통째로 `***`로 가린다 (정상 키는 항상 8자 이상).
// PR 2 시점엔 호출지 없음 — PR 3 commands/settings·llm 에서 hot path에 박힘.
#[allow(dead_code)]
pub fn mask_api_key(key: &str) -> String {
    if key.len() < 8 {
        return "***".to_string();
    }
    // bytes 단위 슬라이스 — API 키는 ASCII만 사용한다는 가정 하 안전.
    format!("{}***{}", &key[..7], &key[key.len() - 4..])
}

/// 절대 경로의 홈 디렉토리 이하 부분을 `~/.../filename` 으로 가린다.
/// 홈 외부 경로는 파일 이름만 노출.
#[allow(dead_code)]
pub fn mask_path(p: &Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    if !home.as_os_str().is_empty() && p.starts_with(&home) {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        return format!("~/.../{name}");
    }
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn mask_api_key_keeps_prefix_and_last_four() {
        let masked = mask_api_key("sk-ant-abcdefghijkl1234");
        assert_eq!(masked, "sk-ant-***1234");
    }

    #[test]
    fn mask_api_key_short_input_is_fully_masked() {
        assert_eq!(mask_api_key("short"), "***");
        assert_eq!(mask_api_key(""), "***");
    }

    #[test]
    fn mask_path_under_home_is_collapsed() {
        let home = dirs::home_dir().expect("home_dir on test platform");
        let p = home.join("books/rust.md");
        assert_eq!(mask_path(&p), "~/.../rust.md");
    }

    #[test]
    fn mask_path_outside_home_keeps_only_filename() {
        let p = PathBuf::from("/var/log/airis.log");
        assert_eq!(mask_path(&p), "airis.log");
    }
}
