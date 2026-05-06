// 책 등록 폼/카드 — NewStudyDialog와 StudySettingsDialog 공유 (PR 59).
//
// BookDraft: 클라이언트 측 임시 책 메타 (백엔드 ID 부여 전 또는 표시용).
// BookCard: 등록된 책 미리보기 (제목·경로·저자·role_note + 삭제 버튼).
// BookForm: 파일 선택 + 메타 입력 + add/cancel.

import { open } from "@tauri-apps/plugin-dialog";
import {
  CheckCircle2,
  FileCode,
  FileText,
  FileType,
  Loader2,
  Pause,
  Play,
  RefreshCcw,
  Square,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  type BookDraft,
  inferTitleFromPath,
  newBookDraftId,
  SUPPORTED_BOOK_EXTS,
} from "@/components/book/bookDraft";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

/** PR A3 (v0.3.2): BookCard에 인덱싱 상태 표시.
 *
 * v0.4.2 PR 3:
 *   * `indexing`에 `jobId`·`pauseReason` 추가 — 백엔드 index:progress 이벤트가 v042
 *     job 단계부터 함께 emit. 기존 v0.3.2 emit는 `jobId`/`pauseReason` 없이도 동작
 *     (옵셔널이라 무파괴).
 *   * `paused` 상태 추가 — 사용자/배터리/절전 등 *현재 일시정지* 표시.
 */
export type BookIndexingStatus =
  | { state: "done" }
  | {
      state: "indexing";
      percent: number;
      step?: string;
      jobId?: number | null;
    }
  | {
      state: "paused";
      percent: number;
      step?: string;
      jobId: number;
      pauseReason: string;
    }
  | { state: "pending" };

