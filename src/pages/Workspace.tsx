// 워크스페이스 — dockview 도커블 셸 (PR 42, v0.3.1, D-070+).
//
// 패널 5종(toc / viewer / chat + slideup의 quiz/notes/srs/progress/memory)을 dockview로 관리.
// 사용자가 드래그해 zone 재배치 + 리사이즈 + 같은 zone에 묶이면 탭화.
// 분리(popout window)는 disableFloatingGroups로 차단.
//
// 각 스터디별 레이아웃 = `airis.layout.<study_slug>` localStorage에 persist.
// 활성 스터디 변경 시 layout reload.

import {
  DockviewReact,
  type AddPanelOptions,
  type DockviewApi,
  type DockviewReadyEvent,
  type IDockviewPanelProps,
  type SerializedDockview,
} from "dockview-react";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

import { BookViewer } from "@/components/BookViewer";
import { ChatPanel } from "@/components/ChatPanel";
import { FileViewer } from "@/components/FileViewer";
import { MemoryPanelContent } from "@/components/MemoryPanelContent";
import { PomodoroPanelContent } from "@/components/PomodoroPanelContent";
import { QuizContent } from "@/components/slideup/QuizContent";
import { SrsDeckContent } from "@/components/slideup/SrsDeckContent";
import { StudySidebar } from "@/components/StudySidebar";
import { TopBar } from "@/components/TopBar";
import { useActiveBookStore } from "@/store/activeBookStore";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

interface Props {
  registerChatHandle?: Parameters<typeof ChatPanel>[0]["registerHandle"];
}

type PanelId =
  | "toc"
  | "viewer"
  | "chat"
  | "quiz"
  | "notes"
  | "srs"
  | "progress"
  | "memory"
  | "pomodoro";

/** 패널 ID → i18n 라벨 키. 저장된 layout 복원 후 title 강제 동기에 사용. */
const PANEL_TITLE_KEY: Record<PanelId, string> = {
  toc: "workspace.panel_toc",
  viewer: "workspace.panel_viewer",
  chat: "workspace.panel_chat",
  quiz: "workspace.panel_quiz",
  notes: "workspace.panel_notes",
  srs: "workspace.panel_srs",
  progress: "workspace.panel_progress",
  memory: "workspace.panel_memory",
  pomodoro: "workspace.panel_pomodoro",
};

function syncPanelTitles(api: DockviewApi, t: (key: string) => string) {
  for (const id of Object.keys(PANEL_TITLE_KEY) as PanelId[]) {
    const panel = api.getPanel(id);
    if (panel) {
      panel.api.setTitle(t(PANEL_TITLE_KEY[id]));
    }
  }
}

const LAYOUT_KEY_PREFIX = "airis.layout.";

function layoutKey(slug: string | null): string {
  return `${LAYOUT_KEY_PREFIX}${slug ?? "default"}`;
}

