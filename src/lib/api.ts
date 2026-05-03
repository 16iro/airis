// Tauri invoke 래퍼. Rust 백엔드 commands와 1:1 매칭.
// 에러는 { kind, ... } AppError 형태로 그대로 던진다 — 호출자가 isAppError로 분기.

import { invoke } from "@tauri-apps/api/core";

import type {
  ChatHistoryMessage,
  ChatJobHandle,
  FailedJob,
  FileMeta,
  Provider,
  Settings,
  StudyMeta,
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
};
