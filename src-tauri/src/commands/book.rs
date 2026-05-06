// F2 — 책 등록·인덱싱·목록·삭제.
//
// PR 11 범위 (D-064 결정에 따른 단순화):
//   * MD/HTML/TXT 인덱싱 활성 — FTS5 키워드 검색의 기본 단위.
//   * PDF는 등록 가능하지만 *인덱싱은 PR 12*에서 BookViewer + PDFium 동봉과 함께 정식화.
//   * 임베딩·멀티 백엔드는 v0.3+ (D-065).
//
// 시그니처는 api-contract.md F2 그대로. IndexOptions는 PR 11 단계엔 단순화.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::index::keyword;
use crate::index::v041::embedder::Embedder;
use crate::index::v041::indexer::{
    index_book_with_cache as v041_index_book_with_cache, BookSource as V041BookSource,
};
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
    /// PR 60+63: 책 표지 썸네일. PDF면 등록 시 첫 페이지 자동 PNG 생성. 사용자 임의 변경 X — md/txt/html은 프론트에서 file_format 아이콘 표시.
    pub thumbnail_path: Option<String>,
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

/// F2.8 stale 감지 — 책별 변경 정황 보고.
#[derive(Debug, Clone, Serialize)]
pub struct StaleReport {
    pub book_id: String,
    pub title: String,
    /// `missing` = 파일 자체 없음 / `changed` = hash 다름 / `fresh` = 일치 (보고서엔 안 실림).
    pub status: &'static str,
    pub current_hash: Option<String>,
    pub stored_hash: String,
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
    /// 원본 파일 텍스트 (MD/HTML/TXT). PDF는 빈 문자열 — pdfjs가 source_path로 직접 로드.
    pub content: String,
    /// 원본 파일 절대 경로 — PDF 뷰어가 `convertFileSrc(path)`로 webview-safe URL 생성.
    pub source_path: String,
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

/// F2.8/F12.2 — 활성 스터디의 모든 책에 대해 *원본 파일 hash*를 비교, 변경된 책만 반환.
/// 파일이 없거나(`missing`) hash가 다르면(`changed`) 보고. 일치하는 책은 빈 보고서.
#[tauri::command]
pub fn check_stale(state: State<'_, AppState>, study_slug: String) -> AppResult<Vec<StaleReport>> {
    if study_slug.trim().is_empty() {
        return Err(AppError::InvalidInput {
            message: "스터디 슬러그가 비어 있습니다".into(),
        });
    }
    let books = {
        let db = state.db.lock().expect("db mutex");
        fetch_books_for_study(db.conn(), &study_slug)?
    };

    let mut reports = Vec::new();
    for book in books {
        match fs::read(&book.source_path) {
            Ok(bytes) => {
                let cur = hex_sha256(&bytes);
                if cur != book.file_hash {
                    reports.push(StaleReport {
                        book_id: book.id,
                        title: book.title,
                        status: "changed",
                        current_hash: Some(cur),
                        stored_hash: book.file_hash,
                    });
                }
            }
            Err(_) => {
                reports.push(StaleReport {
                    book_id: book.id,
                    title: book.title,
                    status: "missing",
                    current_hash: None,
                    stored_hash: book.file_hash,
                });
            }
        }
    }
    Ok(reports)
}

/// 변경된 파일 재인덱싱 — 사용자가 명시 클릭하는 자리.
///
/// v0.4.1 PR 4: 두 가지 인덱싱을 *함께* 수행한다.
///   1. paragraphs FTS rebuild (v0.3.2 흐름) — 무파괴 호환을 위해 항상 유지.
///   2. v0.4.1 chunks + chunks_fts + vectors_t1 적재 — 새 RAG 엔진의 진입.
///
/// 직렬화: AppState.indexer_lock으로 single-flight 큐 (D-076). 같은 책 두 번 누름도 자연 차단.
/// 임베더: AppState.embedder는 lazy init — 첫 reindex 호출 때만 ~120MB fastembed 모델 로드.
///
/// 파일이 missing이면 InvalidInput. 사용자가 *원본 위치*를 다시 지정해야 한다 (UI: remove + re-add).
#[tauri::command]
pub async fn reindex_book(
    app: AppHandle,
    state: State<'_, AppState>,
    study_slug: String,
    book_id: String,
) -> AppResult<IndexJobHandle> {
    // 1) 현재 파일 읽기 + hash 계산.
    let book = {
        let db = state.db.lock().expect("db mutex");
        fetch_book(db.conn(), &study_slug, &book_id)?.ok_or_else(|| AppError::NotFound {
            message: format!("책 '{book_id}'을 찾을 수 없습니다"),
        })?
    };
    let bytes = fs::read(&book.source_path).map_err(|e| AppError::InvalidInput {
        message: format!("원본 파일을 읽을 수 없습니다 ({}): {e}", book.source_path),
    })?;
    let new_hash = hex_sha256(&bytes);
    let new_size = bytes.len() as i64;

    // 2) DB 갱신 — hash·size·last_modified.
    {
        let db = state.db.lock().expect("db mutex");
        db.conn().execute(
            "UPDATE books SET file_hash = ?1, file_size = ?2, last_modified = datetime('now')
             WHERE id = ?3",
            params![new_hash, new_size, book_id],
        )?;
    }
    info!(
        target: "book",
        study = %study_slug,
        book = %book_id,
        "reindex: hash updated, starting indexing"
    );

    // state를 start_indexing 호출이 소모하기 *전*에 v041 단계가 필요로 하는 핸들을 모두 추출.
    let app_data_dir = state.data_dir.clone();
    let pdfium_lib_dir = state.pdfium_lib_dir.clone();
    let indexer_lock = state.indexer_lock.clone();
    let embedder_slot = state.embedder.clone();
    // v0.4.2 PR 4 (D-084) — 인덱서가 batch별로 활용. 인덱서 commit과 같은 트랜잭션 안에서
    // cache row도 영속(같은 conn → 같은 tx 핸들).
    let embedding_cache = state.embedding_cache.clone();

    // 3) v0.3.2 paragraphs FTS rebuild — 기존 흐름 (무파괴 보존).
    //    재인덱싱 진입 시점에 *response_cache* 의 해당 책 row 모두 무효화 (D-084 invalidation 트리거).
    //    chunks가 갱신되면 같은 query라도 다른 chunk_ids 셋이 나올 수 있으므로 stale row를 통째로 제거.
    {
        let db = state.db.lock().expect("db mutex");
        match state.response_cache.invalidate_book(db.conn(), &book_id) {
            Ok(removed) if removed > 0 => {
                info!(
                    target: "cache",
                    book = %book_id,
                    removed,
                    "reindex: response_cache invalidated for book"
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    target: "cache",
                    book = %book_id,
                    error = %e,
                    "reindex: response_cache invalidate 실패 (non-fatal)"
                );
            }
        }
    }
    let handle = start_indexing(app.clone(), state, study_slug.clone(), book_id.clone()).await?;

    // 4) v0.4.1 chunks 적재 — single-flight 큐로 직렬화.
    //    파싱은 이미 paragraphs 빌드에서 한 번 했지만, 두 인덱서가 청크 정의가 다르므로 다시.
    //    파싱 비용(MD/HTML/TXT)은 무시할 수준. PDF는 무거우니 폴백 단위(page) 기준으로 분리.
    let format = BookFormat::from_extension(&book.file_format).ok_or_else(|| AppError::InvalidInput {
        message: format!("지원하지 않는 형식: {}", book.file_format),
    })?;
    let source_path = book.source_path.clone();
    let book_id_for_v041 = book.id.clone();
    let app_emit = app.clone();
    let book_id_for_emit = book.id.clone();
    let app_for_db = app.clone();

    // spawn_blocking — 파일 I/O + 파서 + 임베딩 + DB 쓰기 모두 동기 작업.
    let v041_outcome = tokio::task::spawn_blocking(move || -> AppResult<usize> {
        // single-flight: 같은 시점 1개 책만 인덱싱.
        let _guard = indexer_lock.lock().expect("indexer_lock poisoned");

        // 4-1) 파싱 (start_indexing에서 한 번 했지만 v041은 ChunkRecord 단위라 재파싱 필요).
        let v041_source = parse_for_v041(&source_path, format, pdfium_lib_dir.as_deref())?;

        // 4-2) Embedder lazy init — 모델 다운로드는 첫 호출에서 한 번.
        emit_progress(&app_emit, &book_id_for_emit, 70, "embed_init");
        let embedder = ensure_embedder(&embedder_slot, &app_data_dir)?;

        // 4-3) chunks 적재 + 임베딩.
        emit_progress(&app_emit, &book_id_for_emit, 75, "embed");
        let outcome_chunks = {
            let state = app_for_db.state::<AppState>();
            let mut db = state.db.lock().expect("db mutex");
            let src = match &v041_source {
                V041Parsed::Sections(s) => V041BookSource::Sections(s),
                V041Parsed::Pages(p) => V041BookSource::Pages(p),
            };
            let outcome = v041_index_book_with_cache(
                db.conn_mut(),
                &book_id_for_v041,
                src,
                Some(&embedder),
                &app_data_dir,
                Some(embedding_cache.as_ref()),
            )?;
            outcome.chunks_inserted
        };

        emit_progress(&app_emit, &book_id_for_emit, 100, "done");
        Ok(outcome_chunks)
    })
    .await
    .map_err(|e| AppError::Internal {
        message: format!("v041 reindex spawn join error: {e}"),
    })??;

    info!(
        target: "book",
        study = %study_slug,
        book = %book_id,
        v041_chunks = v041_outcome,
        "reindex: v041 chunks indexed"
    );

    Ok(handle)
}

/// v041 인덱서 입력 — 파서 결과의 *소유* 버전(=spawn_blocking 안에서 만들어 호출 측이 보유).
enum V041Parsed {
    Sections(Vec<Section>),
    Pages(Vec<String>),
}

/// 책 본문을 v041 인덱서가 받아갈 파싱 결과로 변환.
fn parse_for_v041(
    path: &str,
    format: BookFormat,
    pdfium_lib_dir: Option<&Path>,
) -> AppResult<V041Parsed> {
    Ok(match format {
        BookFormat::Md | BookFormat::Txt => {
            let raw = fs::read_to_string(path)?;
            V041Parsed::Sections(markdown::parse(&raw))
        }
        BookFormat::Html => {
            let raw = fs::read_to_string(path)?;
            V041Parsed::Sections(html::parse(&raw))
        }
        BookFormat::Pdf => {
            let lib = pdfium_lib_dir.ok_or_else(|| AppError::InvalidInput {
                message: "PDFium 라이브러리가 설치되지 않았습니다.".into(),
            })?;
            let result = pdf::parse(Path::new(path), Some(lib))?;
            // PDF는 페이지 단위 폴백 — 섹션 재구성은 v0.4.2 영역.
            // result.sections에서 page를 부모 키로 묶어 본문만 재구성.
            let mut pages: Vec<String> = Vec::new();
            let mut current_page: Option<i64> = None;
            for s in &result.sections {
                let page = s.page.unwrap_or(0) as i64;
                if Some(page) != current_page {
                    pages.push(s.body.clone());
                    current_page = Some(page);
                } else if let Some(last) = pages.last_mut() {
                    last.push('\n');
                    last.push_str(&s.body);
                }
            }
            // PDF가 sections 0개를 돌려주면 빈 vec — index_book이 graceful 처리.
            V041Parsed::Pages(pages)
        }
    })
}

/// AppState.embedder lazy init — 처음 1회만 fastembed 모델 다운로드 + 로드.
///
/// 결과는 `Arc<Embedder>`로 반환 — 호출 측이 mutex를 잡고 있는 동안 모델을 잡지 않도록
/// 짧게 lock을 잡고 Arc clone만 반환.
fn ensure_embedder(
    slot: &Arc<std::sync::Mutex<Option<Arc<Embedder>>>>,
    app_data_dir: &Path,
) -> AppResult<Arc<Embedder>> {
    {
        let guard = slot.lock().expect("embedder slot poisoned");
        if let Some(emb) = guard.as_ref() {
            return Ok(emb.clone());
        }
    }
    // 동시 init 경합이 와도 Embedder::new는 idempotent (cache hit) — 두 번 init 비용은
    // 첫 호출 외에는 사실상 0. 락을 짧게 잡기 위해 lock 밖에서 init.
    let new_emb = Arc::new(Embedder::new(app_data_dir)?);
    let mut guard = slot.lock().expect("embedder slot poisoned");
    if let Some(emb) = guard.as_ref() {
        // 다른 thread가 먼저 채워 넣었다면 그걸 사용 (init 결과는 폐기).
        return Ok(emb.clone());
    }
    *guard = Some(new_emb.clone());
    Ok(new_emb)
}

// `app_data_dir`는 spawn_blocking으로 넘겨야 해서 PathBuf로 한 번 보유.
// (비공개 헬퍼 — 시그니처 일관성 + future-proof)
#[allow(dead_code)]
fn app_data_dir_buf(state: &AppState) -> PathBuf {
    state.data_dir.clone()
}

/// 책의 raw 본문 + 형식 반환.
/// MD/HTML/TXT는 본문 텍스트를, PDF는 빈 content + source_path를 반환 — pdfjs가 직접 로드.
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
    let content = if book.file_format == "pdf" {
        String::new()
    } else {
        fs::read_to_string(&book.source_path)?
    };
    Ok(BookContent {
        book_id: book.id,
        format: book.file_format,
        content,
        source_path: book.source_path,
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

    // PDF면 첫 페이지 PNG로 자동 썸네일 생성. 실패해도 책 등록 자체는 살림.
    if matches!(format, BookFormat::Pdf) && state.pdfium_lib_dir.is_some() {
        let thumb_path =
            book_thumbnail_target(&state.data_dir, study_slug, &book_id, "png");
        if let Err(e) = pdf::render_first_page_png(
            p,
            state.pdfium_lib_dir.as_deref(),
            &thumb_path,
            BOOK_THUMBNAIL_PX,
        ) {
            warn!(target: "book", book = %book_id, error = %e, "pdf thumbnail render failed (non-fatal)");
        } else {
            let db = state.db.lock().expect("db mutex");
            db.conn().execute(
                "UPDATE books SET thumbnail_path = ?1 WHERE id = ?2",
                params![thumb_path.to_string_lossy(), book_id],
            )?;
        }
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

/// 책별 썸네일 저장 경로 — `<data_dir>/studies/<slug>/thumbnails/<book_id>.<ext>`.
///
/// PR 65: `.thumbnails` → `thumbnails`. asset:// 스코프 glob이 점 prefix 디렉토리를 거부해
/// webview가 이미지를 로드 못하던 버그 회피. 기존 사용자는 v10 마이그레이션이 DB path를 갱신.
fn book_thumbnail_target(
    data_dir: &Path,
    study_slug: &str,
    book_id: &str,
    ext: &str,
) -> std::path::PathBuf {
    data_dir
        .join("studies")
        .join(study_slug)
        .join("thumbnails")
        .join(format!("{book_id}.{ext}"))
}

const BOOK_THUMBNAIL_PX: u32 = 480;

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
           file_format, file_size, file_hash, added_at, last_modified, indexed_at,
           thumbnail_path
    FROM books
    WHERE id = ?1 AND study_slug = ?2
";

const BOOK_LIST_SELECT: &str = "
    SELECT id, study_slug, role, role_note, title, author, source_path,
           file_format, file_size, file_hash, added_at, last_modified, indexed_at,
           thumbnail_path
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
        thumbnail_path: r.get(13)?,
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

/// v0.4.2 PR 3 — 일시정지/재개 UI에 필요한 *확장* index:progress payload.
/// 기존 v0.3.2 listener와 호환: 같은 이벤트 채널, 추가 필드는 optional이라 무시.
fn emit_progress_v042(
    app: &AppHandle,
    book_id: &str,
    percent: u32,
    step: &str,
    job_id: Option<i64>,
    status: Option<&str>,
    pause_reason: Option<&str>,
) {
    let mut payload = serde_json::json!({
        "book_id": book_id,
        "percent": percent,
        "current_step": step,
    });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(jid) = job_id {
            obj.insert("job_id".to_string(), serde_json::json!(jid));
        }
        if let Some(s) = status {
            obj.insert("status".to_string(), serde_json::json!(s));
        }
        if let Some(r) = pause_reason {
            obj.insert("pause_reason".to_string(), serde_json::json!(r));
        }
    }
    if let Err(e) = app.emit("index:progress", payload) {
        warn!(target: "book", error = %e, "index:progress (v042) emit failed");
    }
}

// =============================================================================
// v0.4.2 PR 3 — 일시정지/재개 UI + T2 빌드 wiring + 4 트리거 OS 통합
// =============================================================================

use crate::index::v042::active_index::write_active_index_atomic;
use crate::index::v042::embedder_t2::EmbedderT2;
use crate::index::v042::indexer_t2::{
    build_t2_for_chunks_with_cache, create_t2_job, PassageEmbedder, T2Outcome,
};
use crate::index::v042::manifest::{
    ensure_tier_dir, manifest_path, read_manifest, write_manifest_atomic, IndexKind, Manifest,
};
use crate::index::v042::resume::{mark_job_completed, mark_job_paused, mark_job_running};
use crate::index::v042::worker::{IndexingWorker, PauseReason, Tier};
use crate::power_monitor::priority::{can_auto_resume, should_override};
use crate::power_monitor::PowerEvent;

/// `start_t2_build` 결과 핸들 — frontend가 job_id로 진행률 추적.
#[derive(Debug, Serialize)]
pub struct StartT2BuildHandle {
    pub job_id: i64,
    pub book_id: String,
    pub total_chunks: i64,
}

/// T1 인덱싱 ready 검증. `manifest_t1.status == 'ready'` 또는 v0.4.1 호환 케이스(기존
/// 책에 chunks가 적재돼 있고 vectors_t1이 N≥1) 둘 중 하나면 통과.
///
/// 폴더 layout이 v0.4.2 표준(notebooks/<book_id>/indexes/v1_me5-small/manifest.json)
/// 인 책은 manifest 우선, v0.4.1까지 채워진 책은 chunks 적재로 폴백.
fn assert_t1_ready(
    state: &AppState,
    book_id: &str,
    app_data_dir: &Path,
) -> AppResult<()> {
    // 1. manifest_t1.json 확인.
    let manifest_t1 = manifest_path(app_data_dir, book_id, IndexKind::V1Me5Small);
    if let Some(m) = read_manifest(&manifest_t1)? {
        if matches!(
            m.status,
            crate::index::v042::manifest::ManifestStatus::Ready
        ) {
            return Ok(());
        }
    }
    // 2. 폴백 — v0.4.1까지 채워진 책: chunks_inserted ≥ 1 + vectors_t1 ≥ 1.
    let db = state.db.lock().expect("db mutex");
    let chunk_n: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE book_id = ?1",
            params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let vec_n: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM vectors_t1 v \
             JOIN chunks c ON c.id = v.chunk_id WHERE c.book_id = ?1",
            params![book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if chunk_n > 0 && vec_n > 0 {
        return Ok(());
    }
    Err(AppError::InvalidInput {
        message: "T1 인덱싱 먼저 진행하세요 (chunks·vectors_t1 미적재)".to_string(),
    })
}

/// T2 임베더 lazy slot — 첫 호출 시 ~2GB BGE-M3 다운로드 + 로드. ensure_embedder(T1)와
/// 같은 패턴 (handoff §1.2).
fn ensure_embedder_t2(
    slot: &Arc<std::sync::Mutex<Option<Arc<EmbedderT2>>>>,
    app_data_dir: &Path,
) -> AppResult<Arc<EmbedderT2>> {
    {
        let guard = slot.lock().expect("embedder_t2 slot poisoned");
        if let Some(emb) = guard.as_ref() {
            return Ok(emb.clone());
        }
    }
    let new_emb = Arc::new(EmbedderT2::new(app_data_dir)?);
    let mut guard = slot.lock().expect("embedder_t2 slot poisoned");
    if let Some(emb) = guard.as_ref() {
        return Ok(emb.clone());
    }
    *guard = Some(new_emb.clone());
    Ok(new_emb)
}

/// T2 인덱싱(BGE-M3) 백그라운드 시작. T1 ready 검증 → embedder_t2 lazy init →
/// indexer_lock single-flight → spawn_blocking으로 격리.
///
/// 진행률은 `index:progress` 이벤트로 step in {`embed_t2`, `manifest_swap`, `done`}.
/// 완료 시 manifest_t2.status='ready' + active_index를 V2BgeM3로 핫스왑.
#[tauri::command]
pub async fn start_t2_build(
    app: AppHandle,
    state: State<'_, AppState>,
    book_id: String,
) -> AppResult<StartT2BuildHandle> {
    let app_data_dir = state.data_dir.clone();
    let indexer_lock = state.indexer_lock.clone();
    let embedder_t2_slot = state.embedder_t2.clone();
    let workers_registry = state.indexing_workers.clone();
    let power_monitor = state.power_monitor.clone();
    // v0.4.2 PR 4 (D-084) — T2 빌드도 embedding cache 활용 (재인덱싱·중복 텍스트 회피).
    let embedding_cache = state.embedding_cache.clone();

    // T1 ready 검증.
    assert_t1_ready(&state, &book_id, &app_data_dir)?;

    // 처리할 청크 수 사전 계산 (선언적 — manifest hint 용).
    let chunks_pending: Vec<(i64, String)> = {
        let db = state.db.lock().expect("db mutex");
        let mut stmt = db.conn().prepare(
            "SELECT id, text FROM chunks \
             WHERE book_id = ?1 \
               AND (embed_status_t2 IS NULL OR embed_status_t2 = 'failed') \
             ORDER BY ord ASC",
        )?;
        let rows = stmt
            .query_map(params![book_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let total_chunks = chunks_pending.len() as i64;
    if total_chunks == 0 {
        return Err(AppError::InvalidInput {
            message: "T2 인덱싱 대상 청크가 없습니다 (이미 모두 적재 완료)".into(),
        });
    }

    // indexing_jobs row + manifest_t2 building 기록.
    let job_id = {
        let db = state.db.lock().expect("db mutex");
        create_t2_job(db.conn(), &book_id, total_chunks as usize)?
    };
    {
        let m = Manifest::new_building(
            IndexKind::V2BgeM3,
            now_epoch_ms(),
            Some(total_chunks),
        );
        ensure_tier_dir(&app_data_dir, &book_id, IndexKind::V2BgeM3)?;
        let path = manifest_path(&app_data_dir, &book_id, IndexKind::V2BgeM3);
        if let Err(e) = write_manifest_atomic(&path, &m) {
            warn!(target: "book", error = %e, "manifest_t2 building 쓰기 실패 (non-fatal)");
        }
    }

    // worker 핸들 생성 + 레지스트리 등록.
    let worker = Arc::new(IndexingWorker::new(job_id, Tier::T2BgeM3));
    {
        let mut map = workers_registry.lock().expect("indexing_workers mutex");
        map.insert(job_id, worker.clone());
    }

    // PowerMonitor → IndexingWorker 통합 — D-081 우선순위 정책 적용.
    {
        let worker_for_power = worker.clone();
        let app_for_emit = app.clone();
        let book_id_for_emit = book_id.clone();
        power_monitor.subscribe(Arc::new(move |event| {
            handle_power_event(
                event,
                &worker_for_power,
                &app_for_emit,
                &book_id_for_emit,
                job_id,
            );
        }));
    }

    emit_progress_v042(
        &app,
        &book_id,
        70,
        "embed_t2",
        Some(job_id),
        Some("running"),
        None,
    );

    let app_data_dir_for_task = app_data_dir.clone();
    let book_id_for_task = book_id.clone();
    let app_for_task = app.clone();
    let app_for_db = app.clone();
    let worker_for_task = worker.clone();

    let _outcome = tokio::task::spawn_blocking(move || -> AppResult<T2Outcome> {
        let _guard = indexer_lock.lock().expect("indexer_lock poisoned");

        // embedder lazy init (~2GB 첫 다운로드).
        emit_progress_v042(
            &app_for_task,
            &book_id_for_task,
            72,
            "embed_t2",
            Some(job_id),
            Some("running"),
            None,
        );
        let embedder = ensure_embedder_t2(&embedder_t2_slot, &app_data_dir_for_task)?;

        // build_t2_for_chunks 호출 — pause/cancel 점검은 worker가 책임.
        let outcome = {
            let state = app_for_db.state::<AppState>();
            let mut db = state.db.lock().expect("db mutex");
            let embedder_dyn: &dyn PassageEmbedder = embedder.as_ref();
            build_t2_for_chunks_with_cache(
                db.conn_mut(),
                job_id,
                &chunks_pending,
                embedder_dyn,
                worker_for_task.as_ref(),
                Some(embedding_cache.as_ref()),
            )?
        };

        // manifest_t2 ready 전환 + active_index 핫스왑.
        emit_progress_v042(
            &app_for_task,
            &book_id_for_task,
            95,
            "manifest_swap",
            Some(job_id),
            Some("running"),
            None,
        );

        if !outcome.cancelled {
            let mut m = match read_manifest(&manifest_path(
                &app_data_dir_for_task,
                &book_id_for_task,
                IndexKind::V2BgeM3,
            ))? {
                Some(m) => m,
                None => Manifest::new_building(IndexKind::V2BgeM3, now_epoch_ms(), Some(total_chunks)),
            };
            m.mark_ready(now_epoch_ms(), total_chunks);
            let path =
                manifest_path(&app_data_dir_for_task, &book_id_for_task, IndexKind::V2BgeM3);
            write_manifest_atomic(&path, &m)?;
            write_active_index_atomic(
                &app_data_dir_for_task,
                &book_id_for_task,
                IndexKind::V2BgeM3,
            )?;

            // indexing_jobs 완료 마킹.
            let state = app_for_db.state::<AppState>();
            let db = state.db.lock().expect("db mutex");
            mark_job_completed(db.conn(), job_id)?;
        }
        Ok(outcome)
    })
    .await
    .map_err(|e| AppError::Internal {
        message: format!("start_t2_build spawn join error: {e}"),
    })??;

    // 레지스트리에서 제거.
    {
        let mut map = state.indexing_workers.lock().expect("indexing_workers mutex");
        map.remove(&job_id);
    }

    emit_progress_v042(
        &app,
        &book_id,
        100,
        "done",
        Some(job_id),
        Some("completed"),
        None,
    );

    Ok(StartT2BuildHandle {
        job_id,
        book_id,
        total_chunks,
    })
}

/// 사용자 명시 일시정지 — `pause_reason='user'`. D-081에서 가장 강한 사유.
#[tauri::command]
pub fn pause_indexing_job(
    state: State<'_, AppState>,
    job_id: i64,
) -> AppResult<()> {
    let worker = {
        let map = state
            .indexing_workers
            .lock()
            .expect("indexing_workers mutex");
        map.get(&job_id).cloned()
    };
    let worker = worker.ok_or_else(|| AppError::NotFound {
        message: format!("진행 중인 인덱싱 잡 {job_id}을 찾을 수 없습니다"),
    })?;
    worker.pause(PauseReason::User);

    let db = state.db.lock().expect("db mutex");
    mark_job_paused(db.conn(), job_id, PauseReason::User)?;
    info!(target: "book", job_id, "사용자 일시정지");
    Ok(())
}

/// 사용자 명시 재개 — pause_reason 클리어.
#[tauri::command]
pub fn resume_indexing_job(
    state: State<'_, AppState>,
    job_id: i64,
) -> AppResult<()> {
    let worker = {
        let map = state
            .indexing_workers
            .lock()
            .expect("indexing_workers mutex");
        map.get(&job_id).cloned()
    };
    let worker = worker.ok_or_else(|| AppError::NotFound {
        message: format!("진행 중인 인덱싱 잡 {job_id}을 찾을 수 없습니다"),
    })?;
    worker.resume();

    let db = state.db.lock().expect("db mutex");
    mark_job_running(db.conn(), job_id)?;
    info!(target: "book", job_id, "사용자 재개");
    Ok(())
}

/// 사용자 명시 취소 — worker.cancel() + DB status 갱신.
///
/// 본 PR에선 `indexing_jobs.status` CHECK 제약이 ('queued','running','paused','completed','failed')
/// 만 허용 — 별도 'cancelled' 추가는 v16 마이그가 필요하다(범위 외). 따라서 *'failed' +
/// `error='cancelled by user'` 마커*로 영속한다. UI/검색 로직 모두 'failed'로 동등 취급.
#[tauri::command]
pub fn cancel_indexing_job(
    state: State<'_, AppState>,
    job_id: i64,
) -> AppResult<()> {
    let worker = {
        let map = state
            .indexing_workers
            .lock()
            .expect("indexing_workers mutex");
        map.get(&job_id).cloned()
    };
    let worker = worker.ok_or_else(|| AppError::NotFound {
        message: format!("진행 중인 인덱싱 잡 {job_id}을 찾을 수 없습니다"),
    })?;
    worker.cancel();
    // pause 상태일 수도 있으므로 wakeup해서 cancel 점검을 가능하게 함.
    worker.resume();

    let db = state.db.lock().expect("db mutex");
    db.conn().execute(
        "UPDATE indexing_jobs SET \
            status = 'failed', \
            pause_reason = NULL, \
            error = 'cancelled by user', \
            finished_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000, \
            updated_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
         WHERE id = ?1",
        params![job_id],
    )?;
    info!(target: "book", job_id, "사용자 취소");
    Ok(())
}

/// PowerEvent 콜백 → IndexingWorker pause/resume + DB 영속.
///
/// D-081 우선순위:
///   * 들어온 사유가 현재 사유보다 *강한 경우*에만 pause 갱신 (`should_override`).
///   * 자동 사유(BatteryOk·SleepResumed)는 user pause면 클리어 X (`can_auto_resume`).
fn handle_power_event(
    event: PowerEvent,
    worker: &Arc<IndexingWorker>,
    app: &AppHandle,
    book_id: &str,
    job_id: i64,
) {
    let current_reason = worker.pause_gate.last_reason();
    match event {
        PowerEvent::BatteryLow => {
            if should_override(current_reason, PauseReason::BatteryLow) {
                apply_auto_pause(worker, app, book_id, job_id, PauseReason::BatteryLow);
            }
        }
        PowerEvent::Thermal => {
            if should_override(current_reason, PauseReason::Thermal) {
                apply_auto_pause(worker, app, book_id, job_id, PauseReason::Thermal);
            }
        }
        PowerEvent::SleepEntering => {
            // 슬립 진입은 thermal과 동급으로 일단 일시정지 (slip 후 깨어났을 때 복귀).
            if should_override(current_reason, PauseReason::Thermal) {
                apply_auto_pause(worker, app, book_id, job_id, PauseReason::Thermal);
            }
        }
        PowerEvent::BatteryOk | PowerEvent::SleepResumed => {
            // 자동 해제 — user pause는 보호.
            if can_auto_resume(current_reason) {
                worker.resume();
                if let Some(map_state) = app.try_state::<AppState>() {
                    let db = map_state.db.lock().expect("db mutex");
                    if let Err(e) = mark_job_running(db.conn(), job_id) {
                        warn!(
                            target: "book",
                            error = %e,
                            job_id,
                            "auto resume DB 영속 실패"
                        );
                    }
                }
                emit_progress_v042(
                    app,
                    book_id,
                    0,
                    "auto_resume",
                    Some(job_id),
                    Some("running"),
                    None,
                );
                info!(
                    target: "book",
                    job_id,
                    ?event,
                    "자동 재개 (D-081: user pause는 보호)"
                );
            } else {
                tracing::debug!(
                    target: "book",
                    job_id,
                    ?event,
                    "자동 재개 skip — user pause 보호"
                );
            }
        }
        PowerEvent::AppQuitRequested => {
            // graceful shutdown — pause(AppQuit) + cancel.
            worker.pause(PauseReason::AppQuit);
            worker.cancel();
            worker.resume();
            if let Some(map_state) = app.try_state::<AppState>() {
                let db = map_state.db.lock().expect("db mutex");
                if let Err(e) = mark_job_paused(db.conn(), job_id, PauseReason::AppQuit) {
                    warn!(target: "book", error = %e, job_id, "AppQuit pause DB 실패");
                }
            }
        }
    }
}

fn apply_auto_pause(
    worker: &Arc<IndexingWorker>,
    app: &AppHandle,
    book_id: &str,
    job_id: i64,
    reason: PauseReason,
) {
    worker.pause(reason);
    if let Some(map_state) = app.try_state::<AppState>() {
        let db = map_state.db.lock().expect("db mutex");
        if let Err(e) = mark_job_paused(db.conn(), job_id, reason) {
            warn!(target: "book", error = %e, job_id, ?reason, "auto pause DB 영속 실패");
        }
    }
    emit_progress_v042(
        app,
        book_id,
        0,
        "auto_pause",
        Some(job_id),
        Some("paused"),
        Some(reason.as_db_str()),
    );
    info!(target: "book", job_id, ?reason, "자동 일시정지 (D-081)");
}

fn now_epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
