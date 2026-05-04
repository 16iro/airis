// UI 라우팅·테마 상태. design/architecture/state-stores.md 의 uiStore 슬라이스 (v0.1 부분).

import { create } from "zustand";

export type Page = "welcome" | "workspace" | "settings" | "library";

export type Density = "compact" | "normal" | "comfortable";

export type SlideupTab = "quiz" | "notes" | "srs" | "progress" | "memory";

const DENSITY_KEY = "airis.density";
const OFFLINE_KEY = "airis.offline";

function readDensity(): Density {
  if (typeof window === "undefined") return "normal";
  const v = window.localStorage.getItem(DENSITY_KEY);
  return v === "compact" || v === "comfortable" ? v : "normal";
}

function readOffline(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem(OFFLINE_KEY) === "1";
}

interface UiStore {
  page: Page;
  setPage: (page: Page) => void;
  /** 다크/라이트 effective 결과 (system이면 OS 따라 결정된 값). */
  effectiveTheme: "light" | "dark";
  setEffectiveTheme: (t: "light" | "dark") => void;
  /** UI 밀도 — `data-density` 속성으로 spacing 토큰 변동. localStorage에 persist. */
  density: Density;
  setDensity: (d: Density) => void;
  /** 의도적 오프라인 모드 토글 — TopBar Wifi 아이콘. localStorage에 persist. */
  offline: boolean;
  setOffline: (v: boolean) => void;
  /** Memory 슬라이드업 패널 열림 여부 — 모든 페이지 위에 floating. */
  memoryOpen: boolean;
  setMemoryOpen: (open: boolean) => void;
  /** Pomodoro 미니 패널 열림 여부. */
  pomodoroOpen: boolean;
  setPomodoroOpen: (open: boolean) => void;
  /** SRS 슬라이드업 패널 열림 여부. */
  srsOpen: boolean;
  setSrsOpen: (open: boolean) => void;
  /** 회상 챌린지 슬라이드업 패널 열림 여부. */
  recallOpen: boolean;
  setRecallOpen: (open: boolean) => void;
  /** 단축키 도움말 다이얼로그 — `Mod+/`로 토글. */
  shortcutsOpen: boolean;
  setShortcutsOpen: (open: boolean) => void;
  /** 좌측 사이드바 열림 — `Mod+B`로 토글. */
  sidebarOpen: boolean;
  setSidebarOpen: (open: boolean) => void;
  /** 우측 챗 패널 열림 — `Mod+J`로 토글. */
  chatOpen: boolean;
  setChatOpen: (open: boolean) => void;
  /** 활성 슬라이드업 탭 — null이면 닫힘. `Mod+1`~`Mod+5`로 토글. */
  slideupTab: SlideupTab | null;
  setSlideupTab: (tab: SlideupTab | null) => void;
  /** 새 스터디 마법사 모달 열림 — Library의 "새 스터디" 버튼이 토글. */
  newStudyOpen: boolean;
  setNewStudyOpen: (open: boolean) => void;
}

export const useUiStore = create<UiStore>((set) => ({
  page: "welcome",
  setPage: (page) => set({ page }),
  effectiveTheme: "light",
  setEffectiveTheme: (effectiveTheme) => set({ effectiveTheme }),
  density: readDensity(),
  setDensity: (density) => {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(DENSITY_KEY, density);
    }
    set({ density });
  },
  offline: readOffline(),
  setOffline: (offline) => {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(OFFLINE_KEY, offline ? "1" : "0");
    }
    set({ offline });
  },
  memoryOpen: false,
  setMemoryOpen: (memoryOpen) => set({ memoryOpen }),
  pomodoroOpen: false,
  setPomodoroOpen: (pomodoroOpen) => set({ pomodoroOpen }),
  srsOpen: false,
  setSrsOpen: (srsOpen) => set({ srsOpen }),
  recallOpen: false,
  setRecallOpen: (recallOpen) => set({ recallOpen }),
  shortcutsOpen: false,
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  sidebarOpen: true,
  setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
  chatOpen: true,
  setChatOpen: (chatOpen) => set({ chatOpen }),
  slideupTab: null,
  setSlideupTab: (slideupTab) => set({ slideupTab }),
  newStudyOpen: false,
  setNewStudyOpen: (newStudyOpen) => set({ newStudyOpen }),
}));
