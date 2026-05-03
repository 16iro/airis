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

// 백엔드 src-tauri/src/settings.rs::Provider — D-005 부분 supersede 후 v0.2부터 3개.
export type Provider = "anthropic" | "openai" | "gemini";

export const PROVIDERS: Provider[] = ["anthropic", "openai", "gemini"];

// 백엔드 src-tauri/src/settings.rs::Settings 와 동일한 모양.
export interface Settings {
  active_provider: Provider;
  /** key = Provider 문자열. value = 해당 프로바이더 모델 id. */
  models: Record<string, string>;
  /** v0.1 호환 — 신규 코드는 models[active_provider] 사용. */
  model: string;
  language: string;
  theme: "system" | "light" | "dark";
  welcome_seen: boolean;
  intervention_level: InterventionLevel;
}

export const DEFAULT_SETTINGS: Settings = {
  active_provider: "anthropic",
  models: {
    anthropic: "claude-opus-4-7",
    openai: "gpt-4.1",
    gemini: "gemini-2.5-pro",
  },
  model: "claude-opus-4-7",
  language: "ko",
  theme: "system",
  welcome_seen: false,
  intervention_level: "confirm",
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
  /** Memory.Corrections active 항목 위반 의심 hits — 노란 배너 표시. */
  violations?: ViolationHit[];
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

// 백엔드 commands/book.rs::BookEntry
export interface BookEntry {
  id: string;
  study_slug: string;
  role: "main" | "sub";
  role_note: string | null;
  title: string;
  author: string | null;
  source_path: string;
  file_format: "md" | "html" | "pdf" | "txt";
  file_size: number;
  file_hash: string;
  added_at: string;
  last_modified: string | null;
  indexed_at: string | null;
}

export interface BookMetaInput {
  title: string;
  author: string | null;
}

// 백엔드 commands/book.rs::IndexJobHandle
export interface IndexJobHandle {
  book_id: string;
  paragraph_count: number;
}

// 백엔드 commands/search.rs::SearchHit
export interface SearchHit {
  book_id: string;
  book_title: string;
  section_path: string;
  section_label: string;
  page: number | null;
  snippet: string;
  score: number;
}

// 백엔드 commands/book.rs::BookContent
export interface BookContent {
  book_id: string;
  format: "md" | "html" | "pdf" | "txt";
  /** MD/HTML/TXT는 raw 본문 텍스트. PDF는 빈 문자열 — pdfjs가 source_path로 직접 로드. */
  content: string;
  source_path: string;
  indexed: boolean;
}

// 백엔드 commands/book.rs::ActiveSection
export interface ActiveSection {
  book_id: string;
  section_path: string;
}

// 백엔드 commands/memory.rs::MemoryDoc
export interface MemoryDoc {
  study: string;
  updated: string;
  body: string;
}

// 백엔드 commands/memory.rs::MemoryReadResult
export interface MemoryReadResult {
  doc: MemoryDoc;
  external_edited: boolean;
  file_existed: boolean;
}

// 백엔드 commands/memory.rs::MemoryFingerprint
export interface MemoryFingerprint {
  mtime_unix: number;
  hash: string;
}

// 백엔드 commands/triggers.rs::TriggerKind
export type TriggerKind = "preference" | "correction" | "goal";

// 백엔드 commands/triggers.rs::TriggerHit
export interface TriggerHit {
  kind: TriggerKind;
  matched_text: string;
  suggested_entry: string;
}

// 백엔드 settings.rs::InterventionLevel
export type InterventionLevel = "confirm" | "auto" | "off";

// 백엔드 commands/validation.rs::ViolationHit
export interface ViolationHit {
  correction_item: string;
  forbidden: string;
  matched_in_response: string;
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

// v0.1 호환 — 신규 코드는 PROVIDER_MODELS 사용.
export const ANTHROPIC_MODELS = [
  { id: "claude-opus-4-7", labelKey: "settings.model.opus_label" },
  { id: "claude-sonnet-4-6", labelKey: "settings.model.sonnet_label" },
  { id: "claude-haiku-4-5", labelKey: "settings.model.haiku_label" },
] as const;

/**
 * PR 13 — 프로바이더별 정적 모델 목록 (handoff 결정 #3).
 * list-models 런타임 fetch는 v0.3 이후 도입 검토.
 */
export const PROVIDER_MODELS: Record<
  Provider,
  ReadonlyArray<{ id: string; labelKey: string }>
> = {
  anthropic: ANTHROPIC_MODELS,
  openai: [
    { id: "gpt-4.1", labelKey: "settings.model.openai.gpt41" },
    { id: "gpt-4.1-mini", labelKey: "settings.model.openai.gpt41_mini" },
    { id: "o4-mini", labelKey: "settings.model.openai.o4_mini" },
  ],
  gemini: [
    { id: "gemini-2.5-pro", labelKey: "settings.model.gemini.pro_25" },
    { id: "gemini-2.5-flash", labelKey: "settings.model.gemini.flash_25" },
  ],
};

export const PROVIDER_KEY_HINT: Record<Provider, { prefix: string; placeholder: string }> = {
  anthropic: { prefix: "sk-ant-", placeholder: "sk-ant-..." },
  openai: { prefix: "sk-", placeholder: "sk-..." },
  gemini: { prefix: "AIza", placeholder: "AIza..." },
};
