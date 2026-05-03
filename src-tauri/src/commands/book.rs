// F2 — 책 등록·인덱싱·목록·삭제.
//
// PR 11 범위 (D-064 결정에 따른 단순화):
//   * MD/HTML/TXT 인덱싱 활성 — FTS5 키워드 검색의 기본 단위.
//   * PDF는 등록 가능하지만 *인덱싱은 PR 12*에서 BookViewer + PDFium 동봉과 함께 정식화.
//   * 임베딩·멀티 백엔드는 v0.3+ (D-065).
//
// 시그니처는 api-contract.md F2 그대로. IndexOptions는 PR 11 단계엔 단순화.

use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::index::keyword;
use crate::parsers::types::{BookFormat, Section};
use crate::parsers::{html, markdown, pdf};
use crate::AppState;

const MAX_BOOK_BYTES: u64 = 50 * 1024 * 1024; // 50MB
const ALLOWED_EXTENSIONS: &[&str] = &["md", "markdown", "html", "htm", "pdf", "txt"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookEntry {
    pub id: String,
    pub study_slug: String,
    pub role: String,
    pub role_note: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub source_path: String,
    pub file_format: String,
    pub file_size: i64,
    pub file_hash: String,
    pub added_at: String,
    pub last_modified: Option<String>,
    pub indexed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BookMetaInput {
    pub title: String,
    pub author: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IndexJobHandle {
    pub book_id: String,
    pub paragraph_count: u32,
}

/// 사용자가 BookViewer에서 마지막 클릭한 헤딩. AppState 캐시.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveSection {
    pub book_id: String,
    pub section_path: String,
}

#[derive(Debug, Serialize)]
pub struct BookContent {
    pub book_id: String,
    pub format: String,
    /// 원본 파일 텍스트 (MD/HTML/TXT). PDF는 PR 12.5에서 지원.
    pub content: String,
    pub indexed: bool,
}

// ---- Tauri commands -------------------------------------------------------

#[tauri::command]
pub fn add_main_book(
    state: State<'_, AppState>,
    study_slug: String,
    path: String,
    meta: BookMetaInput,
) -> AppResult<BookEntry> {
    add_book_internal(&state, &study_slug, &path, "main", None, meta)
}

#[tauri::command]
pub fn add_sub_book(
    state: State<'_, AppState>,
    study_slug: String,
    path: String,
    meta: BookMetaInput,
    role_note: Option<String>,
) -> AppResult<BookEntry> {
    add_book_internal(&state, &study_slug, &path, "sub", role_note, meta)
}

#[tauri::command]
pub fn list_books(state: State<'_, AppState>, study_slug: String) -> AppResult<Vec<BookEntry>> {
    let db = state.db.lock().expect("db mutex");
    fetch_books_for_study(db.conn(), &study_slug)
}

#[tauri::command]
pub fn remove_book(
    state: State<'_, AppState>,
    study_slug: String,
    book_id: String,
) -> AppResult<()> {
    let db = state.db.lock().expect("db mutex");
    let affected = db.conn().execute(
        "DELETE FROM books WHERE id = ?1 AND study_slug = ?2",
        params![book_id, study_slug],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            message: format!("책 '{book_id}'을 스터디 '{study_slug}'에서 찾을 수 없습니다"),
        });
    }
    info!(target: "book", study = %study_slug, book = %book_id, "remove_book");
    Ok(())
}

