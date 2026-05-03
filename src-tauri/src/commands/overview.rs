// Overview.md — 스터디 의도(stated_goal·deadline 등)를 담는 사용자 영역 파일.
//
// 위치: `{data_dir}/studies/{slug}/Overview.md`
// 형식: YAML frontmatter + 마크다운 본문 (design/Overview.template.md)
//
// 정책:
//   * frontmatter는 *고정 6 필드*만 인식 — schema_version 1 기준.
//     사용자가 임의 키를 추가해도 무시(보존만 — round-trip 보장은 v0.3+).
//   * 파일 자체는 사용자가 외부 에디터로 직접 편집 가능 (Memory.md와 동일 정신).
//     앱은 *읽고 컨텍스트에 활용*만 하며 임의로 덮어쓰지 않는다.
//   * 쓰기는 "원자적" — tmp 파일에 작성 후 rename.
//
// frontmatter 파서 정책 (단순 + 안전):
//   * `key: value` 형식만 인식. 값의 양끝 큰따옴표·작은따옴표 trim.
//   * `# 주석`은 줄 끝 주석으로 해석해 제거.
//   * 알 수 없는 키는 *무시*. 형식 어긋난 줄도 *무시* (앱이 죽지 않음).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

pub const OVERVIEW_FILENAME: &str = "Overview.md";
pub const OVERVIEW_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudyOverview {
    pub study: String,
    /// LLM 응답 언어. 기본 "ko".
    pub language: String,
    /// 생성일 (ISO 8601 또는 YYYY-MM-DD).
    pub created: String,
    pub schema_version: u32,
    /// 메타인지 제동의 목표 챕터. 비어있으면 F11.3·F11.4 비활성.
    pub stated_goal_chapter: String,
    /// 마감일 ISO 날짜. 비어있으면 페이스 vs 마감 비활성.
    pub deadline: String,
    /// frontmatter 아래 마크다운 본문 (사용자 자유 작성).
    pub body: String,
}

impl StudyOverview {
    /// 사용자가 마법사로 첫 생성 시 채워질 기본값.
    pub fn new_default(slug: &str, language: &str, created: &str) -> Self {
        Self {
            study: slug.to_string(),
            language: language.to_string(),
            created: created.to_string(),
            schema_version: OVERVIEW_SCHEMA_VERSION,
            stated_goal_chapter: String::new(),
            deadline: String::new(),
            body: default_body_template(),
        }
    }
}

pub fn study_dir(data_dir: &Path, slug: &str) -> PathBuf {
    data_dir.join("studies").join(slug)
}

pub fn overview_path(data_dir: &Path, slug: &str) -> PathBuf {
    study_dir(data_dir, slug).join(OVERVIEW_FILENAME)
}

/// 디스크에서 Overview.md 읽기. 파일 없으면 default 반환.
pub fn read(data_dir: &Path, slug: &str) -> AppResult<StudyOverview> {
    let path = overview_path(data_dir, slug);
    if !path.exists() {
        return Ok(StudyOverview::new_default(slug, "ko", ""));
    }
    let raw = fs::read_to_string(&path)?;
    Ok(parse(&raw, slug))
}

/// Overview.md 원자적 쓰기 — `.tmp`에 작성 후 rename.
pub fn write(data_dir: &Path, overview: &StudyOverview) -> AppResult<()> {
    let dir = study_dir(data_dir, &overview.study);
    fs::create_dir_all(&dir)?;
    let path = dir.join(OVERVIEW_FILENAME);
    let tmp = path.with_extension("md.tmp");
    let serialized = serialize(overview);
    fs::write(&tmp, serialized.as_bytes())?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// 마법사 생성 흐름 — 슬러그·언어·생성일을 받아 default 본문으로 Overview.md 첫 작성.
pub fn create_default(
    data_dir: &Path,
    slug: &str,
    language: &str,
    created: &str,
) -> AppResult<StudyOverview> {
    let overview = StudyOverview::new_default(slug, language, created);
    write(data_dir, &overview)?;
    Ok(overview)
}

/// 사용자가 입력한 stated_goal·deadline을 기존 Overview에 병합 후 저장.
/// body는 *덮어쓰지 않고* 기존 그대로 유지 — 사용자 자유 작성 영역이라.
pub fn patch_meta(
    data_dir: &Path,
    slug: &str,
    stated_goal_chapter: &str,
    deadline: &str,
) -> AppResult<StudyOverview> {
    let mut overview = read(data_dir, slug)?;
    overview.stated_goal_chapter = stated_goal_chapter.trim().to_string();
    overview.deadline = deadline.trim().to_string();
    write(data_dir, &overview)?;
    Ok(overview)
}

// ---- frontmatter parser / serializer --------------------------------------

fn parse(raw: &str, fallback_slug: &str) -> StudyOverview {
    let (front, body) = split_frontmatter(raw);
    let mut overview = StudyOverview::new_default(fallback_slug, "ko", "");
    overview.body = body.to_string();

    for line in front.lines() {
        let trimmed = strip_comment(line.trim());
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = trim_quotes(value.trim());
        match key {
            "study" => overview.study = value.to_string(),
            "language" => overview.language = value.to_string(),
            "created" => overview.created = value.to_string(),
            "schema_version" => {
                if let Ok(v) = value.parse::<u32>() {
                    overview.schema_version = v;
                }
            }
            "stated_goal_chapter" => overview.stated_goal_chapter = value.to_string(),
            "deadline" => overview.deadline = value.to_string(),
            _ => {} // 알 수 없는 키 무시
        }
    }
    overview
}

fn serialize(o: &StudyOverview) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&fmt_field("study", &o.study));
    out.push_str(&fmt_field("language", &o.language));
    out.push_str(&fmt_field("created", &o.created));
    out.push_str(&format!("schema_version: {}\n", o.schema_version));
    out.push_str(&fmt_field("stated_goal_chapter", &o.stated_goal_chapter));
    out.push_str(&fmt_field("deadline", &o.deadline));
    out.push_str("---\n\n");
    out.push_str(&o.body);
    if !o.body.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn fmt_field(key: &str, value: &str) -> String {
    // 값이 비어있어도 키는 유지 — 사용자가 외부 에디터에서 채워 넣을 수 있게.
    // 따옴표 또는 콜론·해시·줄바꿈 포함이면 큰따옴표로 감싸기.
    let needs_quote =
        value.is_empty() || value.contains([':', '#', '"', '\n']) || value.starts_with([' ', '\t']);
    if needs_quote {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{key}: \"{escaped}\"\n")
    } else {
        format!("{key}: {value}\n")
    }
}

