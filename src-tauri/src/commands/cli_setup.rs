// PR 24 (D-066) — CLI 인증 흐름 Tauri 커맨드.
//
// 프론트가 CliSetupDialog에서 호출하는 명령:
// - cli_runtime_detect       Node/npm 존재·버전
// - cli_status               개별 프로바이더 CLI 설치 상태
// - cli_install              npm install -g (또는 @latest) 실행
// - cli_auth_status          `claude auth status` 등 — JSON 파싱해서 로그인/구독 상태 노출
// - cli_login                `claude auth login` 류 spawn — 사용자 브라우저 OAuth 흐름
//
// 각 명령은 자기 완결적이라 진행률 이벤트는 없다 — npm install이 길어질 수 있어 프론트는 *대기 UI*로 처리.

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::process::Command;
use tracing::{info, warn};

use crate::cli_install::{self, CliPkg, CliStatus};
use crate::error::{AppError, AppResult};
use crate::runtime::{self, RuntimeInfo};
use crate::settings::{self, Provider};
use crate::AppState;

#[tauri::command]
pub async fn cli_runtime_detect() -> AppResult<RuntimeInfo> {
    runtime::detect().await
}

#[tauri::command]
pub async fn cli_status(provider: Provider) -> AppResult<CliStatus> {
    cli_install::status(provider).await
}

#[tauri::command]
pub async fn cli_install_provider(
    state: State<'_, AppState>,
    provider: Provider,
    force_latest: bool,
) -> AppResult<CliStatus> {
    // detect — 없으면 NodeMissing.
    runtime::detect().await?;
    cli_install::install(provider, force_latest).await?;
    let status = cli_install::status(provider).await?;
    if let Some(version) = &status.version {
        persist_cli_version(&state, provider, version)?;
    }
    Ok(status)
}

/// Claude Code의 `claude auth status` JSON 파싱.
/// Gemini/Codex는 PR 25/26에서 별도 cli_auth_status_* 커맨드 추가.
#[tauri::command]
pub async fn cli_auth_status_claude() -> AppResult<ClaudeAuthInfo> {
    let bin = cli_install::locate_binary("claude")?.ok_or_else(|| AppError::CliMissing {
        provider: "anthropic".into(),
    })?;

    let output = Command::new(&bin)
        .arg("auth")
        .arg("status")
        .output()
        .await
        .map_err(|e| AppError::CliRuntime {
            message: format!("claude auth status spawn 실패: {e}"),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(ClaudeAuthInfo {
            logged_in: false,
            auth_method: None,
            subscription_type: None,
            email: None,
        });
    }

    let parsed: ClaudeAuthStatusRaw = serde_json::from_str(&stdout).map_err(|e| {
        warn!(target: "cli_setup", error = %e, stdout = %stdout, "auth status JSON 파싱 실패");
        AppError::CliRuntime {
            message: format!("claude auth status JSON 파싱 실패: {e}"),
        }
    })?;

    Ok(ClaudeAuthInfo {
        logged_in: parsed.logged_in.unwrap_or(false),
        auth_method: parsed.auth_method,
        subscription_type: parsed.subscription_type,
        email: parsed.email,
    })
}

/// `<cli> auth login` 또는 동등한 OAuth 트리거를 spawn.
/// - claude: `claude auth login` (브라우저 자동 오픈)
/// - 그 외 프로바이더는 PR 25/26에서 분기 추가.
///
/// 명령은 *블로킹*이다 — 사용자가 OAuth 마치고 CLI가 종료할 때까지 대기.
/// 프론트는 별도 spinner를 띄우거나, `console: true` 옵션을 줘 비대화형 인증 토큰 기반으로 변경 검토.
#[tauri::command]
pub async fn cli_login(provider: Provider, console: bool) -> AppResult<()> {
    let pkg = CliPkg::for_provider(provider);
    let bin = cli_install::locate_binary(pkg.binary)?.ok_or_else(|| AppError::CliMissing {
        provider: provider.as_str().into(),
    })?;

    let mut cmd = Command::new(&bin);
    match provider {
        Provider::Anthropic => {
            cmd.arg("auth").arg("login");
            if console {
                cmd.arg("--console");
            }
        }
        Provider::Gemini | Provider::Openai => {
            // PR 25/26에서 각자 어울리는 인자로 채움 — 우선 NotImplemented처럼 안내.
            return Err(AppError::CliRuntime {
                message: format!("{} CLI 로그인은 PR 25/26에서 추가됩니다", provider.as_str()),
            });
        }
    }

    info!(
        target: "cli_setup",
        provider = provider.as_str(),
        binary = %bin.display(),
        console,
        "spawn cli login"
    );

    let status = cmd.status().await.map_err(|e| AppError::CliRuntime {
        message: format!("CLI login spawn 실패: {e}"),
    })?;

    if !status.success() {
        return Err(AppError::CliRuntime {
            message: format!("CLI login 종료 코드 {:?}", status.code()),
        });
    }
    Ok(())
}

fn persist_cli_version(state: &AppState, provider: Provider, version: &str) -> AppResult<()> {
    let mut settings = state.settings.lock().expect("settings mutex");
    settings
        .cli_versions
        .insert(provider.as_str().to_string(), version.to_string());
    settings::write(&state.settings_path, &settings)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ClaudeAuthStatusRaw {
    #[serde(rename = "loggedIn", default)]
    logged_in: Option<bool>,
    #[serde(rename = "authMethod", default)]
    auth_method: Option<String>,
    #[serde(rename = "subscriptionType", default)]
    subscription_type: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClaudeAuthInfo {
    pub logged_in: bool,
    pub auth_method: Option<String>,
    pub subscription_type: Option<String>,
    pub email: Option<String>,
}
