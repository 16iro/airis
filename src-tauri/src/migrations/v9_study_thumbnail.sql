-- v9: 스터디 표지 thumbnail_path 컬럼 추가 (PR 62).
--   * 라이브러리 카드 cover에 표시할 표지 이미지 경로.
--   * 사용자가 임의 이미지로 등록 (`set_study_thumbnail` command).
--   * NULL이면 프론트엔드가 hue gradient + 첫 글자 placeholder 표시.

ALTER TABLE studies ADD COLUMN thumbnail_path TEXT;
