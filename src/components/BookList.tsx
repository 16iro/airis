// 워크스페이스 상단 — 활성 스터디의 책 목록 + 검색 + "책 추가".
//
// 클릭 동작:
//   * 책 카드 클릭 → activeBookStore.open → BookViewer 진입
//   * 검색 입력 → search_sections → 인라인 dropdown
//   * dropdown 결과 클릭 → activeBookStore.jumpTo (책 열기 + 섹션 점프 + 활성 박기)

import { BookOpen, Plus, Search, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { AddBookDialog } from "@/components/AddBookDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import type { SearchHit } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useActiveBookStore } from "@/store/activeBookStore";
import { useBookStore } from "@/store/bookStore";
import { useStudyStore } from "@/store/studyStore";

export function BookList() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const books = useBookStore((s) => s.books);
  const refresh = useBookStore((s) => s.refresh);
  const remove = useBookStore((s) => s.remove);
  const openBook = useActiveBookStore((s) => s.open);
  const jumpTo = useActiveBookStore((s) => s.jumpTo);
  const activeBookId = useActiveBookStore((s) => s.bookId);

  const [adding, setAdding] = useState(false);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [searching, setSearching] = useState(false);

  useEffect(() => {
    if (activeStudy) {
      void refresh(activeStudy.slug);
    }
  }, [activeStudy, refresh]);

  // 검색 디바운스 — 300ms. 빈 쿼리는 onChange에서 hits=null 처리하므로 effect 안 들어옴.
  useEffect(() => {
    if (!activeStudy) return;
    const trimmed = query.trim();
    if (trimmed.length < 1) return;
    const handle = setTimeout(() => {
      setSearching(true);
      api
        .searchSections(activeStudy.slug, trimmed, 5)
        .then((r) => setHits(r))
        .catch(() => setHits([]))
        .finally(() => setSearching(false));
    }, 300);
    return () => {
      clearTimeout(handle);
    };
  }, [query, activeStudy]);

  if (!activeStudy) return null;

  const indexedCount = books.filter((b) => b.indexed_at).length;

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
          {books.map((b) => {
            const active = activeBookId === b.id;
            const canOpen = b.file_format !== "pdf";
            return (
              <li
                key={b.id}
                className={cn(
                  "flex shrink-0 items-center gap-2 rounded-md border px-2 py-1 text-xs",
                  active ? "border-primary bg-primary/5" : "border-border",
                  b.indexed_at ? "bg-card" : "bg-muted/40",
                  canOpen ? "cursor-pointer" : "cursor-default opacity-70",
                )}
                onClick={() => {
                  if (canOpen) void openBook(activeStudy.slug, b.id);
                }}
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
                  onClick={(e) => {
                    e.stopPropagation();
                    void remove(activeStudy.slug, b.id);
                  }}
                  aria-label={t("books.remove")}
                  className="text-muted-foreground hover:text-destructive"
                >
                  <Trash2 size={12} />
                </button>
              </li>
            );
          })}
        </ul>
      )}

      {indexedCount > 0 ? (
        <div className="relative mt-2">
          <div className="flex items-center gap-2">
            <Search size={14} className="text-muted-foreground" />
            <Input
              value={query}
              onChange={(e) => {
                const v = e.target.value;
                setQuery(v);
                if (v.trim().length < 1) setHits(null);
              }}
              placeholder={t("books.search_placeholder")}
              className="h-7 text-xs"
            />
          </div>
          {hits ? (
            <ul className="absolute left-0 right-0 top-full z-30 mt-1 max-h-72 overflow-y-auto rounded-md border border-border bg-card shadow-lg">
              {hits.length === 0 ? (
                <li className="px-3 py-2 text-xs text-muted-foreground">
                  {searching ? "…" : t("books.search_no_results")}
                </li>
              ) : (
                hits.map((h) => (
                  <li
                    key={`${h.book_id}-${h.section_path}`}
                    className="cursor-pointer border-b border-border px-3 py-2 text-xs last:border-b-0 hover:bg-muted/40"
                    onClick={() => {
                      void jumpTo(activeStudy.slug, h.book_id, h.section_path);
                      setQuery("");
                      setHits(null);
                    }}
                  >
                    <div className="flex items-center gap-2">
                      <span className="font-medium">{h.book_title}</span>
                      <span className="text-muted-foreground">·</span>
                      <span>{h.section_label}</span>
                      {h.page != null ? (
                        <span className="text-muted-foreground">
                          (p.{h.page})
                        </span>
                      ) : null}
                    </div>
                    <p
                      className="mt-0.5 line-clamp-2 text-muted-foreground"
                      // FTS5 snippet의 << >> 마커를 강조로 변환.
                      dangerouslySetInnerHTML={{
                        __html: highlightSnippet(h.snippet),
                      }}
                    />
                  </li>
                ))
              )}
            </ul>
          ) : null}
        </div>
      ) : (
        <p className="mt-2 text-[11px] text-muted-foreground">
          {t("books.search_not_indexed")}
        </p>
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

/**
 * FTS5 snippet의 `<<...>>` 마커를 `<mark>`로 변환.
 * 사용자 입력 X — 백엔드가 만든 snippet이라 추가 sanitize 비필요.
 */
function highlightSnippet(raw: string): string {
  const escaped = raw
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  // < > 이스케이프 후엔 << >>도 &lt;&lt; / &gt;&gt;로 바뀜.
  return escaped
    .replace(/&lt;&lt;/g, "<mark>")
    .replace(/&gt;&gt;/g, "</mark>");
}
