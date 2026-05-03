// PR 24 (D-066) — Node·npm 런타임 감지 + airis 전용 npm prefix 관리.
//
// 왜 별도 prefix? 시스템 Node에서 `npm install -g`가 sudo를 요구하는 경우가 있음.
// `~/.airis/npm`을 prefix로 강제하면 사용자 영역에 격리되어 sudo 불필요.
// PATH 추가는 *시스템에 안 함* — 자식 프로세스 spawn 시 PATH 환경변수에만 prepend.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::process::Command;

use crate::error::{AppError, AppResult};

/// 감지된 런타임 정보. UI에 노출하면 사용자가 자신의 환경 점검 가능.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeInfo {
    pub node_path: String,
    pub node_version: String,
    pub npm_path: String,
    pub npm_version: String,
}

/// `which` 셸 명령으로 PATH에서 바이너리 찾기. 못 찾으면 None.
/// (which crate 추가 회피 — 의존성 최소화)
pub fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Windows .cmd / .exe 보조.
        if cfg!(windows) {
            for ext in ["cmd", "exe", "bat"] {
                let candidate_ext = dir.join(format!("{name}.{ext}"));
                if candidate_ext.is_file() {
                    return Some(candidate_ext);
                }
            }
        }
    }
    None
}

/// `<binary> --version` 호출. stdout 첫 줄 trim.
pub async fn version_of(binary: &Path) -> AppResult<String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .await
        .map_err(|e| AppError::CliRuntime {
            message: format!("{} --version: {e}", binary.display()),
        })?;
    if !output.status.success() {
        return Err(AppError::CliRuntime {
            message: format!(
                "{} --version exit {:?}",
                binary.display(),
                output.status.code()
            ),
        });
    }
    let s = String::from_utf8_lossy(&output.stdout);
    Ok(s.trim().to_string())
}

/// 런타임 감지. node·npm 둘 다 PATH에 있어야 OK.
pub async fn detect() -> AppResult<RuntimeInfo> {
    let node = which("node").ok_or_else(|| AppError::NodeMissing {
        message: "PATH에서 'node' 실행 파일을 찾을 수 없습니다".into(),
    })?;
    let npm = which("npm").ok_or_else(|| AppError::NodeMissing {
        message: "PATH에서 'npm' 실행 파일을 찾을 수 없습니다".into(),
    })?;
    let node_version = version_of(&node).await?;
    let npm_version = version_of(&npm).await?;
    Ok(RuntimeInfo {
        node_path: node.display().to_string(),
        node_version,
        npm_path: npm.display().to_string(),
        npm_version,
    })
}

/// airis 전용 npm prefix 디렉토리. `~/.airis/npm`.
/// 미존재 시 생성. (sudo 회피의 핵심)
pub fn airis_npm_prefix() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Internal {
        message: "home_dir 확인 실패".into(),
    })?;
    let prefix = home.join(".airis").join("npm");
    std::fs::create_dir_all(&prefix)?;
    Ok(prefix)
}

/// prefix 안에서 `<cli_name>` 바이너리의 절대 경로.
/// Linux/macOS: `<prefix>/bin/<name>`, Windows: `<prefix>/<name>.cmd`.
pub fn cli_binary_path(prefix: &Path, cli_name: &str) -> PathBuf {
    if cfg!(windows) {
        prefix.join(format!("{cli_name}.cmd"))
    } else {
        prefix.join("bin").join(cli_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_binary_path_unix() {
        let path = cli_binary_path(Path::new("/tmp/airis-npm"), "claude");
        if cfg!(windows) {
            assert!(path.ends_with("claude.cmd"));
        } else {
            assert!(path.ends_with("bin/claude"));
        }
    }

    #[test]
    fn airis_npm_prefix_creates_directory() {
        // 기존 디렉토리가 있어도 idempotent — 두 번 호출해도 OK.
        let p1 = airis_npm_prefix().unwrap();
        let p2 = airis_npm_prefix().unwrap();
        assert_eq!(p1, p2);
        assert!(p1.is_dir());
    }
}
