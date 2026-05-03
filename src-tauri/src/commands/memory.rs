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

/// 압축본 — 활성(`(active`) 항목만 5섹션에서 추출.
///
/// 결과:
///   * `l1` = Preferences + Corrections (가장 critical, 매 응답에 영향)
///   * `l2` = Progress + Meta + Goals (배경 컨텍스트)
///
/// PR 16 정책: char 한도 기준 truncation (l1=2000자, l2=4000자). 초과 시 가장 오래된 항목부터 drop.
/// 토큰 정확 계산은 v0.3+ tiktoken/anthropic 어댑터 도입 시.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MemoryCompressed {
    pub l1: String,
    pub l2: String,
}

const L1_CHAR_BUDGET: usize = 2000;
const L2_CHAR_BUDGET: usize = 4000;

pub fn compress(body: &str) -> MemoryCompressed {
    let sections = parse_sections(body);
    let mut l1 = String::new();
    let mut l2 = String::new();

    for (heading, items) in &sections {
        let active: Vec<&str> = items
            .iter()
            .filter(|line| line.contains("(active"))
            .copied()
            .collect();
        if active.is_empty() {
            continue;
        }
        let target = if heading.contains("Preferences") || heading.contains("Corrections") {
            &mut l1
        } else if heading.contains("Progress")
            || heading.contains("Meta")
            || heading.contains("Goals")
        {
            &mut l2
        } else {
            continue;
        };
        target.push_str(heading);
        target.push('\n');
        for line in active {
            target.push_str(line);
            target.push('\n');
        }
        target.push('\n');
    }

    MemoryCompressed {
        l1: truncate_keep_lines(&l1, L1_CHAR_BUDGET),
        l2: truncate_keep_lines(&l2, L2_CHAR_BUDGET),
    }
}

/// h2 헤딩 기준 섹션 분해. 각 섹션의 list item(`- ...`) 라인만 수집.
fn parse_sections(body: &str) -> Vec<(String, Vec<&str>)> {
    let mut out: Vec<(String, Vec<&str>)> = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in body.lines() {
        if line.starts_with("## ") {
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some((line.to_string(), Vec::new()));
        } else if let Some((_, items)) = current.as_mut() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("- ") {
                items.push(line);
            }
        }
    }
    if let Some(prev) = current {
        out.push(prev);
    }
    out
}

/// 한도 초과 시 *맨 위 헤딩은 보존*하고 가장 오래된(아래) 항목부터 drop.
/// 단순 구현: 한도 초과면 끝에서부터 \n 단위 trim.
fn truncate_keep_lines(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut buf: String = s.chars().take(limit).collect();
    if let Some(idx) = buf.rfind('\n') {
        buf.truncate(idx);
    }
    buf.push_str("\n…\n");
    buf
}

