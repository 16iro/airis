// v0.4.1 PR 5 — A/B 비교 dev 전용 패널.
//
// settings.dev_ab_compare가 ON일 때만 ChatPanel에서 토글 가능. 디폴트 OFF.
// 동작:
//   1. 사용자가 질문을 입력 → chat_send_ab_compare 호출.
//   2. 두 응답이 좌우 칸에 *동시 stream*. 좌우 위치는 *마운트 시점에 무작위*
//      (handoff §9 confounder 회피 — "왼쪽이 항상 새 엔진" 학습 방지).
//   3. 사용자가 더 좋은 쪽 클릭 → reveal + dev_ab_record_choice 호출. tie도 가능.
//   4. 누적 stats를 패널 하단에 항시 표시 (10건 도달 시 강조).
//
// chatStore와 *완전 분리* — chat:chunk / chat:done / chat:context 등은 본 컴포넌트가
// 듣지 않는다. 새 이벤트 chat:ab_chunk / chat:ab_done / chat:ab_complete / chat:ab_error를
// 별도 구독.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Loader2, Send, Sparkles } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import {
  type AbChoice,
  type AbChunkPayload,
  type AbCompletePayload,
  type AbDonePayload,
  type AbErrorPayload,
  type AbExportResult,
  type CacheStatsPayload,
  type ChatResponseTiming,
  type CitationAccuracy,
  type FollowupSkipRate,
  type PrefixCacheRatio,
  type ResponseCacheHitRatio,
  appErrorMessage,
  isAppError,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSettingsStore } from "@/store/settingsStore";
import { useStudyStore } from "@/store/studyStore";

/// 두 응답의 *내부* 식별자. 좌우 위치는 별개 — leftIsBaseline에 따라 swap.
type TrackId = "baseline" | "v041";

interface TrackState {
  text: string;
  /** undefined = 진행 중, true = chat:ab_done 도착, "error" = chat:ab_error 도착. */
  done: boolean | "error";
  /** v041 트랙만 source_count > 0. baseline은 항상 0/0. */
  citation_violations: { total: number; outOfRange: number; sourceCount: number };
  errorMessage?: string;
}

const EMPTY_TRACK: TrackState = {
  text: "",
  done: false,
  citation_violations: { total: 0, outOfRange: 0, sourceCount: 0 },
};

