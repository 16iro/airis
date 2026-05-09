// ReportsPage — v0.5 PR 5 (D-102).
//
// 구성:
//   - ExportButton (HTML 내보내기 — 브라우저 window.print 기반)
//   - SelfRatingForm (자가 평가 0~100)
//   - DevPanel (acceptance gate 5종 + citation)
//   - BatchReviewQueue (신뢰도 낮은 facts 일괄 검토)
//   - FactsList (MemoryPanelContent mode="editable")
//
// DevPanel은 settings.learning_dev_panel_enabled === true 이거나
// import.meta.env.DEV === true 일 때 표시.

import { Loader2, Printer } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { MemoryPanelContent } from "@/components/MemoryPanelContent";
import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import type { AcceptanceMetrics, Fact } from "@/lib/types";
import { appErrorMessage, isAppError } from "@/lib/types";
import { useSettingsStore } from "@/store/settingsStore";
import { useStudyStore } from "@/store/studyStore";

// ---------------------------------------------------------------------------
// ExportButton — window.print() 기반. CSS @media print로 스타일 분리됨.
// ---------------------------------------------------------------------------
function ExportButton() {
  const { t } = useTranslation();
  const [printing, setPrinting] = useState(false);

  function handleExport() {
    setPrinting(true);
    // 짧은 지연 후 프린트 다이얼로그 — 레이아웃 렌더 완료 대기.
    setTimeout(() => {
      window.print();
      setPrinting(false);
    }, 150);
  }

  return (
    <Button
      variant="outline"
      size="sm"
      onClick={handleExport}
      disabled={printing}
    >
      {printing ? (
        <Loader2 size={14} className="animate-spin" />
      ) : (
        <Printer size={14} />
      )}
      {printing ? t("reports.export.exporting") : t("reports.export.button")}
    </Button>
  );
}