export function BookCard({
  book,
  kind,
  disabled,
  onRemove,
  onReindex,
  reindexing = false,
  removable = true,
  thumbnailSrc,
  fileFormat,
  indexingStatus,
  onPauseIndexing,
  onResumeIndexing,
  onCancelIndexing,
}: {
  book: BookDraft;
  kind: "main" | "sub";
  disabled: boolean;
  onRemove?: () => void;
  /** v0.4.1 PR 4 — 사용자가 명시적으로 v041 chunks 적재를 트리거. 미지정이면 버튼 숨김. */
  onReindex?: () => void;
  /** v0.4.1 PR 4 — 재인덱싱이 진행 중이면 버튼을 spinner로 비활성. */
  reindexing?: boolean;
  /** false면 삭제 버튼 숨김 (주교재 read-only용). */
  removable?: boolean;
  /** 표시용 webview-safe URL (convertFileSrc 결과). PDF는 백엔드가 1페이지 PNG로 자동 생성. null이면 file_format 아이콘. */
  thumbnailSrc?: string | null;
  /** PR 63: md/txt/html은 file_format 기반 아이콘 표시. 모르는 형식이면 placeholder. */
  fileFormat?: string;
  /** v0.3.2 A3: 인덱싱 상태. 미지정이면 표시 안 함. */
  indexingStatus?: BookIndexingStatus;
  /** v0.4.2 PR 3 — 진행 중 사용자 일시정지. jobId 인자는 BookCard 내부에서 status에서 추출. */
  onPauseIndexing?: (jobId: number) => void;
  onResumeIndexing?: (jobId: number) => void;
  onCancelIndexing?: (jobId: number) => void;
}) {
  const { t } = useTranslation();
  const displayTitle = book.title.trim() || inferTitleFromPath(book.path);
  return (
    <div className="flex items-start justify-between gap-2 rounded-md border border-border bg-card px-3 py-2">
      <BookThumbnail src={thumbnailSrc} title={displayTitle} fileFormat={fileFormat} />
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
            {t("new_study.sub_role_prefix")}: {book.roleNote.trim()}
          </p>
        ) : null}
        {indexingStatus ? <IndexingStatusBadge status={indexingStatus} /> : null}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {/* v0.4.2 PR 3 — 진행 중일 때 일시정지/재개/취소. */}
        {indexingStatus?.state === "indexing" && indexingStatus.jobId && onPauseIndexing ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onPauseIndexing(indexingStatus.jobId!)}
            disabled={disabled}
            aria-label={t("books.pause_indexing")}
            title={t("books.pause_indexing")}
          >
            <Pause className="h-4 w-4" />
          </Button>
        ) : null}
        {indexingStatus?.state === "paused" && onResumeIndexing ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onResumeIndexing(indexingStatus.jobId)}
            disabled={disabled}
            aria-label={t("books.resume_indexing")}
            title={t("books.resume_indexing")}
          >
            <Play className="h-4 w-4" />
          </Button>
        ) : null}
        {(indexingStatus?.state === "indexing" || indexingStatus?.state === "paused") &&
        onCancelIndexing &&
        indexingStatus.jobId ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onCancelIndexing(indexingStatus.jobId!)}
            disabled={disabled}
            aria-label={t("books.cancel_indexing")}
            title={t("books.cancel_indexing")}
          >
            <Square className="h-4 w-4" />
          </Button>
        ) : null}
        {onReindex ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={onReindex}
            disabled={disabled || reindexing}
            aria-label={t("books.reindex")}
            title={t("books.reindex")}
          >
            {reindexing ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RefreshCcw className="h-4 w-4" />
            )}
          </Button>
        ) : null}
        {removable && onRemove ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={onRemove}
            disabled={disabled}
            aria-label={t("new_study.book_remove")}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function IndexingStatusBadge({ status }: { status: BookIndexingStatus }) {
  const { t } = useTranslation();
  if (status.state === "done") {
    return (
      <p className="flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400">
        <CheckCircle2 className="h-3 w-3" />
        {t("books.indexing_state_done")}
      </p>
    );
  }
  if (status.state === "indexing") {
    const stepLabel = status.step ? indexingStepLabel(status.step, t) : null;
    return (
      <p className="flex items-center gap-1 text-xs text-muted-foreground">
        <Loader2 className="h-3 w-3 animate-spin" />
        {t("books.indexing_state_indexing", { percent: status.percent })}
        {stepLabel ? <span className="text-[10px] opacity-80">· {stepLabel}</span> : null}
      </p>
    );
  }
  if (status.state === "paused") {
    const reasonLabel = pauseReasonLabel(status.pauseReason, t);
    return (
      <p className="flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400">
        <Pause className="h-3 w-3" />
        {t("books.indexing_state_paused", { percent: status.percent })}
        <span className="text-[10px] opacity-80">· {reasonLabel}</span>
      </p>
    );
  }
  return (
    <p className="text-xs text-muted-foreground">
      {t("books.indexing_state_pending")}
    </p>
  );
}

/** v0.4.2 PR 3 — pause_reason DB 값 → 한국어 라벨 (i18n key 폴백). */
function pauseReasonLabel(
  reason: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (reason) {
    case "user":
      return t("books.pause_reason.user");
    case "battery_low":
      return t("books.pause_reason.battery_low");
    case "thermal":
      return t("books.pause_reason.thermal");
    case "app_quit":
      return t("books.pause_reason.app_quit");
    default:
      return reason;
  }
}

/** v0.4.2 PR 3 — index:progress current_step → 한국어 라벨. */
function indexingStepLabel(
  step: string,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (step) {
    case "parse":
      return t("books.step.parse");
    case "chunk":
      return t("books.step.chunk");
    case "embed":
    case "embed_init":
      return t("books.step.embed_t1");
    case "embed_t1":
      return t("books.step.embed_t1");
    case "embed_t2":
      return t("books.step.embed_t2");
    case "manifest_swap":
      return t("books.step.manifest_swap");
    case "auto_pause":
      return t("books.step.auto_pause");
    case "auto_resume":
      return t("books.step.auto_resume");
    case "done":
      return t("books.step.done");
    default:
      return step;
  }
}

function BookThumbnail({
  src,
  title,
  fileFormat,
}: {
  src: string | null | undefined;
  title: string;
  fileFormat?: string;
}) {
  // PR 63: PDF 썸네일은 비율 보존 (h-14 고정 + w-auto, max-w로 폭 가드).
  if (src) {
    return (
      <div className="flex h-14 w-auto max-w-[60px] shrink-0 items-center justify-center overflow-hidden rounded bg-muted">
        <img
          src={src}
          alt={title}
          className="h-full w-auto object-contain"
          loading="lazy"
        />
      </div>
    );
  }

  // PR 63: md/txt/html은 file_format 기반 아이콘. v0.4 로드맵에서 콘텐츠 일부 렌더링으로 대체 예정.
  return (
    <div
      className="flex h-14 w-10 shrink-0 items-center justify-center overflow-hidden rounded bg-muted text-muted-foreground"
      aria-hidden
    >
      <FormatIcon format={fileFormat} fallback={title.trim().charAt(0) || "?"} className="h-6 w-6" />
    </div>
  );
}

export function FormatIcon({
  format,
  fallback,
  className,
}: {
  format?: string;
  fallback: string;
  className?: string;
}) {
  switch (format) {
    case "md":
    case "txt":
      return <FileText className={className} />;
    case "html":
      return <FileCode className={className} />;
    case "pdf":
      return <FileType className={className} />;
    default:
      return <span className="font-mono text-base font-bold">{fallback}</span>;
  }
}

export function BookForm({
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
  const isUnsupported = path !== null && !SUPPORTED_BOOK_EXTS.includes(ext);

  async function handlePickFile() {
    const selected = await open({
      multiple: false,
      filters: [{ name: t("addbook.title"), extensions: SUPPORTED_BOOK_EXTS }],
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
