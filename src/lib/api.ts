// Tauri invoke 래퍼. Rust 백엔드 commands와 1:1 매칭.
// 에러는 { kind, ... } AppError 형태로 그대로 던진다 — 호출자가 isAppError로 분기.

import { invoke } from "@tauri-apps/api/core";
import type { Provider, Settings } from "@/lib/types";

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
};