/// `---` 첫 줄 + 닫는 `---` 사이 = front. 닫힘 못 찾으면 front 빈 문자열, body 전체.
fn split_frontmatter(raw: &str) -> (&str, &str) {
    let trimmed = raw.trim_start_matches('\u{feff}'); // BOM 안전
    let stripped = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"));
    let Some(rest) = stripped else {
        return ("", raw);
    };

    // 닫는 `---` 줄을 찾는다.
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
    // YAML 인라인 주석 — 큰따옴표 안의 #은 무시. 단순 처리: " 짝수 개수일 때만 # 적용.
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
    "# 스터디 개요\n\n\
     (이 스터디가 다루는 분야·범위를 한 단락으로 — 마법사 후 직접 편집 가능)\n\n\
     ## 핵심 키워드\n\n\
     - \n\n\
     # 스터디 목적\n\n\
     ## 최종 산출물\n\n\
     - \n\n\
     ## 함양하려는 스킬\n\n\
     - \n\n\
     # 사전 지식\n\n\
     - **프로그래밍 일반**: \n\
     - **인접 분야 경험**: \n\
     - **이 분야 자체**: \n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_keeps_all_fields() {
        let mut o = StudyOverview::new_default("rust-study", "ko", "2026-05-03");
        o.stated_goal_chapter = "Ch09".to_string();
        o.deadline = "2026-08-31".to_string();
        o.body = "# 본문\n임의 마크다운\n".to_string();
        let s = serialize(&o);
        let parsed = parse(&s, "fallback");
        assert_eq!(parsed, o);
    }

    #[test]
    fn parse_ignores_unknown_keys() {
        let raw = "---\nstudy: foo\nfuture_key: ignored\nlanguage: en\n---\n\nbody\n";
        let parsed = parse(raw, "fallback");
        assert_eq!(parsed.study, "foo");
        assert_eq!(parsed.language, "en");
        assert!(parsed.body.contains("body"));
    }

    #[test]
    fn parse_strips_inline_comments() {
        let raw = "---\nstudy: rust # 주석\nlanguage: ko    # 한국어\n---\n";
        let parsed = parse(raw, "x");
        assert_eq!(parsed.study, "rust");
        assert_eq!(parsed.language, "ko");
    }

    #[test]
    fn parse_handles_quoted_values_with_colons() {
        let raw = "---\nstated_goal_chapter: \"Ch04: State\"\n---\n";
        let parsed = parse(raw, "x");
        assert_eq!(parsed.stated_goal_chapter, "Ch04: State");
    }

    #[test]
    fn parse_missing_frontmatter_returns_defaults() {
        let raw = "no frontmatter here";
        let parsed = parse(raw, "fallback");
        assert_eq!(parsed.study, "fallback");
        assert_eq!(parsed.schema_version, OVERVIEW_SCHEMA_VERSION);
    }

    #[test]
    fn write_then_read_round_trip_on_disk() {
        let dir = TempDir::new().unwrap();
        let mut o = StudyOverview::new_default("disk-study", "ko", "2026-05-03");
        o.stated_goal_chapter = "Ch10".to_string();
        write(dir.path(), &o).unwrap();
        let loaded = read(dir.path(), "disk-study").unwrap();
        assert_eq!(loaded.stated_goal_chapter, "Ch10");
        assert_eq!(loaded.study, "disk-study");
    }

    #[test]
    fn patch_meta_preserves_body() {
        let dir = TempDir::new().unwrap();
        create_default(dir.path(), "x", "ko", "2026-05-03").unwrap();
        // 사용자가 외부에서 본문 편집했다고 가정.
        let mut existing = read(dir.path(), "x").unwrap();
        existing.body = "# 직접 편집한 본문\n사용자가 작성한 내용\n".to_string();
        write(dir.path(), &existing).unwrap();

        patch_meta(dir.path(), "x", "Ch07", "2026-12-31").unwrap();
        let after = read(dir.path(), "x").unwrap();
        assert_eq!(after.stated_goal_chapter, "Ch07");
        assert_eq!(after.deadline, "2026-12-31");
        assert!(after.body.contains("직접 편집한 본문"));
    }

    #[test]
    fn read_missing_file_returns_default_with_slug() {
        let dir = TempDir::new().unwrap();
        let o = read(dir.path(), "ghost").unwrap();
        assert_eq!(o.study, "ghost");
        assert_eq!(o.schema_version, OVERVIEW_SCHEMA_VERSION);
    }
}
