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
import { appErrorMessage, isAppError } from "@/lib/types";
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
    if (!title) {
      const filename = selected.split(/[\\/]/).pop() ?? "";
      const stem = filename.replace(/\.[^.]+$/, "");
      setTitle(stem);
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
