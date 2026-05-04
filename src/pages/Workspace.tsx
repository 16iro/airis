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
} from "dockview-react";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

import { BookViewer } from "@/components/BookViewer";
import { ChatPanel } from "@/components/ChatPanel";
import { FileViewer } from "@/components/FileViewer";
import { MemoryPanelContent } from "@/components/MemoryPanelContent";
import { QuizContent } from "@/components/slideup/QuizContent";
import { SrsDeckContent } from "@/components/slideup/SrsDeckContent";
import { StudySidebar } from "@/components/StudySidebar";
import { TopBar } from "@/components/TopBar";
import { useActiveBookStore } from "@/store/activeBookStore";
import { useStudyStore } from "@/store/studyStore";

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
  | "memory";

const LAYOUT_KEY_PREFIX = "airis.layout.";

function layoutKey(slug: string | null): string {
  return `${LAYOUT_KEY_PREFIX}${slug ?? "default"}`;
}

export function Workspace({ registerChatHandle }: Props) {
  const { t } = useTranslation();
  const apiRef = useRef<DockviewApi | null>(null);
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
    }),
    [t],
  );

  // 활성 스터디 slug 변경 시 layout 새로 로드.
  useEffect(() => {
    const api = apiRef.current;
    if (!api) return;
    rebuildLayout(api, activeSlug, t);
  }, [activeSlug, t]);

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
        togglePanel(api, "toc", t);
      } else if (k === "j") {
        e.preventDefault();
        togglePanel(api, "chat", t);
      } else if (["1", "2", "3", "4", "5"].includes(e.key)) {
        e.preventDefault();
        const map: Record<string, PanelId> = {
          "1": "quiz",
          "2": "notes",
          "3": "srs",
          "4": "progress",
          "5": "memory",
        };
        focusOrAddPanel(api, map[e.key], t);
      } else if (k === "l") {
        // 챗 입력 포커스 — dockview 안에선 ref 직접 접근이 어려워 CustomEvent로 위임.
        e.preventDefault();
        focusOrAddPanel(api, "chat", t);
        window.dispatchEvent(new CustomEvent("airis:focus-chat-input"));
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [t]);

  function onReady(event: DockviewReadyEvent) {
    apiRef.current = event.api;
    rebuildLayout(event.api, activeSlug, t);

    // 레이아웃 변경 → localStorage save (debounce는 v0.4 이후, 이벤트 빈도 높지 않음).
    const disposable = event.api.onDidLayoutChange(() => {
      try {
        const json = event.api.toJSON();
        window.localStorage.setItem(layoutKey(activeSlug), JSON.stringify(json));
      } catch (e) {
        console.warn("layout save failed:", e);
      }
    });
    return () => disposable.dispose();
  }

  return (
    <div className="flex h-full flex-col bg-background text-foreground">
      <TopBar />
      <div className="min-h-0 flex-1">
        <DockviewReact
          className="dockview-theme-airis h-full w-full"
          components={components as Record<string, React.FC<IDockviewPanelProps>>}
          disableFloatingGroups
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
      return;
    } catch (e) {
      console.warn("layout restore failed, falling back to default:", e);
    }
  }
  buildDefaultLayout(api, t);
}

/**
 * 패널 닫기/열기 토글. 닫혀 있으면 기본 위치에 다시 추가.
 */
function togglePanel(api: DockviewApi, id: PanelId, t: (key: string) => string) {
  const existing = api.getPanel(id);
  if (existing) {
    existing.api.close();
  } else {
    api.addPanel({
      id,
      component: id,
      title: t(`workspace.panel_${id}`),
      position: DEFAULT_POSITIONS[id]?.(api) ?? undefined,
      ...(DEFAULT_SIZES[id] ?? {}),
    });
  }
}

/**
 * 패널이 없으면 추가 + 활성. 있으면 활성만.
 */
function focusOrAddPanel(
  api: DockviewApi,
  id: PanelId,
  t: (key: string) => string,
) {
  const existing = api.getPanel(id);
  if (existing) {
    existing.api.setActive();
    return;
  }
  api.addPanel({
    id,
    component: id,
    title: t(`workspace.panel_${id}`),
    position: DEFAULT_POSITIONS[id]?.(api) ?? undefined,
    ...(DEFAULT_SIZES[id] ?? {}),
  });
  api.getPanel(id)?.api.setActive();
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
    title: t("slideup.quiz"),
    initialHeight: 280,
    position: { referencePanel: "viewer", direction: "below" },
  });
  api.addPanel({
    id: "notes",
    component: "notes",
    title: t("slideup.notes"),
    position: { referencePanel: "quiz", direction: "within" },
  });
  api.addPanel({
    id: "srs",
    component: "srs",
    title: t("slideup.srs"),
    position: { referencePanel: "quiz", direction: "within" },
  });
  api.addPanel({
    id: "progress",
    component: "progress",
    title: t("slideup.progress"),
    position: { referencePanel: "quiz", direction: "within" },
  });
  api.addPanel({
    id: "memory",
    component: "memory",
    title: t("slideup.memory"),
    position: { referencePanel: "quiz", direction: "within" },
  });

  // 시작 시 viewer가 활성. quiz 탭은 첫 탭이라 활성 (사용자가 다른 탭 클릭하면 그쪽으로).
  api.getPanel("viewer")?.focus();
}
