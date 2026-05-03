// Memory.md — 사용자 성향·진도·이해도 누적 (시스템 자동 갱신, 사용자 편집 가능).
//
// 위치: `{data_dir}/studies/{slug}/Memory.md`
// 형식: YAML frontmatter (updated·study) + 5섹션 마크다운 본문
//   1. Preferences  2. Corrections  3. Progress  4. Meta  5. Goals
//
// PR 14 정책 (분량 적정 + 사용자 의도 보존):
//   * read/write는 *frontmatter + body 전체*만 다룸. 5섹션 *분해 편집*은 v0.3+
//   * 형식 보존은 사용자 책임 (직접 편집 가능 영역). 시스템은 read/write만.
//   * 원자적 쓰기 — `.tmp` → atomic rename (SEQ-8 정신)
//   * 외부 편집 감지 — 마지막 write 시점 mtime+hash를 메모리에 보관, read 때 비교
//     변경 감지되면 응답에 `external_edited: true` 플래그.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

pub const MEMORY_FILENAME: &str = "Memory.md";

/// 5섹션 헤딩 — 사용자 file이 이 형식 안 따르면 *경고*만 (강제 X).
pub const SECTION_HEADINGS: &[&str] = &[
    "## 1. 사용자 선호 (Preferences)",
    "## 2. 금지·교정 (Corrections)",
    "## 3. 진도·이해도 (Progress)",
    "## 4. 메타인지 패턴 (Meta)",
    "## 5. 학습 목표 (Goals)",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDoc {
    pub study: String,
    /// ISO 8601 갱신 시각.
    pub updated: String,
    /// frontmatter 다음의 마크다운 본문 전체.
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryReadResult {
    pub doc: MemoryDoc,
    /// 마지막 write 이후 사용자가 외부 에디터로 *수정한 정황*이 감지됐는지.
    /// 파일이 처음 읽히는 경우(이전 hash 없음) false.
    pub external_edited: bool,
    /// 파일이 디스크에 존재하는지 — false면 default template로 응답.
    pub file_existed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryFingerprint {
    pub mtime_unix: u64,
    pub hash: String,
}

impl MemoryDoc {
    pub fn new_default(slug: &str, updated: &str) -> Self {
        Self {
            study: slug.to_string(),
            updated: updated.to_string(),
            body: default_body_template(),
        }
    }
}

pub fn study_dir(data_dir: &Path, slug: &str) -> PathBuf {
    data_dir.join("studies").join(slug)
}

pub fn memory_path(data_dir: &Path, slug: &str) -> PathBuf {
    study_dir(data_dir, slug).join(MEMORY_FILENAME)
}

pub fn read(
    data_dir: &Path,
    slug: &str,
    last_fingerprint: Option<&MemoryFingerprint>,
) -> AppResult<MemoryReadResult> {
    let path = memory_path(data_dir, slug);
    if !path.exists() {
        return Ok(MemoryReadResult {
            doc: MemoryDoc::new_default(slug, ""),
            external_edited: false,
            file_existed: false,
        });
    }
    let raw = fs::read_to_string(&path)?;
    let doc = parse(&raw, slug);

    let cur_fp = fingerprint(&path, raw.as_bytes())?;
    let external_edited = match last_fingerprint {
        Some(prev) => prev.hash != cur_fp.hash,
        None => false,
    };

    Ok(MemoryReadResult {
        doc,
        external_edited,
        file_existed: true,
    })
}

pub fn write(data_dir: &Path, doc: &MemoryDoc) -> AppResult<MemoryFingerprint> {
    let dir = study_dir(data_dir, &doc.study);
    fs::create_dir_all(&dir)?;
    let path = dir.join(MEMORY_FILENAME);
    let tmp = path.with_extension("md.tmp");
    let serialized = serialize(doc);
    fs::write(&tmp, serialized.as_bytes())?;
    fs::rename(&tmp, &path)?;
    fingerprint(&path, serialized.as_bytes())
}

/// 파일 mtime + sha256 hex.
fn fingerprint(path: &Path, contents: &[u8]) -> AppResult<MemoryFingerprint> {
    let metadata = fs::metadata(path)?;
    let mtime = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(contents);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    Ok(MemoryFingerprint {
        mtime_unix: mtime,
        hash: hex,
    })
}

// ---- frontmatter parser / serializer (Overview와 같은 정책, 다른 키 셋) ----

fn parse(raw: &str, fallback_slug: &str) -> MemoryDoc {
    let (front, body) = split_frontmatter(raw);
    let mut doc = MemoryDoc {
        study: fallback_slug.to_string(),
        updated: String::new(),
        body: body.to_string(),
    };
    for line in front.lines() {
        let trimmed = strip_comment(line.trim());
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = trim_quotes(value.trim());
        match key {
            "study" => doc.study = value.to_string(),
            "updated" => doc.updated = value.to_string(),
            _ => {}
        }
    }
    doc
}

fn serialize(doc: &MemoryDoc) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&fmt_field("updated", &doc.updated));
    out.push_str(&fmt_field("study", &doc.study));
    out.push_str("---\n\n");
    out.push_str(&doc.body);
    if !doc.body.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn fmt_field(key: &str, value: &str) -> String {
    let needs_quote =
        value.is_empty() || value.contains([':', '#', '"', '\n']) || value.starts_with([' ', '\t']);
    if needs_quote {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{key}: \"{escaped}\"\n")
    } else {
        format!("{key}: {value}\n")
    }
}

fn split_frontmatter(raw: &str) -> (&str, &str) {
    let trimmed = raw.trim_start_matches('\u{feff}');
    let stripped = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"));
    let Some(rest) = stripped else {
        return ("", raw);
    };
    if let Some((front, body)) = find_closing_dashes(rest) {
        (front, body.trim_start_matches(['\n', '\r']))
    } else {
        ("", raw)
    }
}

fn find_closing_dashes(rest: &str) -> Option<(&str, &str)> {
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let line_no_eol = line.trim_end_matches(['\n', '\r']);
        if line_no_eol == "---" {
            let front = &rest[..offset];
            let after = &rest[offset + line.len()..];
            return Some((front, after));
        }
        offset += line.len();
    }
    None
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut prev_was_space = true;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quote = !in_quote,
            '#' if !in_quote && prev_was_space => return line[..i].trim_end(),
            _ => {}
        }
        prev_was_space = ch.is_whitespace();
    }
    line
}

