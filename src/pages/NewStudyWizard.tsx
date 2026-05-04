// 새 스터디 마법사 (v0.3 트랙 B 재구성).
//
// 흐름:
//   1) 이름 입력 (autofocus, OS 금지문자 즉시 strip)
//   2) 주교재 1권 (필수) + 부교재 N권 (옵션, 각 부교재마다 role_note)
//   3) 요약 + "스터디 만들기" — 트랜잭션으로 study/book 한꺼번에 생성
//
// Step 2는 *백엔드 호출 없이 메모리에만* 책 메타 보관. Step 3 만들기 클릭 시점에
// create_study → add_main_book → add_sub_book ×N → start_indexing 차례 호출.
// 스터디·주교재 등록까지 실패하면 *부분 성공 안내* + 에러 표시. 인덱싱은 백그라운드.
//
// 슬러그는 사용자에게 보이지 않는다. 백엔드가 이름에서 자동 도출 + 충돌 시 ` (2)` suffix.

import { open } from "@tauri-apps/plugin-dialog";
import { Loader2, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { StepIndicator } from "@/components/StepIndicator";
import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { stripForbiddenChars } from "@/lib/sanitizeName";
import { appErrorMessage, isAppError } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

const TOTAL_STEPS = 3;
const SUPPORTED_EXTS = ["md", "markdown", "html", "htm", "txt", "pdf"];

interface BookDraft {
  /** 클라이언트 측 임시 ID — 부교재 목록 key로만 사용. 백엔드는 다른 ID 부여. */
  id: string;
  path: string;
  title: string;
  author: string;
  /** 부교재 전용 — 이 책을 어떤 용도로 참고하는지 LLM에게 알려 주는 짧은 메모. */
  roleNote: string;
}

function newBookDraftId(): string {
  return `draft-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function inferTitleFromPath(path: string): string {
  const filename = path.split(/[\\/]/).pop() ?? "";
  return filename.replace(/\.[^.]+$/, "");
}

export function NewStudyWizard() {
  const { t } = useTranslation();
  const setPage = useUiStore((s) => s.setPage);
  const create = useStudyStore((s) => s.create);

  const [step, setStep] = useState(1);
  const [name, setName] = useState("");
  const [mainBook, setMainBook] = useState<BookDraft | null>(null);
  const [subBooks, setSubBooks] = useState<BookDraft[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [progressLabel, setProgressLabel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const trimmedName = name.trim();
  const canNextFromStep1 = trimmedName.length > 0;
  const canNextFromStep2 = mainBook !== null;

  function handleNameChange(value: string) {
    setName(stripForbiddenChars(value));
    setError(null);
  }

  async function handleCreate() {
    if (!mainBook) return;
    setSubmitting(true);
    setError(null);
    try {
      setProgressLabel(t("wizard.progress_create_study"));
      const study = await create(trimmedName);

      setProgressLabel(t("wizard.progress_add_main"));
      const mainEntry = await api.addMainBook(study.slug, mainBook.path, {
        title: mainBook.title.trim() || inferTitleFromPath(mainBook.path),
        author: mainBook.author.trim() || null,
      });

      const subEntries = [];
      for (const sub of subBooks) {
        setProgressLabel(t("wizard.progress_add_sub", { title: sub.title.trim() || inferTitleFromPath(sub.path) }));
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

      // 인덱싱은 백그라운드. 워크스페이스에 진입한 후에도 진행률 표시 가능.
      const allEntries = [mainEntry, ...subEntries];
      for (const entry of allEntries) {
        void api.startIndexing(study.slug, entry.id).catch((e) => {
          console.warn("startIndexing failed:", entry.id, e);
        });
      }

      if (!study.is_active) {
        await useStudyStore.getState().select(study.slug);
      }
      setProgressLabel(null);
      setPage("workspace");
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
      setProgressLabel(null);
    } finally {
      setSubmitting(false);
    }
  }

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
            labels={[
              t("wizard.step1_title"),
              t("wizard.step2_title"),
              t("wizard.step3_title"),
            ]}
          />
        </div>

        <Card>
          <CardHeader>
            <CardTitle>{stepTitle(step, t)}</CardTitle>
            <p className="text-sm text-muted-foreground">
              {stepSubtitle(step, t)}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            {step === 1 ? (
              <Step1 name={name} onNameChange={handleNameChange} />
            ) : step === 2 ? (
              <Step2
                mainBook={mainBook}
                subBooks={subBooks}
                disabled={submitting}
                onMainBookChange={setMainBook}
                onSubBooksChange={setSubBooks}
              />
            ) : (
              <Step3
                name={trimmedName}
                mainBook={mainBook}
                subBooks={subBooks}
                progressLabel={progressLabel}
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
                  disabled={
                    submitting ||
                    (step === 1 && !canNextFromStep1) ||
                    (step === 2 && !canNextFromStep2)
                  }
                >
                  {t("wizard.next")}
                </Button>
              ) : (
                <Button
                  onClick={() => void handleCreate()}
                  disabled={submitting || !canNextFromStep1 || !canNextFromStep2}
                >
                  {submitting ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : null}
                  {t("wizard.step3_finish")}
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
  onNameChange,
}: {
  name: string;
  onNameChange: (v: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <Label htmlFor="study-name">{t("wizard.step1_name_label")}</Label>
      <Input
        id="study-name"
        value={name}
        onChange={(e) => onNameChange(e.target.value)}
        placeholder={t("wizard.step1_name_placeholder")}
        autoFocus
      />
      <p className="text-xs text-muted-foreground">
        {t("wizard.step1_name_hint")}
      </p>
    </div>
  );
}

function Step2({
  mainBook,
  subBooks,
  disabled,
  onMainBookChange,
  onSubBooksChange,
}: {
  mainBook: BookDraft | null;
  subBooks: BookDraft[];
  disabled: boolean;
  onMainBookChange: (b: BookDraft | null) => void;
  onSubBooksChange: (subs: BookDraft[]) => void;
}) {
  const { t } = useTranslation();
  const [showMainForm, setShowMainForm] = useState(mainBook === null);
  const [showSubForm, setShowSubForm] = useState(false);

  function handleMainAdd(book: BookDraft) {
    onMainBookChange(book);
    setShowMainForm(false);
  }

  function handleMainRemove() {
    onMainBookChange(null);
    setShowMainForm(true);
  }

  function handleSubAdd(book: BookDraft) {
    onSubBooksChange([...subBooks, book]);
    setShowSubForm(false);
  }

  function handleSubRemove(id: string) {
    onSubBooksChange(subBooks.filter((s) => s.id !== id));
  }

  return (
    <div className="space-y-6">
      <section className="space-y-2">
        <h3 className="text-sm font-medium">{t("wizard.step2_main_label")}</h3>
        <p className="text-xs text-muted-foreground">
          {t("wizard.step2_main_hint")}
        </p>
        {mainBook ? (
          <BookCard
            book={mainBook}
            kind="main"
            disabled={disabled}
            onRemove={handleMainRemove}
          />
        ) : showMainForm ? (
          <BookForm
            kind="main"
            disabled={disabled}
            onAdd={handleMainAdd}
            onCancel={() => setShowMainForm(false)}
          />
        ) : (
          <Button
            variant="outline"
            onClick={() => setShowMainForm(true)}
            disabled={disabled}
          >
            <Plus className="mr-2 h-4 w-4" />
            {t("wizard.step2_main_add")}
          </Button>
        )}
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-medium">{t("wizard.step2_sub_label")}</h3>
        <p className="text-xs text-muted-foreground">
          {t("wizard.step2_sub_hint")}
        </p>
        {subBooks.length > 0 ? (
          <ul className="space-y-2">
            {subBooks.map((sub) => (
              <li key={sub.id}>
                <BookCard
                  book={sub}
                  kind="sub"
                  disabled={disabled}
                  onRemove={() => handleSubRemove(sub.id)}
                />
              </li>
            ))}
          </ul>
        ) : null}
        {showSubForm ? (
          <BookForm
            kind="sub"
            disabled={disabled}
            onAdd={handleSubAdd}
            onCancel={() => setShowSubForm(false)}
          />
        ) : (
          <Button
            variant="outline"
            onClick={() => setShowSubForm(true)}
            disabled={disabled}
          >
            <Plus className="mr-2 h-4 w-4" />
            {t("wizard.step2_sub_add")}
          </Button>
        )}
      </section>
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
    <div className="flex items-start justify-between gap-2 rounded-md border bg-card px-3 py-2">
      <div className="min-w-0 flex-1 space-y-1">
        <p className="truncate text-sm font-medium">{displayTitle}</p>
        <p className="truncate text-xs text-muted-foreground">{book.path}</p>
        {book.author.trim() ? (
          <p className="truncate text-xs text-muted-foreground">
            {book.author.trim()}
          </p>
        ) : null}
        {kind === "sub" && book.roleNote.trim() ? (
          <p className="truncate text-xs text-muted-foreground">
            {t("wizard.step2_sub_role_prefix")}: {book.roleNote.trim()}
          </p>
        ) : null}
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={onRemove}
        disabled={disabled}
        aria-label={t("wizard.step2_book_remove")}
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
        {
          name: t("addbook.title"),
          extensions: SUPPORTED_EXTS,
        },
      ],
    });
    if (typeof selected !== "string") return;
    setPath(selected);
    if (!title) {
      setTitle(inferTitleFromPath(selected));
    }
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
    <div className="space-y-3 rounded-md border bg-muted/30 p-3">
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
            {t("wizard.step2_sub_role_label")}
          </Label>
          <Input
            id="book-role-note"
            value={roleNote}
            onChange={(e) => setRoleNote(e.target.value)}
            placeholder={t("wizard.step2_sub_role_placeholder")}
            disabled={disabled}
          />
          <p className="text-xs text-muted-foreground">
            {t("wizard.step2_sub_role_hint")}
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
          {t("wizard.step2_book_add")}
        </Button>
      </div>
    </div>
  );
}

function Step3({
  name,
  mainBook,
  subBooks,
  progressLabel,
}: {
  name: string;
  mainBook: BookDraft | null;
  subBooks: BookDraft[];
  progressLabel: string | null;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4 text-sm">
      <p>{t("wizard.step3_summary_intro")}</p>
      <dl className="space-y-2 rounded-md border bg-muted/30 p-3">
        <div className="flex gap-3">
          <dt className="w-20 shrink-0 text-xs text-muted-foreground">
            {t("wizard.step3_summary_name")}
          </dt>
          <dd className="text-sm font-medium">{name}</dd>
        </div>
        <div className="flex gap-3">
          <dt className="w-20 shrink-0 text-xs text-muted-foreground">
            {t("wizard.step3_summary_main")}
          </dt>
          <dd className="text-sm">
            {mainBook
              ? mainBook.title.trim() || inferTitleFromPath(mainBook.path)
              : "—"}
          </dd>
        </div>
        <div className="flex gap-3">
          <dt className="w-20 shrink-0 text-xs text-muted-foreground">
            {t("wizard.step3_summary_sub")}
          </dt>
          <dd className="text-sm">
            {subBooks.length === 0
              ? t("wizard.step3_summary_sub_empty")
              : t("wizard.step3_summary_sub_count", { count: subBooks.length })}
          </dd>
        </div>
      </dl>
      <p className="text-xs text-muted-foreground">
        {t("wizard.step3_indexing_note")}
      </p>
      {progressLabel ? (
        <p className="text-xs text-muted-foreground">{progressLabel}</p>
      ) : null}
    </div>
  );
}

function stepTitle(step: number, t: (key: string) => string): string {
  if (step === 1) return t("wizard.step1_title");
  if (step === 2) return t("wizard.step2_title");
  return t("wizard.step3_title");
}

function stepSubtitle(step: number, t: (key: string) => string): string {
  if (step === 1) return t("wizard.step1_subtitle");
  if (step === 2) return t("wizard.step2_subtitle");
  return t("wizard.step3_subtitle");
}