export function Workspace({ registerChatHandle }: Props) {
  const { t } = useTranslation();
  const apiRef = useRef<DockviewApi | null>(null);
  /** 패널 close 직전 group ID 저장. close 후 group이 살아있으면(panels.length > 1) 그 자리 복원. */
  const lastPositionRef = useRef<Map<PanelId, string>>(new Map());
  /** group이 close 시 폐기될 때(panels.length === 1) 전체 layout snapshot 저장. 다시 열 때 fromJSON. */
  const lastSnapshotRef = useRef<Map<PanelId, SerializedDockview>>(new Map());
  const activeStudy = useStudyStore((s) => s.active);
  const activeSlug = activeStudy?.slug ?? null;
  const chatRegisterRef = useRef(registerChatHandle);
  useEffect(() => {
    chatRegisterRef.current = registerChatHandle;
  }, [registerChatHandle]);

  const components = useMemo<Record<PanelId, React.FC<IDockviewPanelProps>>>(
    () => ({
      toc: () => <StudySidebar />,
      viewer: () => <ViewerPanel />,
      chat: () => <ChatPanel registerHandle={chatRegisterRef.current} />,
      quiz: () => <QuizContent />,
      notes: () => (
        <p className="p-4 text-xs text-muted-foreground">
          {t("slideup.notes_placeholder")}
        </p>
      ),
      srs: () => <SrsDeckContent />,
      progress: () => (
        <p className="p-4 text-xs text-muted-foreground">
          {t("slideup.progress_placeholder")}
        </p>
      ),
      memory: () => <MemoryPanelContent />,
      pomodoro: () => <PomodoroPanelContent />,
    }),
    [t],
  );

  // 활성 스터디 slug 변경 시 layout 새로 로드.
  useEffect(() => {
    const api = apiRef.current;
    if (!api) return;
    rebuildLayout(api, activeSlug, t);
  }, [activeSlug, t]);

  // TopBar에서 패널 토글 요청 → dockview API로 처리.
  const pendingPanelToggle = useUiStore((s) => s.pendingPanelToggle);
  const clearPendingPanelToggle = useUiStore((s) => s.clearPendingPanelToggle);
  useEffect(() => {
    if (!pendingPanelToggle) return;
    const api = apiRef.current;
    if (!api) return;
    togglePanel(
      api,
      pendingPanelToggle.id,
      t,
      lastPositionRef.current,
      lastSnapshotRef.current,
    );
    clearPendingPanelToggle();
  }, [pendingPanelToggle, clearPendingPanelToggle, t]);

  // 워크스페이스 단축키 — Mod+B/J/1~5/L (dockview API 직접 호출).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      const api = apiRef.current;
      if (!api) return;

      const k = e.key.toLowerCase();
      if (k === "b") {
        e.preventDefault();
        togglePanel(api, "toc", t, lastPositionRef.current, lastSnapshotRef.current);
      } else if (k === "j") {
        e.preventDefault();
        togglePanel(api, "chat", t, lastPositionRef.current, lastSnapshotRef.current);
      } else if (["1", "2", "3", "4", "5"].includes(e.key)) {
        e.preventDefault();
        const map: Record<string, PanelId> = {
          "1": "quiz",
          "2": "notes",
          "3": "srs",
          "4": "progress",
          "5": "memory",
        };
        focusOrAddPanel(api, map[e.key], t, lastPositionRef.current, lastSnapshotRef.current);
      } else if (k === "l") {
        // 챗 입력 포커스 — dockview 안에선 ref 직접 접근이 어려워 CustomEvent로 위임.
        e.preventDefault();
        focusOrAddPanel(api, "chat", t, lastPositionRef.current, lastSnapshotRef.current);
        window.dispatchEvent(new CustomEvent("airis:focus-chat-input"));
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [t]);

  function onReady(event: DockviewReadyEvent) {
    apiRef.current = event.api;
    rebuildLayout(event.api, activeSlug, t);

    // 레이아웃 변경 → localStorage save. fromJSON 호출 시 dockview가 layout change를
    // 5~10번 폭발적으로 발화하는 케이스가 있어 200ms debounce로 IO 누적 차단.
    let saveTimer: ReturnType<typeof setTimeout> | null = null;
    const disposable = event.api.onDidLayoutChange(() => {
      if (saveTimer) clearTimeout(saveTimer);
      saveTimer = setTimeout(() => {
        try {
          const json = event.api.toJSON();
          window.localStorage.setItem(
            layoutKey(activeSlug),
            JSON.stringify(json),
          );
        } catch (e) {
          console.warn("layout save failed:", e);
        }
      }, 200);
    });
    return () => {
      if (saveTimer) clearTimeout(saveTimer);
      disposable.dispose();
    };
  }

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <TopBar />
      <div className="dockview-shell-isolated min-h-0 flex-1">
        <DockviewReact
          className="dockview-theme-airis dv-separator-border h-full w-full"
          components={components as Record<string, React.FC<IDockviewPanelProps>>}
          disableFloatingGroups
          // PDF canvas/pdfjs 콘텐츠가 detach 시 손실되는 버그 회피.
          // 'always': 패널을 DOM에서 detach하지 않고 absolute positioning으로 항상 유지.
          defaultRenderer="always"
          onReady={onReady}
        />
      </div>
    </div>
  );
}

/**
 * BookViewer 또는 FileViewer를 활성 책 여부에 따라 표시.
 * dockview 패널 안에 들어가는 wrapper.
 */
function ViewerPanel() {
  const activeBookId = useActiveBookStore((s) => s.bookId);
  return (
    <div className="h-full w-full overflow-hidden">
      {activeBookId ? <BookViewer /> : <FileViewer />}
    </div>
  );
}

/**
 * 스터디 layout 재구성. localStorage에 saved layout 있으면 fromJSON, 없으면 default 3-pane + bottom slideup.
 */
