// PR 25 (D-066) — Gemini CLI subprocess 어댑터.
//
// 호출: `~/.airis/npm/bin/gemini "<query>" -o stream-json -m <model>`
// 출력 (실측):
//   {"type":"init","session_id":...,"model":"..."}                           — 무시
//   {"type":"message","role":"user","content":"..."}                          — echo, 무시
//   {"type":"message","role":"assistant","content":"<text>","delta":true}     — 텍스트 (진짜 델타)
//   {"type":"result","status":"success","stats":{total_tokens,input_tokens,output_tokens,cached,...}}
//
// Claude와 달리 *진짜 스트리밍*임 — assistant 라인이 매번 *증분*만 옴 (delta:true).
// 누적 차분 계산 X — 그대로 emit.

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

pub struct GeminiCliProvider {
    binary: PathBuf,
    cwd: PathBuf,
}

impl GeminiCliProvider {
    pub fn new(binary: PathBuf, cwd: PathBuf) -> Self {
        Self { binary, cwd }
    }
}

#[async_trait]
impl LlmProvider for GeminiCliProvider {
    async fn chat_stream(&self, request: ChatRequest) -> AppResult<ChatStream> {
        let user_prompt = render_user_prompt(&request);
        let model = request.model.clone();

        // Gemini CLI는 시스템 프롬프트 명시 옵션이 비공식 — system은 user 본문 앞에 prepend.
        let final_prompt = match &request.system {
            Some(sys) if !sys.is_empty() => format!("{sys}\n\n---\n\n{user_prompt}"),
            _ => user_prompt,
        };

        let mut cmd = Command::new(&self.binary);
        cmd.arg(&final_prompt)
            .arg("-o")
            .arg("stream-json")
            .arg("-m")
            .arg(&model);

        cmd.current_dir(&self.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        debug!(
            target: "gemini_cli",
            binary = %self.binary.display(),
            model = %model,
            "spawn gemini CLI"
        );

        let mut child = cmd.spawn().map_err(|e| AppError::CliRuntime {
            message: format!("gemini CLI spawn 실패: {e}"),
        })?;

        let stdout = child.stdout.take().ok_or_else(|| AppError::CliRuntime {
            message: "gemini CLI stdout 핸들 부재".into(),
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
                        warn!(target: "gemini_cli", stderr = %line);
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
                Parsed::Delta { text } => {
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
                            message: format!("gemini CLI 종료 코드 {:?}", status.code()),
                        })?;
                    }
                    Err(e) => {
                        Err(AppError::CliRuntime {
                            message: format!("gemini CLI wait 실패: {e}"),
                        })?;
                    }
                }
            }
        }
    })
}

#[derive(Debug)]
enum Parsed {
    Delta { text: String },
    Done { usage: Usage },
    Error { message: String },
    Skip,
}

#[derive(Deserialize)]
struct LineEnvelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    stats: Option<Stats>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct Stats {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    /// `cached` = cache_read_input_tokens 대응. CLI가 보고하는 단일 캐시 카운터.
    #[serde(default)]
    cached: Option<u64>,
}

fn parse_line(line: &str) -> AppResult<Parsed> {
    let env: LineEnvelope = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            warn!(target: "gemini_cli", error = %e, line = %line, "JSON parse failed");
            return Ok(Parsed::Skip);
        }
    };

    match env.kind.as_str() {
        "message" => {
            // role=user는 echo일 뿐, role=assistant 텍스트만 통과.
            if env.role.as_deref() != Some("assistant") {
                return Ok(Parsed::Skip);
            }
            let text = env.content.unwrap_or_default();
            Ok(Parsed::Delta { text })
        }
        "result" => {
            let success = env.status.as_deref() == Some("success");
            if !success {
                let msg = env.error.clone().unwrap_or_else(|| {
                    format!("gemini CLI result error (status={:?})", env.status)
                });
                return Ok(Parsed::Error { message: msg });
            }
            let stats = env.stats.unwrap_or_default();
            let usage = Usage {
                input_tokens: stats.input_tokens.unwrap_or(0) as u32,
                output_tokens: stats.output_tokens.unwrap_or(0) as u32,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: stats.cached.unwrap_or(0) as u32,
            };
            Ok(Parsed::Done { usage })
        }
        _ => Ok(Parsed::Skip), // init·기타 무시
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_message_yields_delta() {
        let line = r#"{"type":"message","role":"assistant","content":" world","delta":true}"#;
        let parsed = parse_line(line).unwrap();
        match parsed {
            Parsed::Delta { text } => assert_eq!(text, " world"),
            other => panic!("expected Delta, got {other:?}"),
        }
    }

    #[test]
    fn user_message_is_skipped_as_echo() {
        let line = r#"{"type":"message","role":"user","content":"query"}"#;
        assert!(matches!(parse_line(line).unwrap(), Parsed::Skip));
    }

    #[test]
    fn result_success_yields_done_with_stats() {
        let line = r#"{"type":"result","status":"success","stats":{"total_tokens":8344,"input_tokens":8328,"output_tokens":1,"cached":1024,"duration_ms":2083}}"#;
        let parsed = parse_line(line).unwrap();
        match parsed {
            Parsed::Done { usage } => {
                assert_eq!(usage.input_tokens, 8328);
                assert_eq!(usage.output_tokens, 1);
                assert_eq!(usage.cache_read_input_tokens, 1024);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn result_failure_yields_error() {
        let line = r#"{"type":"result","status":"error","error":"quota exceeded"}"#;
        let parsed = parse_line(line).unwrap();
        match parsed {
            Parsed::Error { message } => assert_eq!(message, "quota exceeded"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn init_line_is_skipped() {
        let line = r#"{"type":"init","session_id":"x","model":"gemini-2.5-flash"}"#;
        assert!(matches!(parse_line(line).unwrap(), Parsed::Skip));
    }

    #[test]
    fn malformed_json_skipped() {
        assert!(matches!(parse_line("not json").unwrap(), Parsed::Skip));
    }
}
