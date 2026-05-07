// v0.4.4 PR 4 (D-094) — 하드웨어 자동 감지 + 모델 티어링 추천 카드.
//
// Settings 모달의 "진단 → 하드웨어 · 모델 추천" 섹션. 첫 진입 시 자동으로 사양을 측정하고
// RecommendedTier를 카드로 보여줍니다. 사용자가 "이 추천을 따르기"를 누르면
// settings.hardware_tier_override = null + hardware_recommended_at = now로 영속.
// "수동 선택"은 라디오로 등급을 직접 고르며, 그 즉시 settings에 반영됩니다.
//
// 본 컴포넌트는 props만 받는 stateless에 가깝지만, 추천 호출은 mount 시 invoke로 1회.
// vitest에서는 api invoke를 mock해서 격리 마운트 가능 (`SearchStrengthSection.test`와 같은 패턴).

import { Check, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import type {
  HardwareInfo,
  HardwareTier,
  RecommendationDetail,
} from "@/lib/types";

const TIER_ORDER: HardwareTier[] = ["conservative", "balanced", "aggressive"];

export function HardwareRecommendation({
  override,
  onChange,
}: {
  /** 사용자 수동 등급 (null = 자동 추천 따름). */
  override: HardwareTier | null;
  /**
   * 사용자가 등급 선택을 변경했을 때 호출.
   * - tier=null → 자동 추천 따름
   * - tier=값  → 수동 등급 영속
   * 호출자가 settings에 hardware_tier_override + hardware_recommended_at 함께 영속.
   */
  onChange: (tier: HardwareTier | null) => Promise<void> | void;
}) {
  const { t } = useTranslation();
  const [info, setInfo] = useState<HardwareInfo | null>(null);
  const [recommendation, setRecommendation] =
    useState<RecommendationDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [applying, setApplying] = useState<HardwareTier | "auto" | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        const [hw, rec] = await Promise.all([
          api.devProbeHardware(),
          api.devGetModelRecommendation(),
        ]);
        if (cancelled) return;
        setInfo(hw);
        setRecommendation(rec);
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, []);

  async function applyTier(tier: HardwareTier | null) {
    setApplying(tier ?? "auto");
    try {
      await onChange(tier);
    } finally {
      setApplying(null);
    }
  }

  // 화면에 표시할 *현재 등급* — override가 있으면 그걸, 없으면 추천을 따름.
  const effectiveTier: HardwareTier | null =
    override ?? recommendation?.tier ?? null;

  return (
    <div className="space-y-6">
      <div>
        <h3 className="mb-1 text-base font-semibold">
          {t("settings.hardware.section_title")}
        </h3>
        <p className="text-sm text-muted-foreground">
          {t("settings.hardware.section_desc")}
        </p>
      </div>

      <div>
        <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("settings.hardware.info_label")}
        </Label>
        <div className="rounded-md border border-border bg-card p-3 text-sm">
          {loading ? (
            <p className="text-muted-foreground">
              <Loader2 className="mr-1 inline h-3 w-3 animate-spin" />
              {t("settings.hardware.loading")}
            </p>
          ) : error ? (
            <p className="text-destructive" role="alert">
              {t("settings.hardware.error", { message: error })}
            </p>
          ) : info ? (
            <ul className="space-y-1">
              <li>{t("settings.hardware.info_cores", { cores: info.cpu_cores })}</li>
              <li>
                {t("settings.hardware.info_ram", {
                  total: info.total_ram_gb.toFixed(1),
                  avail: info.available_ram_gb.toFixed(1),
                })}
              </li>
              <li>
                {t("settings.hardware.info_os", {
                  os: info.os,
                  arch: info.arch,
                })}
              </li>
            </ul>
          ) : null}
        </div>
      </div>

      <div>
        <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("settings.hardware.recommendation_label")}
        </Label>
        {recommendation ? (
          <div className="space-y-3">
            <div
              className={cn(
                "rounded-md border p-3 text-sm",
                recommendation.below_minimum
                  ? "border-destructive bg-destructive/10"
                  : "border-primary bg-primary-soft",
              )}
            >
              <p className="font-medium">
                {t(`settings.hardware.tier_${recommendation.tier}`)}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {recommendation.reason}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.hardware.size_label", {
                  mb: recommendation.total_model_size_mb,
                })}
              </p>
              {recommendation.below_minimum ? (
                <p className="mt-2 text-xs font-medium text-destructive">
                  {t("settings.hardware.below_minimum")}
                </p>
              ) : null}
              <Button
                variant={override === null ? "default" : "outline"}
                size="sm"
                className="mt-3"
                onClick={() => void applyTier(null)}
                disabled={applying !== null}
              >
                {applying === "auto" ? (
                  <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                ) : null}
                {override === null
                  ? t("settings.hardware.applied")
                  : t("settings.hardware.follow_recommendation")}
              </Button>
            </div>

            <div>
              <Label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                {t("settings.hardware.manual_label")}
              </Label>
              <div className="space-y-2">
                {TIER_ORDER.map((tier) => (
                  <TierRadio
                    key={tier}
                    selected={override === tier}
                    onClick={() => void applyTier(tier)}
                    isEffective={effectiveTier === tier && override === null}
                    label={t(`settings.hardware.tier_${tier}`)}
                    sub={t(`settings.hardware.tier_${tier}_desc`)}
                    sizeMb={tierSizeMb(tier)}
                    sizeLabel={t("settings.hardware.size_label", {
                      mb: tierSizeMb(tier),
                    })}
                    disabled={applying !== null}
                  />
                ))}
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function tierSizeMb(tier: HardwareTier): number {
  // 백엔드 hardware_probe.rs 상수와 동일 값. 변경 시 양쪽 동시 수정.
  // T1=120, T2=2200, T3=600.
  switch (tier) {
    case "conservative":
      return 120;
    case "balanced":
      return 120 + 2200;
    case "aggressive":
      return 120 + 2200 + 600;
  }
}

function TierRadio({
  selected,
  isEffective,
  onClick,
  label,
  sub,
  sizeLabel,
  disabled,
}: {
  selected: boolean;
  isEffective: boolean;
  onClick: () => void;
  label: string;
  sub: string;
  sizeMb: number;
  sizeLabel: string;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "flex w-full cursor-pointer items-start gap-2.5 rounded-lg border p-3 text-left transition-all",
        selected
          ? "border-primary bg-primary-soft"
          : isEffective
            ? "border-border-strong bg-card"
            : "border-border bg-card hover:border-border-strong",
        disabled && "opacity-60",
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
        <span className="mt-0.5 block text-xs text-muted-foreground">{sub}</span>
        <span className="mt-0.5 block text-xs text-muted-foreground">
          {sizeLabel}
        </span>
      </span>
    </button>
  );
}
