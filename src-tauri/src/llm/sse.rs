// SSE (Server-Sent Events) 파서 — W3C spec 1층만 다룬다.
// `data:` 줄의 JSON 의미 해석은 호출자(anthropic.rs)에서.
//
// 4종 에러 분류 (결정 2):
//   - Wire    : SSE 1층 깨짐 (UTF-8·줄 형식 위반)  → 스트림 중단
//   - UnknownEvent / UnknownPayload : 호출자가 처리 (warn + skip)
//   - Json    : data: 페이로드 JSON 파싱 실패     → 호출자에서 분류
//
// 본 파일은 Wire 에러만 발생시킨다. JSON·event 분류는 anthropic.rs 책임.

use std::str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseParseError {
    /// SSE 1층 깨짐. 통신 규격 변경 가능성.
    Wire {
        reason: &'static str,
        /// 디버그용 raw — 길면 잘려서 들어옴.
        raw: String,
    },
}

const RAW_TRUNC_LEN: usize = 256;

fn truncate_for_log(s: &str) -> String {
    if s.len() <= RAW_TRUNC_LEN {
        s.to_string()
    } else {
        format!("{}…(len={})", &s[..RAW_TRUNC_LEN], s.len())
    }
}

pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// 새 청크 누적 후 *완성된* 이벤트 반환.
    /// 한 이벤트 경계를 넘지 못한 부분은 내부 버퍼에 남는다.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, SseParseError> {
        let s = str::from_utf8(chunk).map_err(|e| SseParseError::Wire {
            reason: "invalid utf-8 in stream chunk",
            raw: format!("error_at_byte={}", e.valid_up_to()),
        })?;
        self.buffer.push_str(s);

        let mut events = Vec::new();
        // SSE는 한 이벤트가 빈 줄(\n\n 또는 \r\n\r\n)로 종료된다.
        while let Some(end) = find_event_end(&self.buffer) {
            let block = self.buffer[..end.start].to_string();
            self.buffer.drain(..end.consumed);

            if let Some(event) = parse_event_block(&block)? {
                events.push(event);
            }
        }

        Ok(events)
    }

    #[cfg(test)]
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

struct EventBoundary {
    start: usize,
    consumed: usize,
}

fn find_event_end(buf: &str) -> Option<EventBoundary> {
    if let Some(idx) = buf.find("\n\n") {
        return Some(EventBoundary {
            start: idx,
            consumed: idx + 2,
        });
    }
    if let Some(idx) = buf.find("\r\n\r\n") {
        return Some(EventBoundary {
            start: idx,
            consumed: idx + 4,
        });
    }
    None
}

fn parse_event_block(block: &str) -> Result<Option<SseEvent>, SseParseError> {
    let mut event_type: Option<String> = None;
    let mut data_lines: Vec<&str> = Vec::new();

    for raw_line in block.split('\n') {
        // \r\n 대응 — \n으로 split했으니 trailing \r 제거.
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        // SSE 주석 — `:` 로 시작.
        if line.starts_with(':') {
            continue;
        }
        // 빈 줄은 split 결과에 없을 것이지만 안전.
        if line.is_empty() {
            continue;
        }

        // field:value 또는 field: value 형식. ':' 없으면 wire 위반.
        let Some(colon) = line.find(':') else {
            return Err(SseParseError::Wire {
                reason: "field line missing colon",
                raw: truncate_for_log(line),
            });
        };

        let field = &line[..colon];
        let mut value = &line[colon + 1..];
        // 표준: 첫 공백 1개 제거.
        if let Some(stripped) = value.strip_prefix(' ') {
            value = stripped;
        }

        match field {
            "event" => event_type = Some(value.to_string()),
            "data" => data_lines.push(value),
            // id·retry·기타 모르는 필드는 W3C 따라 무시.
            _ => {}
        }
    }

    if data_lines.is_empty() {
        // 빈 keepalive (예: ":\n\n") 또는 event-only 블록은 우리에게 의미 X.
        return Ok(None);
    }

    Ok(Some(SseEvent {
        event_type,
        data: data_lines.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_event() {
        let mut p = SseParser::new();
        let events = p.feed(b"event: foo\ndata: hello\n\n").expect("parse ok");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_deref(), Some("foo"));
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn handles_chunk_split_mid_event() {
        let mut p = SseParser::new();
        // 한 이벤트가 두 청크에 걸쳐서 도착해도 합쳐서 파싱.
        assert_eq!(p.feed(b"event: foo\ndata: ").unwrap().len(), 0);
        assert_eq!(p.feed(b"hel").unwrap().len(), 0);
        let events = p.feed(b"lo\n\n").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn handles_multiple_events_in_one_chunk() {
        let mut p = SseParser::new();
        let events = p.feed(b"data: one\n\ndata: two\n\n").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "one");
        assert_eq!(events[1].data, "two");
    }

    #[test]
    fn ignores_comments_and_unknown_fields() {
        let mut p = SseParser::new();
        let events = p
            .feed(b": this is a keepalive comment\nid: 42\nretry: 3000\nevent: msg\ndata: ok\n\n")
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_deref(), Some("msg"));
        assert_eq!(events[0].data, "ok");
    }

    #[test]
    fn keepalive_only_block_yields_no_event() {
        let mut p = SseParser::new();
        let events = p.feed(b": keepalive\n\n").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn supports_crlf_line_endings() {
        let mut p = SseParser::new();
        let events = p.feed(b"event: x\r\ndata: y\r\n\r\n").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "y");
    }

    #[test]
    fn multiple_data_lines_concatenate_with_newline() {
        // SSE 표준 — 한 이벤트의 data 줄 여러 개는 \n으로 합친다.
        let mut p = SseParser::new();
        let events = p.feed(b"data: line1\ndata: line2\n\n").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn wire_error_on_invalid_utf8() {
        let mut p = SseParser::new();
        let bytes = [0xff, 0xfe, b'\n', b'\n'];
        let err = p.feed(&bytes).unwrap_err();
        match err {
            SseParseError::Wire { reason, .. } => {
                assert!(reason.contains("utf-8"));
            }
        }
    }

    #[test]
    fn wire_error_on_field_line_without_colon() {
        let mut p = SseParser::new();
        let err = p.feed(b"this-line-has-no-colon\n\n").unwrap_err();
        match err {
            SseParseError::Wire { reason, .. } => {
                assert!(reason.contains("colon"));
            }
        }
    }

    #[test]
    fn buffer_holds_partial_event() {
        let mut p = SseParser::new();
        let events = p.feed(b"data: incomplete").unwrap();
        assert!(events.is_empty());
        assert!(p.buffer_len() > 0);
    }
}
