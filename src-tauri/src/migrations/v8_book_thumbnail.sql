-- v8: 책 썸네일 path 컬럼 추가 (PR 60, PR 63 단순화).
--   * PDF 책 등록 시 1페이지를 PNG로 자동 생성해 저장 (backend pdfium-render).
--   * md/txt/html은 NULL — 프론트엔드가 file_format 아이콘 표시.
--   * v0.4 로드맵: 콘텐츠 일부를 렌더링한 썸네일로 대체.

ALTER TABLE books ADD COLUMN thumbnail_path TEXT;
