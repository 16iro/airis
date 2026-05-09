// F8 SRS 복습 슬라이드업 패널.
//
// UI 흐름:
//   1) 활성 스터디의 due 카드 fetch.
//   2) 한 카드씩 — 앞면 표시, "뒷면 보기" 클릭 시 CSS flip 애니메이션 (transform rotateY).
//   3) 평가 4단계 (again/hard/good/easy → SM-2 quality 0/3/4/5).
//   4) 다음 카드 또는 "오늘 복습 끝" 안내.
//   5) 카드 추가 폼 (수동) — front/back/section_ref.
// v0.5 PR 2 (D-099/D-103): on-demand 카드 생성 버튼 + LLM 토글 + progress bar + onboarding.

import { BookOpen, Loader2, Plus, Sparkles, X, Zap } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import {
  appErrorMessage,
  isAppError,
  type SrsCard,
  type SrsGenerateDone,
  type SrsGenerateProgress,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import { useActiveBookStore } from "@/store/activeBookStore";
import { useStudyStore } from "@/store/studyStore";

const ONBOARDING_KEY = "airis_srs_onboarding_seen";

interface Props {
  onClose: () => void;
}

const QUALITY_BUTTONS: Array<{ key: "again" | "hard" | "good" | "easy"; quality: number }> = [
  { key: "again", quality: 0 },
  { key: "hard", quality: 3 },
  { key: "good", quality: 4 },
  { key: "easy", quality: 5 },
];

export function SrsPanel({ onClose }: Props) {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const activeBook = useActiveBookStore((s) => s.content);

  const [queue, setQueue] = useState<SrsCard[]>([]);
  const [flipped, setFlipped] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  // v0.5 PR 2 — generation state.
  const [llmEnabled, setLlmEnabled] = useState(true);
  const [generating, setGenerating] = useState(false);
  const [genProgress, setGenProgress] = useState<SrsGenerateProgress | null>(null);
  const [genDoneMsg, setGenDoneMsg] = useState<string | null>(null);
  // onboarding: lazy initializer — localStorage read는 render 시점에 1회만.
  const [showOnboarding, setShowOnboarding] = useState(() => {
    try {
      return !localStorage.getItem(ONBOARDING_KEY);
    } catch {
      return false;
    }
  });

  // Tauri 이벤트 리스너 — srs:generate:progress / srs:generate:done.
  // activeStudy를 ref로 캡처해 done 콜백에서 최신값 참조 (deps 순환 방지).
  const activeStudyRef = useRef(activeStudy);
  useEffect(() => { activeStudyRef.current = activeStudy; }, [activeStudy]);

  const unlistenRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const u1 = await listen<SrsGenerateProgress>("srs:generate:progress", (ev) => {
          if (!cancelled) setGenProgress(ev.payload);
        });
        const u2 = await listen<SrsGenerateDone>("srs:generate:done", (ev) => {
          if (!cancelled) {
            const { total_inserted, total_skipped } = ev.payload;
            setGenProgress(null);
            setGenerating(false);
            if (total_inserted === 0) {
              setGenDoneMsg(t("srs.generate.done_toast_zero"));
            } else {
              setGenDoneMsg(
                t("srs.generate.done_toast", {
                  inserted: total_inserted,
                  skipped: total_skipped,
                }),
              );
            }
            // 완료 후 큐 갱신 — ref로 최신 activeStudy 참조.
            const study = activeStudyRef.current;
            if (study) {
              void api.srsListDue(study.slug).then(setQueue).catch(() => null);
            }
          }
        });
        unlistenRef.current = () => {
          u1();
          u2();
        };
      } catch {
        // tauri events not available in test env.
      }
    })();
    return () => {
      cancelled = true;
      unlistenRef.current?.();
    };
  }, [t]);

  // 첫 로드.
  useEffect(() => {
    if (!activeStudy) return;
    let cancelled = false;
    void (async () => {
      try {
        const list = await api.srsListDue(activeStudy.slug);
        if (!cancelled) setQueue(list);
      } catch (e) {
        if (!cancelled) setError(isAppError(e) ? appErrorMessage(e) : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [activeStudy]);

  const current = queue[0] ?? null;

  async function review(quality: number) {
    if (!current) return;
    setBusy(true);
    try {
      await api.srsReviewCard(current.id, quality);
      setQueue((q) => q.slice(1));
      setFlipped(false);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function refreshQueue() {
    if (!activeStudy) return;
    const list = await api.srsListDue(activeStudy.slug).catch(() => []);
    setQueue(list);
  }

  async function handleGenerateBook() {
    if (!activeStudy || !activeBook || generating) return;
    setGenerating(true);
    setGenDoneMsg(null);
    setError(null);
    try {
      await api.srsGenerateBook(activeStudy.slug, activeBook.book_id, llmEnabled);
      // done 이벤트가 리스너에서 처리하므로 여기선 추가 처리 불필요.
    } catch (e) {
      setGenerating(false);
      setGenProgress(null);
      const msg = isAppError(e) ? appErrorMessage(e) : String(e);
      setError(t("srs.generate.error_toast", { msg }));
    }
  }

  async function handleGenerateWeak() {
    if (!activeStudy || generating) return;
    setGenerating(true);
    setGenDoneMsg(null);
    setError(null);
    try {
      await api.srsGenerateWeakPriority(activeStudy.slug, 30, llmEnabled);
    } catch (e) {
      setGenerating(false);
      setGenProgress(null);
      const msg = isAppError(e) ? appErrorMessage(e) : String(e);
      setError(t("srs.generate.error_toast", { msg }));
    }
  }

  if (!activeStudy) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="srs-title"
      className="fixed inset-0 z-50 flex items-end justify-center bg-black/40"
      onClick={onClose}
    >
      <Card
        className="w-full max-w-2xl rounded-b-none"
        onClick={(e) => e.stopPropagation()}
      >
        <CardHeader>
          <div className="flex items-start justify-between gap-2">
            <div>
              <CardTitle id="srs-title">{t("srs.title")}</CardTitle>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("srs.subtitle")}
              </p>
            </div>
            <div className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2"
                onClick={() => setAdding(true)}
              >
                <Plus size={14} />
                {t("srs.add_card")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2"
                onClick={onClose}
              >
                <X size={14} />
              </Button>
            </div>
          </div>

          {/* v0.5 PR 2 — 카드 생성 버튼 영역 */}
          <div className="mt-3 rounded-md border border-border bg-muted/30 p-3 space-y-2">
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <span className="text-xs font-medium text-muted-foreground flex items-center gap-1">
                <BookOpen size={12} />
                {t("srs.generate.book_btn")}
              </span>
              <div className="flex items-center gap-1 flex-wrap">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 px-2 text-xs"
                  disabled={generating || !activeBook}
                  onClick={() => void handleGenerateBook()}
                >
                  {generating && genProgress ? (
                    <>
                      <Loader2 size={12} className="animate-spin" />
                      {t("srs.generate.in_progress", {
                        current: genProgress.current,
                        total: genProgress.total,
                      })}
                    </>
                  ) : (
                    <>
                      <Sparkles size={12} />
                      {t("srs.generate.book_btn")}
                    </>
                  )}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 px-2 text-xs"
                  disabled={generating}
                  onClick={() => void handleGenerateWeak()}
                >
                  <Zap size={12} />
                  {t("srs.generate.weak_btn")}
                </Button>
              </div>
            </div>

            {/* LLM 토글 */}
            <div className="flex items-center gap-2">
              <input
                type="checkbox"
                id="srs-llm-toggle"
                checked={llmEnabled}
                onChange={(e) => setLlmEnabled(e.target.checked)}
                className="h-4 w-4 cursor-pointer accent-primary"
                aria-label={t("srs.generate.llm_toggle")}
              />
              <Label htmlFor="srs-llm-toggle" className="text-xs cursor-pointer select-none">
                {t("srs.generate.llm_toggle")}
              </Label>
              {llmEnabled && (
                <span className="text-[11px] text-muted-foreground">
                  · {t("srs.generate.llm_cost_hint")}
                </span>
              )}
            </div>

            {/* 진행 중 progress bar */}
            {generating && genProgress && (
              <div className="space-y-1">
                <div className="h-1.5 w-full rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full bg-primary rounded-full transition-all"
                    style={{
                      width: `${Math.round((genProgress.current / genProgress.total) * 100)}%`,
                    }}
                  />
                </div>
                <p className="text-[11px] text-muted-foreground">
                  {t("srs.generate.in_progress", {
                    current: genProgress.current,
                    total: genProgress.total,
                  })}
                </p>
              </div>
            )}

            {/* done 토스트 */}
            {genDoneMsg && !generating && (
              <p className="text-xs text-primary font-medium">{genDoneMsg}</p>
            )}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {!current ? (
            <p className="py-12 text-center text-sm text-muted-foreground">
              {t("srs.no_due")}
            </p>
          ) : (
            <>
              <p className="text-right text-[11px] text-muted-foreground">
                {t("srs.remaining", { n: queue.length })}
              </p>
              <CardFlip card={current} flipped={flipped} />
              {!flipped ? (
                <Button
                  className="w-full"
                  onClick={() => setFlipped(true)}
                >
                  {t("srs.show_back")}
                </Button>
              ) : (
                <div className="grid grid-cols-4 gap-2">
                  {QUALITY_BUTTONS.map((b) => (
                    <Button
                      key={b.key}
                      variant={b.key === "again" ? "destructive" : "outline"}
                      size="sm"
                      onClick={() => void review(b.quality)}
                      disabled={busy}
                    >
                      {t(`srs.${b.key}`)}
                    </Button>
                  ))}
                </div>
              )}
            </>
          )}

          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </CardContent>
      </Card>

      {adding ? (
        <AddCardDialog
          studySlug={activeStudy.slug}
          onClose={(saved) => {
            setAdding(false);
            if (saved) void refreshQueue();
          }}
        />
      ) : null}

      {showOnboarding ? (
        <OnboardingDialog
          onClose={() => {
            try {
              localStorage.setItem(ONBOARDING_KEY, "1");
            } catch {
              // ignore in test/SSR environments.
            }
            setShowOnboarding(false);
          }}
        />
      ) : null}
    </div>
  );
}

function CardFlip({ card, flipped }: { card: SrsCard; flipped: boolean }) {
  return (
    <div
      className="relative h-48 w-full"
      style={{ perspective: "1000px" }}
    >
      <div
        className={cn(
          "relative h-full w-full transition-transform duration-500",
        )}
        style={{
          transformStyle: "preserve-3d",
          transform: flipped ? "rotateY(180deg)" : "rotateY(0deg)",
        }}
      >
        <div
          className="absolute inset-0 flex items-center justify-center rounded-md border border-border bg-card p-6 text-center text-base"
          style={{ backfaceVisibility: "hidden" }}
        >
          {card.front}
        </div>
        <div
          className="absolute inset-0 flex items-center justify-center rounded-md border border-primary bg-primary/5 p-6 text-center text-base"
          style={{
            backfaceVisibility: "hidden",
            transform: "rotateY(180deg)",
          }}
        >
          {card.back}
        </div>
      </div>
    </div>
  );
}

function AddCardDialog({
  studySlug,
  onClose,
}: {
  studySlug: string;
  onClose: (saved: boolean) => void;
}) {
  const { t } = useTranslation();
  const [front, setFront] = useState("");
  const [back, setBack] = useState("");
  const [sectionRef, setSectionRef] = useState("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function save() {
    setSaving(true);
    setErr(null);
    try {
      await api.srsAddCard(studySlug, {
        front,
        back,
        section_ref: sectionRef.trim() || null,
        page_ref: null,
      });
      onClose(true);
    } catch (e) {
      setErr(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50"
      onClick={() => onClose(false)}
    >
      <Card className="w-full max-w-md" onClick={(e) => e.stopPropagation()}>
        <CardHeader>
          <CardTitle>{t("srs.add_card")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-1">
            <Label htmlFor="srs-front">{t("srs.front_label")}</Label>
            <Textarea
              id="srs-front"
              value={front}
              onChange={(e) => setFront(e.target.value)}
              rows={3}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="srs-back">{t("srs.back_label")}</Label>
            <Textarea
              id="srs-back"
              value={back}
              onChange={(e) => setBack(e.target.value)}
              rows={3}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="srs-section">{t("srs.section_ref_optional")}</Label>
            <Input
              id="srs-section"
              value={sectionRef}
              onChange={(e) => setSectionRef(e.target.value)}
              placeholder="Ch04 §State"
            />
          </div>
          {err ? (
            <p className="text-sm text-destructive" role="alert">
              {err}
            </p>
          ) : null}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="outline" onClick={() => onClose(false)} disabled={saving}>
              {t("srs.cancel")}
            </Button>
            <Button
              onClick={() => void save()}
              disabled={saving || !front.trim() || !back.trim()}
            >
              {t("srs.save")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

/** 최초 진입 시 1회 노출되는 카드 생성 안내 다이얼로그. */
function OnboardingDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/50"
      onClick={onClose}
    >
      <Card className="w-full max-w-sm" onClick={(e) => e.stopPropagation()}>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Sparkles size={18} className="text-primary" />
            <CardTitle className="text-base">{t("srs.generate.onboarding_title")}</CardTitle>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">
            {t("srs.generate.onboarding_body")}
          </p>
          <div className="flex justify-end">
            <Button onClick={onClose}>{t("srs.generate.onboarding_ok")}</Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