/// 5섹션 중 하나에 항목 append. heading 라인 다음의 *첫 빈 줄 이전*에 박는다.
/// heading이 없으면 body 끝에 새 섹션을 만들어 append.
/// 반환: 갱신된 body (호출자가 MemoryDoc.body로 박은 후 write).
pub fn append_to_section(body: &str, heading: &str, item: &str) -> String {
    let normalized_item = format!("- {}", item.trim_start_matches("- ").trim());
    let prefix = format!("{heading}\n");
    if let Some(start) = body.find(&prefix) {
        // heading 라인 끝 → 다음 ## 또는 EOF 직전.
        let after = start + prefix.len();
        let rest = &body[after..];
        let next_h2 = rest
            .find("\n## ")
            .map(|p| after + p + 1) // \n 다음 # 위치
            .unwrap_or(body.len());
        let mut new_body = String::new();
        new_body.push_str(&body[..next_h2]);
        if !new_body.ends_with('\n') {
            new_body.push('\n');
        }
        if !new_body.ends_with("\n\n") {
            new_body.push('\n');
        }
        new_body.push_str(&normalized_item);
        new_body.push('\n');
        new_body.push_str(&body[next_h2..]);
        new_body
    } else {
        // heading 부재 — body 끝에 새 섹션.
        let mut new_body = body.to_string();
        if !new_body.ends_with('\n') {
            new_body.push('\n');
        }
        new_body.push_str(&format!("\n{heading}\n\n{normalized_item}\n"));
        new_body
    }
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
    fn compress_extracts_only_active_items() {
        let body = "# Memory\n\n\
## 1. 사용자 선호 (Preferences)\n\n\
- (active, since 2026-04-15) 빠른 결과 우선\n\
- (deprecated 2026-04-30) 옛 선호\n\n\
## 2. 금지·교정 (Corrections)\n\n\
- (active) 영어 그대로 둘 것\n\n\
## 3. 진도·이해도 (Progress)\n\n\
- (active) Ch04 진행 중\n";
        let c = compress(body);
        assert!(c.l1.contains("빠른 결과 우선"));
        assert!(c.l1.contains("영어 그대로 둘 것"));
        assert!(!c.l1.contains("옛 선호"));
        assert!(c.l2.contains("Ch04 진행 중"));
    }

    #[test]
    fn compress_returns_empty_when_no_active_items() {
        let body = "# Memory\n\n\
## 1. 사용자 선호 (Preferences)\n\n\
(아직 누적된 선호 없음)\n\n\
## 2. 금지·교정 (Corrections)\n\n\
- (resolved 2026-04-20) 짧게\n";
        let c = compress(body);
        assert!(c.l1.is_empty());
        assert!(c.l2.is_empty());
    }

    #[test]
    fn compress_truncates_l1_when_over_budget() {
        // 한도 초과 active 항목 — 끝에 …
        let item = "- (active) 매우 긴 내용 ".repeat(200);
        let body = format!("## 1. 사용자 선호 (Preferences)\n\n{item}\n");
        let c = compress(&body);
        assert!(c.l1.chars().count() <= L1_CHAR_BUDGET + 10);
        assert!(c.l1.ends_with("…\n"));
    }

    #[test]
    fn append_inserts_into_existing_section() {
        let body = "# Memory\n\n## 1. 사용자 선호 (Preferences)\n\n(아직 없음)\n\n## 2. 금지·교정 (Corrections)\n\n(아직 없음)\n";
        let updated = append_to_section(
            body,
            "## 1. 사용자 선호 (Preferences)",
            "(active, since 2026-05-03) 빠른 결과 우선",
        );
        assert!(updated.contains("빠른 결과 우선"));
        // Preferences 섹션 안에 있고, Corrections 헤딩보다 먼저.
        let pos_pref = updated.find("빠른 결과 우선").unwrap();
        let pos_corr = updated.find("Corrections").unwrap();
        assert!(pos_pref < pos_corr);
    }

    #[test]
    fn append_creates_section_when_missing() {
        let body = "# Memory\n\nempty body\n";
        let updated = append_to_section(body, "## 5. 학습 목표 (Goals)", "Ch09 까지 끝내기");
        assert!(updated.contains("## 5. 학습 목표 (Goals)"));
        assert!(updated.contains("Ch09 까지 끝내기"));
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

use crate::commands::triggers::{self, TriggerHit};
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

/// 사용자 발화에서 트리거 감지 — chat_send 직후 hook이 호출.
/// 결과는 *제안* — 사용자가 다이얼로그에서 OK 누르면 `memory_apply_trigger` 호출.
#[tauri::command]
pub fn memory_detect_triggers(text: String) -> AppResult<Vec<TriggerHit>> {
    Ok(triggers::detect(&text))
}

/// 트리거를 Memory에 *추가* — Memory 읽기 → append_to_section → 쓰기.
#[tauri::command]
pub fn memory_apply_trigger(
    state: State<'_, AppState>,
    slug: String,
    hit: TriggerHit,
) -> AppResult<MemoryFingerprint> {
    if slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 슬러그가 비어 있습니다".into(),
        });
    }
    let last = LAST_FP.lock().expect("memory fp mutex").clone();
    let result = read(&state.data_dir, &slug, last.as_ref())?;
    let now = chrono_iso_date_only();
    let entry = format!("(active, since {now}) {}", hit.suggested_entry.trim());
    let new_body = append_to_section(&result.doc.body, hit.kind.section_heading(), &entry);
    let updated_doc = MemoryDoc {
        study: slug.clone(),
        updated: now,
        body: new_body,
    };
    let fp = write(&state.data_dir, &updated_doc)?;
    *LAST_FP.lock().expect("memory fp mutex") = Some(fp.clone());
    info!(
        target: "memory",
        slug = %slug,
        kind = ?hit.kind,
        "memory_apply_trigger"
    );
    Ok(fp)
}

/// chrono crate 추가 비용을 피하려고 std로 YYYY-MM-DD 생성.
fn chrono_iso_date_only() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 1970-01-01부터 일수 계산.
    let days = (secs / 86400) as i64;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    // 단순 구현 — 1970-01-01 기준. 윤년 보정.
    let mut year: i32 = 1970;
    loop {
        let leap = is_leap(year);
        let yd = if leap { 366 } else { 365 };
        if days < yd as i64 {
            break;
        }
        days -= yd as i64;
        year += 1;
    }
    let months: [u32; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0;
    for (i, dm) in months.iter().enumerate() {
        if days < *dm as i64 {
            m = i + 1;
            break;
        }
        days -= *dm as i64;
    }
    (year, m as u32, days as u32 + 1)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}
