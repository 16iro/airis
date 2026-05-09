// Rust src-tauri/src/error.rs::AppError 와 1:1 매칭.
// serde(tag = "kind") 직렬화 결과 = 평탄 union.
export type AppError =
  | { kind: "NotFound"; message: string }
  | { kind: "InvalidInput"; message: string }
  | { kind: "LlmApi"; message: string }
  | { kind: "LlmQueued"; job_id: number }
  | { kind: "AuthRequired" }
  | { kind: "NodeMissing"; message: string }
  | { kind: "CliMissing"; provider: string }
  | { kind: "CliAuthRequired"; provider: string }
  | { kind: "CliRuntime"; message: string }
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

// PR 24 (D-066) — 인증 경로. cli가 v0.2.1 메인, api_key가 Advanced 백업.
export type AuthMode = "api_key" | "cli";

/** v0.4.3 PR 1 (D-086) — 검색 강도. 백엔드 SearchStrength enum과 1:1. */
export type SearchStrength = "fast" | "balanced" | "accurate";

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
  /** PR 24 (D-066) — CLI subprocess vs 직접 API 키 호출. */
  auth_mode: AuthMode;
  /** 마지막으로 감지/설치한 CLI 버전. key=Provider. 없으면 미설치. */
  cli_versions: Record<string, string>;
  /** v0.4.1 PR 5 — A/B 비교 dev 토글. 디폴트 OFF. */
  dev_ab_compare: boolean;
  /** v0.4.3 PR 1 (D-086) — 검색 강도 토글. 디폴트 "balanced". */
  search_strength: SearchStrength;
  /**
   * v0.4.4 PR 2 (D-092) — dev raw chat event 콘솔 로그 토글. 디폴트 OFF.
   * ON이면 ChatPanel/AbComparePanel이 chat:* 이벤트마다 console.debug로 payload를
   * 출력 — BUG-002 같은 listener 누수 회귀를 디버깅할 때 사용.
   */
  dev_event_log: boolean;
  /**
   * v0.4.4 PR 4 (D-094) — 사용자 수동 등급 override. null이면 자동 추천을 따름.
   */
  hardware_tier_override: HardwareTier | null;
  /**
   * v0.4.4 PR 4 (D-094) — 첫 추천 표시 시점 (epoch ms). null이면 카드 미노출 상태 →
   * 첫 진입 시 자동 표시.
   */
  hardware_recommended_at: number | null;
}

/** v0.4.4 PR 4 (D-094) — 백엔드 RecommendedTier enum과 1:1 (lowercase). */
export type HardwareTier = "conservative" | "balanced" | "aggressive";

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
  auth_mode: "api_key",
  cli_versions: {},
  dev_ab_compare: false,
  search_strength: "balanced",
  dev_event_log: false,
  hardware_tier_override: null,
  hardware_recommended_at: null,
};

/** v0.4.4 PR 4 (D-094) — 사용자 머신 사양. 백엔드 HardwareInfo와 1:1. */
export interface HardwareInfo {
  cpu_cores: number;
  total_ram_gb: number;
  available_ram_gb: number;
  os: string;
  arch: string;
}

/** v0.4.4 PR 4 (D-094) — 등급 + 이유 + 모델 사이즈. 백엔드 RecommendationDetail와 1:1. */
export interface RecommendationDetail {
  tier: HardwareTier;
  reason: string;
  t1_enabled: boolean;
  t2_enabled: boolean;
  t3_enabled: boolean;
  total_model_size_mb: number;
  below_minimum: boolean;
}

// PR 24 — Node·npm 런타임 정보.
export interface RuntimeInfo {
  node_path: string;
  node_version: string;
  npm_path: string;
  npm_version: string;
}

// PR 24 — 한 프로바이더의 CLI 설치 상태.
export interface CliStatus {
  provider: string;
  installed: boolean;
  binary_path: string | null;
  version: string | null;
}

// PR 24 — `claude auth status` JSON 정제본.
export interface ClaudeAuthInfo {
  logged_in: boolean;
  auth_method: string | null;
  subscription_type: string | null;
  email: string | null;
}

// PR 25 — Gemini CLI 인증 상태 (status 명령 부재 → 짧은 ping 호출의 exit code로 추정).
export interface GeminiAuthInfo {
  logged_in: boolean;
}

// PR 26 — Codex CLI 인증 상태 (`codex login status` exit code).
export interface CodexAuthInfo {
  logged_in: boolean;
}

// PR 25 — cli_login 결과. Anthropic은 OAuth 흐름을 직접 띄워 Completed.
// Gemini/Codex는 비대화형 login이 마땅치 않아 TerminalInstruction(command + hint) 반환.
export type CliLoginOutcome =
  | { kind: "Completed" }
  | { kind: "TerminalInstruction"; command: string; hint: string };

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

// v0.4.1 PR 5 — A/B 비교.
export interface AbCompareHandle {
  handle: string;
}

export type AbChoice = "baseline" | "v041" | "tie";

export interface AbExportResult {
  baseline: number;
  v041: number;
  tie: number;
  total: number;
  markdown: string;
}

