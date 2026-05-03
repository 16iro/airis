// PR 26 (D-066) — OpenAI Codex CLI subprocess 어댑터.
//
// 호출: `~/.airis/npm/bin/codex exec --json "<query>" --model <model>`
// JSONL 출력 (공식 문서 + 사용자 검증 기반):
//   {"type":"thread.started",...}                                                 — 무시
//   {"type":"turn.started",...}                                                   — 무시
//   {"type":"item.started","item":{...}}                                          — 무시 (interim)
//   {"type":"item.completed","item":{"id":"item_X","type":"agent_message","text":"<full>"}}
//                                                                                  → ChatEvent::TextDelta
//   {"type":"turn.completed","usage":{"input_tokens":N,"cached_input_tokens":N,"output_tokens":N,"reasoning_output_tokens":N}}
//                                                                                  → ChatEvent::Done
//   {"type":"turn.failed",...} / {"type":"error",...}                             → AppError::CliRuntime
//
// 진짜 스트리밍은 X — agent_message가 *완성 후 한 번*에 옴. 첫 토큰 지연이 길 수 있음.

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

struct ChildGuard {
    child: Option<Child>,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.start_kill();
        }
    }
}

pub struct CodexCliProvider {
    binary: PathBuf,
    cwd: PathBuf,
}

impl CodexCliProvider {
    pub fn new(binary: PathBuf, cwd: PathBuf) -> Self {
        Self { binary, cwd }
    }
}

#[async_trait]
impl LlmProvider for CodexCliProvider {
    async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream> {
        let user_prompt = render_user_prompt(&request);
        // Codex도 시스템 프롬프트 명시 옵션이 마땅치 않아 user 본문 앞에 prepend.
        let final_prompt = match &request.system {
            Some(sys) if !sys.is_empty() => format!("{sys}\n\n---\n\n{user_prompt}"),
            _ => user_prompt,
        };

        let mut cmd = Command::new(&self.binary);
        cmd.arg("exec")
            .arg("--json")
            .arg("--model")
            .arg(&request.model)
            .arg(&final_prompt);

        cmd.current_dir(&self.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        debug!(
            target: "codex_cli",
            binary = %self.binary.display(),
            model = %request.model,
            "spawn codex CLI"
        );

        let mut child = cmd.spawn().map_err(|e| AppError::CliRuntime {
            message: format!("codex CLI spawn 실패: {e}"),
        })?;

        let stdout = child.stdout.take().ok_or_else(|| AppError::CliRuntime {
            message: "codex CLI stdout 핸들 부재".into(),
        })?;
        let stderr = child.stderr.take();

        let stream = build_event_stream(child, stdout, stderr);
        Ok(Box::pin(stream))
    }
}

fn render_user_prompt(request: &ChatRequest) -> String {
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

        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if !line.trim().is_empty() {
                        warn!(target: "codex_cli", stderr = %line);
                    }
                }
            });
        }

        let mut reader = BufReader::new(stdout).lines();
        let mut emitted_done = false;

        while let Some(line) = reader.next_line().await.map_err(|e| AppError::CliRuntime {
            message: format!("stdout read: {e}"),
        })? {
            if line.trim().is_empty() {
                continue;
            }
            match parse_line(&line)? {
                Parsed::Text { text } => {
                    if !text.is_empty() {
                        yield ChatEvent::TextDelta { text };
                    }
                }
                Parsed::Done { usage } => {
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
            if let Some(mut child) = guard.child.take() {
                match child.wait().await {
                    Ok(status) if status.success() => {
                        yield ChatEvent::Done { usage: Usage::default() };
                    }
                    Ok(status) => {
                        Err(AppError::CliRuntime {
                            message: format!("codex CLI 종료 코드 {:?}", status.code()),
                        })?;
                    }
                    Err(e) => {
                        Err(AppError::CliRuntime {
                            message: format!("codex CLI wait 실패: {e}"),
                        })?;
                    }
                }
            }
        }
    })
}

