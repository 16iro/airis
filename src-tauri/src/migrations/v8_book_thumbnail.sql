-- v8: 책 썸네일 path 컬럼 추가 (PR 60).
--   * PDF 책 등록 시 1페이지를 PNG로 자동 생성해 저장 (backend pdfium-render).
--   * 사용자가 임의 이미지로 변경 가능 (`set_book_thumbnail` command).
--   * NULL이면 프론트엔드가 placeholder 표시.

ALTER TABLE books ADD COLUMN thumbnail_path TEXT;