// v0.4.2 PR 4 — cache stats (D-084 dev panel 가시화).
export interface CacheStatsView {
  rows: number;
  hit_count: number;
  miss_count: number;
  hit_ratio: number;
}

export interface CacheStatsPayload {
  embedding: CacheStatsView;
  response: CacheStatsView;
}

// v0.4.2 PR 5 — acceptance 측정 dev 명령 (D-083 + handoff §3 4 gate).
/** gate 1: 비정상 종료 시뮬 결과. pending_chunks_on_restart ≤ 32(BATCH_SIZE)면 PASS. */
export interface AbnormalShutdownSimulation {
  job_id: number;
  book_id: string;
  pending_chunks_on_restart: number;
}

/** gate 2: active_index 일관성 점검 결과. */
export interface ActiveIndexInspection {
  book_id: string;
  active_kind: "v0_bm25" | "v1_me5-small" | "v2_bge-m3";
  manifest_t1_status: "building" | "ready" | "failed" | null;
  manifest_t2_status: "building" | "ready" | "failed" | null;
  chunks_count: number;
  vectors_t1_count: number;
  vectors_t2_count: number;
}

/** gate 3: 같은 study 내 user→assistant 평균 응답 시간 측정 결과. */
export interface ChatResponseTiming {
  samples: number;
  avg_ms: number;
}

/** gate 4: response_cache 누적 hit/miss + ratio. */
export interface ResponseCacheHitRatio {
  rows: number;
  hit_count: number;
  miss_count: number;
  hit_ratio: number;
}

/** v0.4.3 gate 1 (인용 정확도) — 최근 N건 chat citation_scores 통계. */
export interface CitationAccuracy {
  messages: number;
  markers: number;
  pass: number;
  low: number;
  no_match: number;
  pass_ratio: number;
  avg_score: number;
}

/** v0.4.3 gate 2 (follow-up 효율) — user 메시지 follow-up 분류 통계. */
export interface FollowupSkipRate {
  user_messages: number;
  followups: number;
  reusable_followups: number;
  skip_rate: number;
}

/** v0.4.3 gate 3 (prompt prefix cache hit ratio). */
export interface PrefixCacheRatio {
  messages: number;
  cache_read_total: number;
  input_total: number;
  hit_ratio: number;
}

export type AbTrack = "baseline" | "v041";

export interface AbChunkPayload {
  handle: string;
  track: AbTrack;
  text: string;
}

export interface AbCitationViolations {
  total_markers: number;
  out_of_range: number;
  source_count: number;
}

export interface AbDonePayload {
  handle: string;
  track: AbTrack;
  text: string;
  citation_violations: AbCitationViolations;
}

export interface AbCompletePayload {
  handle: string;
}

export interface AbErrorPayload {
  handle: string;
  track: AbTrack;
  error: AppError;
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
  /** v0.3.2 B1: 어시스턴트 응답이 받은 컨텍스트 요약. user 메시지는 항상 null. */
  context?: ChatContextSummary | null;
  /** v0.4.4.x followup §1.3 — 어시스턴트 메시지를 만든 provider id (`anthropic`·`openai`·`gemini`).
   *  user 메시지는 undefined. 신규 stream 메시지는 beginAssistantStream 시 active_provider로 채움. */
  provider?: string | null;
  created_at: string; // ISO 8601
}

// 백엔드 commands/llm.rs::ChatContextSummary — chat:context 이벤트 + DB 영속.
export interface ChatContextSummary {
  /** "active_section" | "fts" | "current_file" | "v041_hybrid" | "none" */
  kind: string;
  hits: ChatContextHit[];
  /** v0.4.1 PR 4 — 인용 마커 [Sx] → chunks.id 매핑. 없으면 v0.3.2 흐름. */
  v041_chunks?: ChatV041ChunkRef[] | null;
  /** v0.4.3 PR 3 (D-087) — HyDE 사용 여부. 빠름·균형 모드는 false. */
  used_hyde?: boolean;
  /** v0.4.3 PR 4 (D-090) — 인용 검증 verdict 리스트 (chunk별 cross-encoder 점수). */
  citation_scores?: CitationVerdict[] | null;
}

// 백엔드 index/v043/citation_check.rs::CitationVerdict — `[Sx]` 인용 신뢰도.
export interface CitationVerdict {
  /** 1-base source 인덱스. ChatV041ChunkRef.marker 의 숫자 부분과 일치. */
  source_idx: number;
  /** cross-encoder raw score (또는 substring 폴백 0.6/0.0). */
  score: number;
  /** "pass" — 통과, "low" — 의심 (경고 톤), "no_match" — 매우 낮음. */
  verdict: "pass" | "low" | "no_match";
}

export interface ChatContextHit {
  book_id: string | null;
  book_title: string | null;
  book_role: string | null;
  section_label: string | null;
  section_path: string | null;
  page: number | null;
}

