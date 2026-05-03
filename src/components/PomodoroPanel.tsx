// F9 Pomodoro 미니 패널 — 1초 polling으로 잔여 시간 표시.
//
// 동작:
//   * 활성 스터디 있을 때만 시작/정지 가능.
//   * 시작 — 25분 집중 또는 5분 휴식.
//   * 정지 — 사용자 명시 or 자동 만료(잔여=0). pomodoro_cycles에 row INSERT.
//   * 자동 만료 시 인앱 토스트 (alert 대체용 — OS 네이티브는 v0.3+).

import { Pause, Play, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { api } from "@/lib/api";
import {
  appErrorMessage,
  isAppError,
  type PomodoroPhase,
  type PomodoroState,
} from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";

interface Props {
  onClose: () => void;
}

export function PomodoroPanel({ onClose }: Props) {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const [state, setState] = useState<PomodoroState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 자동 만료 토스트를 *한 번만* 띄우기 위한 가드.
  const expiredHandledRef = useRef(false);

  // 1초 polling — 잔여 시간 갱신.
  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const s = await api.getPomodoroState();
        if (!cancelled) {
          setState(s);
          // 자동 만료 감지: running + remaining=0.
          if (
            s.running &&
            s.remaining_sec === 0 &&
            s.session &&
            !expiredHandledRef.current
          ) {
            expiredHandledRef.current = true;
            await api.stopPomodoro(true, null).catch(() => {});
            const phaseLabel =
              s.session.phase === "focus"
                ? t("pomodoro.phase_focus")
                : t("pomodoro.phase_break");
            console.info(
              t("pomodoro.completed_toast", { phase: phaseLabel }),
            );
          }
          if (!s.running) {
            expiredHandledRef.current = false;
          }
        }
      } catch (e) {
        if (!cancelled) {
          console.error("pomodoro poll failed:", e);
        }
      }
    }
    void poll();
    const id = setInterval(() => void poll(), 1000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [t]);

  async function startPhase(focus: boolean) {
    if (!activeStudy) return;
    setBusy(true);
    setError(null);
    try {
      await api.startPomodoro(activeStudy.slug, focus, null);
      const s = await api.getPomodoroState();
      setState(s);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function stop(completed: boolean) {
    setBusy(true);
    setError(null);
    try {
      await api.stopPomodoro(completed, null);
      const s = await api.getPomodoroState();
      setState(s);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  const running = state?.running ?? false;
  const remaining = state?.remaining_sec ?? 0;
  const phase: PomodoroPhase | null = state?.session?.phase ?? null;

  return (
    <div
      role="dialog"
      aria-modal="false"
      aria-labelledby="pomodoro-title"
      className="fixed bottom-6 right-6 z-40"
    >
      <Card className="w-72 shadow-lg">
        <CardHeader>
          <div className="flex items-start justify-between gap-2">
            <CardTitle id="pomodoro-title" className="text-base">
              {t("pomodoro.title")}
            </CardTitle>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2"
              onClick={onClose}
              aria-label={t("pomodoro.title")}
            >
              <X size={14} />
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          {!activeStudy ? (
            <p className="text-xs text-muted-foreground">
              {t("pomodoro.no_active_study")}
            </p>
          ) : running ? (
            <>
              <div className="text-center">
                <p className="text-xs text-muted-foreground">
                  {phase === "focus"
                    ? t("pomodoro.phase_focus")
                    : t("pomodoro.phase_break")}
                </p>
                <p className="font-mono text-3xl font-semibold tabular-nums">
                  {formatRemaining(remaining)}
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                className="w-full"
                onClick={() => void stop(false)}
                disabled={busy}
              >
                <Pause size={14} />
                {t("pomodoro.stop")}
              </Button>
            </>
          ) : (
            <>
              <p className="text-xs text-muted-foreground">
                {t("pomodoro.idle")}
              </p>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  className="flex-1"
                  onClick={() => void startPhase(true)}
                  disabled={busy}
                >
                  <Play size={14} />
                  {t("pomodoro.start_focus")}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  className="flex-1"
                  onClick={() => void startPhase(false)}
                  disabled={busy}
                >
                  {t("pomodoro.start_break")}
                </Button>
              </div>
            </>
          )}

          {error ? (
            <p className="text-xs text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}

function formatRemaining(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}
