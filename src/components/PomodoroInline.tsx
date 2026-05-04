// TopBar 인라인 Pomodoro 카운터 — prototype 100% 충실 (PR 34, D-070).
//
// idle: Timer 아이콘만. 클릭 시 25분 focus 시작.
// running: 아이콘 + MM:SS 카운터 (mono). 클릭 시 정지(완료 처리 안 함).
// 자동 만료(remaining=0): 백엔드에 stopPomodoro(true) 호출. 알림은 콘솔.
//
// 1초 polling. 활성 스터디 없으면 disabled.

import { Pause, Timer } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { type PomodoroState } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";

function formatMmSs(totalSec: number): string {
  const m = Math.max(0, Math.floor(totalSec / 60));
  const s = Math.max(0, totalSec % 60);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function PomodoroInline() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const [state, setState] = useState<PomodoroState | null>(null);
  const [busy, setBusy] = useState(false);
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

  async function toggle() {
    if (busy) return;
    setBusy(true);
    try {
      if (state?.running) {
        await api.stopPomodoro(false, null);
      } else if (activeStudy) {
        await api.startPomodoro(activeStudy.slug, true, null);
      }
      const s = await api.getPomodoroState();
      setState(s);
    } catch (e) {
      console.warn("pomodoro toggle failed:", e);
    } finally {
      setBusy(false);
    }
  }

  const running = state?.running ?? false;
  const remaining = state?.remaining_sec ?? 0;

  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => void toggle()}
      disabled={!activeStudy && !running}
      aria-label={t("pomodoro.topbar_tooltip")}
      title={running ? t("pomodoro.stop") : t("pomodoro.start_focus")}
      className="gap-1.5"
    >
      {running ? (
        <Pause size={12} className="text-primary" />
      ) : (
        <Timer size={14} />
      )}
      {running ? (
        <span className="font-mono text-[11px] tabular-nums">
          {formatMmSs(remaining)}
        </span>
      ) : null}
    </Button>
  );
}
