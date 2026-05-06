// PR 24 (D-066) — Claude Code subprocess 어댑터.
//
// 호출 흐름:
//   1. `~/.airis/npm/bin/claude -p "<query>" --model <m> --output-format stream-json --verbose ...`
//   2. cwd를 app_data_dir로 강제 → 사용자 다른 프로젝트의 CLAUDE.md 자동 발견 차단.
//   3. stdout JSONL을 줄 단위로 파싱.
//      - {type:"assistant",message:{content:[{type:"text",text}],usage:{...}}} → ChatEvent::TextDelta
//        (델타 차분 계산 — 누적 텍스트가 아니라 *증분*만 emit)
//      - {type:"result",subtype:"success",result:"<전체>",usage:{...}}        → ChatEvent::Done
//      - {type:"system",...} / 기타 → 무시
//   4. 자식 프로세스가 stream Drop될 때 SIGTERM (좀비 방지).
//
// 인증: subprocess가 `claude auth status` 결과를 그대로 사용. 로그인은 사용자 터미널에서 `claude auth login` 또는
// PR 24 commands/cli_setup.rs가 별도 spawn해 OAuth 흐름을 띄움.

use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;

use async_stream::try_stream;
use async_trait::async_trait;
use futures_util::Stream;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tracing::{debug, warn};

use super::{ChatEvent, ChatRequest, ChatStream, LlmProvider, Role, Usage};
use crate::error::{AppError, AppResult};

/// 자식 프로세스가 stream보다 오래 살지 못하도록 묶어두는 가드.
/// stream Drop 시 자동 SIGKILL — Tokio Child의 기본 동작 (`kill_on_drop(true)`).
struct ChildGuard {
    child: Option<Child>,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            // tokio::process::Child는 kill_on_drop(true)면 자동 SIGKILL.
            // 여기서는 명시적 start_kill로 신뢰성 ↑ (kill_on_drop 미사용 시에도 안전).
            let _ = c.start_kill();
        }
    }
}

pub struct ClaudeCliProvider {
    binary: PathBuf,
    cwd: PathBuf,
}

impl ClaudeCliProvider {
    pub fn new(binary: PathBuf, cwd: PathBuf) -> Self {
        Self { binary, cwd }
    }
}

/// v0.4.3 PR 1 (D-086) — Claude CLI provider의 빠른 보조 모델. CLI는 `--model` 인자에
/// 그대로 전달 — Anthropic 어댑터와 동일 모델 이름.
const CLAUDE_CLI_FAST_MODEL: &str = "claude-haiku-4-5";

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    fn fast_model(&self) -> &str {
        CLAUDE_CLI_FAST_MODEL
    }

    async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream> {
        let user_prompt = render_user_prompt(&request);
        let system_prompt = request.system.clone().unwrap_or_default();
        let model = request.model.clone();

        let mut cmd = Command::new(&self.binary);
        cmd.arg("-p")
            .arg(&user_prompt)
            .arg("--model")
            .arg(&model)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--no-session-persistence")
            .arg("--tools")
            .arg("")
            .arg("--setting-sources")
            .arg("")
            .arg("--include-partial-messages");

        if !system_prompt.is_empty() {
            cmd.arg("--system-prompt").arg(&system_prompt);
        }

        cmd.current_dir(&self.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        debug!(
            target: "claude_cli",
            binary = %self.binary.display(),
            model = %model,
            cwd = %self.cwd.display(),
            "spawn claude CLI"
        );

        let mut child = cmd.spawn().map_err(|e| AppError::CliRuntime {
            message: format!("claude CLI spawn 실패: {e}"),
        })?;

        let stdout = child.stdout.take().ok_or_else(|| AppError::CliRuntime {
            message: "claude CLI stdout 핸들 부재".into(),
        })?;
        let stderr = child.stderr.take();

        let stream = build_event_stream(child, stdout, stderr);
        Ok(Box::pin(stream))
    }
}

fn render_user_prompt(request: &ChatRequest) -> String {
    // 멀티턴이지만 v0.2.1 첫 컷은 사용자 last message만 보냄 — 기존 chat_send도 1턴 구조.
    // 향후 multi-turn 시 --input-format stream-json 사용 검토.
    request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

fn build_event_stream(
    child: Child,
    stdout: tokio::process::ChildStdout,
    stderr: Option<tokio::process::ChildStderr>,
) -> Pin<Box<dyn Stream<Item = AppResult<ChatEvent>> + Send>> {
    Box::pin(try_stream! {
        let mut guard = ChildGuard { child: Some(child) };

        // stderr는 별도 task로 흘려보내 로그만 — Drop 시 같이 정리.
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if !line.trim().is_empty() {
                        warn!(target: "claude_cli", stderr = %line);
                    }
                }
            });
        }

        let mut reader = BufReader::new(stdout).lines();
        let mut accumulated_text = String::new();
        let mut emitted_done = false;

        while let Some(line) = reader.next_line().await.map_err(|e| AppError::CliRuntime {
            message: format!("stdout read: {e}"),
        })? {
            if line.trim().is_empty() {
                continue;
            }
            match parse_line(&line, &accumulated_text)? {
                Parsed::Delta { delta, total } => {
                    accumulated_text = total;
                    if !delta.is_empty() {
                        yield ChatEvent::TextDelta { text: delta };
                    }
                }
                Parsed::Done { result, usage } => {
                    // 일부 환경에선 stream 완료 직전에 result만 오고 assistant delta가 누락될 수 있음 —
                    // 그 경우 result로 한 번에 보강.
                    if accumulated_text.is_empty() && !result.is_empty() {
                        yield ChatEvent::TextDelta { text: result.clone() };
                    }
                    emitted_done = true;
                    yield ChatEvent::Done { usage };
                    break;
                }
                Parsed::Error { message } => {
                    Err(AppError::CliRuntime { message })?;
                    unreachable!();
                }
                Parsed::Skip => {}
            }
        }

        if !emitted_done {
            // 자식이 결과 라인 없이 종료한 경우 — exit status 확인 후 에러로 변환.
            if let Some(mut child) = guard.child.take() {
                match child.wait().await {
                    Ok(status) if status.success() => {
                        // 빈 응답으로 간주 — Done usage=0 emit.
                        yield ChatEvent::Done { usage: Usage::default() };
                    }
                    Ok(status) => {
                        Err(AppError::CliRuntime {
                            message: format!("claude CLI 종료 코드 {:?}", status.code()),
                        })?;
                    }
                    Err(e) => {
                        Err(AppError::CliRuntime {
                            message: format!("claude CLI wait 실패: {e}"),
                        })?;
                    }
                }
            }
        }
    })
}

