// 새 스터디 마법사 — 한 화면 + step indicator (옵션 A, v0.2_HANDOFF.md 결정).
//
// PR 9 시점엔 *2단계*만:
//   1) 이름·슬러그
//   2) 학습 목표 (옵션) — Overview.md frontmatter에 stated_goal_chapter·deadline 박힘
//
// PR 10에서 책 등록, PR 11에서 인덱싱이 단계로 추가될 예정.
// 단계 정의·검증·API 호출을 셸과 분리해 PR 21 framer-motion 도입 후 슬라이드(B)로 셸 교체 가능.

import { useState } from "react";
import { useTranslation } from "react-i18next";

import { StepIndicator } from "@/components/StepIndicator";
import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

const SLUG_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const TOTAL_STEPS = 2;

export function NewStudyWizard() {
  const { t } = useTranslation();
  const setPage = useUiStore((s) => s.setPage);
  const create = useStudyStore((s) => s.create);

  const [step, setStep] = useState(1);
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [slugTouched, setSlugTouched] = useState(false);
  const [goalChapter, setGoalChapter] = useState("");
  const [deadline, setDeadline] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const effectiveSlug = slugTouched ? slug : autoSlug(name);
  const slugValid = SLUG_RE.test(effectiveSlug);

  function handleNameChange(value: string) {
    setName(value);
    setError(null);
  }

  function handleSlugChange(value: string) {
    setSlug(value);
    setSlugTouched(true);
    setError(null);
  }

  async function handleCreate() {
    setSubmitting(true);
    setError(null);
    try {
      const created = await create(effectiveSlug, name);
      // 사용자가 입력한 목표·마감을 Overview에 반영. 실패해도 스터디는 생성된 상태라 무시.
      if (goalChapter || deadline) {
        try {
          await api.studyOverviewWriteMeta(
            created.slug,
            goalChapter,
            deadline,
          );
        } catch (e) {
          console.warn("studyOverviewWriteMeta failed:", e);
        }
      }
      // 활성 스터디로 자동 전환되지 않은 경우 명시 전환 (이미 활성된 스터디가 있을 때).
      if (!created.is_active) {
        await useStudyStore.getState().select(created.slug);
      }
      setPage("workspace");
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setSubmitting(false);
    }
  }

  const canNextFromStep1 = name.trim().length > 0 && slugValid;

  return (
    <div className="flex h-full flex-col bg-background">
      <TopBar />
      <main className="mx-auto w-full max-w-xl flex-1 overflow-y-auto px-6 py-8">
        <div className="mb-6 flex items-center justify-between gap-4">
          <h1 className="text-2xl font-semibold tracking-tight">
            {t("wizard.title")}
          </h1>
          <span className="text-xs text-muted-foreground">
            {t("wizard.step", { current: step, total: TOTAL_STEPS })}
          </span>
        </div>

        <div className="mb-6">
          <StepIndicator
            current={step}
            total={TOTAL_STEPS}
            labels={[t("wizard.step1_title"), t("wizard.step2_title")]}
          />
        </div>

        <Card>
          <CardHeader>
            <CardTitle>
              {step === 1 ? t("wizard.step1_title") : t("wizard.step2_title")}
            </CardTitle>
            <p className="text-sm text-muted-foreground">
              {step === 1
                ? t("wizard.step1_subtitle")
                : t("wizard.step2_subtitle")}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            {step === 1 ? (
              <Step1
                name={name}
                slug={effectiveSlug}
                slugTouched={slugTouched}
                slugValid={slugValid}
                onNameChange={handleNameChange}
                onSlugChange={handleSlugChange}
              />
            ) : (
              <Step2
                goalChapter={goalChapter}
                deadline={deadline}
                onGoalChapterChange={setGoalChapter}
                onDeadlineChange={setDeadline}
              />
            )}

            {error ? (
              <p className="text-sm text-destructive" role="alert">
                {error}
              </p>
            ) : null}

            <div className="flex justify-between pt-2">
              <Button
                variant="outline"
                onClick={() =>
                  step === 1 ? setPage("library") : setStep(step - 1)
                }
                disabled={submitting}
              >
                {t("wizard.back")}
              </Button>
              {step < TOTAL_STEPS ? (
                <Button
                  onClick={() => setStep(step + 1)}
                  disabled={!canNextFromStep1}
                >
                  {t("wizard.next")}
                </Button>
              ) : (
                <Button
                  onClick={() => void handleCreate()}
                  disabled={submitting || !canNextFromStep1}
                >
                  {t("wizard.create")}
                </Button>
              )}
            </div>
          </CardContent>
        </Card>
      </main>
    </div>
  );
}

function Step1({
  name,
  slug,
  slugTouched,
  slugValid,
  onNameChange,
  onSlugChange,
}: {
  name: string;
  slug: string;
  slugTouched: boolean;
  slugValid: boolean;
  onNameChange: (v: string) => void;
  onSlugChange: (v: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="study-name">{t("wizard.step1_name_label")}</Label>
        <Input
          id="study-name"
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          placeholder={t("wizard.step1_name_placeholder")}
          autoFocus
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="study-slug">{t("wizard.step1_slug_label")}</Label>
        <Input
          id="study-slug"
          value={slug}
          onChange={(e) => onSlugChange(e.target.value)}
          placeholder={t("wizard.step1_slug_placeholder")}
          className="font-mono"
        />
        <p className="text-xs text-muted-foreground">
          {t("wizard.step1_slug_hint")}
        </p>
        {slugTouched && slug && !slugValid ? (
          <p className="text-xs text-destructive" role="alert">
            {t("wizard.step1_slug_invalid")}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function Step2({
  goalChapter,
  deadline,
  onGoalChapterChange,
  onDeadlineChange,
}: {
  goalChapter: string;
  deadline: string;
  onGoalChapterChange: (v: string) => void;
  onDeadlineChange: (v: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="study-goal">{t("wizard.step2_goal_label")}</Label>
        <Input
          id="study-goal"
          value={goalChapter}
          onChange={(e) => onGoalChapterChange(e.target.value)}
          placeholder={t("wizard.step2_goal_placeholder")}
        />
        <p className="text-xs text-muted-foreground">
          {t("wizard.step2_goal_hint")}
        </p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="study-deadline">{t("wizard.step2_deadline_label")}</Label>
        <Input
          id="study-deadline"
          type="date"
          value={deadline}
          onChange={(e) => onDeadlineChange(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          {t("wizard.step2_deadline_hint")}
        </p>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("wizard.step2_skip_note")}
      </p>
    </div>
  );
}

/**
 * 스터디 이름 → 슬러그 자동 추정.
 * 영문은 lower + 공백→하이픈, 한글·기타 문자는 사용자가 직접 입력하도록 빈값.
 */
function autoSlug(name: string): string {
  const trimmed = name.trim().toLowerCase();
  const ascii = trimmed.replace(/[^a-z0-9-\s]/g, "").replace(/\s+/g, "-");
  return ascii.slice(0, 64).replace(/^-+|-+$/g, "");
}
