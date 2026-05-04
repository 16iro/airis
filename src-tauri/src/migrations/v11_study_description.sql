-- v11: 스터디 자유 메모/설명 컬럼 추가 (PR 68).
--   * 스터디 설정 모달에서 사용자가 짧은 메모를 남길 수 있게 한다.
--   * Memory.md(F10)와는 별개 — 사용자 의도/맥락 노트.
--   * NULL이면 비어 있는 상태로 취급.

ALTER TABLE studies ADD COLUMN description TEXT;
