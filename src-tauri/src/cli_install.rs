// PR 24 (D-066) — npm i/update 래퍼 + 패키지 메타.
//
// 각 프로바이더 → npm 패키지 매핑. install/update는 모두 airis 전용 prefix를 사용해
// 시스템 Node 영역을 건드리지 않는다 (sudo 회피).

use std::path::PathBuf;

use serde::Serialize;
use tokio::process::Command;
use tracing::{info, warn};

use crate::error::{AppError, AppResult};
use crate::runtime;
use crate::settings::Provider;

/// 한 프로바이더에 매칭되는 CLI 메타.
pub struct CliPkg {
    /// npm registry 패키지 이름.
    pub package: &'static str,
    /// 설치 후 PATH/prefix bin/에 노출되는 실행 파일 이름.
    pub binary: &'static str,
}

impl CliPkg {
    pub fn for_provider(provider: Provider) -> Self {
        match provider {
            Provider::Anthropic => CliPkg {
                package: "@anthropic-ai/claude-code",
                binary: "claude",
            },
            Provider::Gemini => CliPkg {
                package: "@google/gemini-cli",
                binary: "gemini",
            },
            Provider::Openai => CliPkg {
                package: "@openai/codex",
                binary: "codex",
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CliStatus {
    pub provider: String,
    pub installed: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
}

/// CLI 현재 상태 점검 — airis prefix와 시스템 PATH 둘 다 검사.
pub async fn status(provider: Provider) -> AppResult<CliStatus> {
    let pkg = CliPkg::for_provider(provider);
    let binary = locate_binary(pkg.binary)?;
    if let Some(path) = binary {
        let version = runtime::version_of(&path).await.ok();
        Ok(CliStatus {
            provider: provider.as_str().to_string(),
            installed: true,
            binary_path: Some(path.display().to_string()),
            version,
        })
    } else {
        Ok(CliStatus {
            provider: provider.as_str().to_string(),
            installed: false,
            binary_path: None,
            version: None,
        })
    }
}

/// airis prefix bin/ 우선, 없으면 시스템 PATH에서 fallback.
/// 사용자가 npm으로 직접 깐 CLI(예: nvm 환경의 `claude`)도 인정.
pub fn locate_binary(name: &str) -> AppResult<Option<PathBuf>> {
    let prefix = runtime::airis_npm_prefix()?;
    let in_prefix = runtime::cli_binary_path(&prefix, name);
    if in_prefix.is_file() {
        return Ok(Some(in_prefix));
    }
    Ok(runtime::which(name))
}

/// `npm install -g --prefix=<airis> <package>` (또는 @latest로 강제 업데이트).
/// stdout/stderr는 tracing으로 흘림 — UI는 진행 시작/끝만 이벤트로 받음.
pub async fn install(provider: Provider, force_latest: bool) -> AppResult<()> {
    let pkg = CliPkg::for_provider(provider);
    let prefix = runtime::airis_npm_prefix()?;
    let npm = runtime::which("npm").ok_or_else(|| AppError::NodeMissing {
        message: "npm을 찾을 수 없습니다".into(),
    })?;
    let spec = if force_latest {
        format!("{}@latest", pkg.package)
    } else {
        pkg.package.to_string()
    };

    info!(
        target: "cli_install",
        provider = provider.as_str(),
        package = pkg.package,
        prefix = %prefix.display(),
        force_latest,
        "npm install start"
    );

    let output = Command::new(&npm)
        .args([
            "install",
            "-g",
            "--prefix",
            prefix.to_str().ok_or_else(|| AppError::Internal {
                message: "prefix path utf8 conversion 실패".into(),
            })?,
            &spec,
        ])
        .env("npm_config_prefix", &prefix)
        .output()
        .await
        .map_err(|e| AppError::CliRuntime {
            message: format!("npm install spawn 실패: {e}"),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        warn!(
            target: "cli_install",
            provider = provider.as_str(),
            stdout = %stdout,
            stderr = %stderr,
            code = ?output.status.code(),
            "npm install failed"
        );
        return Err(AppError::CliRuntime {
            message: format!(
                "npm install 실패 (exit {:?}): {}",
                output.status.code(),
                stderr.lines().next().unwrap_or("").trim()
            ),
        });
    }

    info!(
        target: "cli_install",
        provider = provider.as_str(),
        "npm install done"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkg_for_provider_matches_expected() {
        let claude = CliPkg::for_provider(Provider::Anthropic);
        assert_eq!(claude.package, "@anthropic-ai/claude-code");
        assert_eq!(claude.binary, "claude");

        let gemini = CliPkg::for_provider(Provider::Gemini);
        assert_eq!(gemini.package, "@google/gemini-cli");
        assert_eq!(gemini.binary, "gemini");

        let codex = CliPkg::for_provider(Provider::Openai);
        assert_eq!(codex.package, "@openai/codex");
        assert_eq!(codex.binary, "codex");
    }
}
