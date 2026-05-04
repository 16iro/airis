// Quiz slideup 콘텐츠 — 회상 챌린지 시작 위치 (PR 34, D-070).
//
// prototype: 알림 박스 + "챌린지 시작" 버튼 + 최근 결과 list.
// 우리 v0.3은 데이터 인프라 (최근 회상 결과 누적) v0.4 이후. 안내 + 시작 버튼만.

import { Brain } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useUiStore } from "@/store/uiStore";

export function QuizContent() {
  const { t } = useTranslation();
  const setRecallOpen = useUiStore((s) => s.setRecallOpen);

  return (
    <div className="space-y-4">
      <h3 className="text-base font-semibold">{t("recall.title")}</h3>
      <div className="flex gap-3 rounded-md border border-border bg-primary-soft p-3">
        <Brain className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
        <p className="text-xs leading-relaxed">{t("recall.subtitle")}</p>
      </div>
      <Button onClick={() => setRecallOpen(true)}>
        {t("recall.start_button")}
      </Button>
    </div>
  );
}