fn trim_quotes(value: &str) -> &str {
    if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
        || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn default_body_template() -> String {
    let mut out = String::new();
    out.push_str("# Memory\n\n");
    for (i, h) in SECTION_HEADINGS.iter().enumerate() {
        out.push_str(h);
        out.push_str("\n\n");
        out.push_str(match i {
            0 => "(아직 누적된 선호 없음 — 학습 진행하며 시스템이 채움)\n\n",
            1 => "(아직 누적된 교정 없음)\n\n",
            2 => "(아직 진도 기록 없음)\n\n",
            3 => "(아직 패턴 누적 없음)\n\n",
            _ => "(아직 진행 목표 없음)\n\n",
        });
    }
    out.push_str("---\n\n*시스템이 자동 갱신하지만 사용자가 직접 수정해도 됨.*\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_keeps_frontmatter_and_body() {
        let doc = MemoryDoc {
            study: "rust-study".to_string(),
            updated: "2026-05-03".to_string(),
            body: "# Memory\n\n## 1. ...\n\nbody\n".to_string(),
        };
        let s = serialize(&doc);
        let parsed = parse(&s, "fallback");
        assert_eq!(parsed.study, "rust-study");
        assert_eq!(parsed.updated, "2026-05-03");
        assert!(parsed.body.contains("body"));
    }

    #[test]
    fn parse_missing_frontmatter_uses_fallback_slug() {
        let parsed = parse("no frontmatter", "fb");
        assert_eq!(parsed.study, "fb");
        assert_eq!(parsed.updated, "");
        assert!(parsed.body.contains("no frontmatter"));
    }

    #[test]
    fn write_then_read_returns_same_doc() {
        let dir = TempDir::new().unwrap();
        let doc = MemoryDoc {
            study: "x".into(),
            updated: "2026-05-03".into(),
            body: "## 1. 사용자 선호 (Preferences)\n\n- (active) 빠른 결과\n".into(),
        };
        write(dir.path(), &doc).unwrap();
        let r = read(dir.path(), "x", None).unwrap();
        assert!(r.file_existed);
        assert_eq!(r.doc.study, "x");
        assert!(r.doc.body.contains("빠른 결과"));
        assert!(!r.external_edited);
    }

    #[test]
    fn missing_file_returns_default_template_with_section_headings() {
        let dir = TempDir::new().unwrap();
        let r = read(dir.path(), "ghost", None).unwrap();
        assert!(!r.file_existed);
        assert_eq!(r.doc.study, "ghost");
        for h in SECTION_HEADINGS {
            assert!(r.doc.body.contains(h), "default body must include {h}");
        }
    }

    #[test]
    fn external_edit_detected_when_hash_changes() {
        let dir = TempDir::new().unwrap();
        let doc = MemoryDoc {
            study: "x".into(),
            updated: "t1".into(),
            body: "original\n".into(),
        };
        let fp = write(dir.path(), &doc).unwrap();

        // 외부 편집 시뮬: 파일을 직접 수정.
        let path = memory_path(dir.path(), "x");
        std::fs::write(&path, "tampered content\n").unwrap();

        let r = read(dir.path(), "x", Some(&fp)).unwrap();
        assert!(r.external_edited);
    }

    #[test]
    fn no_external_edit_when_fingerprint_matches() {
        let dir = TempDir::new().unwrap();
        let doc = MemoryDoc {
            study: "x".into(),
            updated: "t1".into(),
            body: "stable\n".into(),
        };
        let fp = write(dir.path(), &doc).unwrap();
        let r = read(dir.path(), "x", Some(&fp)).unwrap();
        assert!(!r.external_edited);
    }

    #[test]
    fn write_is_atomic_no_tmp_file_left() {
        // 정상 케이스: tmp 파일 잔류 없음.
        let dir = TempDir::new().unwrap();
        let doc = MemoryDoc {
            study: "x".into(),
            updated: "t".into(),
            body: "body\n".into(),
        };
        write(dir.path(), &doc).unwrap();
        let tmp = study_dir(dir.path(), "x").join("Memory.md.tmp");
        assert!(!tmp.exists());
    }
}

// ---- Tauri commands -------------------------------------------------------

use std::sync::Mutex;
use tauri::State;
use tracing::info;

use crate::AppState;

/// 활성 스터디의 *마지막 fingerprint*를 모듈 단위 캐시. v0.2엔 단일 active study라 OK.
/// v0.3 다중 탭 시 HashMap<slug, Fingerprint> 도입.
static LAST_FP: Mutex<Option<MemoryFingerprint>> = Mutex::new(None);

#[tauri::command]
pub fn memory_read(state: State<'_, AppState>, slug: String) -> AppResult<MemoryReadResult> {
    if slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 슬러그가 비어 있습니다".into(),
        });
    }
    let last = LAST_FP.lock().expect("memory fp mutex").clone();
    let result = read(&state.data_dir, &slug, last.as_ref())?;
    if result.file_existed {
        if let Ok(fp) = fingerprint(
            &memory_path(&state.data_dir, &slug),
            result.doc.body.as_bytes(),
        ) {
            *LAST_FP.lock().expect("memory fp mutex") = Some(fp);
        }
    }
    Ok(result)
}

#[tauri::command]
pub fn memory_write(state: State<'_, AppState>, doc: MemoryDoc) -> AppResult<MemoryFingerprint> {
    if doc.study.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "study 슬러그가 비어 있습니다".into(),
        });
    }
    let fp = write(&state.data_dir, &doc)?;
    *LAST_FP.lock().expect("memory fp mutex") = Some(fp.clone());
    info!(target: "memory", slug = %doc.study, "memory_write");
    Ok(fp)
}
