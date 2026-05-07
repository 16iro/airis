// v0.4.4 PR 5 (D-095) — BYOK 클라우드 임베딩 설정 섹션.
//
// Settings 모달의 "진단 → 고급 · BYOK 임베딩" 섹션. 일반 사용자에게는 권장하지 않는
// 고급 토글로 노출 — fastembed (로컬, 무료) → cloud (Voyage / Gemini) 라우팅을
// 사용자 키로 활성화한다.
//
// 본 컴포넌트는 다음을 묶어서 표시:
//   1. BYOK 활성/비활성 토글 (settings.byok_embedding ↔ null/Some).
//   2. provider 선택 라디오 (voyage / gemini).
//   3. 모델 dropdown (provider별 추천 모델).
//   4. API 키 입력 (password input + show/hide 토글) + 저장/삭제.
//   5. 예상 비용 카드 (chunks × avg_tokens × 단가).
//   6. 현재 라우팅 상태 (dev_byok_routing_check).
//
// 키 저장은 keyring에 영속 — settings.json에는 cfg(provider+model)만 박힘.

import { Eye, EyeOff, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  appErrorMessage,
  isAppError,
  type ByokConfig,
  type ByokCostEstimate,
  type ByokProvider,
  type ByokRoutingResult,
} from "@/lib/types";

/** provider별 dropdown 모델 — 백엔드 known_dim과 일관성 유지 (변경 시 양쪽 동시 수정). */
const PROVIDER_MODELS: Record<ByokProvider, ReadonlyArray<{ id: string; labelKey: string }>> = {
  voyage: [
    { id: "voyage-3-lite", labelKey: "settings.byok.model_voyage_3_lite" },
    { id: "voyage-3", labelKey: "settings.byok.model_voyage_3" },
  ],
  gemini: [
    { id: "text-embedding-004", labelKey: "settings.byok.model_text_embedding_004" },
  ],
};

const KEY_PLACEHOLDER: Record<ByokProvider, string> = {
  voyage: "pa-...",
  gemini: "AIza...",
};

const DEFAULT_MODEL: Record<ByokProvider, string> = {
  voyage: "voyage-3-lite",
  gemini: "text-embedding-004",
};

interface Props {
  /** settings.byok_embedding — null이면 비활성. */
  config: ByokConfig | null;
  /** cfg 변경 → settings.json 영속. null이면 BYOK 비활성. */
  onChange: (next: ByokConfig | null) => Promise<void> | void;
}

export function ByokSection({ config, onChange }: Props) {
  const { t } = useTranslation();
  const enabled = config !== null;
  const provider: ByokProvider = config?.provider ?? "voyage";
  const model = config?.model ?? DEFAULT_MODEL[provider];

  return (
    <div className="space-y-6">
      <div>
        <h3 className="mb-1 text-base font-semibold">
          {t("settings.byok.section_title")}
        </h3>
        <p className="text-sm text-muted-foreground">
          {t("settings.byok.section_desc")}
        </p>
      </div>

      <EnableToggle
        enabled={enabled}
        onToggle={async () => {
          if (enabled) {
            await onChange(null);
          } else {
            await onChange({ provider: "voyage", model: DEFAULT_MODEL.voyage });
          }
        }}
      />

      {enabled ? (
        <>
          <ProviderPicker
            provider={provider}
            onChange={async (p) => {
              await onChange({ provider: p, model: DEFAULT_MODEL[p] });
            }}
          />

          <ModelPicker
            provider={provider}
            model={model}
            onChange={async (m) => {
              await onChange({ provider, model: m });
            }}
          />

          <KeyInput provider={provider} />

          {provider === "voyage" && model.startsWith("voyage-3-lite") ? (
            <p className="text-xs text-amber-600 dark:text-amber-400" role="note">
              {t("settings.byok.warn_voyage_dim")}
            </p>
          ) : null}

          <CostCard provider={provider} model={model} />

          <RoutingStatus />
        </>
      ) : null}
    </div>
  );
}

