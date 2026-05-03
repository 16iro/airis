// F14 인앱 업데이트 알림 — GitHub Releases API 폴링.
//
// 정책 (PR 23):
//   * 앱 시작 시 1회 + 24시간 throttle (호출자 = 프론트가 last_check timestamp localStorage)
//   * 폴링: GET https://api.github.com/repos/16iro/airis/releases/latest
//   * latest > current_version이면 UpdateInfo 반환 (브라우저로 release 페이지 open은 프론트)
//   * SHA256 검증은 release-pipeline.md 무서명 정책 — release notes에 명시 가정 (PR 23엔 *표시*만)

use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;

const RELEASES_URL: &str = "https://api.github.com/repos/16iro/airis/releases/latest";

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub release_url: String,
    pub published_at: String,
    /// release notes 본문 (markdown). UI가 "변경 내역" 표시.
    pub body: String,
    /// SHA256 manifest 첨부가 있는지 — release notes에 표시된 패턴 체크.
    pub has_sha256: bool,
}

#[tauri::command]
pub async fn check_for_update(state: State<'_, AppState>) -> AppResult<Option<UpdateInfo>> {
    let _ = state; // 향후 캐싱·throttle 시 사용
    let current = env!("CARGO_PKG_VERSION").to_string();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("airis-update-check")
        .build()
        .map_err(|e| AppError::Internal {
            message: format!("http client init: {e}"),
        })?;

    let resp = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                AppError::NetworkUnavailable
            } else {
                AppError::Internal {
                    message: format!("update check: {e}"),
                }
            }
        })?;

    if resp.status().as_u16() == 404 {
        // 릴리즈 자체가 아직 없음 — graceful.
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(AppError::Internal {
            message: format!("update check HTTP {}", resp.status()),
        });
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| AppError::Internal {
        message: format!("update json: {e}"),
    })?;

    let tag = body
        .get("tag_name")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();
    let release_url = body
        .get("html_url")
        .and_then(|t| t.as_str())
        .unwrap_or(RELEASES_URL)
        .to_string();
    let published_at = body
        .get("published_at")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let notes = body
        .get("body")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    if tag.is_empty() || !is_newer(&tag, &current) {
        return Ok(None);
    }

    let has_sha256 = notes.to_lowercase().contains("sha256");
    Ok(Some(UpdateInfo {
        current,
        latest: tag,
        release_url,
        published_at,
        body: notes,
        has_sha256,
    }))
}

/// SemVer 단순 비교 — `a > b`이면 true. parse 실패 시 false.
fn is_newer(a: &str, b: &str) -> bool {
    let pa = parse_semver(a);
    let pb = parse_semver(b);
    match (pa, pb) {
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.splitn(3, '.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch_raw = parts.next()?;
    // pre-release suffix (`0.2.0-rc1`) 자르기.
    let patch = patch_raw
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_newer() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
    }

    #[test]
    fn semver_handles_pre_release_suffix() {
        // "0.2.0-rc1" → patch 0. 비교만 우선 (단순 휴리스틱).
        assert!(parse_semver("0.2.0-rc1").is_some());
        assert!(parse_semver("0.2.0").is_some());
    }

    #[test]
    fn semver_invalid_returns_none() {
        assert!(parse_semver("abc").is_none());
        assert!(parse_semver("0.2").is_none());
    }
}