#[derive(Debug)]
enum Parsed {
    Delta { delta: String, total: String },
    Done { result: String, usage: Usage },
    Error { message: String },
    Skip,
}

#[derive(Deserialize)]
struct LineEnvelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    message: Option<AssistantMessage>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    usage: Option<UsageRaw>,
    #[serde(default)]
    is_error: Option<bool>,
}

#[derive(Deserialize)]
struct AssistantMessage {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Default)]
struct UsageRaw {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

fn parse_line(line: &str, accumulated: &str) -> AppResult<Parsed> {
    let env: LineEnvelope = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            warn!(target: "claude_cli", error = %e, line = %line, "JSON parse failed");
            return Ok(Parsed::Skip);
        }
    };

    match env.kind.as_str() {
        "assistant" => {
            let Some(msg) = env.message else {
                return Ok(Parsed::Skip);
            };
            let mut total = String::new();
            for block in &msg.content {
                if block.kind == "text" {
                    if let Some(t) = &block.text {
                        total.push_str(t);
                    }
                }
            }
            // 누적 텍스트 차분 — 새 라인이 더 짧으면(드뭄) 그대로 두고 빈 델타.
            let delta = match total.strip_prefix(accumulated) {
                Some(rest) => rest.to_string(),
                None => total.clone(),
            };
            Ok(Parsed::Delta { delta, total })
        }
        "stream_event" => {
            // --include-partial-messages 출력. 본 어댑터는 assistant snapshot을 truth로 사용 (단순화).
            Ok(Parsed::Skip)
        }
        "result" => {
            let is_error = env.is_error.unwrap_or(false)
                || env.subtype.as_deref().is_some_and(|s| s != "success");
            if is_error {
                let msg = env.result.clone().unwrap_or_else(|| {
                    format!("claude CLI result error (subtype={:?})", env.subtype)
                });
                return Ok(Parsed::Error { message: msg });
            }
            let usage = usage_from_raw(env.usage.unwrap_or_default());
            Ok(Parsed::Done {
                result: env.result.unwrap_or_default(),
                usage,
            })
        }
        _ => Ok(Parsed::Skip),
    }
}

fn usage_from_raw(raw: UsageRaw) -> Usage {
    Usage {
        input_tokens: raw.input_tokens.unwrap_or(0) as u32,
        output_tokens: raw.output_tokens.unwrap_or(0) as u32,
        cache_creation_input_tokens: raw.cache_creation_input_tokens.unwrap_or(0) as u32,
        cache_read_input_tokens: raw.cache_read_input_tokens.unwrap_or(0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_delta_extracts_diff_against_accumulated() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}"#;
        let parsed = parse_line(line, "").unwrap();
        match parsed {
            Parsed::Delta { delta, total } => {
                assert_eq!(delta, "Hello");
                assert_eq!(total, "Hello");
            }
            other => panic!("expected Delta, got {other:?}"),
        }

        // 두 번째 assistant 라인이 *누적* 텍스트일 때 — 차분만 emit.
        let line2 =
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello world"}]}}"#;
        let parsed = parse_line(line2, "Hello").unwrap();
        match parsed {
            Parsed::Delta { delta, total } => {
                assert_eq!(delta, " world");
                assert_eq!(total, "Hello world");
            }
            other => panic!("expected Delta, got {other:?}"),
        }
    }

    #[test]
    fn result_success_yields_done_with_usage() {
        let line = r#"{"type":"result","subtype":"success","result":"ok","is_error":false,
            "usage":{"input_tokens":6,"output_tokens":1,"cache_creation_input_tokens":52168,"cache_read_input_tokens":0}}"#;
        let parsed = parse_line(line, "ok").unwrap();
        match parsed {
            Parsed::Done { result, usage } => {
                assert_eq!(result, "ok");
                assert_eq!(usage.input_tokens, 6);
                assert_eq!(usage.output_tokens, 1);
                assert_eq!(usage.cache_creation_input_tokens, 52168);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn result_error_subtype_yields_error() {
        let line =
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"limit"}"#;
        let parsed = parse_line(line, "").unwrap();
        match parsed {
            Parsed::Error { message } => {
                assert_eq!(message, "limit");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn system_init_line_is_skipped() {
        let line = r#"{"type":"system","subtype":"init","cwd":"/x","session_id":"abc"}"#;
        assert!(matches!(parse_line(line, "").unwrap(), Parsed::Skip));
    }

    #[test]
    fn malformed_json_is_skipped_not_fatal() {
        let parsed = parse_line("not json", "").unwrap();
        assert!(matches!(parsed, Parsed::Skip));
    }
}
