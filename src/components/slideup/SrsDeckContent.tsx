// SRS Deck slideup 콘텐츠 — prototype 100% 충실 (PR 34, D-070).
//
// stat 카드 4개 (due/내일/이번주/전체) + 대기 카드 list 미리보기 + "복습 시작" 버튼.
// 복습 시작 → setSrsOpen(true) → 기존 SrsPanel modal에서 카드 풀이.
//
// 백엔드 srs_list_due만 정확. 내일/이번주/전체는 v0.4 이후 별도 API. 현재 placeholder.

import { Layers } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { type SrsCard } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

export function SrsDeckContent() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const setSrsOpen = useUiStore((s) => s.setSrsOpen);
  const [due, setDue] = useState<SrsCard[]>([]);
  const [loading, setLoading] = useState<boolean>(!!activeStudy);

  useEffect(() => {
    if (!activeStudy) return;
    let cancelled = false;
    void (async () => {
      try {
        const list = await api.srsListDue(activeStudy.slug);
        if (!cancelled) setDue(list);
      } catch (e) {
        console.warn("srsListDue failed:", e);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [activeStudy]);

  if (!activeStudy) {
    return (
      <p className="text-xs text-muted-foreground">{t("srs.no_active_study")}</p>
    );
  }

  return (
    <div>
      <div className="mb-3 flex items-baseline justify-between">
        <h3 className="text-base font-semibold">{t("srs.deck_title")}</h3>
        <Button size="sm" onClick={() => setSrsOpen(true)} disabled={due.length === 0 && !loading}>
          {due.length === 0 ? t("srs.no_due") : t("srs.start_review")}
        </Button>
      </div>
      <div className="mb-4 grid grid-cols-4 gap-2">
        <Stat label={t("srs.stat_due")} value={loading ? "…" : String(due.length)} accent />
        <Stat label={t("srs.stat_tomorrow")} value="—" />
        <Stat label={t("srs.stat_this_week")} value="—" />
        <Stat label={t("srs.stat_total")} value="—" />
      </div>
      <p className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {t("srs.queued")}
      </p>
      {loading ? (
        <p className="text-xs text-muted-foreground">{t("common.loading")}</p>
      ) : due.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t("srs.no_due")}</p>
      ) : (
        <ul className="space-y-1.5">
          {due.slice(0, 5).map((card) => (
            <li
              key={card.id}
              className="flex items-center gap-2 rounded-md border border-border bg-card px-2.5 py-2 text-xs"
            >
              <Layers size={14} className="shrink-0 text-primary" />
              <span className="flex-1 truncate">{card.front}</span>
              {card.section_ref ? (
                <span className="rounded-full border border-border px-2 py-0.5 font-mono text-[10px] text-muted-foreground">
                  {card.section_ref}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Stat({
  label,
  value,
  accent,
}: {
  label: string;
  value: string;
  accent?: boolean;
}) {
  return (
    <div
      className={
        "rounded-md border px-2.5 py-2 " +
        (accent
          ? "border-primary bg-primary-soft"
          : "border-border bg-card")
      }
    >
      <div className="text-[10px] uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
      <div
        className={
          "mt-0.5 font-mono text-xl font-semibold tabular-nums " +
          (accent ? "text-primary" : "text-foreground")
        }
      >
        {value}
      </div>
    </div>
  );
}