function rebuildLayout(
  api: DockviewApi,
  slug: string | null,
  t: (key: string) => string,
) {
  api.clear();
  const saved = window.localStorage.getItem(layoutKey(slug));
  if (saved) {
    try {
      api.fromJSON(JSON.parse(saved));
      // 저장된 layout의 title이 *과거 i18n 키 자체*로 박힌 경우가 있어 매번 강제 동기.
      // ko.json 키 변경 후에도 사용자가 saved layout 복원하면 일관 라벨 보장.
      syncPanelTitles(api, t);
      return;
    } catch (e) {
      console.warn("layout restore failed, falling back to default:", e);
    }
  }
  buildDefaultLayout(api, t);
  syncPanelTitles(api, t);
}

/**
 * 패널 close/add 토글.
 *
 * close 시점:
 *   - group에 panel이 2개 이상이면 → group이 close 후 살아남음. group ID 저장
 *   - group에 panel이 1개(우리 panel)면 → close 시 group 폐기. 전체 layout snapshot 저장
 *
 * add 시점 (resolveAddPosition):
 *   1. groupMemory에 저장된 group ID가 살아있으면 → 그 group에 within
 *   2. snapshotMemory에 snapshot이 있으면 → fromJSON으로 복원
 *      (※ 부작용: 다른 패널이 그 동안 옮겨졌어도 snapshot 시점으로 함께 복귀)
 *   3. 둘 다 없으면 → DEFAULT_POSITIONS fallback
 */
function togglePanel(
  api: DockviewApi,
  id: PanelId,
  t: (key: string) => string,
  groupMemory: Map<PanelId, string>,
  snapshotMemory: Map<PanelId, SerializedDockview>,
) {
  const existing = api.getPanel(id);
  if (existing) {
    const group = existing.api.group;
    if (group && group.panels.length > 1) {
      groupMemory.set(id, group.id);
      snapshotMemory.delete(id);
    } else {
      // group이 close 시 폐기됨 → 전체 layout snapshot 저장
      try {
        snapshotMemory.set(id, api.toJSON());
        groupMemory.delete(id);
      } catch (e) {
        console.warn("layout snapshot capture failed:", e);
      }
    }
    existing.api.close();
  } else {
    if (tryRestoreSnapshot(api, id, snapshotMemory)) {
      groupMemory.delete(id);
      return;
    }
    api.addPanel({
      id,
      component: id,
      title: t(`workspace.panel_${id}`),
      position: resolveAddPosition(api, id, groupMemory),
      ...(DEFAULT_SIZES[id] ?? {}),
    });
    groupMemory.delete(id);
  }
}

/**
 * 패널이 없으면 추가 + 활성. 있으면 활성만.
 */
function focusOrAddPanel(
  api: DockviewApi,
  id: PanelId,
  t: (key: string) => string,
  groupMemory: Map<PanelId, string>,
  snapshotMemory: Map<PanelId, SerializedDockview>,
) {
  const existing = api.getPanel(id);
  if (existing) {
    existing.api.setActive();
    return;
  }
  if (tryRestoreSnapshot(api, id, snapshotMemory)) {
    groupMemory.delete(id);
    api.getPanel(id)?.api.setActive();
    return;
  }
  api.addPanel({
    id,
    component: id,
    title: t(`workspace.panel_${id}`),
    position: resolveAddPosition(api, id, groupMemory),
    ...(DEFAULT_SIZES[id] ?? {}),
  });
  groupMemory.delete(id);
  api.getPanel(id)?.api.setActive();
}

/** snapshotMemory에 panel이 포함된 layout이 있으면 fromJSON으로 복원. true 반환.
 *  `reuseExistingPanels: true`로 호출해 살아있는 패널들은 unmount/remount 없이 *임시 그룹 → 재배치* 흐름.
 *  → BookViewer/ChatPanel 등의 비싼 mount effect 재실행과 IPC 폭발 방지. */
function tryRestoreSnapshot(
  api: DockviewApi,
  id: PanelId,
  snapshotMemory: Map<PanelId, SerializedDockview>,
): boolean {
  const snapshot = snapshotMemory.get(id);
  if (!snapshot) return false;
  if (!snapshot.panels || !snapshot.panels[id]) return false;
  try {
    api.fromJSON(snapshot, { reuseExistingPanels: true });
    snapshotMemory.delete(id);
    // dockview의 패널 reorganize 흐름이 BookViewer canvas를 일시 detach해 PDF 콘텐츠가
    // 빈 캔버스로 보이는 경우가 있어 강제 재렌더 신호. layout 안정화 후 한 frame 뒤 dispatch.
    requestAnimationFrame(() => {
      window.dispatchEvent(new CustomEvent("airis:pdf-rerender"));
    });
    return true;
  } catch (e) {
    console.warn("layout snapshot restore failed:", e);
    snapshotMemory.delete(id);
    return false;
  }
}

