// 상단 바 — 좌측 로고 + 활성 스터디 라벨 + 우측 컨트롤(라이브러리·Memory·테마·설정).

import {
  BookOpenText,
  Brain,
  Settings as SettingsIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { ThemeToggle } from "@/components/ThemeToggle";
import { Button } from "@/components/ui/button";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

export function TopBar() {
  const { t } = useTranslation();
  const setPage = useUiStore((s) => s.setPage);
  const setMemoryOpen = useUiStore((s) => s.setMemoryOpen);
  const activeStudy = useStudyStore((s) => s.active);

  return (
    <header className="flex h-12 items-center justify-between border-b border-border bg-background px-4">
      <div className="flex items-center gap-3">
        <span className="font-semibold tracking-tight">{t("app.name")}</span>
        {activeStudy ? (
          <span
            className="rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground"
            title={t("topbar.active_study")}
          >
            {activeStudy.name}
          </span>
        ) : null}
      </div>
      <div className="flex items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setPage("library")}
          aria-label={t("topbar.open_library")}
          title={t("topbar.open_library_tooltip")}
        >
          <BookOpenText size={18} />
        </Button>
        {activeStudy ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setMemoryOpen(true)}
            aria-label={t("memory.open_button")}
            title={t("memory.topbar_tooltip")}
          >
            <Brain size={18} />
          </Button>
        ) : null}
        <ThemeToggle />
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setPage("settings")}
          aria-label={t("topbar.open_settings")}
          title={t("topbar.open_settings_tooltip")}
        >
          <SettingsIcon size={18} />
        </Button>
      </div>
    </header>
  );
}