export function AbComparePanel() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  // v0.4.4 PR 2 (D-092) — dev raw event log 토글. ON이면 chat:ab_* 이벤트 카운터+payload
  // 콘솔 출력 (BUG-002 listener 누수 회귀 가시화).
  const devEventLog = useSettingsStore((s) => s.settings.dev_event_log);

  const [input, setInput] = useState("");
  const [handle, setHandle] = useState<string | null>(null);
  const [query, setQuery] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [errMessage, setErrMessage] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [recorded, setRecorded] = useState(false);
  const [stats, setStats] = useState<AbExportResult | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  // v0.4.2 PR 4 (D-084) — embedding/response cache 통계 표시.
  const [cacheStats, setCacheStats] = useState<CacheStatsPayload | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // 좌우 무작위 — 마운트 시점에 *1회만* 결정. 이후 같은 패널에선 고정. (사용자가
  // 화면을 닫고 다시 열면 새 무작위 적용.)
  // useState lazy init으로 처리 — useMemo + Math.random은 purity 룰 위반.
  const [leftIsBaseline] = useState(() => Math.random() < 0.5);

  const [tracks, setTracks] = useState<Record<TrackId, TrackState>>({
    baseline: EMPTY_TRACK,
    v041: EMPTY_TRACK,
  });

  // 누적 stats hydrate.
  async function refreshStats() {
    try {
      const result = await api.devAbExportResults();
      setStats(result);
      setExportError(null);
    } catch (e) {
      const msg = isAppError(e) ? appErrorMessage(e) : String(e);
      setExportError(msg);
    }
  }
  // v0.4.2 PR 4 — cache stats hydrate. 실패는 silent (dev panel만 영향).
  async function refreshCacheStats() {
    try {
      const result = await api.devCacheStats();
      setCacheStats(result);
    } catch {
      setCacheStats(null);
    }
  }
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refreshStats();
    void refreshCacheStats();
  }, []);

  // 이벤트 구독. 한 번만 등록.
  //
  // BUG-002 (v0.4.4 PR 2, D-092): ChatPanel과 동일한 listener race 패턴.
  // `listen()`은 비동기라 등록 Promise resolve 전에 컴포넌트가 unmount되면
  // (StrictMode dev / dockview 재마운트) cleanup이 빈 unlisteners 배열만 비우고
  // 끝나 listener가 영구 누수 → 다음 mount의 listener와 함께 chat:ab_chunk를
  // N회 처리. 모범: ChatPanel cancelled flag + .then(unlisten) 체이닝.
  useEffect(() => {
    let cancelled = false;
    const settled: UnlistenFn[] = [];
    const counters = { ab_chunk: 0, ab_done: 0, ab_complete: 0, ab_error: 0 };

    function track(p: Promise<UnlistenFn>) {
      void p.then((u) => {
        if (cancelled) {
          u();
        } else {
          settled.push(u);
        }
      });
    }

    track(
      listen<AbChunkPayload>("chat:ab_chunk", (event) => {
        if (devEventLog) {
          counters.ab_chunk += 1;
          console.debug("chat:ab_chunk", {
            count: counters.ab_chunk,
            payload: event.payload,
          });
        }
        const { handle: evHandle, track: evTrack, text } = event.payload;
        setHandle((current) => {
          if (current !== evHandle) return current;
          setTracks((s) => ({
            ...s,
            [evTrack]: {
              ...s[evTrack],
              text: s[evTrack].text + text,
            },
          }));
          return current;
        });
      }),
    );

    track(
      listen<AbDonePayload>("chat:ab_done", (event) => {
        if (devEventLog) {
          counters.ab_done += 1;
          console.debug("chat:ab_done", {
            count: counters.ab_done,
            payload: event.payload,
          });
        }
        const { handle: evHandle, track: evTrack, text, citation_violations } = event.payload;
        setHandle((current) => {
          if (current !== evHandle) return current;
          setTracks((s) => ({
            ...s,
            [evTrack]: {
              ...s[evTrack],
              // 일부 어댑터는 done 시점에 누적 텍스트를 *통째*로 다시 보낼 수 있음 — 길이 비교로
              // 더 긴 쪽을 신뢰. 일반적으론 chunk로 누적된 게 정확.
              text: text.length > s[evTrack].text.length ? text : s[evTrack].text,
              done: true,
              citation_violations: {
                total: citation_violations.total_markers,
                outOfRange: citation_violations.out_of_range,
                sourceCount: citation_violations.source_count,
              },
            },
          }));
          return current;
        });
      }),
    );

    track(
      listen<AbCompletePayload>("chat:ab_complete", (event) => {
        if (devEventLog) {
          counters.ab_complete += 1;
          console.debug("chat:ab_complete", {
            count: counters.ab_complete,
            payload: event.payload,
          });
        }
        const { handle: evHandle } = event.payload;
        setHandle((current) => {
          if (current !== evHandle) return current;
          setBusy(false);
          return current;
        });
      }),
    );

    track(
      listen<AbErrorPayload>("chat:ab_error", (event) => {
        if (devEventLog) {
          counters.ab_error += 1;
          console.debug("chat:ab_error", {
            count: counters.ab_error,
            payload: event.payload,
          });
        }
        const { handle: evHandle, track: evTrack, error } = event.payload;
        const errMsg = isAppError(error) ? appErrorMessage(error) : String(error);
        setHandle((current) => {
          if (current !== evHandle) return current;
          setTracks((s) => ({
            ...s,
            [evTrack]: { ...s[evTrack], done: "error", errorMessage: errMsg },
          }));
          return current;
        });
      }),
    );

    return () => {
      cancelled = true;
      for (const u of settled) u();
    };
  }, [devEventLog]);

  async function handleSend() {
    const trimmed = input.trim();
    if (!trimmed || busy || !activeStudy) return;
    setBusy(true);
    setErrMessage(null);
    setRevealed(false);
    setRecorded(false);
    setTracks({ baseline: EMPTY_TRACK, v041: EMPTY_TRACK });
    try {
      const { handle: newHandle } = await api.chatSendAbCompare(activeStudy.slug, trimmed);
      setHandle(newHandle);
      setQuery(trimmed);
      setInput("");
    } catch (e) {
      const msg = isAppError(e) ? appErrorMessage(e) : String(e);
      setErrMessage(msg);
      setBusy(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key === "Enter") {
      e.preventDefault();
      void handleSend();
    }
  }

  async function handleChoose(chose: AbChoice) {
    if (recorded || !handle || !query) return;
    setRevealed(true);
    setRecorded(true);
    try {
      await api.devAbRecordChoice(
        handle,
        query,
        tracks.baseline.text,
        tracks.v041.text,
        chose,
        null,
      );
      await refreshStats();
    } catch (e) {
      const msg = isAppError(e) ? appErrorMessage(e) : String(e);
      setErrMessage(msg);
      setRecorded(false);
    }
  }

  const leftTrack: TrackId = leftIsBaseline ? "baseline" : "v041";
  const rightTrack: TrackId = leftIsBaseline ? "v041" : "baseline";
  const hasResponse = handle !== null;

  return (
    <div className="flex h-full flex-col">
      <header className="flex shrink-0 items-center gap-2 border-b border-border px-4 py-3">
        <Sparkles size={14} className="text-primary" />
        <h2 className="text-sm font-semibold">{t("ab_compare.title")}</h2>
        <span className="text-[11px] text-muted-foreground">
          {t("ab_compare.subtitle")}
        </span>
      </header>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="grid min-h-0 flex-1 grid-cols-2 divide-x divide-border overflow-hidden">
          <ResponseColumn
            title={t("ab_compare.column_left")}
            placeholder={t("ab_compare.placeholder")}
            track={leftTrack}
            state={tracks[leftTrack]}
            revealed={revealed}
            hasResponse={hasResponse}
            onChoose={() =>
              void handleChoose(leftTrack === "baseline" ? "baseline" : "v041")
            }
          />
          <ResponseColumn
            title={t("ab_compare.column_right")}
            placeholder={t("ab_compare.placeholder")}
            track={rightTrack}
            state={tracks[rightTrack]}
            revealed={revealed}
            hasResponse={hasResponse}
            onChoose={() =>
              void handleChoose(rightTrack === "baseline" ? "baseline" : "v041")
            }
          />
        </div>

        <div className="shrink-0 border-t border-border bg-muted/30 px-4 py-2">
          <div className="flex items-center justify-between gap-3">
            <StatsBadge stats={stats} t={t} />
            {hasResponse && tracks.baseline.done && tracks.v041.done && !recorded ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => void handleChoose("tie")}
                aria-label={t("ab_compare.choose_tie")}
              >
                {t("ab_compare.choose_tie")}
              </Button>
            ) : null}
          </div>
          {exportError ? (
            <p className="mt-1 text-[11px] text-destructive" role="alert">
              {exportError}
            </p>
          ) : null}
          {errMessage ? (
            <p className="mt-1 text-[11px] text-destructive" role="alert">
              {errMessage}
            </p>
          ) : null}
          <CacheStatsLine stats={cacheStats} onRefresh={refreshCacheStats} />
          <AcceptanceGateMeasurements
            studySlug={activeStudy?.slug ?? null}
          />
        </div>
      </div>

      <div className="shrink-0 border-t border-border p-3">
        <div className="flex items-end gap-2">
          <Textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t("ab_compare.input_placeholder")}
            rows={2}
            disabled={busy}
            className="flex-1 resize-none font-sans"
          />
          <Button
            onClick={() => void handleSend()}
            disabled={!input.trim() || busy}
            size="sm"
            aria-label={t("ab_compare.send")}
          >
            {busy ? <Loader2 size={16} className="animate-spin" /> : <Send size={16} />}
          </Button>
        </div>
      </div>
    </div>
  );
}

