// Tauri invoke 래퍼. Rust 백엔드 commands와 1:1 매칭.
// 에러는 { kind, ... } AppError 형태로 그대로 던진다 — 호출자가 isAppError로 분기.

import { invoke } from "@tauri-apps/api/core";

import type {
  BookEntry,
  BookMetaInput,
  ChatHistoryMessage,
  ChatJobHandle,
  FailedJob,
  FileMeta,
  IndexJobHandle,
  Provider,
  SearchHit,
  Settings,
  StudyMeta,
  StudyOverview,
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

  createStudy: (slug: string, name: string, language: string | null = null) =>
    invoke<StudyMeta>("create_study", { slug, name, language }),

  selectStudy: (slug: string) => invoke<void>("select_study", { slug }),

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

  // F5 — 검색.
  searchSections: (studySlug: string, query: string, limit: number | null = null) =>
    invoke<SearchHit[]>("search_sections", { studySlug, query, limit }),
};
