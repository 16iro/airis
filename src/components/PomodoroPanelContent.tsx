// Pomodoro 패널 콘텐츠 — dockview 패널로 통합 (PR 43).
//
// 1초 polling으로 잔여 시간 표시. 시작/정지 + interruption 사유 입력.
// 자동 만료(remaining=0) 시 백엔드 stopPomodoro(true) 호출.

import { Loader2, Pause, Play } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { type PomodoroState } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";

function formatMmSs(totalSec: number): string {
  const m = Math.max(0, Math.floor(totalSec / 60));
  const s = Math.max(0, totalSec % 60);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function PomodoroPanelContent() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const [state, setState] = useState<PomodoroState | null>(null);
  const [busy, setBusy] = useState(false);
  const [interruption, setInterruption] = useState("");
  const expiredHandledRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const s = await api.getPomodoroState();
        if (cancelled) return;
        setState(s);
        if (
          s.running &&
          s.remaining_sec === 0 &&
          s.session &&
          !expiredHandledRef.current
        ) {
          expiredHandledRef.current = true;
          await api.stopPomodoro(true, null).catch(() => {});
        }
        if (!s.running) {
          expiredHandledRef.current = false;
        }
      } catch (e) {
        if (!cancelled) console.warn("pomodoro poll failed:", e);
      }
    }
    void poll();
    const id = setInterval(() => void poll(), 1000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  async function startFocus() {
    if (busy || !activeStudy) return;
    setBusy(true);
    try {
      await api.startPomodoro(activeStudy.slug, true, null);
      const s = await api.getPomodoroState();
      setState(s);
    } catch (e) {
      console.warn("startPomodoro failed:", e);
    } finally {
      setBusy(false);
    }
  }

  async function startBreak() {
    if (busy || !activeStudy) return;
    setBusy(true);
    try {
      await api.startPomodoro(activeStudy.slug, false, null);
      const s = await api.getPomodoroState();
      setState(s);
    } catch (e) {
      console.warn("startPomodoro break failed:", e);
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    if (busy) return;
    setBusy(true);
    try {
      const note = interruption.trim() ? interruption.trim() : null;
      await api.stopPomodoro(false, note);
      const s = await api.getPomodoroState();
      setState(s);
      setInterruption("");
    } catch (e) {
      console.warn("stopPomodoro failed:", e);
    } finally {
      setBusy(false);
    }
  }

  const running = state?.running ?? false;
  const remaining = state?.remaining_sec ?? 0;
  const phase = state?.session?.phase;

  if (!activeStudy) {
    return (
      <p className="p-4 text-xs text-muted-foreground">
        {t("pomodoro.no_active_study")}
      </p>
    );
  }

  return (
    <div className="flex h-full flex-col gap-4 p-4">
      <div className="flex flex-col items-center gap-1 rounded-md border border-border bg-muted/30 py-6">
        <span className="text-[11px] uppercase tracking-wider text-muted-foreground">
          {running
            ? phase === "focus"
              ? t("pomodoro.phase_focus")
              : t("pomodoro.phase_break")
            : t("pomodoro.idle")}
        </span>
        <span className="font-mono text-4xl font-semibold tabular-nums">
          {running ? formatMmSs(remaining) : "—"}
        </span>
      </div>

      {running ? (
        <div className="space-y-2">
          <Label htmlFor="pomo-interrupt" className="text-xs text-muted-foreground">
            {t("pomodoro.interruption_label")}
          </Label>
          <Input
            id="pomo-interrupt"
            value={interruption}
            onChange={(e) => setInterruption(e.target.value)}
            placeholder={t("pomodoro.interruption_placeholder")}
            disabled={busy}
          />
          <Button
            variant="outline"
            onClick={() => void stop()}
            disabled={busy}
            className="w-full"
          >
            {busy ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : <Pause className="mr-1 h-4 w-4" />}
            {t("pomodoro.stop")}
          </Button>
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-2">
          <Button
            onClick={() => void startFocus()}
            disabled={busy}
          >
            {busy ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : <Play className="mr-1 h-4 w-4" />}
            {t("pomodoro.start_focus")}
          </Button>
          <Button
            variant="outline"
            onClick={() => void startBreak()}
            disabled={busy}
          >
            {t("pomodoro.start_break")}
          </Button>
        </div>
      )}
    </div>
  );
}
