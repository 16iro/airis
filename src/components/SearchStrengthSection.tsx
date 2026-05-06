// v0.4.3 PR 1 (D-086) — Settings 모달의 "검색 강도" 라디오 섹션.
//
// 본 컴포넌트는 props만 받는 stateless. Settings 페이지 트리(uiStore·settingsStore 등)
// 를 끌어오지 않아 vitest에서 격리 마운트 가능 (localStorage 등 환경 의존 X).
// Settings.tsx가 본 파일을 import해 사용한다.

import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import type { SearchStrength } from "@/lib/types";

const ORDER: SearchStrength[] = ["fast", "balanced", "accurate"];

export function SearchStrengthSection({
  strength,
  onChange,
}: {
  strength: SearchStrength;
  onChange: (s: SearchStrength) => void;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <h3 className="mb-1 text-base font-semibold">
        {t("settings.search_strength.section_title")}
      </h3>
      <p className="mb-3 text-sm text-muted-foreground">
        {t("settings.search_strength.section_desc")}
      </p>
      <div className="space-y-2">
        {ORDER.map((s) => (
          <SearchStrengthRadio
            key={s}
            selected={strength === s}
            onClick={() => onChange(s)}
            label={t(`settings.search_strength.${s}`)}
            sub={t(`settings.search_strength.${s}_desc`)}
          />
        ))}
      </div>
    </div>
  );
}

/**
 * Settings.tsx의 RadioCard와 동일 모양 — 본 컴포넌트만의 단위 테스트가 RadioCard 트리
 * 의존을 피하도록 inline. RadioCard 자체는 Settings.tsx 안 다른 라디오에서 그대로 재사용.
 */
function SearchStrengthRadio({
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
