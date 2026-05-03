// Settings 페이지 — Tabs 3 섹션 (프로바이더 / 모델 / 언어).
//
// PR 13 v0.2b — 다중 LLM 프로바이더(Anthropic·OpenAI·Gemini) 지원:
//   * "프로바이더" 탭: 활성 라디오 + 3개 카드 (각 키 입력)
//   * "모델" 탭: 활성 프로바이더 기준 모델 셀렉터
//   * "언어" 탭: 그대로 (한국어만, 영어는 v1)

import { ArrowLeft } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { ApiKeyInput } from "@/components/ApiKeyInput";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  type InterventionLevel,
  PROVIDER_MODELS,
  PROVIDERS,
  type Provider,
} from "@/lib/types";
import { useSettingsStore } from "@/store/settingsStore";
import { useUiStore } from "@/store/uiStore";

export function Settings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const loaded = useSettingsStore((s) => s.loaded);
  const load = useSettingsStore((s) => s.load);
  const update = useSettingsStore((s) => s.update);
  const setPage = useUiStore((s) => s.setPage);

  useEffect(() => {
    if (!loaded) {
      load();
    }
  }, [loaded, load]);

  function handleClose() {
    setPage(settings.welcome_seen ? "workspace" : "welcome");
  }

  function handleProviderChange(provider: Provider) {
    update({ active_provider: provider });
  }

  function handleModelChange(modelId: string) {
    const next = { ...settings.models, [settings.active_provider]: modelId };
    update({ models: next, model: modelId });
  }

  function handleInterventionChange(level: InterventionLevel) {
    update({ intervention_level: level });
  }

  const activeModels = PROVIDER_MODELS[settings.active_provider];

  return (
    <div className="flex min-h-full flex-col bg-background text-foreground">
      <header className="flex h-12 items-center gap-2 border-b border-border px-4">
        <Button
          variant="ghost"
          size="sm"
          onClick={handleClose}
          aria-label={t("common.back")}
        >
          <ArrowLeft size={18} />
        </Button>
        <h1 className="font-semibold">{t("settings.title")}</h1>
      </header>

      <main className="mx-auto w-full max-w-3xl flex-1 px-6 py-8">
        <Tabs defaultValue="provider" className="w-full">
          <TabsList>
            <TabsTrigger value="provider">
              {t("settings.provider.tab_label")}
            </TabsTrigger>
            <TabsTrigger value="model">{t("settings.tabs.model")}</TabsTrigger>
            <TabsTrigger value="intervention">
              {t("intervention.tab_label")}
            </TabsTrigger>
            <TabsTrigger value="language">
              {t("settings.tabs.language")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="provider" className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle>{t("settings.provider.card_title")}</CardTitle>
                <CardDescription>
                  {t("settings.provider.card_desc")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-2">
                {PROVIDERS.map((p) => (
                  <Label
                    key={p}
                    className="flex cursor-pointer items-center gap-3 rounded-md border border-border p-3 hover:bg-accent"
                  >
                    <input
                      type="radio"
                      name="active_provider"
                      value={p}
                      checked={settings.active_provider === p}
                      onChange={() => handleProviderChange(p)}
                    />
                    <span className="flex-1 font-medium">
                      {t(`settings.provider.${p}`)}
                    </span>
                  </Label>
                ))}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>{t("settings.api_key.card_title")}</CardTitle>
                <CardDescription>
                  {t("settings.provider.key_card_desc")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-6">
                {PROVIDERS.map((p) => (
                  <div key={p} className="space-y-2">
                    <h3 className="text-sm font-medium">
                      {t(`settings.provider.${p}`)}
                    </h3>
                    <ApiKeyInput provider={p} />
                  </div>
                ))}
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="model">
            <Card>
              <CardHeader>
                <CardTitle>{t("settings.model.card_title")}</CardTitle>
                <CardDescription>
                  {t(`settings.provider.${settings.active_provider}`)} ·{" "}
                  {t("settings.model.card_desc")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {activeModels.map((m) => {
                  const currentModel =
                    settings.models[settings.active_provider] ?? settings.model;
                  return (
                    <Label
                      key={m.id}
                      className="flex cursor-pointer items-start gap-3 rounded-md border border-border p-3 hover:bg-accent"
                    >
                      <input
                        type="radio"
                        name="model"
                        value={m.id}
                        checked={currentModel === m.id}
                        onChange={() => handleModelChange(m.id)}
                        className="mt-1"
                      />
                      <span className="flex-1">
                        <span className="block font-medium">{m.id}</span>
                        <span className="block text-sm text-muted-foreground">
                          {t(m.labelKey)}
                        </span>
                      </span>
                    </Label>
                  );
                })}
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="intervention">
            <Card>
              <CardHeader>
                <CardTitle>{t("intervention.card_title")}</CardTitle>
                <CardDescription>
                  {t("intervention.card_desc")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {(["confirm", "auto", "off"] as InterventionLevel[]).map(
                  (level) => (
                    <Label
                      key={level}
                      className="flex cursor-pointer items-start gap-3 rounded-md border border-border p-3 hover:bg-accent"
                    >
                      <input
                        type="radio"
                        name="intervention_level"
                        value={level}
                        checked={settings.intervention_level === level}
                        onChange={() => handleInterventionChange(level)}
                        className="mt-1"
                      />
                      <span className="flex-1">
                        <span className="block font-medium">
                          {t(`intervention.${level}`)}
                        </span>
                        <span className="block text-sm text-muted-foreground">
                          {t(`intervention.${level}_desc`)}
                        </span>
                      </span>
                    </Label>
                  ),
                )}
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="language">
            <Card>
              <CardHeader>
                <CardTitle>{t("settings.language.card_title")}</CardTitle>
                <CardDescription>
                  {t("settings.language.card_desc")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {[
                  { id: "ko", labelKey: "settings.language.ko" },
                  {
                    id: "en",
                    labelKey: "settings.language.en_pending",
                    disabled: true,
                  },
                ].map((opt) => (
                  <Label
                    key={opt.id}
                    className={
                      "flex cursor-pointer items-center gap-3 rounded-md border border-border p-3 hover:bg-accent" +
                      (opt.disabled ? " cursor-not-allowed opacity-50" : "")
                    }
                  >
                    <input
                      type="radio"
                      name="language"
                      value={opt.id}
                      checked={settings.language === opt.id}
                      disabled={opt.disabled}
                      onChange={() => update({ language: opt.id })}
                    />
                    <span>{t(opt.labelKey)}</span>
                  </Label>
                ))}
              </CardContent>
            </Card>
          </TabsContent>
        </Tabs>
      </main>
    </div>
  );
}
