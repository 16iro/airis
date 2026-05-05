// Tauri invoke 래퍼. Rust 백엔드 commands와 1:1 매칭.
// 에러는 { kind, ... } AppError 형태로 그대로 던진다 — 호출자가 isAppError로 분기.

import { invoke } from "@tauri-apps/api/core";

import type {
  AbChoice,
  AbCompareHandle,
  AbExportResult,
  ActiveSection,
  BookContent,
  BookEntry,
  BookMetaInput,
  ChatHistoryMessage,
  ChatJobHandle,
  ClaudeAuthInfo,
  CliLoginOutcome,
  CliStatus,
  CodexAuthInfo,
  FailedJob,
  FileMeta,
  GeminiAuthInfo,
  IndexJobHandle,
  MemoryDoc,
  MemoryFingerprint,
  MemoryReadResult,
  PomodoroSession,
  PomodoroState,
  Provider,
  RecallResult,
  RuntimeInfo,
  SrsCard,
  SrsCardInput,
  UpdateInfo,
  SearchHit,
  Settings,
  StaleReport,
  StudyMeta,
  StudyOverview,
  TriggerHit,
} from "@/lib/types";

export const api = {
  apiKeyCheck: (provider: Provider, key: string) =>
    invoke<void>("api_key_check", { provider, key }),

  apiKeySet: (provider: Provider, key: string) =>
    invoke<void>("api_key_set", { provider, key }),

  apiKeyDelete: (provider: Provider) =>
    invoke<void>("api_key_delete", { provider }),

  apiKeyPresent: (provider: Provider) =>
    invoke<boolean>("api_key_present", { provider }),

  settingsRead: () => invoke<Settings>("settings_read"),

  settingsWrite: (settings: Settings) =>
    invoke<void>("settings_write", { settings }),

  fileOpen: (path: string) => invoke<FileMeta>("file_open", { path }),

  fileClose: () => invoke<void>("file_close"),

  fileCurrentContent: () =>
    invoke<string | null>("file_current_content"),

  chatSend: (
    studySlug: string,
    query: string,
    contextSectionId: string | null,
  ) =>
    invoke<ChatJobHandle>("chat_send", {
      studySlug,
      query,
      contextSectionId,
    }),

  chatHistory: (
    studySlug: string,
    limit: number | null = null,
    before: number | null = null,
  ) =>
    invoke<ChatHistoryMessage[]>("chat_history", {
      studySlug,
      limit,
      before,
    }),

  retryFailedJob: (jobId: number) =>
    invoke<ChatJobHandle>("retry_failed_job", { jobId }),

  listFailedJobs: (studySlug: string | null = null) =>
    invoke<FailedJob[]>("list_failed_jobs", { studySlug }),

  deleteFailedJob: (jobId: number) =>
    invoke<void>("delete_failed_job", { jobId }),

  // F1 — 스터디 단위.
  listStudies: () => invoke<StudyMeta[]>("list_studies"),

  createStudy: (name: string, language: string | null = null) =>
    invoke<StudyMeta>("create_study", { name, language }),

  selectStudy: (slug: string) => invoke<void>("select_study", { slug }),

  setStudyThumbnail: (slug: string, srcPath: string) =>
    invoke<StudyMeta>("set_study_thumbnail", { slug, srcPath }),

  clearStudyThumbnail: (slug: string) =>
    invoke<StudyMeta>("clear_study_thumbnail", { slug }),

  updateStudyInfo: (slug: string, name: string, description: string | null) =>
    invoke<StudyMeta>("update_study_info", { slug, name, description }),

  openStudyFolder: (slug: string) =>
    invoke<void>("open_study_folder", { slug }),

  deleteStudy: (slug: string, confirm: boolean) =>
    invoke<void>("delete_study", { slug, confirm }),

  getActiveStudy: () => invoke<StudyMeta | null>("get_active_study"),

  studyOverviewRead: (slug: string) =>
    invoke<StudyOverview>("study_overview_read", { slug }),

  studyOverviewWriteMeta: (
    slug: string,
    statedGoalChapter: string,
    deadline: string,
  ) =>
    invoke<StudyOverview>("study_overview_write_meta", {
      slug,
      statedGoalChapter,
      deadline,
    }),

  // F2 — 책 등록·인덱싱·목록·삭제.
  addMainBook: (studySlug: string, path: string, meta: BookMetaInput) =>
    invoke<BookEntry>("add_main_book", { studySlug, path, meta }),

  addSubBook: (
    studySlug: string,
    path: string,
    meta: BookMetaInput,
    roleNote: string | null = null,
  ) =>
    invoke<BookEntry>("add_sub_book", {
      studySlug,
      path,
      meta,
      roleNote,
    }),

  listBooks: (studySlug: string) =>
    invoke<BookEntry[]>("list_books", { studySlug }),

  removeBook: (studySlug: string, bookId: string) =>
    invoke<void>("remove_book", { studySlug, bookId }),

  startIndexing: (studySlug: string, bookId: string) =>
    invoke<IndexJobHandle>("start_indexing", { studySlug, bookId }),

  // F2.8 stale 감지·재인덱싱.
  checkStale: (studySlug: string) =>
    invoke<StaleReport[]>("check_stale", { studySlug }),

  reindexBook: (studySlug: string, bookId: string) =>
    invoke<IndexJobHandle>("reindex_book", { studySlug, bookId }),

  // F5 — 검색.
  searchSections: (studySlug: string, query: string, limit: number | null = null) =>
    invoke<SearchHit[]>("search_sections", { studySlug, query, limit }),

  // F3 — 책 본문 + 활성 섹션.
  bookReadRaw: (studySlug: string, bookId: string) =>
    invoke<BookContent>("book_read_raw", { studySlug, bookId }),

  setActiveSection: (bookId: string, sectionPath: string) =>
    invoke<void>("set_active_section", { bookId, sectionPath }),

  clearActiveSection: () => invoke<void>("clear_active_section"),

  getActiveSection: () => invoke<ActiveSection | null>("get_active_section"),

  // F10 — Memory.md
  memoryRead: (slug: string) =>
    invoke<MemoryReadResult>("memory_read", { slug }),

  memoryWrite: (doc: MemoryDoc) =>
    invoke<MemoryFingerprint>("memory_write", { doc }),

  memoryDetectTriggers: (text: string) =>
    invoke<TriggerHit[]>("memory_detect_triggers", { text }),

  memoryApplyTrigger: (slug: string, hit: TriggerHit) =>
    invoke<MemoryFingerprint>("memory_apply_trigger", { slug, hit }),

  // F9 — Pomodoro.
  startPomodoro: (
    studySlug: string,
    focus: boolean,
    durationMin: number | null = null,
  ) =>
    invoke<PomodoroSession>("start_pomodoro", { studySlug, focus, durationMin }),

  stopPomodoro: (
    completed: boolean,
    interruption: string | null = null,
  ) => invoke<void>("stop_pomodoro", { completed, interruption }),

  getPomodoroState: () => invoke<PomodoroState>("get_pomodoro_state"),

  // F8 — SRS.
  srsAddCard: (studySlug: string, card: SrsCardInput) =>
    invoke<SrsCard>("srs_add_card", { studySlug, card }),

  srsListDue: (studySlug: string) =>
    invoke<SrsCard[]>("srs_list_due", { studySlug }),

  srsReviewCard: (cardId: number, quality: number) =>
    invoke<SrsCard>("srs_review_card", { cardId, quality }),

  srsDeleteCard: (cardId: number) =>
    invoke<void>("srs_delete_card", { cardId }),

  // F7.7 회상 챌린지.
  recallEvaluate: (studySlug: string, chapterRef: string, userInput: string) =>
    invoke<RecallResult>("recall_evaluate", {
      studySlug,
      chapterRef,
      userInput,
    }),

  // F14 — 인앱 업데이트.
  checkForUpdate: () => invoke<UpdateInfo | null>("check_for_update"),

  // 자동 큐 워커 — 프론트가 30초 polling으로 due 잡을 받아 retryFailedJob 호출.
  listDueJobs: () => invoke<FailedJob[]>("list_due_jobs"),

  // PR 24 (D-066) — CLI 인증 흐름.
  cliRuntimeDetect: () => invoke<RuntimeInfo>("cli_runtime_detect"),

  cliStatus: (provider: Provider) =>
    invoke<CliStatus>("cli_status", { provider }),

  cliInstallProvider: (provider: Provider, forceLatest: boolean) =>
    invoke<CliStatus>("cli_install_provider", { provider, forceLatest }),

  cliAuthStatusClaude: () =>
    invoke<ClaudeAuthInfo>("cli_auth_status_claude"),

  cliAuthStatusGemini: () =>
    invoke<GeminiAuthInfo>("cli_auth_status_gemini"),

  cliAuthStatusCodex: () =>
    invoke<CodexAuthInfo>("cli_auth_status_codex"),

  cliLogin: (provider: Provider, console: boolean) =>
    invoke<CliLoginOutcome>("cli_login", { provider, console }),

  // v0.4.1 PR 5 — A/B 비교 dev panel.
  chatSendAbCompare: (studySlug: string, query: string) =>
    invoke<AbCompareHandle>("chat_send_ab_compare", { studySlug, query }),

  devAbRecordChoice: (
    handle: string,
    query: string,
    baselineText: string,
    v041Text: string,
    chose: AbChoice,
    note: string | null = null,
  ) =>
    invoke<void>("dev_ab_record_choice", {
      handle,
      query,
      baselineText,
      v041Text,
      chose,
      note,
    }),

  devAbExportResults: () => invoke<AbExportResult>("dev_ab_export_results"),
};