/** groupMemory에 살아있는 group ID가 있으면 그 group 안 위치, 없으면 default. */
function resolveAddPosition(
  api: DockviewApi,
  id: PanelId,
  groupMemory: Map<PanelId, string>,
): Position | undefined {
  const savedGroupId = groupMemory.get(id);
  if (savedGroupId && api.getGroup(savedGroupId)) {
    return { referenceGroup: savedGroupId, direction: "within" };
  }
  return DEFAULT_POSITIONS[id]?.(api);
}

type Position = NonNullable<AddPanelOptions["position"]>;

const DEFAULT_POSITIONS: Partial<
  Record<PanelId, (api: DockviewApi) => Position | undefined>
> = {
  toc: (api) =>
    api.getPanel("viewer")
      ? { referencePanel: "viewer", direction: "left" }
      : undefined,
  chat: (api) =>
    api.getPanel("viewer")
      ? { referencePanel: "viewer", direction: "right" }
      : undefined,
  quiz: (api) =>
    api.getPanel("viewer")
      ? { referencePanel: "viewer", direction: "below" }
      : undefined,
  notes: (api) =>
    api.getPanel("quiz")
      ? { referencePanel: "quiz", direction: "within" }
      : api.getPanel("viewer")
        ? { referencePanel: "viewer", direction: "below" }
        : undefined,
  srs: (api) =>
    api.getPanel("quiz")
      ? { referencePanel: "quiz", direction: "within" }
      : api.getPanel("viewer")
        ? { referencePanel: "viewer", direction: "below" }
        : undefined,
  progress: (api) =>
    api.getPanel("quiz")
      ? { referencePanel: "quiz", direction: "within" }
      : api.getPanel("viewer")
        ? { referencePanel: "viewer", direction: "below" }
        : undefined,
  memory: (api) =>
    api.getPanel("quiz")
      ? { referencePanel: "quiz", direction: "within" }
      : api.getPanel("viewer")
        ? { referencePanel: "viewer", direction: "below" }
        : undefined,
  pomodoro: (api) =>
    api.getPanel("quiz")
      ? { referencePanel: "quiz", direction: "within" }
      : api.getPanel("viewer")
        ? { referencePanel: "viewer", direction: "below" }
        : undefined,
};

const DEFAULT_SIZES: Partial<Record<PanelId, { initialWidth?: number; initialHeight?: number }>> = {
  toc: { initialWidth: 260 },
  chat: { initialWidth: 380 },
  quiz: { initialHeight: 280 },
};

function buildDefaultLayout(api: DockviewApi, t: (key: string) => string) {
  // Center: viewer (메인 본문)
  api.addPanel({
    id: "viewer",
    component: "viewer",
    title: t("workspace.panel_viewer"),
  });

  // 좌측: TOC
  api.addPanel({
    id: "toc",
    component: "toc",
    title: t("workspace.panel_toc"),
    initialWidth: 260,
    position: { referencePanel: "viewer", direction: "left" },
  });

  // 우측: Chat
  api.addPanel({
    id: "chat",
    component: "chat",
    title: t("workspace.panel_chat"),
    initialWidth: 380,
    position: { referencePanel: "viewer", direction: "right" },
  });

  // 하단: slideup 5탭이 한 그룹에 탭으로 묶임
  api.addPanel({
    id: "quiz",
    component: "quiz",
    title: t("workspace.panel_quiz"),
    initialHeight: 280,
    position: { referencePanel: "viewer", direction: "below" },
  });
  api.addPanel({
    id: "notes",
    component: "notes",
    title: t("workspace.panel_notes"),
    position: { referencePanel: "quiz", direction: "within" },
  });
  api.addPanel({
    id: "srs",
    component: "srs",
    title: t("workspace.panel_srs"),
    position: { referencePanel: "quiz", direction: "within" },
  });
  api.addPanel({
    id: "progress",
    component: "progress",
    title: t("workspace.panel_progress"),
    position: { referencePanel: "quiz", direction: "within" },
  });
  api.addPanel({
    id: "memory",
    component: "memory",
    title: t("workspace.panel_memory"),
    position: { referencePanel: "quiz", direction: "within" },
  });
  api.addPanel({
    id: "pomodoro",
    component: "pomodoro",
    title: t("workspace.panel_pomodoro"),
    position: { referencePanel: "quiz", direction: "within" },
  });

  // 시작 시 viewer가 활성. quiz 탭은 첫 탭이라 활성 (사용자가 다른 탭 클릭하면 그쪽으로).
  api.getPanel("viewer")?.focus();
}
