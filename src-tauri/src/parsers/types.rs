// 파서 공통 타입 — 결과적으로 인덱서·검색·뷰어 모두가 같은 모델을 본다.

use serde::{Deserialize, Serialize};

/// 지원 책 포맷. 확장자 + magic header로 판별.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BookFormat {
    Md,
    Html,
    Pdf,
    Txt,
}

impl BookFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Md),
            "html" | "htm" => Some(Self::Html),
            "pdf" => Some(Self::Pdf),
            "txt" => Some(Self::Txt),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Md => "md",
            Self::Html => "html",
            Self::Pdf => "pdf",
            Self::Txt => "txt",
        }
    }
}

/// 4계층 섹션 모델의 한 노드. L1(Book)은 별도 — Section은 L2·L3.
/// L4(Paragraph)는 PR 11 인덱서가 본문에서 분할.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// 섹션 *식별 path* — 책 UUID 제외. 예: `Ch04` 또는 `Ch04/§State`.
    /// 풀 ID는 호출자가 `{book_id}/{path}`로 조합.
    pub path: String,
    /// 사람이 읽는 라벨 — 챗 인용에 그대로 쓰임. 예: `Ch04 §State`.
    pub display_label: String,
    pub level: SectionLevel,
    /// 부모 섹션의 path. 최상위는 None.
    pub parent_path: Option<String>,
    /// PDF의 경우 1-base 페이지 번호. MD/HTML은 None.
    pub page: Option<u32>,
    /// 섹션 본문(이 섹션 시작 ~ 다음 같은 레벨 시작 직전). 빈 섹션 가능.
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SectionLevel {
    /// L2 — 챕터 (h1 / Outline 최상위 / 인쇄 챕터)
    Chapter,
    /// L3 — 절 (h2~h6 / 챕터 내 항목)
    Section,
}

/// 책 한 권의 파싱 결과 — `metadata.json`으로 직렬화되는 형태.
/// PR 11 인덱서가 이 결과를 받아 keyword/embedding 인덱스를 만든다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedBook {
    pub metadata: BookMetadata,
    pub sections: Vec<Section>,
}

/// `metadata.json` 파일에 직렬화. 책 디렉토리 루트에 위치.
/// 위치: `{data_dir}/studies/{slug}/books/{book-uuid}/metadata.json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookMetadata {
    /// 책 UUID — 안정 식별자. 제목·파일이 바뀌어도 유지.
    pub book_id: String,
    pub title: String,
    pub author: Option<String>,
    pub language: String,
    pub format: BookFormat,
    /// 원본 파일 경로 (사용자 시스템 절대 경로). stale 감지에 사용.
    pub source_path: String,
    pub file_size: u64,
    /// SHA-256 hex of the source file — 무결성 + 인덱싱 캐시 키.
    pub file_hash: String,
    /// PDF의 총 페이지. 다른 포맷은 None.
    pub page_count: Option<u32>,
    /// L2·L3 섹션 수.
    pub section_count: u32,
    /// 인덱서 스키마 버전 — 마이그레이션 기준.
    pub schema_version: u32,
    /// 파싱 ISO 시각.
    pub parsed_at: String,
}

pub const METADATA_SCHEMA_VERSION: u32 = 1;
