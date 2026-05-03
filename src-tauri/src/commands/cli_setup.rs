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
/// - gemini/openai: 별도 비대화형 login 명령이 미흡 → 외부 터미널 명령 안내로 대체.
///   프론트가 `cli_login` 호출 결과 `CliLoginInstruction`을 받으면 안내 다이얼로그를 띄움.
///
/// 명령은 *블로킹*이다 — 사용자가 OAuth 마치고 CLI가 종료할 때까지 대기.
/// 프론트는 별도 spinner를 띄우거나, `console: true` 옵션을 줘 비대화형 인증 토큰 기반으로 변경 검토.
#[tauri::command]
pub async fn cli_login(provider: Provider, console: bool) -> AppResult<CliLoginOutcome> {
    let pkg = CliPkg::for_provider(provider);
    let bin = cli_install::locate_binary(pkg.binary)?.ok_or_else(|| AppError::CliMissing {
        provider: provider.as_str().into(),
    })?;

    match provider {
        Provider::Anthropic => {
            let mut cmd = Command::new(&bin);
            cmd.arg("auth").arg("login");
            if console {
                cmd.arg("--console");
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
            Ok(CliLoginOutcome::Completed)
        }
        Provider::Gemini => {
            // Gemini CLI는 비대화형 login 명령이 없음 — 사용자가 자기 터미널에서 `gemini`를 한 번 띄워
            // OAuth 흐름을 마쳐야 함. airis는 그 명령 문자열만 알려주고, 후속 cli_auth_status_gemini로 확인.
            Ok(CliLoginOutcome::TerminalInstruction {
                command: "gemini".into(),
                hint: "터미널에서 위 명령을 한 번 실행해 'Sign in with Google'로 로그인 후 종료하세요.".into(),
            })
        }
        Provider::Openai => {
            let mut cmd = Command::new(&bin);
            cmd.arg("login");
            if console {
                cmd.arg("--with-api-key");
            }
            info!(
                target: "cli_setup",
                provider = provider.as_str(),
                binary = %bin.display(),
                console,
                "spawn codex login"
            );
            let status = cmd.status().await.map_err(|e| AppError::CliRuntime {
                message: format!("codex login spawn 실패: {e}"),
            })?;
            if !status.success() {
                return Err(AppError::CliRuntime {
                    message: format!("codex login 종료 코드 {:?}", status.code()),
                });
            }
            Ok(CliLoginOutcome::Completed)
        }
    }
}

/// `codex login status` — exit 0 = 인증됨, 그 외 = 미인증. JSON 출력 X (단순 boolean).
#[tauri::command]
pub async fn cli_auth_status_codex() -> AppResult<CodexAuthInfo> {
    let bin = cli_install::locate_binary("codex")?.ok_or_else(|| AppError::CliMissing {
        provider: "openai".into(),
    })?;
    let output = Command::new(&bin)
        .arg("login")
        .arg("status")
        .output()
        .await
        .map_err(|e| AppError::CliRuntime {
            message: format!("codex login status spawn 실패: {e}"),
        })?;
    Ok(CodexAuthInfo {
        logged_in: output.status.success(),
    })
}

/// 짧은 ping query로 Gemini CLI 인증 상태 추정. 별도 status 명령이 없는 회피책.
/// stats 객체가 정상적으로 돌아오면 logged_in=true.
#[tauri::command]
pub async fn cli_auth_status_gemini() -> AppResult<GeminiAuthInfo> {
    let bin = cli_install::locate_binary("gemini")?.ok_or_else(|| AppError::CliMissing {
        provider: "gemini".into(),
    })?;
    // 비용 최소화 — flash 모델, 1토큰 응답 유도.
    let output = Command::new(&bin)
        .arg(".")
        .arg("-o")
        .arg("json")
        .arg("-m")
        .arg("gemini-2.5-flash")
        .output()
        .await
        .map_err(|e| AppError::CliRuntime {
            message: format!("gemini ping spawn 실패: {e}"),
        })?;
    let logged_in = output.status.success();
    if !logged_in {
        warn!(
            target: "cli_setup",
            stderr = %String::from_utf8_lossy(&output.stderr),
            code = ?output.status.code(),
            "gemini ping failed — assume not authenticated"
        );
    }
    Ok(GeminiAuthInfo { logged_in })
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

#[derive(Debug, Serialize)]
pub struct GeminiAuthInfo {
    pub logged_in: bool,
}

#[derive(Debug, Serialize)]
pub struct CodexAuthInfo {
    pub logged_in: bool,
}

/// `cli_login` 결과 — 즉시 완료(Anthropic) vs 터미널 안내(Gemini/Codex).
#[derive(Debug, Serialize)]
#[serde(tag = "kind")]
pub enum CliLoginOutcome {
    Completed,
    TerminalInstruction { command: String, hint: String },
}
