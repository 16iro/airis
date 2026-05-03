// UI 라우팅·테마 상태. design/architecture/state-stores.md 의 uiStore 슬라이스 (v0.1 부분).

import { create } from "zustand";

export type Page = "welcome" | "workspace" | "settings";

interface UiStore {
  page: Page;
  setPage: (page: Page) => void;
  /** 다크/라이트 effective 결과 (system이면 OS 따라 결정된 값). */
  effectiveTheme: "light" | "dark";
  setEffectiveTheme: (t: "light" | "dark") => void;
}

export const useUiStore = create<UiStore>((set) => ({
  page: "welcome",
  setPage: (page) => set({ page }),
  effectiveTheme: "light",
  setEffectiveTheme: (effectiveTheme) => set({ effectiveTheme }),
}));
