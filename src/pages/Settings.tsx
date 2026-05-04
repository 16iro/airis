// Settings 모달 — prototype 100% 충실 (PR 37, D-070).
//
// 좌측 nav(200px, 4 그룹) + 우측 콘텐츠 패널 구조. prototype/screens-dialogs.jsx의
// SettingsScreen과 1:1 매칭. backdrop / X / Esc로 닫기.
//
// **인증 흐름은 v0.2.1 D-066 그대로 — CLI 브릿지 메인 + API 키 Advanced**.
// prototype은 API 키만 보여주지만 우리 구현은 둘 다 노출(CLI 카드 위, API 키 아래).

import { Check, Loader2, Moon, Sun, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { ApiKeyInput } from "@/components/ApiKeyInput";
import { CliSetupDialog } from "@/components/CliSetupDialog";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
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
  | "llm-models"
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
  const [section, setSection] = useState<SectionId>("llm-models");
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
        { id: "llm-models", label: t("settings.nav.llm_models") },
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
            {section === "llm-models" ? (
              <LlmModelsSection
                onOpenCliSetup={() => setCliSetupOpen(true)}
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

/**
 * LLM 모델 선택 섹션 — prototype을 벗어나 통합 (사용자 결정).
 *
 * 한 화면에 프로바이더 + 모델 + 인증 방식 + 인증 영역 + 연결 테스트.
 * 활성 프로바이더만 펼쳐서 안 영역 노출. 비활성은 라디오 헤더만.
 *
 * Race condition 방지: 프로바이더 전환·인증 방식 전환은 *await update* 끝날 때까지
 * 다른 라디오 클릭 차단 (switching local state).
 */
function LlmModelsSection({
  onOpenCliSetup,
}: {
  onOpenCliSetup: () => void;
}) {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);

  const [providerSwitching, setProviderSwitching] = useState<Provider | null>(null);
  const [authSwitching, setAuthSwitching] = useState<AuthMode | null>(null);

  async function handleProviderChange(p: Provider) {
    if (providerSwitching || p === settings.active_provider) return;
    setProviderSwitching(p);
    try {
      await update({ active_provider: p });
    } finally {
      setProviderSwitching(null);
    }
  }

  async function handleAuthModeChange(m: AuthMode) {
    if (authSwitching || m === settings.auth_mode) return;
    setAuthSwitching(m);
    try {
      await update({ auth_mode: m });
      if (m === "cli") onOpenCliSetup();
    } finally {
      setAuthSwitching(null);
    }
  }

  async function handleModelChange(modelId: string) {
    await update({
      models: { ...settings.models, [settings.active_provider]: modelId },
      model: modelId,
    });
  }

  return (
    <div className="space-y-3">
      <h3 className="text-base font-semibold">
        {t("settings.llm.section_title")}
      </h3>
      <p className="mb-2 text-sm text-muted-foreground">
        {t("settings.llm.section_desc")}
      </p>

      <ul className="space-y-3">
        {PROVIDERS.map((p) => {
          const isActive = settings.active_provider === p;
          const isSwitching = providerSwitching === p;
          const locked = providerSwitching !== null && !isActive;
          return (
            <li key={p}>
              <ProviderCard
                provider={p}
                expanded={isActive}
                switching={isSwitching}
                locked={locked}
                onSelect={() => void handleProviderChange(p)}
              >
                {isActive ? (
                  <ActiveProviderBody
                    provider={p}
                    settings={settings}
                    authSwitching={authSwitching}
                    onAuthModeChange={handleAuthModeChange}
                    onModelChange={(m) => void handleModelChange(m)}
                    onOpenCliSetup={onOpenCliSetup}
                  />
                ) : null}
              </ProviderCard>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function ProviderCard({
  provider,
  expanded,
  switching,
  locked,
  onSelect,
  children,
}: {
  provider: Provider;
  expanded: boolean;
  switching: boolean;
  locked: boolean;
  onSelect: () => void;
  children?: React.ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border transition-all",
        expanded
          ? "border-primary bg-primary-soft"
          : "border-border bg-card hover:border-border-strong",
        locked && "pointer-events-none opacity-50",
      )}
    >
      <button
        type="button"
        onClick={onSelect}
        disabled={switching || locked}
        className="flex w-full items-center gap-3 px-4 py-3 text-left disabled:cursor-not-allowed"
      >
        <span
          className={cn(
            "flex h-4 w-4 shrink-0 items-center justify-center rounded-full border-2",
            expanded ? "border-primary" : "border-[oklch(0.86_0_0)]",
          )}
        >
          {switching ? (
            <Loader2 className="h-2.5 w-2.5 animate-spin text-primary" />
          ) : expanded ? (
            <Check className="h-2.5 w-2.5 text-primary" strokeWidth={3} />
          ) : null}
        </span>
        <span className="text-sm font-medium">
          {t(`settings.provider.${provider}`)}
        </span>
      </button>
      {children ? (
        <div className="border-t border-primary/30 bg-card px-4 py-3">
          {children}
        </div>
      ) : null}
    </div>
  );
}

function ActiveProviderBody({
  provider,
  settings,
  authSwitching,
  onAuthModeChange,
  onModelChange,
  onOpenCliSetup,
}: {
  provider: Provider;
  settings: ReturnType<typeof useSettingsStore.getState>["settings"];
  authSwitching: AuthMode | null;
  onAuthModeChange: (m: AuthMode) => void;
  onModelChange: (m: string) => void;
  onOpenCliSetup: () => void;
}) {
  const { t } = useTranslation();
  const activeModels = PROVIDER_MODELS[provider];
  const currentModel = settings.models[provider] ?? settings.model;
  const authMode = settings.auth_mode;

  return (
    <div className="space-y-5">
      <div>
        <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("settings.llm.model_label")}
        </Label>
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

      <div>
        <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("settings.llm.auth_label")}
        </Label>
        <div className="space-y-2">
          {(["cli", "api_key"] as AuthMode[]).map((mode) => {
            const isCurrent = authMode === mode;
            const isSwitching = authSwitching === mode;
            const locked = authSwitching !== null && !isCurrent;
            return (
              <button
                key={mode}
                type="button"
                onClick={() => onAuthModeChange(mode)}
                disabled={isSwitching || locked}
                className={cn(
                  "flex w-full cursor-pointer items-start gap-2.5 rounded-lg border p-3 text-left transition-all disabled:cursor-not-allowed",
                  isCurrent
                    ? "border-primary bg-primary-soft"
                    : "border-border bg-card hover:border-border-strong",
                  locked && "opacity-50",
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border-2",
                    isCurrent ? "border-primary" : "border-[oklch(0.86_0_0)]",
                  )}
                >
                  {isSwitching ? (
                    <Loader2 className="h-2.5 w-2.5 animate-spin text-primary" />
                  ) : isCurrent ? (
                    <Check className="h-2.5 w-2.5 text-primary" strokeWidth={3} />
                  ) : null}
                </span>
                <span className="flex-1">
                  <span className="block text-sm font-medium">
                    {t(`auth.mode_${mode}`)}
                  </span>
                  <span className="mt-0.5 block text-xs text-muted-foreground">
                    {t(`auth.mode_${mode}_desc`)}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      </div>

      <div>
        <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {authMode === "cli"
            ? t("settings.llm.cli_section")
            : t("settings.llm.key_section")}
        </Label>
        {authMode === "cli" ? (
          <CliPanel provider={provider} onOpenCliSetup={onOpenCliSetup} />
        ) : (
          <ApiKeyInput provider={provider} />
        )}
      </div>
    </div>
  );
}

function CliPanel({
  provider,
  onOpenCliSetup,
}: {
  provider: Provider;
  onOpenCliSetup: () => void;
}) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<{
    installed: boolean;
    version: string | null;
    loggedIn: boolean | null;
  } | null>(null);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function runCheck(opts: { signalCancelled?: () => boolean } = {}) {
    setTesting(true);
    setError(null);
    try {
      const cli = await api.cliStatus(provider);
      if (opts.signalCancelled?.()) return;
      let loggedIn: boolean | null = null;
      if (cli.installed) {
        try {
          if (provider === "anthropic") {
            const a = await api.cliAuthStatusClaude();
            loggedIn = a.logged_in;
          } else if (provider === "gemini") {
            const a = await api.cliAuthStatusGemini();
            loggedIn = a.logged_in;
          } else if (provider === "openai") {
            const a = await api.cliAuthStatusCodex();
            loggedIn = a.logged_in;
          }
        } catch (e) {
          console.warn("auth status check failed:", e);
        }
      }
      if (opts.signalCancelled?.()) return;
      setStatus({
        installed: cli.installed,
        version: cli.version,
        loggedIn,
      });
    } catch (e) {
      if (opts.signalCancelled?.()) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (!opts.signalCancelled?.()) setTesting(false);
    }
  }

  useEffect(() => {
    let cancelled = false;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void runCheck({ signalCancelled: () => cancelled });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider]);

  const ok = status?.installed && status.loggedIn === true;

  return (
    <div className="space-y-3 rounded-md border border-border bg-card p-3">
      {status === null ? (
        <p className="text-xs text-muted-foreground">
          <Loader2 className="mr-1 inline h-3 w-3 animate-spin" />
          {t("settings.llm.test_running")}
        </p>
      ) : status.installed ? (
        <div className="flex items-center gap-2 text-xs">
          <Check className="h-3.5 w-3.5 text-[oklch(0.62_0.16_145)]" />
          <span>
            {t("cli_setup.cli_installed", {
              provider: provider === "anthropic" ? "Claude Code" : provider === "gemini" ? "Gemini CLI" : "Codex CLI",
              version: status.version ?? "?",
            })}
          </span>
        </div>
      ) : (
        <div className="flex items-center gap-2 text-xs text-amber-600 dark:text-amber-400">
          <span>
            {t("cli_setup.cli_not_installed", {
              provider: provider === "anthropic" ? "Claude Code" : provider === "gemini" ? "Gemini CLI" : "Codex CLI",
            })}
          </span>
        </div>
      )}

      {status?.installed ? (
        status.loggedIn === true ? (
          <div className="flex items-center gap-2 text-xs">
            <Check className="h-3.5 w-3.5 text-[oklch(0.62_0.16_145)]" />
            <span>{t("cli_setup.auth_logged_in_simple")}</span>
          </div>
        ) : status.loggedIn === false ? (
          <div className="flex items-center gap-2 text-xs text-amber-600 dark:text-amber-400">
            <span>{t("cli_setup.auth_not_logged_in")}</span>
          </div>
        ) : null
      ) : null}

      {error ? (
        <p className="text-xs text-destructive" role="alert">
          {error}
        </p>
      ) : null}

      <div className="flex gap-2 pt-1">
        <Button
          variant="outline"
          size="sm"
          onClick={() => void runCheck()}
          disabled={testing}
        >
          {testing ? (
            <Loader2 className="mr-1 h-3 w-3 animate-spin" />
          ) : null}
          {t("settings.llm.test_button")}
        </Button>
        <Button variant="outline" size="sm" onClick={onOpenCliSetup}>
          {t("cli_setup.dialog_title")}
        </Button>
        {ok ? null : null}
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
