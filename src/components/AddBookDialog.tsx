// 책 등록 다이얼로그 — 파일 선택 + 메타 입력 + 등록 + 인덱싱.
//
// 흐름:
//   1) 사용자가 파일 선택 (plugin-dialog) → 자동으로 제목 추정 (파일명).
//   2) 제목·저자 편집.
//   3) "등록 + 인덱싱" 클릭 → addMainBook → startIndexing.
//   4) index:progress event 구독으로 진행률 표시.
//
// PDF는 *PR 12에서 활성*. v0.2 PR 11엔 .md/.html/.txt만.

import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError, type ChunkPreview } from "@/lib/types";
import { useBookStore } from "@/store/bookStore";

interface Props {
  studySlug: string;
  onClose: () => void;
}

interface ProgressPayload {
  book_id: string;
  percent: number;
  current_step: string;
}

export function AddBookDialog({ studySlug, onClose }: Props) {
  const { t } = useTranslation();
  const add = useBookStore((s) => s.add);
  const startIndexing = useBookStore((s) => s.startIndexing);

  const [path, setPath] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [progress, setProgress] = useState<ProgressPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  // v0.6.x (D-112) — 청킹 라이브 프리뷰.
  const [preview, setPreview] = useState<ChunkPreview[] | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);

  // BUG-002 (v0.4.4 PR 2, D-092): listener race 가드 — listen() Promise가 cleanup
  // 이후 resolve되면 unlisten이 null인 채 영구 누수. cancelled flag + .then 체이닝
  // 으로 cleanup 이후 도착한 listener도 즉시 해제.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    void listen<ProgressPayload>("index:progress", (e) => {
      if (cancelled) return;
      setProgress(e.payload);
    }).then((u) => {
      if (cancelled) {
        u();
      } else {
        unlisten = u;
      }
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  const ext = path?.split(".").pop()?.toLowerCase() ?? "";
  const isPdf = ext === "pdf";
  // v0.4.4 PR 3 (D-093): DOCX 지원 추가. 등록 흐름은 PDF·MD와 동일 (특별 추가 입력 X).
  const isUnsupported = !["md", "markdown", "html", "htm", "txt", "pdf", "docx"].includes(ext);

  async function handlePickFile() {
    const selected = await open({
      multiple: false,
      filters: [
        {
          name: "교재",
          extensions: ["md", "markdown", "html", "htm", "txt", "pdf", "docx"],
        },
      ],
    });
    if (typeof selected !== "string") return;
    setPath(selected);
    setError(null);
    setPreview(null);
    setPreviewError(null);
    if (!title) {
      const filename = selected.split(/[\\/]/).pop() ?? "";
      const stem = filename.replace(/\.[^.]+$/, "");
      setTitle(stem);
    }
  }

  async function handlePreview() {
    if (!path) return;
    setPreviewLoading(true);
    setPreviewError(null);
    try {
      const chunks = await api.ragPreviewChunks(path);
      setPreview(chunks);
    } catch (e) {
      setPreviewError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setPreviewLoading(false);
    }
  }

  async function handleRegister() {
    if (!path) return;
    setSubmitting(true);
    setError(null);
    setProgress(null);
    try {
      const entry = await add(studySlug, path, {
        title,
        author: author.trim() ? author.trim() : null,
      });
      // PDF도 PR 12.5부터 인덱싱 활성. 시각 뷰어는 PR 12.6 예정.
      await startIndexing(studySlug, entry.id);
      onClose();
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <Card
        className="w-full max-w-md"
        onClick={(e) => e.stopPropagation()}
      >
        <CardHeader>
          <CardTitle>{t("addbook.title")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              onClick={() => void handlePickFile()}
              disabled={submitting}
            >
              {t("addbook.select_file")}
            </Button>
            <span className="truncate text-xs text-muted-foreground">
              {path ?? t("addbook.selected_none")}
            </span>
          </div>

          {isUnsupported && path ? (
            <p className="text-sm text-destructive" role="alert">
              {t("addbook.format_unsupported")}
            </p>
          ) : null}
          {isPdf ? (
            <p className="text-xs text-amber-600 dark:text-amber-400">
              {t("addbook.pdf_note")}
            </p>
          ) : null}

          {/* v0.6.x (D-112) — 청킹 라이브 프리뷰: "이렇게 잘릴 거예요". */}
          {path && !isUnsupported ? (
            <div className="space-y-2 rounded-md border border-border p-2">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium text-muted-foreground">
                  {t("addbook.preview_label")}
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void handlePreview()}
                  disabled={submitting || previewLoading}
                >
                  {previewLoading ? (
                    <Loader2 className="animate-spin" size={12} />
                  ) : null}
                  {t("addbook.preview_run")}
                </Button>
              </div>
              {previewError ? (
                <p className="text-xs text-destructive" role="alert">
                  {previewError}
                </p>
              ) : null}
              {preview ? (
                preview.length === 0 ? (
                  <p className="text-xs text-muted-foreground">
                    {t("addbook.preview_empty")}
                  </p>
                ) : (
                  <div className="space-y-1">
                    <p className="text-xs text-muted-foreground">
                      {t("addbook.preview_summary", {
                        count: preview.length,
                        tokens: preview.reduce((s, c) => s + c.token_count, 0),
                      })}
                    </p>
                    <ul className="max-h-32 space-y-1 overflow-y-auto">
                      {preview.slice(0, 30).map((c) => (
                        <li
                          key={c.ord}
                          className="flex items-center gap-2 text-[11px]"
                          title={c.head}
                        >
                          <span className="w-6 shrink-0 text-right text-muted-foreground">
                            {c.ord + 1}
                          </span>
                          <span
                            className="h-2 shrink-0 rounded-sm bg-primary/60"
                            style={{
                              width: `${Math.max(4, Math.min(100, (c.char_len / 4000) * 100))}px`,
                            }}
                          />
                          <span className="text-muted-foreground">
                            {c.char_len}
                            {t("addbook.preview_chars")}
                          </span>
                          {c.has_code ? (
                            <span className="rounded bg-amber-500/20 px-1 text-amber-700 dark:text-amber-400">
                              {t("addbook.preview_code")}
                            </span>
                          ) : null}
                          <span className="truncate text-muted-foreground/70">
                            {c.head}
                          </span>
                        </li>
                      ))}
                    </ul>
                  </div>
                )
              ) : null}
            </div>
          ) : null}

          <div className="space-y-2">
            <Label htmlFor="book-title">{t("addbook.title_label")}</Label>
            <Input
              id="book-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder={t("addbook.title_placeholder")}
              disabled={submitting}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="book-author">{t("addbook.author_label")}</Label>
            <Input
              id="book-author"
              value={author}
              onChange={(e) => setAuthor(e.target.value)}
              placeholder={t("addbook.author_placeholder")}
              disabled={submitting}
            />
          </div>

          {progress ? (
            <p className="text-xs text-muted-foreground">
              {t(`addbook.step_${progress.current_step}`, {
                defaultValue: progress.current_step,
              })}{" "}
              · {progress.percent}%
            </p>
          ) : null}

          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}

          <div className="flex justify-end gap-2 pt-2">
            <Button variant="outline" onClick={onClose} disabled={submitting}>
              {t("addbook.cancel")}
            </Button>
            <Button
              onClick={() => void handleRegister()}
              disabled={
                !path ||
                !title.trim() ||
                isUnsupported ||
                submitting
              }
            >
              {submitting ? <Loader2 className="animate-spin" size={14} /> : null}
              {submitting ? t("addbook.registering") : t("addbook.register")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
