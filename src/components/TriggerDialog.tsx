// 발화 트리거 감지 후 *Memory에 추가할지* 사용자에게 1회 확인.
//
// 표시:
//   * 매치된 발화 (어디에서 잡혔는지)
//   * 추가될 항목 (suggested_entry — `(active, since 시각)` prefix는 백엔드가 박음)
//   * 추가 / 건너뛰기 두 버튼

import { Loader2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError, type TriggerHit } from "@/lib/types";

interface Props {
  studySlug: string;
  hit: TriggerHit;
  onClose: (applied: boolean) => void;
}

export function TriggerDialog({ studySlug, hit, onClose }: Props) {
  const { t } = useTranslation();
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleConfirm() {
    setSubmitting(true);
    setError(null);
    try {
      await api.memoryApplyTrigger(studySlug, hit);
      onClose(true);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setSubmitting(false);
    }
  }

  const kindLabel = t(`trigger.kind_${hit.kind}`);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="trigger-dialog-title"
      className="fixed inset-0 z-50 flex items-end justify-end bg-transparent p-6"
    >
      <Card className="w-full max-w-sm shadow-lg">
        <CardHeader>
          <CardTitle id="trigger-dialog-title" className="text-base">
            {t("trigger.title")}
          </CardTitle>
          <p className="mt-1 text-xs text-muted-foreground">{kindLabel}</p>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-1">
            <p className="text-xs font-medium text-muted-foreground">
              {t("trigger.matched_label")}
            </p>
            <p className="line-clamp-2 rounded-md bg-muted/50 px-2 py-1 text-xs">
              {hit.matched_text}
            </p>
          </div>
          <div className="space-y-1">
            <p className="text-xs font-medium text-muted-foreground">
              {t("trigger.suggested_label")}
            </p>
            <p className="rounded-md bg-primary/10 px-2 py-1 text-xs">
              {hit.suggested_entry}
            </p>
          </div>

          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}

          <div className="flex justify-end gap-2 pt-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => onClose(false)}
              disabled={submitting}
            >
              {t("trigger.skip")}
            </Button>
            <Button
              size="sm"
              onClick={() => void handleConfirm()}
              disabled={submitting}
            >
              {submitting ? <Loader2 className="animate-spin" size={14} /> : null}
              {submitting ? t("trigger.saving") : t("trigger.confirm")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
