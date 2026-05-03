// 섹션 본문 → 청크 분할.
//
// 정책:
//   * 목표 청크 크기 = ~500 chars (한·영 혼합 텍스트 기준).
//   * *문장 경계 보존* — 한국어 종결("다.","요.","함."·"!?")·영어(".!?") + 줄바꿈.
//   * 너무 짧은 청크는 다음 청크와 합침 (~150 미만이면 합칠 가치).
//   * 너무 긴 단어/단락은 강제 분할 (>=1.5x 목표).

use serde::Serialize;

const TARGET_CHARS: usize = 500;
const MIN_CHARS: usize = 150;
const HARD_MAX_CHARS: usize = 750; // ~1.5x

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Chunk {
    pub content: String,
    /// 섹션 본문에서 *문자(char)* 단위 시작 offset.
    pub char_offset: usize,
}

pub fn chunk_section(body: &str) -> Vec<Chunk> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // 문장 단위로 1차 분해.
    let sentences = split_sentences(trimmed);
    if sentences.is_empty() {
        return vec![Chunk {
            content: trimmed.to_string(),
            char_offset: 0,
        }];
    }

    // 청크에 문장 누적, 목표 크기 초과 시 cut.
    let mut chunks = Vec::new();
    let mut buf = String::new();
    let mut buf_offset: Option<usize> = None;

    for (offset, sentence) in sentences {
        let candidate_len = buf.chars().count() + sentence.chars().count();
        // 단일 문장이 hard max 넘으면 강제 분할 (코드 블록·긴 인용 등).
        if sentence.chars().count() > HARD_MAX_CHARS {
            if !buf.is_empty() {
                chunks.push(Chunk {
                    content: std::mem::take(&mut buf).trim_end().to_string(),
                    char_offset: buf_offset.take().unwrap_or(0),
                });
            }
            for piece in hard_split(sentence, TARGET_CHARS, offset) {
                chunks.push(piece);
            }
            continue;
        }

        if buf_offset.is_none() {
            buf_offset = Some(offset);
        }

        if candidate_len > TARGET_CHARS && !buf.is_empty() {
            chunks.push(Chunk {
                content: std::mem::take(&mut buf).trim_end().to_string(),
                char_offset: buf_offset.take().unwrap_or(0),
            });
            buf_offset = Some(offset);
        }

        buf.push_str(sentence);
        if !sentence.ends_with(char::is_whitespace) {
            buf.push(' ');
        }
    }
    if !buf.trim().is_empty() {
        chunks.push(Chunk {
            content: buf.trim_end().to_string(),
            char_offset: buf_offset.unwrap_or(0),
        });
    }

    merge_short_tail(chunks)
}

/// 마지막 청크가 MIN_CHARS 미만이면 직전 청크에 합침 — *외로운 짧은 꼬리* 방지.
fn merge_short_tail(mut chunks: Vec<Chunk>) -> Vec<Chunk> {
    if chunks.len() < 2 {
        return chunks;
    }
    let last = chunks.last().unwrap();
    if last.content.chars().count() < MIN_CHARS {
        let last = chunks.pop().unwrap();
        let prev = chunks.last_mut().unwrap();
        prev.content.push(' ');
        prev.content.push_str(&last.content);
    }
    chunks
}

/// 본문을 (offset, sentence) 튜플 시퀀스로 분해. 빈 줄·여러 종결 구분자 허용.
fn split_sentences(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut start_char = 0usize;
    let mut char_pos = 0usize;
    let mut prev_terminator = false;

    let mut i = 0usize;
    while i < bytes.len() {
        let ch_len = utf8_char_len(bytes[i]);
        let ch = text[i..i + ch_len].chars().next().unwrap();
        if is_sentence_terminator(ch) {
            prev_terminator = true;
        } else if prev_terminator && (ch.is_whitespace() || ch == '\n') {
            // terminator 직후 공백 = 문장 분리.
            let end = i;
            let sentence = &text[start..end];
            let trimmed = sentence.trim_start();
            if !trimmed.is_empty() {
                let leading_skipped = sentence.len() - trimmed.len();
                let skipped_chars = sentence[..leading_skipped].chars().count();
                out.push((start_char + skipped_chars, trimmed));
            }
            start = end + ch_len;
            start_char = char_pos + 1;
            prev_terminator = false;
        } else if !prev_terminator {
            // 일반 문자 — 그냥 진행.
        } else {
            // terminator 다음에 곧장 다른 문자 (예: "Mr.Smith") → 종결로 보지 않음.
            prev_terminator = false;
        }
        i += ch_len;
        char_pos += 1;
    }
    if start < text.len() {
        let sentence = &text[start..];
        let trimmed = sentence.trim_start();
        if !trimmed.is_empty() {
            let leading_skipped = sentence.len() - trimmed.len();
            let skipped_chars = sentence[..leading_skipped].chars().count();
            out.push((start_char + skipped_chars, trimmed));
        }
    }
    out
}

fn is_sentence_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '\n')
}

fn utf8_char_len(b: u8) -> usize {
    match b {
        0..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// 코드 블록·긴 한 줄 등 hard max 초과 시 강제 분할 — char 기준 균등 분할.
fn hard_split(text: &str, target: usize, base_offset: usize) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chunk_start_char = 0usize;

    for (char_pos, ch) in text.chars().enumerate() {
        if current.chars().count() >= target && (ch.is_whitespace() || current.ends_with(' ')) {
            out.push(Chunk {
                content: current.trim_end().to_string(),
                char_offset: base_offset + chunk_start_char,
            });
            current = String::new();
            chunk_start_char = char_pos;
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        out.push(Chunk {
            content: current.trim_end().to_string(),
            char_offset: base_offset + chunk_start_char,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_yields_no_chunks() {
        assert!(chunk_section("").is_empty());
        assert!(chunk_section("   \n  ").is_empty());
    }

    #[test]
    fn short_body_returns_single_chunk() {
        let chunks = chunk_section("짧은 한 문장입니다.");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("짧은"));
        assert_eq!(chunks[0].char_offset, 0);
    }

    #[test]
    fn long_body_splits_at_sentence_boundaries() {
        // 약 1500자 — 여러 청크 기대.
        let sentence = "이것은 한국어 학습 도우미를 만드는 프로젝트입니다. ".repeat(30);
        let body = format!("{sentence}\n\n{sentence}");
        let chunks = chunk_section(&body);
        assert!(
            chunks.len() >= 2,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        for c in &chunks {
            assert!(!c.content.is_empty());
        }
    }

    #[test]
    fn english_sentences_split_on_period() {
        let body = "Rust ownership is unique. Each value has one owner. \
                    The owner determines the value's lifetime."
            .repeat(20);
        let chunks = chunk_section(&body);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn char_offsets_are_monotonically_non_decreasing() {
        let body = "문장 하나. ".repeat(60);
        let chunks = chunk_section(&body);
        let mut prev = 0;
        for c in &chunks {
            assert!(
                c.char_offset >= prev,
                "char_offset must be non-decreasing: prev={prev} cur={}",
                c.char_offset
            );
            prev = c.char_offset;
        }
    }

    #[test]
    fn very_long_single_sentence_is_hard_split() {
        // 마침표 없는 1500자 한 줄 — hard split 발동.
        let body = "abcdefghij ".repeat(150);
        let chunks = chunk_section(&body);
        assert!(chunks.len() >= 2, "long unbroken text must hard-split");
        for c in &chunks {
            assert!(c.content.chars().count() <= HARD_MAX_CHARS + 50);
        }
    }
}
