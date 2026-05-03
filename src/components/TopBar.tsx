// 상단 바 — 좌측 로고 + 우측 컨트롤(테마·설정).

import { Settings as SettingsIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ThemeToggle } from "@/components/ThemeToggle";
import { Button } from "@/components/ui/button";
import { useUiStore } from "@/store/uiStore";

export function TopBar() {
  const { t } = useTranslation();
  const setPage = useUiStore((s) => s.setPage);

  return (
    <header className="flex h-12 items-center justify-between border-b border-border bg-background px-4">
      <span className="font-semibold tracking-tight">{t("app.name")}</span>
      <div className="flex items-center gap-1">
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
