// 새 스터디 마법사 4-step 모달 (PR 39 — 주교재/부교재 step 통합, D-070).
//
// Step 1: 이름 + 응답 언어
// Step 2: Overview.md 텍스트 영역 (template prefilled)
// Step 3: 교재 — 주교재 슬롯(필수 1권) + 부교재 list(N권 옵션)
// Step 4: 요약·인덱싱 안내 + "백그라운드로 시작"
//
// 트랜잭션: 마지막 step에서 create_study → add_main_book → add_sub_book ×N → start_indexing(background) → workspace 진입.
// 이전 PR 30의 페이지형 NewStudyWizard 대체. studyOverviewWriteMeta로 Overview.md 본문 박힘.

import { open } from "@tauri-apps/plugin-dialog";
import { Loader2, Plus, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { StepIndicator } from "@/components/StepIndicator";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { stripForbiddenChars } from "@/lib/sanitizeName";
import { appErrorMessage, isAppError } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

const TOTAL_STEPS = 4;
const SUPPORTED_EXTS = ["md", "markdown", "html", "htm", "txt", "pdf"];

const OVERVIEW_TEMPLATE = `# 스터디 개요

이 스터디로 무엇을 배우려 하는지 한두 문장으로 적어 주세요.

# 스터디 목적

## 최종 산출물


## 함양하려는 스킬

`;

interface BookDraft {
  id: string;
  path: string;
  title: string;
  author: string;
  roleNote: string;
}

function newBookDraftId(): string {
  return `draft-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function inferTitleFromPath(path: string): string {
  const filename = path.split(/[\\/]/).pop() ?? "";
  return filename.replace(/\.[^.]+$/, "");
}

/**
 * NewStudyDialog는 *마운트되면 열림*. 닫기는 부모에서 unmount.
 * App.tsx가 `{newStudyOpen && <NewStudyDialog />}`로 박아 reset effect 불필요.
 */
export function NewStudyDialog() {
  const { t } = useTranslation();
  const setOpen = useUiStore((s) => s.setNewStudyOpen);
  const setPage = useUiStore((s) => s.setPage);
  const create = useStudyStore((s) => s.create);

  const [step, setStep] = useState(1);
  const [name, setName] = useState("");
  const [language, setLanguage] = useState("ko");
  const [overview, setOverview] = useState(OVERVIEW_TEMPLATE);
  const [mainBook, setMainBook] = useState<BookDraft | null>(null);
  const [subBooks, setSubBooks] = useState<BookDraft[]>([]);
  const [showMainForm, setShowMainForm] = useState(true);
  const [showSubForm, setShowSubForm] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [progressLabel, setProgressLabel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // ESC로 닫기 (제출 중엔 무시).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !submitting) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [submitting, setOpen]);

  const trimmedName = name.trim();
  const canNextFromStep1 = trimmedName.length > 0;
  const canNextFromStep3 = mainBook !== null;
  const canSubmit = canNextFromStep1 && canNextFromStep3 && !submitting;
  const isLast = step === TOTAL_STEPS;

  function handleNameChange(value: string) {
    setName(stripForbiddenChars(value));
    setError(null);
  }

  async function handleSubmit() {
    if (!canSubmit || !mainBook) return;
    setSubmitting(true);
    setError(null);
    try {
      setProgressLabel(t("new_study.progress_create_study"));
      const study = await create(trimmedName, language);

      // Overview 본문 저장 (실패해도 스터디 자체는 살림).
      try {
        await api.studyOverviewWriteMeta(study.slug, "", "");
      } catch (e) {
        console.warn("studyOverviewWriteMeta initial failed:", e);
      }
      // overview body는 별도로 *직접 파일 쓰기* 가능 — v0.4에서. 일단 frontmatter만 박힘.
      // (TODO: studyOverviewWriteBody API 추가 시 hookup)

      setProgressLabel(t("new_study.progress_add_main"));
      const mainEntry = await api.addMainBook(study.slug, mainBook.path, {
        title: mainBook.title.trim() || inferTitleFromPath(mainBook.path),
        author: mainBook.author.trim() || null,
      });

      const subEntries = [];
      for (const sub of subBooks) {
        setProgressLabel(
          t("new_study.progress_add_sub", {
            title: sub.title.trim() || inferTitleFromPath(sub.path),
          }),
        );
        const entry = await api.addSubBook(
          study.slug,
          sub.path,
          {
            title: sub.title.trim() || inferTitleFromPath(sub.path),
            author: sub.author.trim() || null,
          },
          sub.roleNote.trim() || null,
        );
        subEntries.push(entry);
      }

      // 인덱싱은 백그라운드.
      for (const entry of [mainEntry, ...subEntries]) {
        void api.startIndexing(study.slug, entry.id).catch((e) => {
          console.warn("startIndexing failed:", entry.id, e);
        });
      }

      if (!study.is_active) {
        await useStudyStore.getState().select(study.slug);
      }
      setOpen(false);
      setPage("workspace");
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
      setProgressLabel(null);
    } finally {
      setSubmitting(false);
    }
  }

  function handleNext() {
    if (isLast) {
      void handleSubmit();
      return;
    }
    setStep(step + 1);
  }

  const stepLabels = [
    t("new_study.step1_label"),
    t("new_study.step2_label"),
    t("new_study.step3_label"),
    t("new_study.step4_label"),
  ];

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="new-study-title"
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/50 p-4 sm:items-center"
      onClick={() => {
        if (!submitting) setOpen(false);
      }}
    >
      <Card
        className="w-full max-w-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <CardHeader>
          <div className="flex items-start justify-between gap-2">
            <CardTitle id="new-study-title">{t("new_study.title")}</CardTitle>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setOpen(false)}
              disabled={submitting}
              aria-label={t("common.close")}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
          <div className="pt-3">
            <StepIndicator
              current={step}
              total={TOTAL_STEPS}
              labels={stepLabels}
            />
          </div>
        </CardHeader>
        <CardContent className="min-h-[300px] space-y-4">
          {step === 1 ? (
            <Step1
              name={name}
              language={language}
              onNameChange={handleNameChange}
              onLanguageChange={setLanguage}
              disabled={submitting}
            />
          ) : null}
          {step === 2 ? (
            <Step2
              overview={overview}
              onOverviewChange={setOverview}
              disabled={submitting}
            />
          ) : null}
          {step === 3 ? (
            <Step3Books
              mainBook={mainBook}
              subBooks={subBooks}
              showMainForm={showMainForm}
              showSubForm={showSubForm}
              disabled={submitting}
              onMainAdd={(b) => {
                setMainBook(b);
                setShowMainForm(false);
              }}
              onMainRemove={() => {
                setMainBook(null);
                setShowMainForm(true);
              }}
              onMainShowForm={() => setShowMainForm(true)}
              onMainCancelForm={() => setShowMainForm(false)}
              onSubAdd={(b) => {
                setSubBooks([...subBooks, b]);
                setShowSubForm(false);
              }}
              onSubRemove={(id) =>
                setSubBooks(subBooks.filter((s) => s.id !== id))
              }
              onSubShowForm={() => setShowSubForm(true)}
              onSubCancelForm={() => setShowSubForm(false)}
            />
          ) : null}
          {step === 4 ? (
            <Step4Summary
              name={trimmedName}
              mainBook={mainBook}
              subBooksCount={subBooks.length}
              progressLabel={progressLabel}
            />
          ) : null}

          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}

          <div className="flex justify-between pt-2">
            <Button
              variant="outline"
              onClick={() => (step > 1 ? setStep(step - 1) : setOpen(false))}
              disabled={submitting}
            >
              {step === 1 ? t("common.cancel") : t("new_study.prev")}
            </Button>
            <Button
              onClick={handleNext}
              disabled={
                submitting ||
                (step === 1 && !canNextFromStep1) ||
                (step === 3 && !canNextFromStep3) ||
                (isLast && !canSubmit)
              }
            >
              {submitting ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : null}
              {isLast ? t("new_study.submit") : t("new_study.next")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function Step1({
  name,
  language,
  onNameChange,
  onLanguageChange,
  disabled,
}: {
  name: string;
  language: string;
  onNameChange: (v: string) => void;
  onLanguageChange: (v: string) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="study-name">{t("new_study.name_label")}</Label>
        <Input
          id="study-name"
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          placeholder={t("new_study.name_placeholder")}
          disabled={disabled}
          autoFocus
        />
        <p className="text-xs text-muted-foreground">
          {t("new_study.name_hint")}
        </p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="study-lang">{t("new_study.language_label")}</Label>
        <select
          id="study-lang"
          value={language}
          onChange={(e) => onLanguageChange(e.target.value)}
          disabled={disabled}
          className="flex h-9 w-full rounded-md border border-border bg-input px-3 py-1 text-sm shadow-sm focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/30"
        >
          <option value="ko">한국어</option>
          <option value="en" disabled>
            English (지원 예정)
          </option>
        </select>
        <p className="text-xs text-muted-foreground">
          {t("new_study.language_hint")}
        </p>
      </div>
    </div>
  );
}

function Step2({
  overview,
  onOverviewChange,
  disabled,
}: {
  overview: string;
  onOverviewChange: (v: string) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <p className="text-sm text-muted-foreground">{t("new_study.overview_help")}</p>
      <Textarea
        value={overview}
        onChange={(e) => onOverviewChange(e.target.value)}
        disabled={disabled}
        className="min-h-[260px] resize-none font-mono text-xs leading-relaxed"
        spellCheck={false}
      />
      <p className="text-xs text-muted-foreground">
        {t("new_study.overview_note")}
      </p>
    </div>
  );
}

/** 주교재 + 부교재를 한 화면에 통합 (PR 39). */
function Step3Books({
  mainBook,
  subBooks,
  showMainForm,
  showSubForm,
  disabled,
  onMainAdd,
  onMainRemove,
  onMainShowForm,
  onMainCancelForm,
  onSubAdd,
  onSubRemove,
  onSubShowForm,
  onSubCancelForm,
}: {
  mainBook: BookDraft | null;
  subBooks: BookDraft[];
  showMainForm: boolean;
  showSubForm: boolean;
  disabled: boolean;
  onMainAdd: (b: BookDraft) => void;
  onMainRemove: () => void;
  onMainShowForm: () => void;
  onMainCancelForm: () => void;
  onSubAdd: (b: BookDraft) => void;
  onSubRemove: (id: string) => void;
  onSubShowForm: () => void;
  onSubCancelForm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-6">
      <section className="space-y-2">
        <h3 className="text-sm font-semibold">{t("new_study.main_label")}</h3>
        <p className="text-xs text-muted-foreground">{t("new_study.main_hint")}</p>
        {mainBook ? (
          <BookCard book={mainBook} kind="main" disabled={disabled} onRemove={onMainRemove} />
        ) : showMainForm ? (
          <BookForm kind="main" disabled={disabled} onAdd={onMainAdd} onCancel={onMainCancelForm} />
        ) : (
          <Button variant="outline" onClick={onMainShowForm} disabled={disabled}>
            <Plus className="mr-2 h-4 w-4" />
            {t("new_study.main_add")}
          </Button>
        )}
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold">{t("new_study.sub_label")}</h3>
        <p className="text-xs text-muted-foreground">{t("new_study.sub_hint")}</p>
        {subBooks.length > 0 ? (
          <ul className="space-y-2">
            {subBooks.map((sub) => (
              <li key={sub.id}>
                <BookCard
                  book={sub}
                  kind="sub"
                  disabled={disabled}
                  onRemove={() => onSubRemove(sub.id)}
                />
              </li>
            ))}
          </ul>
        ) : null}
        {showSubForm ? (
          <BookForm kind="sub" disabled={disabled} onAdd={onSubAdd} onCancel={onSubCancelForm} />
        ) : (
          <Button variant="outline" onClick={onSubShowForm} disabled={disabled}>
            <Plus className="mr-2 h-4 w-4" />
            {t("new_study.sub_add")}
          </Button>
        )}
      </section>
    </div>
  );
}

function Step4Summary({
  name,
  mainBook,
  subBooksCount,
  progressLabel,
}: {
  name: string;
  mainBook: BookDraft | null;
  subBooksCount: number;
  progressLabel: string | null;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4">
      <p className="text-sm">{t("new_study.summary_intro")}</p>
      <dl className="space-y-2 rounded-md border border-border bg-muted/30 p-3 text-sm">
        <div className="flex gap-3">
          <dt className="w-20 shrink-0 text-xs text-muted-foreground">
            {t("new_study.summary_name")}
          </dt>
          <dd className="font-medium">{name}</dd>
        </div>
        <div className="flex gap-3">
          <dt className="w-20 shrink-0 text-xs text-muted-foreground">
            {t("new_study.summary_main")}
          </dt>
          <dd>
            {mainBook
              ? mainBook.title.trim() || inferTitleFromPath(mainBook.path)
              : "—"}
          </dd>
        </div>
        <div className="flex gap-3">
          <dt className="w-20 shrink-0 text-xs text-muted-foreground">
            {t("new_study.summary_sub")}
          </dt>
          <dd>
            {subBooksCount === 0
              ? t("new_study.summary_sub_empty")
              : t("new_study.summary_sub_count", { count: subBooksCount })}
          </dd>
        </div>
      </dl>
      <p className="text-xs text-muted-foreground">{t("new_study.indexing_note")}</p>
      {progressLabel ? (
        <p className="text-xs text-muted-foreground">{progressLabel}</p>
      ) : null}
    </div>
  );
}

function BookCard({
  book,
  kind,
  disabled,
  onRemove,
}: {
  book: BookDraft;
  kind: "main" | "sub";
  disabled: boolean;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const displayTitle = book.title.trim() || inferTitleFromPath(book.path);
  return (
    <div className="flex items-start justify-between gap-2 rounded-md border border-border bg-card px-3 py-2">
      <div className="min-w-0 flex-1 space-y-1">
        <p className="truncate text-sm font-medium">{displayTitle}</p>
        <p className="truncate text-xs text-muted-foreground">{book.path}</p>
        {book.author.trim() ? (
          <p className="truncate text-xs text-muted-foreground">{book.author.trim()}</p>
        ) : null}
        {kind === "sub" && book.roleNote.trim() ? (
          <p className="truncate text-xs text-muted-foreground">
            {t("new_study.sub_role_prefix")}: {book.roleNote.trim()}
          </p>
        ) : null}
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={onRemove}
        disabled={disabled}
        aria-label={t("new_study.book_remove")}
      >
        <Trash2 className="h-4 w-4" />
      </Button>
    </div>
  );
}

function BookForm({
  kind,
  disabled,
  onAdd,
  onCancel,
}: {
  kind: "main" | "sub";
  disabled: boolean;
  onAdd: (book: BookDraft) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [path, setPath] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [roleNote, setRoleNote] = useState("");

  const ext = path?.split(".").pop()?.toLowerCase() ?? "";
  const isPdf = ext === "pdf";
  const isUnsupported = path !== null && !SUPPORTED_EXTS.includes(ext);

  async function handlePickFile() {
    const selected = await open({
      multiple: false,
      filters: [
        { name: t("addbook.title"), extensions: SUPPORTED_EXTS },
      ],
    });
    if (typeof selected !== "string") return;
    setPath(selected);
    if (!title) setTitle(inferTitleFromPath(selected));
  }

  function handleAdd() {
    if (!path || isUnsupported) return;
    onAdd({
      id: newBookDraftId(),
      path,
      title,
      author,
      roleNote: kind === "sub" ? roleNote : "",
    });
  }

  return (
    <div className="space-y-3 rounded-md border border-border bg-muted/30 p-3">
      <div className="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={() => void handlePickFile()}
          disabled={disabled}
        >
          {t("addbook.select_file")}
        </Button>
        <span className="truncate text-xs text-muted-foreground">
          {path ?? t("addbook.selected_none")}
        </span>
      </div>

      {isUnsupported ? (
        <p className="text-xs text-destructive" role="alert">
          {t("addbook.format_unsupported")}
        </p>
      ) : null}
      {isPdf ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          {t("addbook.pdf_note")}
        </p>
      ) : null}

      <div className="space-y-1">
        <Label htmlFor={`book-title-${kind}`} className="text-xs">
          {t("addbook.title_label")}
        </Label>
        <Input
          id={`book-title-${kind}`}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder={t("addbook.title_placeholder")}
          disabled={disabled}
        />
      </div>
      <div className="space-y-1">
        <Label htmlFor={`book-author-${kind}`} className="text-xs">
          {t("addbook.author_label")}
        </Label>
        <Input
          id={`book-author-${kind}`}
          value={author}
          onChange={(e) => setAuthor(e.target.value)}
          placeholder={t("addbook.author_placeholder")}
          disabled={disabled}
        />
      </div>
      {kind === "sub" ? (
        <div className="space-y-1">
          <Label htmlFor="book-role-note" className="text-xs">
            {t("new_study.sub_role_label")}
          </Label>
          <Input
            id="book-role-note"
            value={roleNote}
            onChange={(e) => setRoleNote(e.target.value)}
            placeholder={t("new_study.sub_role_placeholder")}
            disabled={disabled}
          />
          <p className="text-xs text-muted-foreground">
            {t("new_study.sub_role_hint")}
          </p>
        </div>
      ) : null}

      <div className="flex justify-end gap-2 pt-1">
        <Button
          variant="ghost"
          size="sm"
          onClick={onCancel}
          disabled={disabled}
        >
          {t("common.cancel")}
        </Button>
        <Button
          size="sm"
          onClick={handleAdd}
          disabled={disabled || !path || isUnsupported}
        >
          {t("new_study.book_add")}
        </Button>
      </div>
    </div>
  );
}
