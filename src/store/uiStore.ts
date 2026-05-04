// UI 라우팅·테마 상태. design/architecture/state-stores.md 의 uiStore 슬라이스 (v0.1 부분).

import { create } from "zustand";

export type Page = "welcome" | "workspace" | "library";

export type Density = "compact" | "normal" | "comfortable";

const DENSITY_KEY = "airis.density";
const ACCENT_HUE_KEY = "airis.accentHue";

function readDensity(): Density {
  if (typeof window === "undefined") return "normal";
  const v = window.localStorage.getItem(DENSITY_KEY);
  return v === "compact" || v === "comfortable" ? v : "normal";
}

function readAccentHue(): number {
  if (typeof window === "undefined") return 25;
  const v = parseInt(window.localStorage.getItem(ACCENT_HUE_KEY) ?? "", 10);
  return Number.isFinite(v) && v >= 0 && v <= 360 ? v : 25;
}

/** dockview 패널 ID. TopBar 토글 → Workspace effect가 처리. */
export type DockPanelId =
  | "quiz"
  | "notes"
  | "srs"
  | "progress"
  | "memory"
  | "pomodoro";

interface UiStore {
  page: Page;
  setPage: (page: Page) => void;
  /** 다크/라이트 effective 결과 (system이면 OS 따라 결정된 값). */
  effectiveTheme: "light" | "dark";
  setEffectiveTheme: (t: "light" | "dark") => void;
  /** UI 밀도 — `data-density` 속성으로 spacing 토큰 변동. localStorage에 persist. */
  density: Density;
  setDensity: (d: Density) => void;
  /** TopBar 토글 → Workspace effect가 처리(dockview 패널 토글). 처리 후 null로 reset. */
  pendingPanelToggle: { id: DockPanelId; nonce: number } | null;
  requestPanelToggle: (id: DockPanelId) => void;
  clearPendingPanelToggle: () => void;
  /** Brand accent hue — `<html style="--accent-h: ...">` attribute로 토큰 변동. localStorage에 persist. */
  accentHue: number;
  setAccentHue: (v: number) => void;
  /** SRS 카드 풀이 모달 — slideup의 "복습 시작" 버튼이 토글. */
  srsOpen: boolean;
  setSrsOpen: (open: boolean) => void;
  /** 회상 챌린지 모달 — slideup의 "챌린지 시작" 버튼이 토글. */
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
  /** 새 스터디 마법사 모달 열림 — Library의 "새 스터디" 버튼이 토글. */
  newStudyOpen: boolean;
  setNewStudyOpen: (open: boolean) => void;
  /** Settings 모달 — TopBar 설정 아이콘 또는 `Mod+,`로 토글. */
  settingsOpen: boolean;
  setSettingsOpen: (open: boolean) => void;
  /** 라이브러리 인스펙터 — 카드 클릭 시 띄움. null이면 닫힘. */
  inspectorSlug: string | null;
  setInspectorSlug: (slug: string | null) => void;
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
  pendingPanelToggle: null,
  requestPanelToggle: (id) =>
    set({ pendingPanelToggle: { id, nonce: Date.now() } }),
  clearPendingPanelToggle: () => set({ pendingPanelToggle: null }),
  accentHue: readAccentHue(),
  setAccentHue: (accentHue) => {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(ACCENT_HUE_KEY, String(accentHue));
    }
    set({ accentHue });
  },
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
  newStudyOpen: false,
  setNewStudyOpen: (newStudyOpen) => set({ newStudyOpen }),
  settingsOpen: false,
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  inspectorSlug: null,
  setInspectorSlug: (inspectorSlug) => set({ inspectorSlug }),
}));
