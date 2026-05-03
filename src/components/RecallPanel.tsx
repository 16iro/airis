// F7.7 회상 챌린지 슬라이드업.
//
// 흐름:
//   1) 사용자가 챕터 ref(예: Ch04) + 핵심을 짧게 입력.
//   2) 백엔드가 paragraphs에서 그 챕터 본문 → 빈도 top-N 키워드 추출 → 사용자 입력과 비교.
//   3) 60% 이상 매치면 통과 + SRS 카드 자동 생성, 결과 화면 표시.

import { Loader2, X } from "lucide-react";
import { useState } from "react";
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
  type RecallResult,
} from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";

interface Props {
  onClose: () => void;
}

export function RecallPanel({ onClose }: Props) {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);

  const [chapterRef, setChapterRef] = useState("Ch01");
  const [input, setInput] = useState("");
  const [result, setResult] = useState<RecallResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function evaluate() {
    if (!activeStudy) return;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await api.recallEvaluate(activeStudy.slug, chapterRef, input);
      setResult(r);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!activeStudy) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="recall-title"
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
              <CardTitle id="recall-title">{t("recall.title")}</CardTitle>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("recall.subtitle")}
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2"
              onClick={onClose}
            >
              <X size={14} />
            </Button>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-1">
            <Label htmlFor="recall-chapter">{t("recall.chapter_label")}</Label>
            <Input
              id="recall-chapter"
              value={chapterRef}
              onChange={(e) => setChapterRef(e.target.value)}
              placeholder={t("recall.chapter_placeholder")}
              className="font-mono"
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="recall-input">{t("recall.input_label")}</Label>
            <Textarea
              id="recall-input"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              rows={6}
              disabled={busy}
            />
          </div>
          <div className="flex justify-end">
            <Button
              onClick={() => void evaluate()}
              disabled={busy || !input.trim() || !chapterRef.trim()}
            >
              {busy ? <Loader2 className="animate-spin" size={14} /> : null}
              {busy ? t("recall.evaluating") : t("recall.submit")}
            </Button>
          </div>

          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}

          {result ? <ResultBlock result={result} /> : null}
        </CardContent>
      </Card>
    </div>
  );
}

function ResultBlock({ result }: { result: RecallResult }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2 rounded-md border border-border bg-card p-3 text-xs">
      <p
        className={
          result.passed
            ? "font-medium text-emerald-600 dark:text-emerald-400"
            : "font-medium text-amber-600 dark:text-amber-400"
        }
        role="status"
      >
        {result.passed ? t("recall.passed") : t("recall.failed")}
      </p>
      <KeywordRow label={t("recall.expected_label")} items={result.keywords_expected} />
      <KeywordRow
        label={t("recall.present_label")}
        items={result.keywords_present}
        tone="positive"
      />
      <KeywordRow
        label={t("recall.missing_label")}
        items={result.keywords_missing}
        tone="negative"
      />
    </div>
  );
}

function KeywordRow({
  label,
  items,
  tone,
}: {
  label: string;
  items: string[];
  tone?: "positive" | "negative";
}) {
  return (
    <div>
      <p className="mb-0.5 text-muted-foreground">{label}</p>
      <div className="flex flex-wrap gap-1">
        {items.map((kw) => (
          <span
            key={kw}
            className={
              "rounded px-1.5 py-0.5 text-[10px] font-medium " +
              (tone === "positive"
                ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                : tone === "negative"
                  ? "bg-amber-500/15 text-amber-700 dark:text-amber-300"
                  : "bg-muted text-muted-foreground")
            }
          >
            {kw}
          </span>
        ))}
      </div>
    </div>
  );
}
