// Zustand 슬라이스 — Settings 메모리 캐시 + 백엔드 동기화.
// design/architecture/state-stores.md 의 settingsStore 슬라이스 (v0.1 부분만).

import { create } from "zustand";

import { api } from "@/lib/api";
import { DEFAULT_SETTINGS, type Settings } from "@/lib/types";

interface SettingsStore {
  settings: Settings;
  loaded: boolean;
  /** 백엔드에서 1회 로드. 앱 첫 마운트 시 호출. */
  load: () => Promise<void>;
  /** 부분 갱신 — 메모리 즉시 반영 + 디스크에 비동기 쓰기. */
  update: (patch: Partial<Settings>) => Promise<void>;
}

export const useSettingsStore = create<SettingsStore>((set, get) => ({
  settings: DEFAULT_SETTINGS,
  loaded: false,

  async load() {
    const s = await api.settingsRead();
    set({ settings: s, loaded: true });
  },

  async update(patch) {
    const next = { ...get().settings, ...patch };
    set({ settings: next });
    await api.settingsWrite(next);
  },
}));
