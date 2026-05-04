// 상단 바 — TopBar 우측 컨트롤은 워크스페이스 패널 토글 6개 + Settings (PR 43, D-070+).
//
// 좌: 브랜드 + Library/Workspace 라우트 칩
// 우: [Quiz] [Notes] [SRS] [Progress] [Memory] [Pomodoro] | [Settings]
//
// 라이트/다크/언어/단축키/오프라인은 모두 Settings 모달로 흡수.
// 토글 클릭 = uiStore.requestPanelToggle → Workspace effect가 dockview API 호출.
// Library/Welcome 페이지에선 토글 클릭 시 워크스페이스로 자동 이동 + 패널 토글.

import {
  BookOpenText,
  Brain,
  ChartLine,
  ChevronRight,
  Layers,
  ListChecks,
  Pencil,
  Settings as SettingsIcon,
  Timer,
} from "lucide-react";
import { type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore, type DockPanelId } from "@/store/uiStore";

interface PanelToggleDef {
  id: DockPanelId;
  icon: ReactNode;
  labelKey: string;
}

const PANEL_TOGGLES: PanelToggleDef[] = [
  { id: "quiz", icon: <ListChecks size={13} />, labelKey: "topbar.toggle_quiz" },
  { id: "notes", icon: <Pencil size={13} />, labelKey: "topbar.toggle_notes" },
  { id: "srs", icon: <Layers size={13} />, labelKey: "topbar.toggle_srs" },
  { id: "progress", icon: <ChartLine size={13} />, labelKey: "topbar.toggle_progress" },
  { id: "memory", icon: <Brain size={13} />, labelKey: "topbar.toggle_memory" },
  { id: "pomodoro", icon: <Timer size={13} />, labelKey: "topbar.toggle_pomodoro" },
];

export function TopBar() {
  const { t } = useTranslation();
  const page = useUiStore((s) => s.page);
  const setPage = useUiStore((s) => s.setPage);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const requestPanelToggle = useUiStore((s) => s.requestPanelToggle);
  const activeStudy = useStudyStore((s) => s.active);

  function handlePanelToggle(id: DockPanelId) {
    if (page !== "workspace") {
      setPage("workspace");
    }
    requestPanelToggle(id);
  }

  return (
    <header className="flex h-12 items-center gap-2 border-b border-border bg-card px-3">
      <div className="flex items-center gap-2 font-semibold">
        <span className="inline-flex h-6 w-6 items-center justify-center rounded-md bg-primary text-primary-foreground">
          📚
        </span>
        <span className="text-sm">{t("app.name")}</span>
      </div>
      <span className="text-xs text-muted-foreground">·</span>
      <Button
        variant={page === "library" ? "default" : "ghost"}
        size="sm"
        onClick={() => setPage("library")}
      >
        <BookOpenText size={12} />
        {t("topbar.route_library")}
      </Button>
      {activeStudy ? (
        <>
          <ChevronRight size={12} className="text-muted-foreground" />
          <Button
            variant={page === "workspace" ? "default" : "ghost"}
            size="sm"
            onClick={() => setPage("workspace")}
            title={activeStudy.name}
          >
            <Layers size={12} />
            <span className="max-w-[160px] truncate">{activeStudy.name}</span>
          </Button>
        </>
      ) : null}
      <div className="flex-1" />

      {activeStudy
        ? PANEL_TOGGLES.map((tab) => (
            <Button
              key={tab.id}
              variant="ghost"
              size="sm"
              onClick={() => handlePanelToggle(tab.id)}
              aria-label={t(tab.labelKey)}
              title={t(tab.labelKey)}
              className="h-8 w-8 p-0"
            >
              {tab.icon}
            </Button>
          ))
        : null}

      {activeStudy ? (
        <span className="mx-1 h-5 w-px bg-border" aria-hidden />
      ) : null}

      <Button
        variant="ghost"
        size="sm"
        onClick={() => setSettingsOpen(true)}
        aria-label={t("topbar.open_settings")}
        title={t("topbar.open_settings_tooltip")}
      >
        <SettingsIcon size={14} />
      </Button>
    </header>
  );
}
