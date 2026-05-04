// 상단 바 — prototype 100% 충실 (D-070, PR 31).
//
// 좌: 브랜드 마크 + Library/Workspace 라우트 칩 (활성 스터디 있을 때만 Workspace)
// 우: Shortcuts(Mod+/) · Pomodoro · Wifi(오프라인 토글) · KO/EN · Theme · Settings
//
// Memory/SRS/Recall 진입 버튼은 PR 33/34에서 SlideupTabs로 흡수되므로 여기서 제거.
// Pomodoro 인라인 카운터는 PR 34에서 hookup — 이번 PR엔 모달 토글 버튼만.
// Shortcuts 다이얼로그는 PR 36에서 신설 — 이번 PR엔 store 토글만.

import {
  BookOpenText,
  ChevronRight,
  Keyboard,
  Layers,
  Settings as SettingsIcon,
  Wifi,
  WifiOff,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { PomodoroInline } from "@/components/PomodoroInline";
import { ThemeToggle } from "@/components/ThemeToggle";
import { Button } from "@/components/ui/button";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

export function TopBar() {
  const { t } = useTranslation();
  const page = useUiStore((s) => s.page);
  const setPage = useUiStore((s) => s.setPage);
  const setShortcutsOpen = useUiStore((s) => s.setShortcutsOpen);
  const offline = useUiStore((s) => s.offline);
  const setOffline = useUiStore((s) => s.setOffline);
  const activeStudy = useStudyStore((s) => s.active);

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
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setShortcutsOpen(true)}
        aria-label={t("topbar.shortcuts")}
        title={t("topbar.shortcuts_tooltip")}
      >
        <Keyboard size={14} />
      </Button>
      <PomodoroInline />
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setOffline(!offline)}
        aria-label={offline ? t("topbar.offline_on") : t("topbar.offline_off")}
        title={offline ? t("topbar.offline_on") : t("topbar.offline_off")}
      >
        {offline ? (
          <WifiOff size={14} className="text-[oklch(0.7_0.18_50)]" />
        ) : (
          <Wifi size={14} className="text-[oklch(0.62_0.16_145)]" />
        )}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        disabled
        className="font-mono text-[11px]"
        title={t("topbar.lang_pending")}
      >
        KO
      </Button>
      <ThemeToggle />
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setPage("settings")}
        aria-label={t("topbar.open_settings")}
        title={t("topbar.open_settings_tooltip")}
      >
        <SettingsIcon size={14} />
      </Button>
    </header>
  );
}
