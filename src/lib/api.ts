// Tauri invoke 래퍼. Rust 백엔드 commands와 1:1 매칭.
// 에러는 { kind, ... } AppError 형태로 그대로 던진다 — 호출자가 isAppError로 분기.

import { invoke } from "@tauri-apps/api/core";

import type {
  AbChoice,
  AbCompareHandle,
  AbExportResult,
  AbnormalShutdownSimulation,
  ActiveIndexInspection,
  ByokCostEstimate,
  ByokProvider,
  ByokRoutingResult,
  CacheStatsPayload,
  ChatResponseTiming,
  ResponseCacheHitRatio,
  CitationAccuracy,
  FollowupSkipRate,
  PrefixCacheRatio,
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
  HardwareInfo,
  IndexJobHandle,
  MemoryDoc,
  MemoryFingerprint,
  MemoryReadResult,
  PomodoroSession,
  PomodoroState,
  Provider,
  RecallResult,
  RecommendationDetail,
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

  /** v0.4.4.x followup §1.1 — 진행 중 chat 스트리밍 취소.
   *  backend가 spawn한 claude/gemini/codex CLI subprocess를 SIGKILL + chat:error emit. */
  cancelChatStream: (handle: string) =>
    invoke<void>("cancel_chat_stream", { handle }),

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

  // v0.4.2 PR 3 — T2 BGE-M3 백그라운드 빌드 시작.
  // T1 ready 검증 후 BGE-M3 (~2GB) 다운로드·로드 + chunks 임베딩.
  startT2Build: (bookId: string) =>
    invoke<{ job_id: number; book_id: string; total_chunks: number }>(
      "start_t2_build",
      { bookId },
    ),

  // v0.4.2 PR 3 — 사용자 명시 일시정지/재개/취소.
  // pause는 D-081 가장 강한 사유라 어떤 자동 트리거도 덮어쓰지 못함.
  pauseIndexingJob: (jobId: number) =>
    invoke<void>("pause_indexing_job", { jobId }),

  resumeIndexingJob: (jobId: number) =>
    invoke<void>("resume_indexing_job", { jobId }),

  cancelIndexingJob: (jobId: number) =>
    invoke<void>("cancel_indexing_job", { jobId }),

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

  // v0.4.2 PR 4 — embedding/response cache 통계 (D-084 dev panel).
  devCacheStats: () => invoke<CacheStatsPayload>("dev_cache_stats"),

  // v0.4.2 PR 5 — acceptance 측정 dev 명령 (D-083 + handoff §3 4 gate).
  /** gate 1 측정 — pending_chunks_on_restart ≤ 32(BATCH_SIZE) 이면 PASS. */
  devSimulateAbnormalShutdown: (bookId: string) =>
    invoke<AbnormalShutdownSimulation>("dev_simulate_abnormal_shutdown", {
      bookId,
    }),
  /** gate 2 점검 — active_index 일관성·manifest 상태·vectors 카운트. */
  devInspectActiveIndexState: (bookId: string) =>
    invoke<ActiveIndexInspection>("dev_inspect_active_index_state", { bookId }),
  /** gate 3 측정 — 같은 study 내 user→assistant 평균 응답 시간. */
  devMeasureChatResponseMs: (studySlug: string, lastN: number) =>
    invoke<ChatResponseTiming>("dev_measure_chat_response_ms", {
      studySlug,
      lastN,
    }),
  /** gate 4 측정 — response_cache 누적 hit/miss. */
  devResponseCacheHitRatio: () =>
    invoke<ResponseCacheHitRatio>("dev_response_cache_hit_ratio"),

  // v0.4.3 PR 5 — acceptance 측정 (handoff §1.3 — 4 gate).
  /** v0.4.3 gate 1 — 최근 N건 chat citation_scores 통계 (pass 비율 ≥ 85% 면 PASS). */
  devMeasureCitationAccuracy: (studySlug: string, lastN: number) =>
    invoke<CitationAccuracy>("dev_measure_citation_accuracy", {
      studySlug,
      lastN,
    }),
  /** v0.4.3 gate 2 — follow-up 효율 (재사용 가능 비율 ≥ 60% 면 PASS). */
  devMeasureFollowupSkipRate: (studySlug: string, lastN: number) =>
    invoke<FollowupSkipRate>("dev_measure_followup_skip_rate", {
      studySlug,
      lastN,
    }),
  /** v0.4.3 gate 3 — prompt prefix cache hit ratio (≥ 70% 면 PASS). */
  devMeasurePrefixCacheRatio: (studySlug: string, lastN: number) =>
    invoke<PrefixCacheRatio>("dev_measure_prefix_cache_ratio", {
      studySlug,
      lastN,
    }),

  // v0.4.4 PR 4 (D-094) — 하드웨어 자동 감지 + 모델 티어링 추천.
  /** 사용자 머신 사양 1회 측정 (CPU·RAM·OS·arch). */
  devProbeHardware: () => invoke<HardwareInfo>("dev_probe_hardware"),
  /** 추천 등급 + 이유 + 모델 사이즈 합계. */
  devGetModelRecommendation: () =>
    invoke<RecommendationDetail>("dev_get_model_recommendation"),

  // v0.4.4 PR 5 (D-095) — BYOK 클라우드 임베딩.
  /** 형식 검증 → keyring 저장. provider별 prefix·최소 길이 검증. */
  byokKeySet: (provider: ByokProvider, key: string) =>
    invoke<void>("byok_key_set", { provider, key }),
  /** 키 존재 여부만 반환 (값 자체는 절대 노출 X). */
  byokKeyPresent: (provider: ByokProvider) =>
    invoke<boolean>("byok_key_present", { provider }),
  /** keyring에서 키 삭제. */
  byokKeyDelete: (provider: ByokProvider) =>
    invoke<void>("byok_key_delete", { provider }),
  /** chunks·avg_tokens 기반 비용 추정. UI 카드에 표시. */
  byokEstimateCost: (
    provider: ByokProvider,
    model: string,
    chunks: number,
    avgTokensPerChunk: number,
  ) =>
    invoke<ByokCostEstimate>("byok_estimate_cost", {
      provider,
      model,
      chunks,
      avgTokensPerChunk,
    }),
  /** gate 5 측정 — settings + keyring 상태 한 묶음 (어댑터 라우팅 stub 검증). */
  devByokRoutingCheck: () =>
    invoke<ByokRoutingResult>("dev_byok_routing_check"),
};