#[tauri::command]
pub async fn start_indexing(
    app: AppHandle,
    state: State<'_, AppState>,
    study_slug: String,
    book_id: String,
) -> AppResult<IndexJobHandle> {
    let book = {
        let db = state.db.lock().expect("db mutex");
        fetch_book(db.conn(), &study_slug, &book_id)?.ok_or_else(|| AppError::NotFound {
            message: format!("책 '{book_id}'을 찾을 수 없습니다"),
        })?
    };

    let format =
        BookFormat::from_extension(&book.file_format).ok_or_else(|| AppError::InvalidInput {
            message: format!("지원하지 않는 형식: {}", book.file_format),
        })?;

    if matches!(format, BookFormat::Pdf) && state.pdfium_lib_dir.is_none() {
        return Err(AppError::InvalidInput {
            message: "PDFium 라이브러리가 설치되지 않았습니다. `pnpm pdfium:setup` 실행 후 다시 시도하세요.".into(),
        });
    }

    emit_progress(&app, &book.id, 10, "parse");

    // spawn_blocking: 파일 I/O + 파서 호출은 동기. tokio 런타임 밀어내지 않게.
    let path = book.source_path.clone();
    let parse_format = format;
    let pdfium_lib_dir = state.pdfium_lib_dir.clone();
    let sections = tokio::task::spawn_blocking(move || -> AppResult<Vec<Section>> {
        Ok(match parse_format {
            BookFormat::Md | BookFormat::Txt => {
                let raw = fs::read_to_string(&path)?;
                markdown::parse(&raw)
            }
            BookFormat::Html => {
                let raw = fs::read_to_string(&path)?;
                html::parse(&raw)
            }
            BookFormat::Pdf => {
                let lib = pdfium_lib_dir
                    .as_deref()
                    .expect("pdfium_lib_dir checked above");
                let result = pdf::parse(Path::new(&path), Some(lib))?;
                result.sections
            }
        })
    })
    .await
    .map_err(|e| AppError::Internal {
        message: format!("indexing task join error: {e}"),
    })??;

    emit_progress(&app, &book.id, 50, "chunk");

    // DB 작업 — 메인 thread에서 직접 (Mutex<Db> 보유).
    let book_id_for_index = book.id.clone();
    let count = {
        let mut db = state.db.lock().expect("db mutex");
        let n = keyword::rebuild_book_paragraphs(db.conn_mut(), &book_id_for_index, &sections)?;
        db.conn().execute(
            "UPDATE books SET indexed_at = datetime('now') WHERE id = ?1",
            params![book_id_for_index],
        )?;
        n
    };

    emit_progress(&app, &book.id, 100, "done");
    info!(
        target: "book",
        study = %study_slug,
        book = %book.id,
        sections = sections.len(),
        paragraphs = count,
        "indexing complete"
    );

    Ok(IndexJobHandle {
        book_id: book.id,
        paragraph_count: count,
    })
}

/// 책의 raw 본문 + 형식 반환 — BookViewer가 MD/HTML 렌더 시 사용.
/// PDF는 PR 12.5에서 별도 처리 (이 시점엔 InvalidInput).
#[tauri::command]
pub fn book_read_raw(
    state: State<'_, AppState>,
    study_slug: String,
    book_id: String,
) -> AppResult<BookContent> {
    let book = {
        let db = state.db.lock().expect("db mutex");
        fetch_book(db.conn(), &study_slug, &book_id)?.ok_or_else(|| AppError::NotFound {
            message: format!("책 '{book_id}'을 찾을 수 없습니다"),
        })?
    };
    if book.file_format == "pdf" {
        return Err(AppError::InvalidInput {
            message: "PDF 뷰어는 PR 12.5에서 활성화됩니다".into(),
        });
    }
    let content = fs::read_to_string(&book.source_path)?;
    Ok(BookContent {
        book_id: book.id,
        format: book.file_format,
        content,
        indexed: book.indexed_at.is_some(),
    })
}

#[tauri::command]
pub fn set_active_section(
    state: State<'_, AppState>,
    book_id: String,
    section_path: String,
) -> AppResult<()> {
    *state.active_section.lock().expect("active_section mutex") = Some(ActiveSection {
        book_id,
        section_path,
    });
    Ok(())
}

#[tauri::command]
pub fn clear_active_section(state: State<'_, AppState>) -> AppResult<()> {
    *state.active_section.lock().expect("active_section mutex") = None;
    Ok(())
}

#[tauri::command]
pub fn get_active_section(state: State<'_, AppState>) -> AppResult<Option<ActiveSection>> {
    Ok(state
        .active_section
        .lock()
        .expect("active_section mutex")
        .clone())
}

/// 활성 섹션의 본문을 paragraphs DB에서 조립 — chat_send가 컨텍스트 주입에 사용.
/// 같은 섹션의 모든 청크를 chunk_index 순으로 concat.
pub fn fetch_section_body(
    conn: &rusqlite::Connection,
    book_id: &str,
    section_path: &str,
) -> AppResult<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT content FROM paragraphs
         WHERE book_id = ?1 AND section_path = ?2
         ORDER BY chunk_index ASC",
    )?;
    let parts: Vec<String> = stmt
        .query_map(params![book_id, section_path], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    if parts.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parts.join("\n\n")))
    }
}

