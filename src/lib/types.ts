// Rust src-tauri/src/error.rs::AppError 와 1:1 매칭.
// serde(tag = "kind") 직렬화 결과 = 평탄 union.
export type AppError =
  | { kind: "NotFound"; message: string }
  | { kind: "InvalidInput"; message: string }
  | { kind: "LlmApi"; message: string }
  | { kind: "LlmQueued"; job_id: number }
  | { kind: "AuthRequired" }
  | { kind: "NetworkUnavailable" }
  | { kind: "RateLimited"; retry_after_seconds: number }
  | { kind: "Db"; message: string }
  | { kind: "Fs"; message: string }
  | { kind: "Parser"; message: string }
  | { kind: "Internal"; message: string };

export const isAppError = (e: unknown): e is AppError =>
  typeof e === "object" && e !== null && "kind" in e;

// AppError → i18n 친화 메시지. 호출자는 t(`errors.${kind}`)를 적절히 활용.
export const appErrorMessage = (e: AppError): string => {
  if ("message" in e) return e.message;
  return e.kind;
};

// 백엔드 src-tauri/src/settings.rs::Settings 와 동일한 모양.
export interface Settings {
  model: string;
  language: string;
  theme: "system" | "light" | "dark";
  welcome_seen: boolean;
}

export type Provider = "anthropic";

export const DEFAULT_SETTINGS: Settings = {
  model: "claude-opus-4-7",
  language: "ko",
  theme: "system",
  welcome_seen: false,
};

// 백엔드 commands/file.rs::FileMeta
export interface FileMeta {
  name: string;
  path: string;
  char_count: number;
}

// 백엔드 commands/llm.rs::ChatJobHandle
export interface ChatJobHandle {
  handle: string;
}

// 챗 메시지 — v0.2부터 DB 영속.
// id는 신규 메시지의 경우 클라이언트가 생성한 임시 문자열,
// chat_history로 로드된 메시지는 서버 행 id를 문자열화한 값.
export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  /** 응답 진행 중이면 true (스트리밍 도중). */
  streaming?: boolean;
  /** 에러 발생 시 표시할 메시지. */
  error?: string;
  /** 에러로 인해 큐에 적재된 잡 id. 있으면 "다시 시도" 버튼 노출. */
  job_id?: number;
  created_at: string; // ISO 8601
}

// 백엔드 commands/llm.rs::ChatHistoryMessage — chat_history 응답 항목.
export interface ChatHistoryMessage {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  created_at: string;
  model: string | null;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
}

// 백엔드 commands/study.rs::StudyMeta
export interface StudyMeta {
  slug: string;
  name: string;
  language: string;
  created_at: string;
  last_opened: string | null;
  is_active: boolean;
  book_count: number;
  session_count: number;
}

// 백엔드 commands/overview.rs::StudyOverview
export interface StudyOverview {
  study: string;
  language: string;
  created: string;
  schema_version: number;
  stated_goal_chapter: string;
  deadline: string;
  body: string;
}

// 백엔드 jobs::FailedJob — list_failed_jobs 응답
export interface FailedJob {
  id: number;
  study_slug: string;
  job_type: string;
  query: string;
  error: string | null;
  attempts: number;
  last_attempt: string | null;
  created_at: string;
}

// 백엔드 ChatEvent (chat:chunk·chat:done payload)
export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
}

// v0.1 모델 목록.
export const ANTHROPIC_MODELS = [
  { id: "claude-opus-4-7", labelKey: "settings.model.opus_label" },
  { id: "claude-sonnet-4-6", labelKey: "settings.model.sonnet_label" },
  { id: "claude-haiku-4-5", labelKey: "settings.model.haiku_label" },
] as const;
