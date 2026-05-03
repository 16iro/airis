// 첫 실행 환영 화면. 사용자가 "건너뛰기" 또는 "설정으로" 누르면 welcome_seen=true 저장.

import { ArrowRight, Key, Lock, FileText } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useSettingsStore } from "@/store/settingsStore";
import { useUiStore } from "@/store/uiStore";

export function Welcome() {
  const { t } = useTranslation();
  const update = useSettingsStore((s) => s.update);
  const setPage = useUiStore((s) => s.setPage);

  async function go(target: "settings" | "workspace") {
    await update({ welcome_seen: true });
    setPage(target);
  }

  return (
    <div className="flex min-h-full flex-col items-center justify-center bg-background p-8">
      <div className="w-full max-w-xl space-y-8">
        <div className="space-y-2 text-center">
          <h1 className="text-3xl font-semibold tracking-tight">
            {t("welcome.title")}
          </h1>
          <p className="text-muted-foreground">{t("welcome.subtitle")}</p>
        </div>

        <p className="text-sm text-muted-foreground">{t("welcome.intro")}</p>

        <section className="space-y-3">
          <h2 className="text-sm font-semibold">{t("welcome.prereq_title")}</h2>
          <ul className="space-y-3">
            <li className="flex gap-3 rounded-md border border-border p-3">
              <Key size={18} className="mt-0.5 shrink-0 text-muted-foreground" />
              <div>
                <p className="text-sm font-medium">
                  {t("welcome.prereq_api_key")}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t("welcome.prereq_api_key_hint")}
                </p>
              </div>
            </li>
            <li className="flex gap-3 rounded-md border border-border p-3">
              <FileText
                size={18}
                className="mt-0.5 shrink-0 text-muted-foreground"
              />
              <div>
                <p className="text-sm font-medium">
                  {t("welcome.prereq_file")}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t("welcome.prereq_file_hint")}
                </p>
              </div>
            </li>
          </ul>
        </section>

        <div className="flex flex-col gap-2 sm:flex-row">
          <Button onClick={() => void go("settings")} className="flex-1">
            {t("welcome.open_settings_cta")}
            <ArrowRight size={16} />
          </Button>
          <Button
            variant="outline"
            onClick={() => void go("workspace")}
            className="flex-1"
          >
            {t("welcome.skip_to_workspace")}
          </Button>
        </div>

        <p className="flex items-center justify-center gap-1.5 text-xs text-muted-foreground">
          <Lock size={12} />
          {t("welcome.privacy_note")}
        </p>
      </div>
    </div>
  );
}
