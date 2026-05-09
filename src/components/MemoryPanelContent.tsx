// MemoryPanelContent — v0.5 PR 1 (D-097/D-098).
//
// 기존 markdown 편집 흐름 → DB facts 리스트로 완전 교체.
// 5섹션 그룹핑: Preferences / Corrections / Progress / Meta / Goals.
// 상단: 최근 7일 추가 placeholder (count 표시).
// edit/delete 버튼: disabled + title 툴팁 (PR 5에서 활성화).
// 빈 상태: "아직 facts가 없어요. chat을 사용하면 자동으로 누적됩니다."

import { Loader2, Pencil, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { api } from "@/lib/api";
import { appErrorMessage, isAppError, type Fact, type FactKind } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";

// confidence 색 바 분류.
type ConfidenceLevel = "low" | "mid" | "high";

function confidenceLevel(c: number): ConfidenceLevel {
  if (c < 0.5) return "low";
  if (c < 0.85) return "mid";
  return "high";
}

function ConfidenceBar({ confidence }: { confidence: number }) {
  const { t } = useTranslation();
  const level = confidenceLevel(confidence);
  const colorClass =
    level === "low"
      ? "bg-muted-foreground/30"
      : level === "mid"
        ? "bg-amber-400"
        : "bg-emerald-500";
  const label =
    level === "low"
      ? t("memory.facts.confidence.low")
      : level === "mid"
        ? t("memory.facts.confidence.mid")
        : t("memory.facts.confidence.high");

  return (
    <div
      className="flex h-1.5 w-12 overflow-hidden rounded-full bg-muted"
      aria-label={label}
      title={`${label} (${Math.round(confidence * 100)}%)`}
    >
      <div
        className={`${colorClass} h-full`}
        style={{ width: `${Math.round(confidence * 100)}%` }}
      />
    </div>
  );
}

const SECTION_KINDS: FactKind[] = [
  "preference",
  "correction",
  "progress",
  "meta",
  "goal",
];

function FactItem({ fact }: { fact: Fact }) {
  const { t } = useTranslation();
  const disabledLabel = t("memory.facts.actions.edit_disabled");
  return (
    <div className="flex items-start gap-2 rounded-md border border-border/40 bg-card px-3 py-2">
      <div className="flex flex-1 flex-col gap-1 min-w-0">
        <p className="text-xs text-foreground leading-snug break-words">
          {fact.content}
        </p>
        <ConfidenceBar confidence={fact.confidence} />
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <button
          disabled
          className="rounded p-1 text-muted-foreground opacity-40 cursor-not-allowed"
          aria-label={disabledLabel}
          title={disabledLabel}
        >
          <Pencil size={12} />
        </button>
        <button
          disabled
          className="rounded p-1 text-muted-foreground opacity-40 cursor-not-allowed"
          aria-label={t("memory.facts.actions.delete_disabled")}
          title={t("memory.facts.actions.delete_disabled")}
        >
          <Trash2 size={12} />
        </button>
      </div>
    </div>
  );
}

function SectionGroup({
  kind,
  facts,
}: {
  kind: FactKind;
  facts: Fact[];
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-1.5">
      <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
        {t(`memory.section.${kind}`)}
      </h3>
      {facts.length === 0 ? (
        <p className="text-xs text-muted-foreground italic">
          {t("memory.facts.empty_section")}
        </p>
      ) : (
        facts.map((f) => <FactItem key={f.id} fact={f} />)
      )}
    </div>
  );
}

export function MemoryPanelContent() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const slug = activeStudy?.slug ?? null;

  const [facts, setFacts] = useState<Fact[]>([]);
  const [recentCount, setRecentCount] = useState<number>(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!slug) return;
    let cancelled = false;

    void (async () => {
      setLoading(true);
      setError(null);
      try {
        const [all, recent] = await Promise.all([
          api.memoryFactsList(slug),
          api.memoryFactsRecent(slug, 7),
        ]);
        if (!cancelled) {
          setFacts(all);
          setRecentCount(recent.length);
        }
      } catch (e) {
        if (!cancelled) {
          setError(isAppError(e) ? appErrorMessage(e) : String(e));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [slug]);

  if (!slug) {
    return (
      <p className="text-xs text-muted-foreground">
        {t("memory.no_active_study")}
      </p>
    );
  }

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="animate-spin" size={20} />
      </div>
    );
  }

  if (error) {
    return (
      <p className="text-sm text-destructive" role="alert">
        {error}
      </p>
    );
  }

  // 섹션별 그룹핑 — active facts만.
  const byKind = Object.fromEntries(
    SECTION_KINDS.map((k) => [k, facts.filter((f) => f.kind === k && f.status === "active")]),
  ) as Record<FactKind, Fact[]>;

  const totalActive = facts.filter((f) => f.status === "active").length;

  return (
    <div className="flex h-full flex-col gap-3 overflow-y-auto">
      {/* 상단: 최근 7일 추가 placeholder */}
      <div className="rounded-md border border-border/50 bg-muted/30 px-3 py-2">
        <p className="text-xs font-medium text-muted-foreground">
          {t("memory.facts.recent_added")}
          {": "}
          <span className="font-semibold text-foreground">{recentCount}</span>
        </p>
      </div>

      {/* 빈 상태 전체 */}
      {totalActive === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 py-8">
          <p className="text-center text-xs text-muted-foreground">
            {t("memory.facts.empty_state")}
          </p>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          {SECTION_KINDS.map((kind) => (
            <SectionGroup key={kind} kind={kind} facts={byKind[kind]} />
          ))}
        </div>
      )}
    </div>
  );
}