function ResponseColumn({
  title,
  placeholder,
  track,
  state,
  revealed,
  hasResponse,
  onChoose,
}: {
  title: string;
  placeholder: string;
  track: TrackId;
  state: TrackState;
  revealed: boolean;
  hasResponse: boolean;
  onChoose: () => void;
}) {
  const { t } = useTranslation();
  const reveal = revealed
    ? track === "baseline"
      ? t("ab_compare.label_baseline")
      : t("ab_compare.label_v041")
    : null;
  const canChoose = hasResponse && state.done === true && !revealed;

  return (
    <div className="flex min-h-0 flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <div className="flex items-center gap-2 text-[11px] font-medium text-muted-foreground">
          <span>{title}</span>
          {reveal ? (
            <span
              className={cn(
                "rounded-md border px-1.5 py-0.5 text-[10px] font-semibold",
                track === "v041"
                  ? "border-primary/40 bg-primary/10 text-primary"
                  : "border-border bg-card text-foreground",
              )}
            >
              {reveal}
            </span>
          ) : null}
          {state.done === true && state.citation_violations.sourceCount > 0 ? (
            <CitationBadge v={state.citation_violations} />
          ) : null}
        </div>
        {canChoose ? (
          <Button size="sm" variant="default" onClick={onChoose}>
            {t("ab_compare.choose_this")}
          </Button>
        ) : null}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-3 text-sm">
        {!hasResponse ? (
          <p className="text-muted-foreground">{placeholder}</p>
        ) : state.done === "error" ? (
          <p className="text-destructive" role="alert">
            {t("ab_compare.track_error")}: {state.errorMessage ?? "unknown"}
          </p>
        ) : (
          <div className="whitespace-pre-wrap">
            {state.text}
            {state.done !== true ? (
              <span className="ml-1 inline-flex items-center gap-1 text-xs text-muted-foreground">
                <Loader2 size={12} className="animate-spin" />
                {t("ab_compare.streaming")}
              </span>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}

function CitationBadge({
  v,
}: {
  v: { total: number; outOfRange: number; sourceCount: number };
}) {
  const { t } = useTranslation();
  const tone = v.outOfRange > 0 ? "border-amber-500/40 bg-amber-500/10 text-amber-700" : "border-border bg-card";
  return (
    <span
      className={cn("rounded-md border px-1.5 py-0.5 text-[10px]", tone)}
      title={t("ab_compare.citation_tooltip", {
        total: v.total,
        oor: v.outOfRange,
        sources: v.sourceCount,
      })}
    >
      [S] {v.total - v.outOfRange}/{v.total}
    </span>
  );
}

function CacheStatsLine({
  stats,
  onRefresh,
}: {
  stats: CacheStatsPayload | null;
  onRefresh: () => void;
}) {
  const { t } = useTranslation();
  if (!stats) {
    return (
      <p className="mt-1 text-[11px] text-muted-foreground">
        {t("ab_compare.cache_stats_unavailable")}
      </p>
    );
  }
  const fmt = (n: number) => `${(n * 100).toFixed(0)}%`;
  return (
    <p className="mt-1 flex items-center gap-2 text-[11px] text-muted-foreground">
      <span>
        {t("ab_compare.cache_stats_embedding", {
          rows: stats.embedding.rows,
          ratio: fmt(stats.embedding.hit_ratio),
        })}
      </span>
      <span aria-hidden="true">·</span>
      <span>
        {t("ab_compare.cache_stats_response", {
          rows: stats.response.rows,
          ratio: fmt(stats.response.hit_ratio),
        })}
      </span>
      <button
        type="button"
        onClick={onRefresh}
        className="ml-auto text-[11px] underline decoration-dotted underline-offset-2 hover:text-foreground"
      >
        {t("ab_compare.cache_stats_refresh")}
      </button>
    </p>
  );
}

/// v0.4.2 PR 5 / v0.4.3 PR 5 — acceptance 측정 dev 패널 (handoff §1.6).
///
/// v0.4.2 측정 (수기 비교):
///   * 응답 시간 — 같은 study chat 응답 시간 평균(최근 5건). T2 빌드 X·진행 중을 두 번
///     실행해 비교 (50% 이내 증가가 PASS).
///   * 응답 캐시 — response_cache 누적 hit/miss + ratio. 같은 5건 재호출 후 hit 5/5면 PASS.
///
/// v0.4.3 4 gate (handoff §3):
///   * gate 1 — 인용 정확도(pass 비율 ≥ 85% 면 PASS).
///   * gate 2 — follow-up 효율(재사용 가능 비율 ≥ 60% 면 PASS).
///   * gate 3 — prompt prefix cache hit ratio(≥ 70% 면 PASS).
///   * gate 4 — A/B 비교 누적 stats(체감 품질 ≥ 8/10 — 본 패널 상단 StatsBadge 로 가시화).
///
/// 책 단위 측정(`dev_simulate_abnormal_shutdown` / `dev_inspect_active_index_state`)은
/// 사용자가 콘솔에서 직접 호출(handoff §1.7 참조).
function AcceptanceGateMeasurements({ studySlug }: { studySlug: string | null }) {
  const { t } = useTranslation();
  const [timing, setTiming] = useState<ChatResponseTiming | null>(null);
  const [hitRatio, setHitRatio] = useState<ResponseCacheHitRatio | null>(null);
  const [citation, setCitation] = useState<CitationAccuracy | null>(null);
  const [followup, setFollowup] = useState<FollowupSkipRate | null>(null);
  const [prefixCache, setPrefixCache] = useState<PrefixCacheRatio | null>(null);
  const [busy, setBusy] = useState(false);

  async function runMeasurements() {
    if (!studySlug || busy) return;
    setBusy(true);
    try {
      const [t1, t4, gate1, gate2, gate3] = await Promise.all([
        api.devMeasureChatResponseMs(studySlug, 5),
        api.devResponseCacheHitRatio(),
        // v0.4.3: 최근 50건 정도 누적이면 비율이 안정적. 사용자 1주 사용 후 의미 있는 값.
        api.devMeasureCitationAccuracy(studySlug, 50),
        api.devMeasureFollowupSkipRate(studySlug, 50),
        api.devMeasurePrefixCacheRatio(studySlug, 50),
      ]);
      setTiming(t1);
      setHitRatio(t4);
      setCitation(gate1);
      setFollowup(gate2);
      setPrefixCache(gate3);
    } catch {
      // dev 패널 — silent fail.
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
      <button
        type="button"
        onClick={() => void runMeasurements()}
        disabled={!studySlug || busy}
        className="rounded-md border border-border bg-card px-2 py-0.5 text-[11px] hover:bg-muted disabled:opacity-50"
        aria-label={t("ab_compare.acceptance_run") as string}
      >
        {busy ? "…" : t("ab_compare.acceptance_run")}
      </button>
      {timing ? (
        <span>
          {t("ab_compare.acceptance_gate3", {
            samples: timing.samples,
            avgMs: timing.avg_ms.toFixed(0),
          })}
        </span>
      ) : null}
      {hitRatio ? (
        <span>
          {t("ab_compare.acceptance_gate4", {
            hit: hitRatio.hit_count,
            miss: hitRatio.miss_count,
            ratio: `${(hitRatio.hit_ratio * 100).toFixed(0)}%`,
          })}
        </span>
      ) : null}
      {citation && citation.markers > 0 ? (
        <span>
          {t("ab_compare.acceptance_v043_citation", {
            pass: citation.pass,
            markers: citation.markers,
            ratio: `${(citation.pass_ratio * 100).toFixed(0)}%`,
            avgScore: citation.avg_score.toFixed(2),
          })}
        </span>
      ) : null}
      {followup && followup.user_messages > 0 ? (
        <span>
          {t("ab_compare.acceptance_v043_followup", {
            reusable: followup.reusable_followups,
            users: followup.user_messages,
            rate: `${(followup.skip_rate * 100).toFixed(0)}%`,
          })}
        </span>
      ) : null}
      {prefixCache && prefixCache.messages > 0 ? (
        <span>
          {t("ab_compare.acceptance_v043_cache", {
            cache: prefixCache.cache_read_total,
            total: prefixCache.cache_read_total + prefixCache.input_total,
            ratio: `${(prefixCache.hit_ratio * 100).toFixed(0)}%`,
          })}
        </span>
      ) : null}
    </div>
  );
}

function StatsBadge({
  stats,
  t,
}: {
  stats: AbExportResult | null;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  if (!stats || stats.total === 0) {
    return <span className="text-[11px] text-muted-foreground">{t("ab_compare.stats_empty")}</span>;
  }
  const milestone = stats.total >= 10;
  return (
    <span
      className={cn(
        "text-[11px]",
        milestone ? "font-semibold text-primary" : "text-muted-foreground",
      )}
      aria-label={t("ab_compare.stats_aria")}
    >
      {t("ab_compare.stats_summary", {
        v041: stats.v041,
        baseline: stats.baseline,
        tie: stats.tie,
        total: stats.total,
      })}
    </span>
  );
}