// 백엔드 commands/llm.rs::ChatV041ChunkRef — [Sx] 칩 클릭 → BookViewer 점프.
export interface ChatV041ChunkRef {
  marker: string;
  chunk_id: number;
  page: number | null;
  section_path: string | null;
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
  context: ChatContextSummary | null;
  /** v0.4.4.x followup §1.3 — 본 메시지를 만든 provider id.
   *  옛 row(NULL)는 v18 마이그가 model prefix로 백필. 여전히 NULL이면 frontend 'unknown' 폴백. */
  provider: string | null;
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
  /** PR 62: 라이브러리 카드 cover 이미지 절대 경로. NULL이면 hue gradient + 첫 글자 placeholder. */
  thumbnail_path: string | null;
  /** PR 68: 사용자가 남기는 자유 메모. 비어 있으면 NULL. */
  description: string | null;
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
  file_format: "md" | "html" | "pdf" | "txt" | "docx";
  file_size: number;
  file_hash: string;
  added_at: string;
  last_modified: string | null;
  indexed_at: string | null;
  /** PR 60 — 책 표지 썸네일 절대 경로. PDF는 자동 첫 페이지, 사용자가 임의 변경 가능. NULL이면 placeholder. */
  thumbnail_path: string | null;
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

// 백엔드 commands/book.rs::StaleReport
export interface StaleReport {
  book_id: string;
  title: string;
  status: "changed" | "missing";
  current_hash: string | null;
  stored_hash: string;
}

// 백엔드 commands/pomodoro.rs::PomodoroPhase
export type PomodoroPhase = "focus" | "break";

// 백엔드 commands/pomodoro.rs::PomodoroSession
export interface PomodoroSession {
  study_slug: string;
  phase: PomodoroPhase;
  duration_min: number;
  started_at: number;
}

// 백엔드 commands/pomodoro.rs::PomodoroState
export interface PomodoroState {
  running: boolean;
  session: PomodoroSession | null;
  remaining_sec: number;
}

// v0.5 PR 2 (D-099/D-103) — SRS on-demand 카드 생성 타입.

/** 백엔드 srs_generation.rs::SrsGenerateResult */
export interface SrsGenerateResult {
  inserted: number[];
  skipped: SkippedSrsCard[];
}

export interface SkippedSrsCard {
  chunk_id: number;
  reason: string;
}

/** generation_method 값 — 백엔드 CHECK constraint 6종과 1:1. */
export type SrsGenerationMethod =
  | "manual"
  | "legacy"
  | "deterministic_cloze"
  | "deterministic_match"
  | "deterministic_order"
  | "llm_mc4";

/** srs:generate:progress 이벤트 payload */
export interface SrsGenerateProgress {
  current: number;
  total: number;
  kind: string;
}

/** srs:generate:done 이벤트 payload */
export interface SrsGenerateDone {
  total_inserted: number;
  total_skipped: number;
  skipped_reasons: string[];
}

// 백엔드 commands/srs.rs::SrsCard
export interface SrsCard {
  id: number;
  study_slug: string;
  front: string;
  back: string;
  section_ref: string | null;
  page_ref: number | null;
  e_factor: number;
  interval_days: number;
  repetitions: number;
  due_date: string;
  last_reviewed: string | null;
  created_at: string;
}

export interface SrsCardInput {
  front: string;
  back: string;
  section_ref: string | null;
  page_ref: number | null;
}

// 백엔드 commands/recall.rs::RecallResult
export interface RecallResult {
  id: number;
  study_slug: string;
  chapter_ref: string;
  keywords_expected: string[];
  keywords_present: string[];
  keywords_missing: string[];
  passed: boolean;
  challenged_at: string;
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
  format: "md" | "html" | "pdf" | "txt" | "docx";
  /** MD/HTML/TXT는 raw 본문 텍스트. PDF는 빈 문자열 — pdfjs가 source_path로 직접 로드.
   * v0.4.4 (D-093) DOCX는 백엔드가 헤딩 단락을 `#`/`##`로 합성한 markdown 문자열을 반환. */
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

// v0.5 PR 1 (D-097/D-098) — memory_facts 타입.

export type FactKind = "preference" | "correction" | "progress" | "meta" | "goal";
export type FactSource = "user" | "trigger" | "srs" | "metacog" | "recall" | "citation" | "manual";
export type FactStatus = "active" | "archived" | "expired";

// 백엔드 commands/memory_facts.rs::Fact
export interface Fact {
  id: number;
  study_id: string;
  kind: FactKind;
  content: string;
  source: FactSource;
  confidence: number;
  status: FactStatus;
  created_at: number;
  updated_at: number;
}

// 백엔드 commands/memory_facts.rs::MemoryInjection
export interface MemoryInjection {
  l1: string;
  l2: string;
  l1_chars: number;
  l2_chars: number;
}

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
  /** ISO 8601, NULL이면 자동 retry 한도 초과 (수동만). */
  next_retry_at: string | null;
  created_at: string;
}

// 백엔드 commands/updates.rs::UpdateInfo
export interface UpdateInfo {
  current: string;
  latest: string;
  release_url: string;
  published_at: string;
  body: string;
  has_sha256: boolean;
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
