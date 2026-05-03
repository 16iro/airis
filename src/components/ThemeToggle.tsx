// 다크/라이트 토글. settings.theme = system|light|dark 순환.
// 실제 .dark 클래스 토글은 useThemeEffect 훅(App.tsx)에서.

import { Monitor, Moon, Sun } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useSettingsStore } from "@/store/settingsStore";

export function ThemeToggle() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);

  const order: Array<typeof settings.theme> = ["system", "light", "dark"];
  const next = () => {
    const idx = order.indexOf(settings.theme);
    void update({ theme: order[(idx + 1) % order.length] });
  };

  const Icon =
    settings.theme === "system"
      ? Monitor
      : settings.theme === "dark"
        ? Moon
        : Sun;

  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={next}
      aria-label={t("topbar.toggle_theme")}
      title={`${t("topbar.toggle_theme")} (${settings.theme})`}
    >
      <Icon size={18} />
    </Button>
  );
}
