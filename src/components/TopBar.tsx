// 상단 바 — TopBar 우측 컨트롤은 워크스페이스 패널 토글 6개 + Settings (PR 43, D-070+).
//
// 좌: 브랜드 + Library/Workspace 라우트 칩
// 우: [Quiz] [Notes] [SRS] [Progress] [Memory] [Pomodoro] | [Settings]
//
// 라이트/다크/언어/단축키/오프라인은 모두 Settings 모달로 흡수.
// 토글 클릭 = uiStore.requestPanelToggle → Workspace effect가 dockview API 호출.
// PR 66: 토글은 page === "workspace"일 때만 표시. 라이브러리에서 워크스페이스 패널 조작은 의미 없음.

import {
  BookOpenText,
  ChevronRight,
  Layers,
  RotateCcw,
  Settings as SettingsIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { AirisLogo } from "@/components/AirisLogo";
import { Button } from "@/components/ui/button";
import { PANEL_ICONS } from "@/lib/panelIcons";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore, type DockPanelId } from "@/store/uiStore";

interface PanelToggleDef {
  id: DockPanelId;
  labelKey: string;
}

const CORE_PANEL_TOGGLES: PanelToggleDef[] = [
  { id: "toc", labelKey: "topbar.toggle_toc" },
  { id: "viewer", labelKey: "topbar.toggle_viewer" },
  { id: "chat", labelKey: "topbar.toggle_chat" },
];

const SLIDEUP_PANEL_TOGGLES: PanelToggleDef[] = [
  { id: "quiz", labelKey: "topbar.toggle_quiz" },
  { id: "notes", labelKey: "topbar.toggle_notes" },
  { id: "srs", labelKey: "topbar.toggle_srs" },
  { id: "progress", labelKey: "topbar.toggle_progress" },
  { id: "memory", labelKey: "topbar.toggle_memory" },
  { id: "pomodoro", labelKey: "topbar.toggle_pomodoro" },
];

export function TopBar() {
  const { t } = useTranslation();
  const page = useUiStore((s) => s.page);
  const setPage = useUiStore((s) => s.setPage);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const requestPanelToggle = useUiStore((s) => s.requestPanelToggle);
  const requestLayoutReset = useUiStore((s) => s.requestLayoutReset);
  const activeStudy = useStudyStore((s) => s.active);

  function handlePanelToggle(id: DockPanelId) {
    requestPanelToggle(id);
  }

  function handleLayoutReset() {
    if (!window.confirm(t("topbar.layout_reset_confirm"))) return;
    requestLayoutReset();
  }

  const showPanelToggles = page === "workspace" && !!activeStudy;

  return (
    <header className="flex h-12 items-center gap-2 border-b border-border bg-card px-3">
      <div className="flex items-center gap-2 font-semibold">
        <span className="inline-flex h-8 w-8 items-center justify-center rounded-md bg-primary text-primary-foreground">
          <AirisLogo className="h-6 w-6" />
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

      {showPanelToggles
        ? CORE_PANEL_TOGGLES.map((tab) => {
            const Icon = PANEL_ICONS[tab.id];
            return (
              <Button
                key={tab.id}
                variant="ghost"
                size="sm"
                onClick={() => handlePanelToggle(tab.id)}
                aria-label={t(tab.labelKey)}
                title={t(tab.labelKey)}
                className="h-8 w-8 p-0"
              >
                <Icon size={13} />
              </Button>
            );
          })
        : null}

      {showPanelToggles ? (
        <span className="mx-1 h-5 w-px bg-border" aria-hidden />
      ) : null}

      {showPanelToggles
        ? SLIDEUP_PANEL_TOGGLES.map((tab) => {
            const Icon = PANEL_ICONS[tab.id];
            return (
              <Button
                key={tab.id}
                variant="ghost"
                size="sm"
                onClick={() => handlePanelToggle(tab.id)}
                aria-label={t(tab.labelKey)}
                title={t(tab.labelKey)}
                className="h-8 w-8 p-0"
              >
                <Icon size={13} />
              </Button>
            );
          })
        : null}

      {showPanelToggles ? (
        <span className="mx-1 h-5 w-px bg-border" aria-hidden />
      ) : null}

      {showPanelToggles ? (
        <Button
          variant="ghost"
          size="sm"
          onClick={handleLayoutReset}
          aria-label={t("topbar.layout_reset")}
          title={t("topbar.layout_reset_tooltip")}
          className="h-8 w-8 p-0"
        >
          <RotateCcw size={13} />
        </Button>
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