#[derive(Debug)]
enum Parsed {
    Text { text: String },
    Done { usage: Usage },
    Error { message: String },
    Skip,
}

#[derive(Deserialize)]
struct LineEnvelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    item: Option<Item>,
    #[serde(default)]
    usage: Option<UsageRaw>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Default)]
struct UsageRaw {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    /// 캐시 hit. Codex는 `cached_input_tokens` 키 사용.
    #[serde(default)]
    cached_input_tokens: Option<u64>,
}

fn parse_line(line: &str) -> AppResult<Parsed> {
    let env: LineEnvelope = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            warn!(target: "codex_cli", error = %e, line = %line, "JSON parse failed");
            return Ok(Parsed::Skip);
        }
    };

    match env.kind.as_str() {
        "item.completed" => {
            let Some(item) = env.item else {
                return Ok(Parsed::Skip);
            };
            // agent_message만 텍스트 — agent_reasoning, command_execution, plan_update 등은 무시.
            if item.kind.as_deref() != Some("agent_message") {
                return Ok(Parsed::Skip);
            }
            let text = item.text.unwrap_or_default();
            Ok(Parsed::Text { text })
        }
        "turn.completed" => {
            let raw = env.usage.unwrap_or_default();
            let usage = Usage {
                input_tokens: raw.input_tokens.unwrap_or(0) as u32,
                output_tokens: raw.output_tokens.unwrap_or(0) as u32,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: raw.cached_input_tokens.unwrap_or(0) as u32,
            };
            Ok(Parsed::Done { usage })
        }
        "turn.failed" | "error" => {
            let msg = env
                .message
                .unwrap_or_else(|| format!("codex CLI {} event", env.kind));
            Ok(Parsed::Error { message: msg })
        }
        _ => Ok(Parsed::Skip), // thread.started, turn.started, item.started 등
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_completed_agent_message_yields_text() {
        let line = r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"Hello world"}}"#;
        let parsed = parse_line(line).unwrap();
        match parsed {
            Parsed::Text { text } => assert_eq!(text, "Hello world"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn item_completed_reasoning_is_skipped() {
        let line = r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_reasoning","text":"thinking..."}}"#;
        assert!(matches!(parse_line(line).unwrap(), Parsed::Skip));
    }

    #[test]
    fn item_completed_command_execution_is_skipped() {
        let line = r#"{"type":"item.completed","item":{"id":"item_4","type":"command_execution"}}"#;
        assert!(matches!(parse_line(line).unwrap(), Parsed::Skip));
    }

    #[test]
    fn turn_completed_yields_done_with_usage() {
        let line = r#"{"type":"turn.completed","usage":{"input_tokens":24763,"cached_input_tokens":24448,"output_tokens":122,"reasoning_output_tokens":0}}"#;
        let parsed = parse_line(line).unwrap();
        match parsed {
            Parsed::Done { usage } => {
                assert_eq!(usage.input_tokens, 24763);
                assert_eq!(usage.output_tokens, 122);
                assert_eq!(usage.cache_read_input_tokens, 24448);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn turn_failed_yields_error() {
        let line = r#"{"type":"turn.failed","message":"context length exceeded"}"#;
        let parsed = parse_line(line).unwrap();
        match parsed {
            Parsed::Error { message } => assert_eq!(message, "context length exceeded"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn thread_started_is_skipped() {
        let line = r#"{"type":"thread.started"}"#;
        assert!(matches!(parse_line(line).unwrap(), Parsed::Skip));
    }

    #[test]
    fn turn_started_is_skipped() {
        let line = r#"{"type":"turn.started"}"#;
        assert!(matches!(parse_line(line).unwrap(), Parsed::Skip));
    }

    #[test]
    fn malformed_json_is_skipped() {
        assert!(matches!(parse_line("garbage").unwrap(), Parsed::Skip));
    }
}