function EnableToggle({
  enabled,
  onToggle,
}: {
  enabled: boolean;
  onToggle: () => Promise<void> | void;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {t("settings.byok.enable_label")}
      </Label>
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        onClick={() => void onToggle()}
        className={cn(
          "flex w-full cursor-pointer items-start gap-2.5 rounded-lg border p-3 text-left transition-all",
          enabled
            ? "border-primary bg-primary-soft"
            : "border-border bg-card hover:border-border-strong",
        )}
      >
        <span
          className={cn(
            "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border-2",
            enabled ? "border-primary" : "border-[oklch(0.86_0_0)]",
          )}
        >
          {enabled ? (
            <span className="h-2 w-2 rounded-full bg-primary" />
          ) : null}
        </span>
        <span className="flex-1">
          <span className="block text-sm font-medium">
            {enabled ? t("settings.byok.enable_label") : t("settings.byok.enable_desc")}
          </span>
          <span className="mt-0.5 block text-xs text-muted-foreground">
            {t("settings.byok.enable_desc")}
          </span>
        </span>
      </button>
    </div>
  );
}

function ProviderPicker({
  provider,
  onChange,
}: {
  provider: ByokProvider;
  onChange: (p: ByokProvider) => void;
}) {
  const { t } = useTranslation();
  const options: { id: ByokProvider; labelKey: string }[] = [
    { id: "voyage", labelKey: "settings.byok.provider_voyage" },
    { id: "gemini", labelKey: "settings.byok.provider_gemini" },
  ];
  return (
    <div>
      <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {t("settings.byok.provider_label")}
      </Label>
      <div className="space-y-2">
        {options.map((o) => (
          <button
            key={o.id}
            type="button"
            role="radio"
            aria-checked={provider === o.id}
            onClick={() => onChange(o.id)}
            className={cn(
              "flex w-full cursor-pointer items-start gap-2.5 rounded-lg border p-3 text-left transition-all",
              provider === o.id
                ? "border-primary bg-primary-soft"
                : "border-border bg-card hover:border-border-strong",
            )}
          >
            <span
              className={cn(
                "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border-2",
                provider === o.id ? "border-primary" : "border-[oklch(0.86_0_0)]",
              )}
            >
              {provider === o.id ? (
                <span className="h-2 w-2 rounded-full bg-primary" />
              ) : null}
            </span>
            <span className="text-sm font-medium">{t(o.labelKey)}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function ModelPicker({
  provider,
  model,
  onChange,
}: {
  provider: ByokProvider;
  model: string;
  onChange: (m: string) => void;
}) {
  const { t } = useTranslation();
  const models = PROVIDER_MODELS[provider];
  return (
    <div>
      <Label
        htmlFor="byok-model-select"
        className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground"
      >
        {t("settings.byok.model_label")}
      </Label>
      <select
        id="byok-model-select"
        value={model}
        onChange={(e) => onChange(e.target.value)}
        className="block w-full rounded-md border border-border bg-card px-3 py-2 text-sm"
      >
        {models.map((m) => (
          <option key={m.id} value={m.id}>
            {t(m.labelKey)}
          </option>
        ))}
      </select>
    </div>
  );
}

function KeyInput({ provider }: { provider: ByokProvider }) {
  const { t } = useTranslation();
  const [keyInput, setKeyInput] = useState("");
  const [reveal, setReveal] = useState(false);
  const [present, setPresent] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .byokKeyPresent(provider)
      .then(setPresent)
      .catch(() => setPresent(false));
  }, [provider]);

  async function handleSave() {
    if (!keyInput.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await api.byokKeySet(provider, keyInput);
      setKeyInput("");
      setPresent(true);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete() {
    setBusy(true);
    setError(null);
    try {
      await api.byokKeyDelete(provider);
      setPresent(false);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <Label htmlFor={`byok-key-${provider}`}>
          {t("settings.byok.key_label")}
        </Label>
        <span className="text-xs text-muted-foreground">
          {present === null
            ? t("common.checking")
            : present
              ? t("settings.byok.key_saved")
              : t("settings.byok.key_missing")}
        </span>
      </div>

      <div className="flex gap-2">
        <div className="relative flex-1">
          <Input
            id={`byok-key-${provider}`}
            type={reveal ? "text" : "password"}
            placeholder={KEY_PLACEHOLDER[provider]}
            value={keyInput}
            onChange={(e) => setKeyInput(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            className="pr-10 font-mono"
          />
          <button
            type="button"
            onClick={() => setReveal((v) => !v)}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            aria-label={
              reveal
                ? t("settings.byok.key_hide")
                : t("settings.byok.key_reveal")
            }
          >
            {reveal ? <EyeOff size={16} /> : <Eye size={16} />}
          </button>
        </div>
        <Button onClick={() => void handleSave()} disabled={!keyInput.trim() || busy}>
          {busy ? <Loader2 className="animate-spin" /> : null}
          {t("settings.byok.key_save")}
        </Button>
      </div>

      {present ? (
        <Button
          variant="outline"
          onClick={() => void handleDelete()}
          disabled={busy}
          className="text-destructive hover:text-destructive"
        >
          {busy ? <Loader2 className="animate-spin" /> : null}
          {t("settings.byok.key_delete")}
        </Button>
      ) : null}

      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}

      <p className="text-xs text-muted-foreground">
        {t("settings.byok.key_footer_note")}
      </p>
    </div>
  );
}

function CostCard({
  provider,
  model,
}: {
  provider: ByokProvider;
  model: string;
}) {
  const { t } = useTranslation();
  const [chunks, setChunks] = useState(1500);
  const [avgTokens, setAvgTokens] = useState(200);
  const [estimate, setEstimate] = useState<ByokCostEstimate | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setBusy(true);
    api
      .byokEstimateCost(provider, model, chunks, avgTokens)
      .then((r) => {
        if (!cancelled) setEstimate(r);
      })
      .catch(() => {
        if (!cancelled) setEstimate(null);
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [provider, model, chunks, avgTokens]);

  return (
    <div>
      <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {t("settings.byok.cost_label")}
      </Label>
      <div className="space-y-3 rounded-md border border-border bg-card p-3 text-sm">
        <div className="grid grid-cols-2 gap-2">
          <label className="block text-xs text-muted-foreground">
            {t("settings.byok.cost_chunks_input")}
            <Input
              type="number"
              min={0}
              value={chunks}
              onChange={(e) => setChunks(Math.max(0, Number(e.target.value) || 0))}
              className="mt-1"
            />
          </label>
          <label className="block text-xs text-muted-foreground">
            {t("settings.byok.cost_avg_tokens_input")}
            <Input
              type="number"
              min={0}
              value={avgTokens}
              onChange={(e) => setAvgTokens(Math.max(0, Number(e.target.value) || 0))}
              className="mt-1"
            />
          </label>
        </div>
        {busy ? (
          <p className="text-muted-foreground">
            <Loader2 className="mr-1 inline h-3 w-3 animate-spin" />
            {t("common.checking")}
          </p>
        ) : estimate ? (
          <p>
            {t("settings.byok.cost_summary", {
              chunks: estimate.chunks,
              tokens: estimate.avg_tokens_per_chunk,
              usd: estimate.usd_estimate.toFixed(4),
              unit: estimate.unit_price_label,
            })}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function RoutingStatus() {
  const { t } = useTranslation();
  const [routing, setRouting] = useState<ByokRoutingResult | null>(null);
  const [busy, setBusy] = useState(false);

  async function refresh() {
    setBusy(true);
    try {
      const r = await api.devByokRoutingCheck();
      setRouting(r);
    } catch {
      // no-op — gate 5는 dev 측정이라 실패 시 silent.
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh();
  }, []);

  return (
    <div>
      <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {t("settings.byok.routing_label")}
      </Label>
      <div className="space-y-2 rounded-md border border-border bg-card p-3 text-sm">
        {routing ? (
          <>
            <p>
              {t("settings.byok.routing_summary", { routed: routing.routed_to })}
            </p>
            {routing.byok_active && !routing.key_present ? (
              <p className="text-xs text-amber-600 dark:text-amber-400" role="alert">
                {t("settings.byok.routing_warn_no_key")}
              </p>
            ) : null}
          </>
        ) : busy ? (
          <p className="text-muted-foreground">
            <Loader2 className="mr-1 inline h-3 w-3 animate-spin" />
            {t("common.checking")}
          </p>
        ) : null}
        <Button variant="outline" size="sm" onClick={() => void refresh()} disabled={busy}>
          {busy ? <Loader2 className="mr-1 h-3 w-3 animate-spin" /> : null}
          {t("settings.byok.routing_refresh")}
        </Button>
      </div>
    </div>
  );
}