/// 활성 섹션의 *디스플레이 라벨* + 책 제목 — chat 컨텍스트 헤더용.
pub fn fetch_section_label(
    conn: &rusqlite::Connection,
    book_id: &str,
    section_path: &str,
) -> AppResult<Option<(String, String)>> {
    conn.query_row(
        "SELECT b.title, p.section_label
         FROM paragraphs p
         JOIN books b ON b.id = p.book_id
         WHERE p.book_id = ?1 AND p.section_path = ?2
         LIMIT 1",
        params![book_id, section_path],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(AppError::from)
}

// ---- helpers --------------------------------------------------------------

fn add_book_internal(
    state: &AppState,
    study_slug: &str,
    path: &str,
    role: &str,
    role_note: Option<String>,
    meta: BookMetaInput,
) -> AppResult<BookEntry> {
    let trimmed_title = meta.title.trim();
    if trimmed_title.is_empty() {
        return Err(AppError::InvalidInput {
            message: "책 제목이 비어 있습니다".into(),
        });
    }

    let p = Path::new(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .ok_or_else(|| AppError::InvalidInput {
            message: "파일 확장자를 인식할 수 없습니다".into(),
        })?;
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(AppError::InvalidInput {
            message: format!("지원하지 않는 형식: .{ext} (지원: md/html/pdf/txt)"),
        });
    }
    let format = BookFormat::from_extension(&ext).ok_or_else(|| AppError::InvalidInput {
        message: format!("지원하지 않는 형식: .{ext}"),
    })?;

    let metadata = fs::metadata(p)?;
    if metadata.len() > MAX_BOOK_BYTES {
        return Err(AppError::InvalidInput {
            message: format!(
                "책이 너무 큽니다 (최대 {}MB)",
                MAX_BOOK_BYTES / (1024 * 1024)
            ),
        });
    }

    let bytes = fs::read(p)?;
    let file_hash = hex_sha256(&bytes);

    let book_id = Uuid::new_v4().to_string();
    let last_modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| {
            // ISO 8601 단순 — full RFC3339는 chrono 의존이라 v0.2엔 epoch seconds.
            format!("epoch:{}", d.as_secs())
        });

    {
        let db = state.db.lock().expect("db mutex");
        // 스터디 실존 확인.
        let exists: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM studies WHERE slug = ?1",
            params![study_slug],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(AppError::NotFound {
                message: format!("스터디 '{study_slug}'를 찾을 수 없습니다"),
            });
        }

        db.conn().execute(
            "INSERT INTO books (
                id, study_slug, role, role_note, title, author, source_path,
                file_format, file_size, file_hash, added_at, last_modified
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'), ?11)",
            params![
                book_id,
                study_slug,
                role,
                role_note,
                trimmed_title,
                meta.author,
                p.to_string_lossy(),
                format.as_str(),
                metadata.len() as i64,
                file_hash,
                last_modified,
            ],
        )?;
    }

    let entry = {
        let db = state.db.lock().expect("db mutex");
        fetch_book(db.conn(), study_slug, &book_id)?
            .expect("book row was just inserted in same lock window")
    };

    info!(
        target: "book",
        study = %study_slug,
        book = %book_id,
        format = format.as_str(),
        bytes = metadata.len(),
        "add_book"
    );
    Ok(entry)
}

fn fetch_book(conn: &Connection, study_slug: &str, book_id: &str) -> AppResult<Option<BookEntry>> {
    conn.query_row(BOOK_SELECT, params![book_id, study_slug], map_book_row)
        .optional()
        .map_err(AppError::from)
}

fn fetch_books_for_study(conn: &Connection, study_slug: &str) -> AppResult<Vec<BookEntry>> {
    let mut stmt = conn.prepare(BOOK_LIST_SELECT)?;
    let rows = stmt.query_map(params![study_slug], map_book_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

const BOOK_SELECT: &str = "
    SELECT id, study_slug, role, role_note, title, author, source_path,
           file_format, file_size, file_hash, added_at, last_modified, indexed_at
    FROM books
    WHERE id = ?1 AND study_slug = ?2
";

const BOOK_LIST_SELECT: &str = "
    SELECT id, study_slug, role, role_note, title, author, source_path,
           file_format, file_size, file_hash, added_at, last_modified, indexed_at
    FROM books
    WHERE study_slug = ?1
    ORDER BY (role = 'main') DESC, added_at ASC
";

fn map_book_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<BookEntry> {
    Ok(BookEntry {
        id: r.get(0)?,
        study_slug: r.get(1)?,
        role: r.get(2)?,
        role_note: r.get(3)?,
        title: r.get(4)?,
        author: r.get(5)?,
        source_path: r.get(6)?,
        file_format: r.get(7)?,
        file_size: r.get(8)?,
        file_hash: r.get(9)?,
        added_at: r.get(10)?,
        last_modified: r.get(11)?,
        indexed_at: r.get(12)?,
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn emit_progress(app: &AppHandle, book_id: &str, percent: u32, step: &str) {
    if let Err(e) = app.emit(
        "index:progress",
        serde_json::json!({ "book_id": book_id, "percent": percent, "current_step": step }),
    ) {
        warn!(target: "book", error = %e, "index:progress emit failed");
    }
}
