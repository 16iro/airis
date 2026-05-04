-- v10: 썸네일 디렉토리 이름 `.thumbnails` → `thumbnails` (PR 65).
--   * Tauri asset:// 스코프 glob이 점(`.`)으로 시작하는 디렉토리를 거부해 webview가
--     이미지를 로드하지 못하는 문제 해결. 디렉토리 자체의 점 prefix가 원인.
--   * DB에 이미 저장된 thumbnail_path 문자열을 새 경로 형태로 일괄 갱신.
--   * 디스크상의 기존 `.thumbnails/` 디렉토리는 startup 훅이 `thumbnails/`로 이동
--     (Db 시퀀스 이후 lib.rs setup에서 처리).

UPDATE studies
   SET thumbnail_path = REPLACE(thumbnail_path, '/.thumbnails/', '/thumbnails/')
 WHERE thumbnail_path LIKE '%/.thumbnails/%';

UPDATE books
   SET thumbnail_path = REPLACE(thumbnail_path, '/.thumbnails/', '/thumbnails/')
 WHERE thumbnail_path LIKE '%/.thumbnails/%';
