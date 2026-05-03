// 워크스페이스 상단 — 활성 스터디의 책 목록 + "책 추가" 버튼.
//
// 카드 클릭은 PR 12에서 BookViewer 진입과 연결 — PR 11 시점엔 *목록 표시 + 등록·삭제*만.

import { BookOpen, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { AddBookDialog } from "@/components/AddBookDialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useBookStore } from "@/store/bookStore";
import { useStudyStore } from "@/store/studyStore";

export function BookList() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const books = useBookStore((s) => s.books);
  const refresh = useBookStore((s) => s.refresh);
  const remove = useBookStore((s) => s.remove);
  const [adding, setAdding] = useState(false);

  useEffect(() => {
    if (activeStudy) {
      void refresh(activeStudy.slug);
    }
  }, [activeStudy, refresh]);

  if (!activeStudy) return null;

  return (
    <div className="border-b border-border bg-background px-4 py-2">
      <div className="mb-1 flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-muted-foreground">
          {t("books.section_title")}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={() => setAdding(true)}
        >
          <Plus size={14} />
          {t("books.add_button")}
        </Button>
      </div>

      {books.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t("books.empty_hint")}</p>
      ) : (
        <ul className="flex gap-2 overflow-x-auto pb-1">
          {books.map((b) => (
            <li
              key={b.id}
              className={cn(
                "flex shrink-0 items-center gap-2 rounded-md border border-border px-2 py-1 text-xs",
                b.indexed_at ? "bg-card" : "bg-muted/40",
              )}
            >
              <BookOpen size={12} />
              <span className="max-w-[200px] truncate font-medium">{b.title}</span>
              <span className="rounded bg-muted px-1 py-0.5 text-[10px] text-muted-foreground">
                {b.role === "main" ? t("books.role_main") : t("books.role_sub")}
              </span>
              <span className="text-[10px] text-muted-foreground">
                {b.indexed_at
                  ? t("books.indexed")
                  : b.file_format === "pdf"
                    ? "PDF"
                    : t("books.not_indexed")}
              </span>
              <button
                type="button"
                onClick={() => void remove(activeStudy.slug, b.id)}
                aria-label={t("books.remove")}
                className="text-muted-foreground hover:text-destructive"
              >
                <Trash2 size={12} />
              </button>
            </li>
          ))}
        </ul>
      )}

      {adding ? (
        <AddBookDialog
          studySlug={activeStudy.slug}
          onClose={() => setAdding(false)}
        />
      ) : null}
    </div>
  );
}