// ---------------------------------------------------------------------------
// SelfRatingForm — 0~100 점수 입력 + 24h 쿨다운 체크.
// ---------------------------------------------------------------------------
function SelfRatingForm() {
  const { t } = useTranslation();
  const [eligible, setEligible] = useState<boolean | null>(null);
  const [score, setScore] = useState<string>("75");
  const [submitting, setSubmitting] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api.learningSelfRatingEligible().then(setEligible).catch(() => setEligible(false));
  }, []);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const n = parseInt(score, 10);
    if (isNaN(n) || n < 0 || n > 100) return;
    setSubmitting(true);
    setError(null);
    try {
      await api.learningSelfRatingRecord(n);
      setDone(true);
      setEligible(false);
    } catch (err) {
      setError(isAppError(err) ? appErrorMessage(err) : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <section className="rounded-lg border border-border bg-card p-4">
      <h2 className="mb-1 text-sm font-semibold">{t("reports.self_rating.title")}</h2>
      <p className="mb-3 text-xs text-muted-foreground">{t("reports.self_rating.desc")}</p>
      {done ? (
        <p className="text-xs text-emerald-600">{t("reports.self_rating.done")}</p>
      ) : eligible === false ? (
        <p className="text-xs text-muted-foreground">{t("reports.self_rating.not_eligible")}</p>
      ) : (
        <form onSubmit={(e) => void handleSubmit(e)} className="flex items-end gap-3">
          <div className="flex flex-col gap-1">
            <label htmlFor="self-rating-score" className="text-xs text-muted-foreground">
              {t("reports.self_rating.label")}
            </label>
            <input
              id="self-rating-score"
              type="number"
              min={0}
              max={100}
              value={score}
              onChange={(e) => setScore(e.target.value)}
              className="w-24 rounded border border-border bg-background px-2 py-1 text-sm focus:outline-none focus:ring-1 focus:ring-ring"
              disabled={submitting}
            />
          </div>
          <Button type="submit" size="sm" disabled={submitting || eligible === null}>
            {submitting ? (
              <><Loader2 size={12} className="animate-spin" />{t("reports.self_rating.submitting")}</>
            ) : t("reports.self_rating.submit")}
          </Button>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
        </form>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// DevPanel — acceptance gate 5종 + citation. dev 빌드 또는 settings 토글 시 표시.
// ---------------------------------------------------------------------------
function GateRow({
  label,
  pass,
  value,
}: {
  label: string;
  pass: boolean | null;
  value: string;
}) {
  const color =
    pass === true
      ? "text-emerald-600"
      : pass === false
        ? "text-destructive"
        : "text-muted-foreground";
  const badge =
    pass === true ? "PASS" : pass === false ? "FAIL" : "—";

  return (
    <div className="flex items-start justify-between gap-2 py-1.5 border-b border-border/30 last:border-0">
      <span className="text-xs text-muted-foreground flex-1">{label}</span>
      <span className={`text-xs font-mono font-medium ${color} shrink-0`}>
        {badge} · {value}
      </span>
    </div>
  );
}

function DevPanel({ studySlug }: { studySlug: string }) {
  const { t } = useTranslation();
  const [metrics, setMetrics] = useState<AcceptanceMetrics | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function runMeasure() {
    setLoading(true);
    setError(null);
    try {
      const m = await api.learningAcceptanceMetrics(studySlug);
      setMetrics(m);
    } catch (err) {
      setError(isAppError(err) ? appErrorMessage(err) : String(err));
    } finally {
      setLoading(false);
    }
  }

  function pct(v: number | null): string {
    if (v === null) return "—";
    return `${Math.round(v * 100)}%`;
  }

  function num(v: number | null, decimals = 2): string {
    if (v === null) return "—";
    return v.toFixed(decimals);
  }

  return (
    <section className="rounded-lg border border-amber-300/60 bg-amber-50/10 p-4">
      <div className="mb-2 flex items-center justify-between">
        <div>
          <h2 className="text-sm font-semibold">{t("reports.dev_panel.title")}</h2>
          <p className="text-xs text-muted-foreground">{t("reports.dev_panel.desc")}</p>
        </div>
        <Button size="sm" variant="outline" onClick={() => void runMeasure()} disabled={loading}>
          {loading ? (
            <><Loader2 size={12} className="animate-spin" />{t("reports.dev_panel.running")}</>
          ) : t("reports.dev_panel.run")}
        </Button>
      </div>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      {metrics ? (
        <div className="mt-2">
          <GateRow
            label={t("reports.dev_panel.gate1")}
            pass={metrics.gate1_memory_keep_rate !== null ? metrics.gate1_memory_keep_rate >= 0.7 : null}
            value={
              metrics.gate1_memory_keep_rate !== null
                ? t("reports.dev_panel.gate1_pass", {
                    rate: Math.round(metrics.gate1_memory_keep_rate * 100),
                    active: metrics.gate1_active_7d,
                    total: metrics.gate1_total_inserted_7d,
                  })
                : t("reports.dev_panel.gate1_na")
            }
          />
          <GateRow
            label={t("reports.dev_panel.gate2")}
            pass={metrics.gate2_srs_quality_rate !== null ? metrics.gate2_srs_quality_rate >= 0.5 : null}
            value={
              metrics.gate2_srs_quality_rate !== null
                ? t("reports.dev_panel.gate2_pass", {
                    rate: Math.round(metrics.gate2_srs_quality_rate * 100),
                    passing: metrics.gate2_passing,
                    total: metrics.gate2_total_auto,
                  })
                : t("reports.dev_panel.gate2_na")
            }
          />
          <GateRow
            label={t("reports.dev_panel.gate3")}
            pass={null}
            value={t("reports.dev_panel.gate3_value", {
              count: metrics.gate3_dismissed_7d,
              total: metrics.gate3_total_signals_7d,
            })}
          />
          <GateRow
            label={t("reports.dev_panel.gate4")}
            pass={metrics.gate4_attempt_rate !== null ? metrics.gate4_attempt_rate >= 0.5 : null}
            value={
              metrics.gate4_attempt_rate !== null
                ? t("reports.dev_panel.gate4_pass", {
                    rate: Math.round(metrics.gate4_attempt_rate * 100),
                    attempted: metrics.gate4_attempted_7d,
                    total: metrics.gate4_total_triggers_7d,
                  })
                : t("reports.dev_panel.gate4_na")
            }
          />
          <GateRow
            label={t("reports.dev_panel.gate5")}
            pass={metrics.gate5_self_rating_avg !== null ? metrics.gate5_self_rating_avg >= 60 : null}
            value={
              metrics.gate5_self_rating_avg !== null
                ? t("reports.dev_panel.gate5_value", {
                    avg: metrics.gate5_self_rating_avg.toFixed(1),
                    count: metrics.gate5_self_rating_count,
                  })
                : t("reports.dev_panel.gate5_na")
            }
          />
          <GateRow
            label={t("reports.dev_panel.citation")}
            pass={null}
            value={
              metrics.citation_avg_last_50 !== null
                ? t("reports.dev_panel.citation_value", { score: num(metrics.citation_avg_last_50) })
                : t("reports.dev_panel.citation_na")
            }
          />
          {/* history_compression_ratio_avg is always null (in-memory only) — skip display */}
        </div>
      ) : !loading ? (
        <p className="mt-2 text-xs text-muted-foreground">
          {pct(null)} — {t("reports.dev_panel.run")}을 눌러 측정하세요.
        </p>
      ) : null}
    </section>
  );
}

// ---------------------------------------------------------------------------
// BatchReviewQueue — 신뢰도 낮은 (< 0.5) active facts 일괄 검토.
// ---------------------------------------------------------------------------
function BatchReviewQueue({ studySlug }: { studySlug: string }) {
  const { t } = useTranslation();
  const [facts, setFacts] = useState<Fact[]>([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [archiving, setArchiving] = useState(false);
  const [doneMsg, setDoneMsg] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setLoading(true);
      try {
        const recent = await api.memoryFactsRecent(studySlug, 7);
        if (!cancelled) {
          const lowConf = recent.filter(
            (f) => f.status === "active" && f.confidence < 0.5,
          );
          setFacts(lowConf);
          setSelected(new Set());
        }
      } catch {
        // silent — no data shown
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [studySlug]);

  function toggleAll() {
    if (selected.size === facts.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(facts.map((f) => f.id)));
    }
  }

  async function archiveSelected() {
    if (selected.size === 0) return;
    setArchiving(true);
    try {
      const ids = Array.from(selected);
      const count = await api.memoryFactsBulkStatus(ids, "archived");
      setFacts((prev) => prev.filter((f) => !selected.has(f.id)));
      setSelected(new Set());
      setDoneMsg(t("reports.batch_review.done", { count }));
    } catch {
      // ignore
    } finally {
      setArchiving(false);
    }
  }

  return (
    <section className="rounded-lg border border-border bg-card p-4">
      <h2 className="mb-1 text-sm font-semibold">{t("reports.batch_review.title")}</h2>
      <p className="mb-3 text-xs text-muted-foreground">{t("reports.batch_review.desc")}</p>
      {loading ? (
        <div className="flex justify-center py-4"><Loader2 size={18} className="animate-spin" /></div>
      ) : facts.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t("reports.batch_review.empty")}</p>
      ) : (
        <>
          <div className="mb-2 flex items-center gap-2">
            <Button size="sm" variant="outline" onClick={toggleAll} disabled={archiving}>
              {selected.size === facts.length
                ? t("reports.batch_review.deselect_all")
                : t("reports.batch_review.select_all")}
            </Button>
            <Button
              size="sm"
              variant="destructive"
              onClick={() => void archiveSelected()}
              disabled={selected.size === 0 || archiving}
            >
              {archiving ? (
                <><Loader2 size={12} className="animate-spin" />{t("reports.batch_review.archiving")}</>
              ) : t("reports.batch_review.archive_selected")}
              {selected.size > 0 ? ` (${selected.size})` : ""}
            </Button>
            {doneMsg ? <span className="text-xs text-emerald-600">{doneMsg}</span> : null}
          </div>
          <div className="flex flex-col gap-1.5 max-h-60 overflow-y-auto">
            {facts.map((f) => (
              <label
                key={f.id}
                className="flex cursor-pointer items-start gap-2 rounded border border-border/40 bg-muted/20 px-3 py-2 hover:bg-muted/40"
              >
                <input
                  type="checkbox"
                  className="mt-0.5 shrink-0"
                  checked={selected.has(f.id)}
                  onChange={(e) => {
                    const next = new Set(selected);
                    if (e.target.checked) next.add(f.id);
                    else next.delete(f.id);
                    setSelected(next);
                  }}
                />
                <div className="min-w-0 flex-1">
                  <p className="text-xs leading-snug break-words">{f.content}</p>
                  <p className="mt-0.5 text-[11px] text-muted-foreground">
                    {f.kind} · {Math.round(f.confidence * 100)}%
                  </p>
                </div>
              </label>
            ))}
          </div>
        </>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// ReportsPage — 메인 페이지 컴포넌트.
// ---------------------------------------------------------------------------
export function ReportsPage() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const settings = useSettingsStore((s) => s.settings);

  const showDevPanel =
    settings.learning_dev_panel_enabled === true ||
    (settings.learning_dev_panel_enabled === null && import.meta.env.DEV);

  if (!activeStudy) {
    return (
      <div className="flex h-full flex-col">
        <TopBar />
        <div className="flex flex-1 items-center justify-center">
          <p className="text-sm text-muted-foreground">{t("reports.no_active_study")}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-4 py-6 flex flex-col gap-5">
          {/* 헤더 */}
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="text-lg font-semibold">{t("reports.title")}</h1>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {t("reports.subtitle")}
              </p>
            </div>
            <ExportButton />
          </div>

          {/* 자가 평가 */}
          <SelfRatingForm />

          {/* Acceptance Dev Panel (dev 빌드 또는 settings 토글) */}
          {showDevPanel ? (
            <DevPanel studySlug={activeStudy.slug} />
          ) : null}

          {/* 일괄 검토 큐 */}
          <BatchReviewQueue studySlug={activeStudy.slug} />

          {/* 학습 기록 편집 */}
          <section className="rounded-lg border border-border bg-card p-4">
            <h2 className="mb-1 text-sm font-semibold">{t("reports.facts.title")}</h2>
            <p className="mb-3 text-xs text-muted-foreground">{t("reports.facts.desc")}</p>
            <div className="h-[400px]">
              <MemoryPanelContent mode="editable" />
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
