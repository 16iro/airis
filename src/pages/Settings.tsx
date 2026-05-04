// Settings 모달 — Tabs 5 섹션 (프로바이더 / 모델 / 메타인지 / 언어 / Advanced).
//
// PR 36 (D-070): 페이지 → 모달로 변환. backdrop 클릭 / X 버튼 / Esc로 닫기.
// PR 13 v0.2b — 다중 LLM 프로바이더(Anthropic·OpenAI·Gemini) 지원.

import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { ApiKeyInput } from "@/components/ApiKeyInput";
import { CliSetupDialog } from "@/components/CliSetupDialog";
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
  type AuthMode,
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
  const setOpen = useUiStore((s) => s.setSettingsOpen);
  const [cliSetupOpen, setCliSetupOpen] = useState(false);

  useEffect(() => {
    if (!loaded) {
      load();
    }
  }, [loaded, load]);

  // ESC로 닫기.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  function handleAuthModeChange(mode: AuthMode) {
    void update({ auth_mode: mode });
    if (mode === "cli") setCliSetupOpen(true);
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
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="settings-title"
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/50 p-4 sm:items-center"
      onClick={() => setOpen(false)}
    >
      <Card
        className="w-full max-w-3xl"
        onClick={(e) => e.stopPropagation()}
      >
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle id="settings-title">{t("settings.title")}</CardTitle>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setOpen(false)}
              aria-label={t("common.close")}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
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
            <TabsTrigger value="advanced">
              {t("settings.tabs.advanced")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="provider" className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle>{t("auth.mode_card_title")}</CardTitle>
                <CardDescription>{t("auth.mode_card_desc")}</CardDescription>
              </CardHeader>
              <CardContent className="space-y-2">
                {(["cli", "api_key"] as AuthMode[]).map((mode) => (
                  <Label
                    key={mode}
                    className="flex cursor-pointer items-start gap-3 rounded-md border border-border p-3 hover:bg-accent"
                  >
                    <input
                      type="radio"
                      name="auth_mode"
                      value={mode}
                      checked={settings.auth_mode === mode}
                      onChange={() => handleAuthModeChange(mode)}
                      className="mt-1"
                    />
                    <span className="flex-1">
                      <span className="block font-medium">
                        {t(`auth.mode_${mode}`)}
                      </span>
                      <span className="block text-sm text-muted-foreground">
                        {t(`auth.mode_${mode}_desc`)}
                      </span>
                    </span>
                  </Label>
                ))}
                {settings.auth_mode === "cli" ? (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setCliSetupOpen(true)}
                  >
                    {t("cli_setup.dialog_title")}
                  </Button>
                ) : null}
              </CardContent>
            </Card>

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

          <TabsContent value="advanced" className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle>{t("settings.api_key.card_title")}</CardTitle>
                <CardDescription>
                  {t("settings.advanced.api_key_desc")}
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
        </Tabs>
        </CardContent>
      </Card>

      {cliSetupOpen ? (
        <CliSetupDialog
          provider={settings.active_provider}
          onClose={() => setCliSetupOpen(false)}
        />
      ) : null}
    </div>
  );
}
