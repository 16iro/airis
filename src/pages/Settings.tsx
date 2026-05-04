// Settings 모달 — prototype 100% 충실 (PR 37, D-070).
//
// 좌측 nav(200px, 4 그룹) + 우측 콘텐츠 패널 구조. prototype/screens-dialogs.jsx의
// SettingsScreen과 1:1 매칭. backdrop / X / Esc로 닫기.
//
// **인증 흐름은 v0.2.1 D-066 그대로 — CLI 브릿지 메인 + API 키 Advanced**.
// prototype은 API 키만 보여주지만 우리 구현은 둘 다 노출(CLI 카드 위, API 키 아래).

import { Check, Moon, Sun, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { ApiKeyInput } from "@/components/ApiKeyInput";
import { CliSetupDialog } from "@/components/CliSetupDialog";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import {
  type AuthMode,
  type InterventionLevel,
  PROVIDER_MODELS,
  PROVIDERS,
  type Provider,
} from "@/lib/types";
import { useSettingsStore } from "@/store/settingsStore";
import { useUiStore, type Density } from "@/store/uiStore";

type SectionId =
  | "llm-key"
  | "llm-model"
  | "llm-budget"
  | "int-meta"
  | "int-mem"
  | "int-val"
  | "ui-theme"
  | "ui-a11y"
  | "ui-keys"
  | "diag-usage";

interface NavGroup {
  group: string;
  items: { id: SectionId; label: string }[];
}

export function Settings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const loaded = useSettingsStore((s) => s.loaded);
  const load = useSettingsStore((s) => s.load);
  const update = useSettingsStore((s) => s.update);
  const setOpen = useUiStore((s) => s.setSettingsOpen);
  const setShortcutsOpen = useUiStore((s) => s.setShortcutsOpen);
  const [section, setSection] = useState<SectionId>("llm-key");
  const [cliSetupOpen, setCliSetupOpen] = useState(false);

  useEffect(() => {
    if (!loaded) load();
  }, [loaded, load]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  const groups: NavGroup[] = [
    {
      group: "LLM",
      items: [
        { id: "llm-key", label: t("settings.nav.llm_key") },
        { id: "llm-model", label: t("settings.nav.llm_model") },
        { id: "llm-budget", label: t("settings.nav.llm_budget") },
      ],
    },
    {
      group: t("settings.nav.group_intensity"),
      items: [
        { id: "int-meta", label: t("settings.nav.int_meta") },
        { id: "int-mem", label: t("settings.nav.int_mem") },
        { id: "int-val", label: t("settings.nav.int_val") },
      ],
    },
    {
      group: t("settings.nav.group_ui"),
      items: [
        { id: "ui-theme", label: t("settings.nav.ui_theme") },
        { id: "ui-a11y", label: t("settings.nav.ui_a11y") },
        { id: "ui-keys", label: t("settings.nav.ui_keys") },
      ],
    },
    {
      group: t("settings.nav.group_diag"),
      items: [{ id: "diag-usage", label: t("settings.nav.diag_usage") }],
    },
  ];

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="settings-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-5"
      onClick={() => setOpen(false)}
    >
      <Card
        className="flex h-[80vh] w-full max-w-4xl flex-col overflow-hidden p-0"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex shrink-0 items-center justify-between border-b border-border px-5 py-3.5">
          <h2 id="settings-title" className="text-base font-semibold">
            {t("settings.title")}
          </h2>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setOpen(false)}
            aria-label={t("common.close")}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div className="flex min-h-0 flex-1 overflow-hidden">
          <nav className="w-[200px] shrink-0 overflow-auto border-r border-border py-2">
            {groups.map((g) => (
              <div key={g.group}>
                <div className="mx-3 mt-3.5 mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                  {g.group}
                </div>
                {g.items.map((it) => (
                  <button
                    key={it.id}
                    type="button"
                    onClick={() => {
                      if (it.id === "ui-keys") {
                        setOpen(false);
                        setShortcutsOpen(true);
                        return;
                      }
                      setSection(it.id);
                    }}
                    className={cn(
                      "mx-1.5 my-px flex w-[calc(100%-12px)] items-center gap-2 rounded-md px-3 py-1.5 text-left text-[13px] hover:bg-muted",
                      section === it.id &&
                        "bg-primary-soft font-medium text-primary",
                    )}
                  >
                    {it.label}
                  </button>
                ))}
              </div>
            ))}
          </nav>

          <div className="flex-1 overflow-auto p-6">
            {section === "llm-key" ? (
              <LlmKeySection
                authMode={settings.auth_mode}
                onAuthModeChange={(m) => {
                  void update({ auth_mode: m });
                  if (m === "cli") setCliSetupOpen(true);
                }}
                onOpenCliSetup={() => setCliSetupOpen(true)}
              />
            ) : null}
            {section === "llm-model" ? (
              <LlmModelSection
                activeProvider={settings.active_provider}
                models={settings.models}
                fallbackModel={settings.model}
                onProviderChange={(p) => update({ active_provider: p })}
                onModelChange={(m) =>
                  update({
                    models: { ...settings.models, [settings.active_provider]: m },
                    model: m,
                  })
                }
              />
            ) : null}
            {section === "llm-budget" ? <PlaceholderSection /> : null}
            {section === "int-meta" ? (
              <InterventionSection
                level={settings.intervention_level}
                onChange={(l) => update({ intervention_level: l })}
              />
            ) : null}
            {section === "int-mem" || section === "int-val" ? (
              <PlaceholderSection />
            ) : null}
            {section === "ui-theme" ? <ThemeSection /> : null}
            {section === "ui-a11y" ? <PlaceholderSection /> : null}
            {section === "diag-usage" ? <PlaceholderSection /> : null}
          </div>
        </div>
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

function PlaceholderSection() {
  const { t } = useTranslation();
  return (
    <div className="py-10 text-center text-sm text-muted-foreground">
      {t("settings.placeholder")}
    </div>
  );
}

function LlmKeySection({
  authMode,
  onAuthModeChange,
  onOpenCliSetup,
}: {
  authMode: AuthMode;
  onAuthModeChange: (m: AuthMode) => void;
  onOpenCliSetup: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-6">
      <div>
        <h3 className="mb-1 text-base font-semibold">
          {t("auth.mode_card_title")}
        </h3>
        <p className="mb-3 text-sm text-muted-foreground">
          {t("auth.mode_card_desc")}
        </p>
        <div className="space-y-2">
          {(["cli", "api_key"] as AuthMode[]).map((mode) => (
            <RadioCard
              key={mode}
              selected={authMode === mode}
              onClick={() => onAuthModeChange(mode)}
              label={t(`auth.mode_${mode}`)}
              sub={t(`auth.mode_${mode}_desc`)}
            />
          ))}
        </div>
        {authMode === "cli" ? (
          <Button
            variant="outline"
            size="sm"
            onClick={onOpenCliSetup}
            className="mt-3"
          >
            {t("cli_setup.dialog_title")}
          </Button>
        ) : null}
      </div>

      <div>
        <h3 className="mb-1 text-base font-semibold">
          {t("settings.api_key.card_title")}
        </h3>
        <p className="mb-3 text-sm text-muted-foreground">
          {t("settings.advanced.api_key_desc")}
        </p>
        <div className="space-y-5">
          {PROVIDERS.map((p) => (
            <div key={p} className="space-y-2">
              <h4 className="text-sm font-medium">
                {t(`settings.provider.${p}`)}
              </h4>
              <ApiKeyInput provider={p} />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function LlmModelSection({
  activeProvider,
  models,
  fallbackModel,
  onProviderChange,
  onModelChange,
}: {
  activeProvider: Provider;
  models: Record<string, string>;
  fallbackModel: string;
  onProviderChange: (p: Provider) => void;
  onModelChange: (m: string) => void;
}) {
  const { t } = useTranslation();
  const activeModels = PROVIDER_MODELS[activeProvider];
  const currentModel = models[activeProvider] ?? fallbackModel;
  return (
    <div className="space-y-6">
      <div>
        <h3 className="mb-1 text-base font-semibold">
          {t("settings.provider.card_title")}
        </h3>
        <p className="mb-3 text-sm text-muted-foreground">
          {t("settings.provider.card_desc")}
        </p>
        <div className="space-y-2">
          {PROVIDERS.map((p) => (
            <RadioCard
              key={p}
              selected={activeProvider === p}
              onClick={() => onProviderChange(p)}
              label={t(`settings.provider.${p}`)}
            />
          ))}
        </div>
      </div>

      <div>
        <h3 className="mb-1 text-base font-semibold">
          {t("settings.model.card_title")}
        </h3>
        <p className="mb-3 text-sm text-muted-foreground">
          {t(`settings.provider.${activeProvider}`)} · {t("settings.model.card_desc")}
        </p>
        <div className="space-y-2">
          {activeModels.map((m) => (
            <RadioCard
              key={m.id}
              selected={currentModel === m.id}
              onClick={() => onModelChange(m.id)}
              label={m.id}
              sub={t(m.labelKey)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function InterventionSection({
  level,
  onChange,
}: {
  level: InterventionLevel;
  onChange: (l: InterventionLevel) => void;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <h3 className="mb-1 text-base font-semibold">{t("intervention.card_title")}</h3>
      <p className="mb-3 text-sm text-muted-foreground">
        {t("intervention.card_desc")}
      </p>
      <div className="space-y-2">
        {(["confirm", "auto", "off"] as InterventionLevel[]).map((l) => (
          <RadioCard
            key={l}
            selected={level === l}
            onClick={() => onChange(l)}
            label={t(`intervention.${l}`)}
            sub={t(`intervention.${l}_desc`)}
          />
        ))}
      </div>
    </div>
  );
}

function ThemeSection() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const density = useUiStore((s) => s.density);
  const setDensity = useUiStore((s) => s.setDensity);
  const accentHue = useUiStore((s) => s.accentHue);
  const setAccentHue = useUiStore((s) => s.setAccentHue);

  const themeOptions = [
    { v: "light" as const, label: t("settings.theme.light"), icon: <Sun size={14} /> },
    { v: "dark" as const, label: t("settings.theme.dark"), icon: <Moon size={14} /> },
  ];
  const densityOptions: { v: Density; label: string }[] = [
    { v: "compact", label: t("settings.density.compact") },
    { v: "normal", label: t("settings.density.normal") },
    { v: "comfortable", label: t("settings.density.comfortable") },
  ];
  const huePresets = [25, 200, 145, 280, 0];

  return (
    <div className="space-y-6">
      <div>
        <h3 className="mb-1 text-base font-semibold">
          {t("settings.theme.section_title")}
        </h3>
      </div>

      <div>
        <Label className="mb-2 block text-sm font-medium">
          {t("settings.theme.theme_label")}
        </Label>
        <div className="flex gap-2">
          {themeOptions.map((o) => (
            <Button
              key={o.v}
              variant={settings.theme === o.v ? "default" : "outline"}
              onClick={() => update({ theme: o.v })}
              className="flex-1"
            >
              {o.icon}
              {o.label}
            </Button>
          ))}
        </div>
      </div>

      <div>
        <Label className="mb-2 block text-sm font-medium">
          {t("settings.density.label")}
        </Label>
        <div className="flex gap-2">
          {densityOptions.map((o) => (
            <Button
              key={o.v}
              variant={density === o.v ? "default" : "outline"}
              onClick={() => setDensity(o.v)}
              className="flex-1"
            >
              {o.label}
            </Button>
          ))}
        </div>
      </div>

      <div>
        <Label className="mb-2 block text-sm font-medium">
          {t("settings.accent.label")}: <span className="font-mono">{accentHue}°</span>
        </Label>
        <input
          type="range"
          min={0}
          max={360}
          value={accentHue}
          onChange={(e) => setAccentHue(parseInt(e.target.value, 10))}
          className="w-full accent-primary"
        />
        <div className="mt-2 flex gap-1.5">
          {huePresets.map((h) => (
            <button
              key={h}
              type="button"
              onClick={() => setAccentHue(h)}
              className={cn(
                "h-7 w-7 rounded-full border-2",
                accentHue === h ? "border-foreground" : "border-border",
              )}
              style={{ background: `oklch(0.62 0.18 ${h})` }}
              aria-label={`Hue ${h}`}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function RadioCard({
  selected,
  onClick,
  label,
  sub,
}: {
  selected: boolean;
  onClick: () => void;
  label: string;
  sub?: string;
}) {
  return (
    <div
      role="radio"
      aria-checked={selected}
      tabIndex={0}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
      className={cn(
        "flex cursor-pointer items-start gap-2.5 rounded-lg border p-3 transition-all",
        selected
          ? "border-primary bg-primary-soft"
          : "border-border bg-card hover:border-border-strong",
      )}
    >
      <span
        className={cn(
          "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border-2",
          selected ? "border-primary" : "border-[oklch(0.86_0_0)]",
        )}
      >
        {selected ? (
          <Check className="h-2.5 w-2.5 text-primary" strokeWidth={3} />
        ) : null}
      </span>
      <span className="flex-1">
        <span className="block text-sm font-medium">{label}</span>
        {sub ? (
          <span className="mt-0.5 block text-xs text-muted-foreground">
            {sub}
          </span>
        ) : null}
      </span>
    </div>
  );
}
