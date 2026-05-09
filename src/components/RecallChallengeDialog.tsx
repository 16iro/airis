// RecallChallengeDialog — v0.5 PR 4 (D-101).
//
// 회상 챌린지 Level 1: weak(cloze)/medium(4지선다)/strong(30초 제한) 모드.
// 모달로 열리며 Esc/Enter 키보드 지원, 30초 카운트다운(strong), 결과 표시.
//
// BUG-002 패턴(D-092): 외부에서 열어 둔 challenge를 unmount 전에 답변 없이 닫으면
// outcome=dismissed 로 자동 기록. cancelled flag + cleanup 일관 적용.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import type { RecallChallenge, RecallOutcome } from "@/lib/types";

const STRONG_COUNTDOWN_SEC = 30;

export interface RecallChallengeDialogProps {
  studySlug: string;
  challenge: RecallChallenge;
  onClose: (outcome: RecallOutcome) => void;
}

export function RecallChallengeDialog({
  studySlug,
  challenge,
  onClose,
}: RecallChallengeDialogProps) {
  const { t } = useTranslation();

  const [answer, setAnswer] = useState("");
  const [selectedMc4, setSelectedMc4] = useState<number | null>(null);
  const [result, setResult] = useState<"correct" | "incorrect" | null>(null);
  const [timeLeft, setTimeLeft] = useState(
    challenge.strength === "strong" ? STRONG_COUNTDOWN_SEC : null,
  );
  // dismissed 여부 추적 — unmount cleanup에서 중복 기록 방지.
  const resolvedRef = useRef(false);
  // unmount cleanup에서 최신 props를 읽기 위한 ref.
  const propsRef = useRef({ studySlug, challenge });
  useEffect(() => {
    propsRef.current = { studySlug, challenge };
  });

  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  // 마운트 시 input 포커스.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // strong 모드: 30초 카운트다운.
  useEffect(() => {
    if (challenge.strength !== "strong" || result !== null) return;
    if (timeLeft === null || timeLeft <= 0) {
      // 시간 초과 처리.
      if (!resolvedRef.current) {
        resolvedRef.current = true;
        void api
          .recallRecordAttempt(
            studySlug,
            challenge.chunk_id,
            challenge.trigger_id,
            challenge.strength,
            "timeout",
          )
          .catch(() => {/* non-fatal */});
        onClose("timeout");
      }
      return;
    }
    const id = setTimeout(() => setTimeLeft((t) => (t !== null ? t - 1 : null)), 1000);
    return () => clearTimeout(id);
  }, [challenge, timeLeft, result, onClose, studySlug]);

  // unmount: 아직 미결이면 dismissed 기록.
  // propsRef를 통해 최신 props 읽기 — 의존성 배열 빈 배열 유지 (unmount-only cleanup).
  useEffect(() => {
    return () => {
      if (!resolvedRef.current) {
        resolvedRef.current = true;
        const { studySlug: slug, challenge: ch } = propsRef.current;
        void api
          .recallRecordAttempt(slug, ch.chunk_id, ch.trigger_id, ch.strength, "dismissed")
          .catch(() => {/* non-fatal */});
      }
    };
  }, []);

  // Esc 키 → dismissed.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        handleDismiss();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function handleDismiss() {
    if (resolvedRef.current) return;
    resolvedRef.current = true;
    void api
      .recallRecordAttempt(
        studySlug,
        challenge.chunk_id,
        challenge.trigger_id,
        challenge.strength,
        "dismissed",
      )
      .catch(() => {/* non-fatal */});
    onClose("dismissed");
  }

  async function handleSubmit() {
    if (resolvedRef.current || result !== null) return;

    // 정답 판정: mc4는 선택지 인덱스, cloze는 텍스트 포함 판정.
    let isCorrect = false;
    if (challenge.strength === "medium" && challenge.mc4_options !== null) {
      // mc4: 선택된 옵션 텍스트가 answer 문자열과 일치 여부.
      if (selectedMc4 !== null) {
        const chosen = challenge.mc4_options[selectedMc4] ?? "";
        isCorrect =
          chosen.trim().toLowerCase() ===
          challenge.answer.trim().toLowerCase();
      }
    } else {
      // cloze: 사용자 입력에 정답 키워드 포함.
      isCorrect =
        answer.trim().toLowerCase().includes(challenge.answer.trim().toLowerCase()) &&
        answer.trim().length > 0;
    }

    const outcome: RecallOutcome = isCorrect ? "correct" : "incorrect";
    setResult(isCorrect ? "correct" : "incorrect");
    resolvedRef.current = true;

    try {
      await api.recallRecordAttempt(
        studySlug,
        challenge.chunk_id,
        challenge.trigger_id,
        challenge.strength,
        outcome,
      );
    } catch {
      // non-fatal — 결과는 이미 UI에 반영됨.
    }

    // 1.2초 후 자동 닫기.
    setTimeout(() => onClose(outcome), 1200);
  }

  const title = t(`recall.dialog.title.${challenge.strength}`);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={title}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={handleDismiss}
    >
      <div
        className="w-full max-w-md rounded-xl border border-border bg-background p-5 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 헤더 */}
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-sm font-semibold">{title}</h2>
          <div className="flex items-center gap-2">
            {challenge.strength === "strong" && timeLeft !== null && result === null ? (
              <span
                className={
                  "min-w-[2rem] text-center text-sm font-mono font-bold " +
                  (timeLeft <= 10 ? "text-destructive" : "text-muted-foreground")
                }
              >
                {timeLeft}s
              </span>
            ) : null}
            <button
              type="button"
              aria-label={t("common.close")}
              onClick={handleDismiss}
              className="rounded p-1 text-muted-foreground hover:text-foreground"
            >
              ✕
            </button>
          </div>
        </div>

        {/* cloze 텍스트 */}
        <p className="mb-4 rounded-md bg-muted/40 px-3 py-2 text-sm leading-relaxed">
          {challenge.masked_text}
        </p>

        {/* 입력 영역 */}
        {result === null ? (
          <>
            {challenge.strength === "medium" && challenge.mc4_options !== null ? (
              /* mc4 선택지 */
              <div className="mb-4 space-y-2">
                {challenge.mc4_options.map((opt, idx) => (
                  <button
                    key={idx}
                    type="button"
                    onClick={() => setSelectedMc4(idx)}
                    className={
                      "w-full rounded-lg border px-3 py-2 text-left text-sm transition-colors " +
                      (selectedMc4 === idx
                        ? "border-primary bg-primary/10 font-medium"
                        : "border-border hover:bg-muted/50")
                    }
                  >
                    {opt}
                  </button>
                ))}
              </div>
            ) : (
              /* cloze 텍스트 입력 */
              <textarea
                ref={inputRef}
                value={answer}
                onChange={(e) => setAnswer(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void handleSubmit();
                  }
                }}
                placeholder={t("recall.dialog.input_placeholder")}
                rows={2}
                className="mb-4 w-full resize-none rounded-lg border border-border bg-background px-3 py-2 text-sm outline-none focus:ring-2 focus:ring-primary/50"
              />
            )}

            <div className="flex justify-end gap-2">
              <Button
                variant="ghost"
                size="sm"
                onClick={handleDismiss}
              >
                {t("recall.dialog.dismiss")}
              </Button>
              <Button
                size="sm"
                disabled={
                  challenge.strength === "medium"
                    ? selectedMc4 === null
                    : answer.trim().length === 0
                }
                onClick={() => void handleSubmit()}
              >
                {t("recall.dialog.submit")}
              </Button>
            </div>
          </>
        ) : (
          /* 결과 표시 */
          <div
            className={
              "rounded-lg px-4 py-3 text-center text-sm font-semibold " +
              (result === "correct"
                ? "bg-green-500/15 text-green-700 dark:text-green-400"
                : "bg-destructive/10 text-destructive")
            }
          >
            {result === "correct"
              ? t("recall.dialog.correct")
              : t("recall.dialog.incorrect")}
          </div>
        )}
      </div>
    </div>
  );
}
