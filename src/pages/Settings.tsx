// Settings 페이지 — Tabs 3 섹션 (API 키 / 모델 / 언어).

import { useEffect } from "react";
import { ArrowLeft } from "lucide-react";
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
import { ANTHROPIC_MODELS } from "@/lib/types";
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
        <Tabs defaultValue="api-key" className="w-full">
          <TabsList>
            <TabsTrigger value="api-key">
              {t("settings.tabs.api_key")}
            </TabsTrigger>
            <TabsTrigger value="model">{t("settings.tabs.model")}</TabsTrigger>
            <TabsTrigger value="language">
              {t("settings.tabs.language")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="api-key">
            <Card>
              <CardHeader>
                <CardTitle>{t("settings.api_key.card_title")}</CardTitle>
                <CardDescription>
                  {t("settings.api_key.card_desc")}
                </CardDescription>
              </CardHeader>
              <CardContent>
                <ApiKeyInput provider="anthropic" />
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="model">
            <Card>
              <CardHeader>
                <CardTitle>{t("settings.model.card_title")}</CardTitle>
                <CardDescription>
                  {t("settings.model.card_desc")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {ANTHROPIC_MODELS.map((m) => (
                  <Label
                    key={m.id}
                    className="flex cursor-pointer items-start gap-3 rounded-md border border-border p-3 hover:bg-accent"
                  >
                    <input
                      type="radio"
                      name="model"
                      value={m.id}
                      checked={settings.model === m.id}
                      onChange={() => update({ model: m.id })}
                      className="mt-1"
                    />
                    <span className="flex-1">
                      <span className="block font-medium">{m.id}</span>
                      <span className="block text-sm text-muted-foreground">
                        {t(m.labelKey)}
                      </span>
                    </span>
                  </Label>
                ))}
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
