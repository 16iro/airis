// 단계 진행 표시 — 마법사 한 화면 패턴 (옵션 A, v0.2_HANDOFF.md 결정).
//
// PR 21 framer-motion 도입 이후 슬라이드(B)로 셸 교체 시에도 *이 컴포넌트는 그대로* 재사용.
// 진행률만 시각화 — 단계 전환 자체는 부모(NewStudyWizard)가 결정.

import { Check } from "lucide-react";

import { cn } from "@/lib/utils";

interface StepIndicatorProps {
  /** 1-base 현재 단계. */
  current: number;
  /** 단계 수. */
  total: number;
  /** 각 단계의 짧은 라벨 (스크린리더용). */
  labels: string[];
}

export function StepIndicator({ current, total, labels }: StepIndicatorProps) {
  return (
    <ol
      className="flex items-center gap-3"
      aria-label={`${current} / ${total}`}
    >
      {Array.from({ length: total }).map((_, i) => {
        const idx = i + 1;
        const state =
          idx < current ? "done" : idx === current ? "active" : "pending";
        return (
          <li key={idx} className="flex items-center gap-3">
            <span
              aria-current={state === "active" ? "step" : undefined}
              aria-label={labels[i] ?? `${idx}`}
              className={cn(
                "flex h-7 w-7 items-center justify-center rounded-full border text-xs font-medium transition-colors",
                state === "done" &&
                  "border-primary bg-primary text-primary-foreground",
                state === "active" &&
                  "border-primary bg-primary/10 text-primary",
                state === "pending" &&
                  "border-border bg-background text-muted-foreground",
              )}
            >
              {state === "done" ? <Check size={14} /> : idx}
            </span>
            {idx < total ? (
              <span
                className={cn(
                  "h-px w-8 transition-colors",
                  state === "done" ? "bg-primary" : "bg-border",
                )}
              />
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}
