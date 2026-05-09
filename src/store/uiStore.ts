// UI 라우팅·테마 상태. design/architecture/state-stores.md 의 uiStore 슬라이스 (v0.1 부분).

import { create } from "zustand";

export type Page = "welcome" | "workspace" | "library" | "reports";

export type Density = "compact" | "normal" | "comfortable";

const DENSITY_KEY = "airis.density";
const ACCENT_PRESET_KEY = "airis.accentPreset";

/** PR 70 — 브랜드 포인트 컬러 프리셋. hex → OKLCH 변환된 L/C/H 값을 사용. */
export type AccentPreset = "sky" | "orange" | "lime";

export const ACCENT_PRESETS: Record<
  AccentPreset,
  { l: number; c: number; h: number; hex: string }
> = {
  // #5BC0D9 — 기본값. 차분한 하늘색.
  sky: { l: 0.7571, c: 0.1001, h: 216.4, hex: "#5BC0D9" },
  // #E86A3C — 기존 v0.3 브랜드 오렌지에 가까운 따뜻한 톤.
  orange: { l: 0.67, c: 0.1679, h: 39.79, hex: "#E86A3C" },
  // #C8FF3D — 형광에 가까운 라임. 강조용.
  lime: { l: 0.9298, c: 0.2149, h: 124.79, hex: "#C8FF3D" },
};

const DEFAULT_ACCENT_PRESET: AccentPreset = "sky";

function readDensity(): Density {
  if (typeof window === "undefined") return "normal";
  const v = window.localStorage.getItem(DENSITY_KEY);
  return v === "compact" || v === "comfortable" ? v : "normal";
}

function readAccentPreset(): AccentPreset {
  if (typeof window === "undefined") return DEFAULT_ACCENT_PRESET;
  const v = window.localStorage.getItem(ACCENT_PRESET_KEY);
  return v === "sky" || v === "orange" || v === "lime" ? v : DEFAULT_ACCENT_PRESET;
}

/** dockview 패널 ID. TopBar 토글 → Workspace effect가 처리. */
export type DockPanelId =
  | "toc"
  | "viewer"
  | "chat"
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
  /** 레이아웃 리셋 요청 — Workspace effect가 처리(localStorage clear + default 재구성). 처리 후 null로 reset. */
  pendingLayoutReset: { nonce: number } | null;
  requestLayoutReset: () => void;
  clearPendingLayoutReset: () => void;
  /** Brand accent 컬러 프리셋 — `<html style="--accent-l/c/h">`로 토큰 변동. localStorage에 persist. PR 70 — 단일 hue slider에서 명명된 프리셋(sky/orange/lime)으로 단순화. */
  accentPreset: AccentPreset;
  setAccentPreset: (v: AccentPreset) => void;
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
  pendingLayoutReset: null,
  requestLayoutReset: () => set({ pendingLayoutReset: { nonce: Date.now() } }),
  clearPendingLayoutReset: () => set({ pendingLayoutReset: null }),
  accentPreset: readAccentPreset(),
  setAccentPreset: (accentPreset) => {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(ACCENT_PRESET_KEY, accentPreset);
    }
    set({ accentPreset });
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
