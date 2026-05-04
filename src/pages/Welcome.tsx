// PR 27 (D-066) — 첫 실행 환영 화면 재작성.
//
// v0.2 흐름: API 키 직접 입력 안내 → Settings 이동.
// v0.2.1 흐름: 사용자 *구독*(Claude Pro / Gemini AI Pro / ChatGPT Plus)을 그대로 활용하도록
// 공식 CLI 연결을 1순위로 노출. API 키 직접 입력은 Advanced 링크로 강등.
//
// 각 프로바이더 카드 클릭 → auth_mode=cli + active_provider 저장 + CliSetupDialog 띄움.
// 다이얼로그 onComplete 시 welcome_seen=true + 워크스페이스 이동.

import { ArrowRight, Key, Lock } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { CliSetupDialog } from "@/components/CliSetupDialog";
import type { Provider } from "@/lib/types";
import { useSettingsStore } from "@/store/settingsStore";
import { useUiStore } from "@/store/uiStore";

const PROVIDER_CARDS: Array<{
  id: Provider;
  titleKey: string;
  subKey: string;
  badgeKey: string;
}> = [
  {
    id: "anthropic",
    titleKey: "welcome.cli.anthropic_title",
    subKey: "welcome.cli.anthropic_sub",
    badgeKey: "welcome.cli.anthropic_badge",
  },
  {
    id: "gemini",
    titleKey: "welcome.cli.gemini_title",
    subKey: "welcome.cli.gemini_sub",
    badgeKey: "welcome.cli.gemini_badge",
  },
  {
    id: "openai",
    titleKey: "welcome.cli.openai_title",
    subKey: "welcome.cli.openai_sub",
    badgeKey: "welcome.cli.openai_badge",
  },
];

export function Welcome() {
  const { t } = useTranslation();
  const update = useSettingsStore((s) => s.update);
  const setPage = useUiStore((s) => s.setPage);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const [setupProvider, setSetupProvider] = useState<Provider | null>(null);

  async function pickCli(provider: Provider) {
    await update({ active_provider: provider, auth_mode: "cli" });
    setSetupProvider(provider);
  }

  async function onCliComplete() {
    await update({ welcome_seen: true });
    setSetupProvider(null);
    setPage("workspace");
  }

  async function goAdvanced() {
    await update({ welcome_seen: true, auth_mode: "api_key" });
    setPage("workspace");
    setSettingsOpen(true);
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
          <h2 className="text-sm font-semibold">
            {t("welcome.cli.section_title")}
          </h2>
          <p className="text-xs text-muted-foreground">
            {t("welcome.cli.section_desc")}
          </p>
          <ul className="space-y-2">
            {PROVIDER_CARDS.map((card) => (
              <li key={card.id}>
                <button
                  type="button"
                  onClick={() => void pickCli(card.id)}
                  className="flex w-full items-center gap-3 rounded-md border border-border p-3 text-left transition-colors hover:bg-accent"
                >
                  <div className="flex-1">
                    <p className="flex items-center gap-2 text-sm font-medium">
                      {t(card.titleKey)}
                      <span className="rounded-sm bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {t(card.badgeKey)}
                      </span>
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {t(card.subKey)}
                    </p>
                  </div>
                  <ArrowRight size={16} className="text-muted-foreground" />
                </button>
              </li>
            ))}
          </ul>
        </section>

        <div className="border-t border-border pt-4">
          <button
            type="button"
            onClick={() => void goAdvanced()}
            className="flex w-full items-center gap-2 rounded-md p-2 text-left text-xs text-muted-foreground hover:bg-accent"
          >
            <Key size={12} />
            <span className="flex-1">{t("welcome.advanced_link")}</span>
            <ArrowRight size={12} />
          </button>
        </div>

        <p className="flex items-center justify-center gap-1.5 text-xs text-muted-foreground">
          <Lock size={12} />
          {t("welcome.privacy_note")}
        </p>
      </div>

      {setupProvider ? (
        <CliSetupDialog
          provider={setupProvider}
          onClose={() => setSetupProvider(null)}
          onComplete={() => void onCliComplete()}
        />
      ) : null}
    </div>
  );
}
