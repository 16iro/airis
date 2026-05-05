-- v0.3.2 B1 — 챗 컨텍스트 가시화.
-- 어시스턴트 응답이 어느 섹션·책을 컨텍스트로 받았는지 JSON으로 보관.
-- 형식은 commands/llm.rs::ChatContextSummary 참고. NULL이면 컨텍스트 없음(또는 v0.3.1 이전 메시지).
ALTER TABLE chat_messages ADD COLUMN context_json TEXT;
