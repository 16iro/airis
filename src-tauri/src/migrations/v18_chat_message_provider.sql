-- v18 — v0.4.4.x followup §1.3 — chat_messages에 provider 컬럼 추가.
--
-- 챗 응답을 만든 *프로바이더* (anthropic·openai·gemini) 를 영속해, UI가 발신자
-- 라벨을 active_provider 변경과 무관하게 *옛 메시지의 옛 provider* 로 정확히 표시.
-- auth_mode와는 별개 — CLI 어댑터든 ApiKey 어댑터든 같은 provider id를 사용.
--
-- 옛 row 백필:
--   * model 컬럼 prefix로 추론. 매칭 실패한 row는 NULL — 프론트에서 'unknown' 폴백.
--   * v2_studies_and_chat.sql의 model 컬럼은 NULL 허용 (빈 사용자 메시지·system 등).

ALTER TABLE chat_messages ADD COLUMN provider TEXT;

UPDATE chat_messages SET provider = 'anthropic'
 WHERE provider IS NULL AND model LIKE 'claude-%';
UPDATE chat_messages SET provider = 'openai'
 WHERE provider IS NULL AND (model LIKE 'gpt-%' OR model LIKE 'o1-%' OR model LIKE 'o3-%');
UPDATE chat_messages SET provider = 'gemini'
 WHERE provider IS NULL AND model LIKE 'gemini-%';
