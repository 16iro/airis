// 라이브러리 우측 인스펙터 — 유니티/옵시디언 스타일 (PR 40, D-070).
//
// 카드 클릭 시 라이브러리 위에 floating으로 슬라이드 인. 활성 전환 X — *조회 only*.
// "진입" 버튼 클릭 시 활성 전환 + workspace 이동.
//
// 부모(Library)에서 inspectorSlug 변경 시 책 list 다시 로드.

import { ArrowRight, BookOpen, Loader2, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import type { BookEntry, StudyMeta } from "@/lib/types";
import { cn } from "@/lib/utils";

function deriveCoverHue(slug: string): number {
  let h = 0;
  for (let i = 0; i < slug.length; i++) {
    h = (h * 31 + slug.charCodeAt(i)) >>> 0;
  }
  return h % 360;
}

function deriveCoverLabel(name: string): string {
  return name.trim().charAt(0) || "?";
}

interface Props {
  study: StudyMeta;
  entering?: boolean;
  enterError?: string | null;
  onClose: () => void;
  onEnter: () => void;
  onDelete: () => void;
}

export function LibraryInspector({
  study,
  entering = false,
  enterError = null,
  onClose,
  onEnter,
  onDelete,
}: Props) {
  const { t } = useTranslation();
  const [books, setBooks] = useState<BookEntry[]>([]);
  const [loading, setLoading] = useState<boolean>(true);

  useEffect(() => {
    let cancelled = false;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setLoading(true);
    void (async () => {
      try {
        const list = await api.listBooks(study.slug);
        if (!cancelled) setBooks(list);
      } catch (e) {
        console.warn("listBooks failed:", e);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [study.slug]);

  // ESC로 닫기.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const hue = deriveCoverHue(study.slug);
  const label = deriveCoverLabel(study.name);
  const mainBooks = books.filter((b) => b.role === "main");
  const subBooks = books.filter((b) => b.role === "sub");

  return (
    <aside
      className="fixed right-0 top-12 z-40 flex h-[calc(100vh-3rem)] w-[480px] flex-col border-l border-border bg-card shadow-lg"
      style={{ animation: "slideInRight 240ms cubic-bezier(0.16, 1, 0.3, 1)" }}
      role="complementary"
      aria-label={t("library.inspector.title")}
    >
      <header className="flex shrink-0 items-center justify-between border-b border-border px-4 py-2.5">
        <span className="text-[12px] font-semibold uppercase tracking-wider text-muted-foreground">
          {t("library.inspector.title")}
        </span>
        <Button
          variant="ghost"
          size="sm"
          onClick={onClose}
          aria-label={t("common.close")}
          className="h-7 w-7 p-0"
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </header>

      <div className="flex-1 overflow-auto">
        <div className="space-y-4 p-4">
          <div
            className="flex h-[120px] items-center justify-center overflow-hidden rounded-lg"
            style={{
              background: `linear-gradient(135deg, oklch(0.92 0.08 ${hue}), oklch(0.78 0.14 ${hue}))`,
            }}
          >
            <span className="font-mono text-[48px] font-bold text-white opacity-90">
              {label}
            </span>
          </div>

          <div>
            <div className="flex items-start justify-between gap-2">
              <h2 className="break-all text-base font-semibold leading-tight">
                {study.name}
              </h2>
              {study.is_active ? (
                <span
                  className="inline-flex h-5 shrink-0 items-center gap-1 rounded-full bg-primary px-2 text-[11px] font-medium text-primary-foreground"
                  aria-label={t("library.active_badge")}
                  title={t("library.active_badge")}
                >
                  <BookOpen className="h-3 w-3" />
                  {t("library.active_badge")}
                </span>
              ) : null}
            </div>
            <p className="mt-1 font-mono text-[11px] text-muted-foreground">
              {study.slug}
            </p>
          </div>

          <dl className="space-y-1.5 rounded-md border border-border bg-muted/30 p-3 text-xs">
            <MetaRow
              label={t("library.inspector.meta_books")}
              value={t("library.card_meta_books", {
                count: study.book_count,
              })}
            />
            <MetaRow
              label={t("library.inspector.meta_last_opened")}
              value={
                study.last_opened
                  ? study.last_opened.slice(0, 10)
                  : t("library.inspector.meta_never_opened")
              }
            />
            <MetaRow
              label={t("library.inspector.meta_created")}
              value={study.created_at.slice(0, 10)}
            />
          </dl>

          <div className="space-y-2">
            <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              <BookOpen className="h-3 w-3" />
              {t("library.inspector.books")}
            </div>
            {loading ? (
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3 w-3 animate-spin" />
                {t("common.loading")}
              </p>
            ) : books.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                {t("library.inspector.no_books")}
              </p>
            ) : (
              <div className="space-y-2">
                {mainBooks.length > 0 ? (
                  <BookGroup
                    label={t("books.role_main")}
                    items={mainBooks}
                    accent
                  />
                ) : null}
                {subBooks.length > 0 ? (
                  <BookGroup
                    label={t("books.role_sub")}
                    items={subBooks}
                  />
                ) : null}
              </div>
            )}
          </div>
        </div>
      </div>

      <footer className="flex shrink-0 flex-col gap-2 border-t border-border bg-card px-4 py-3">
        {enterError ? (
          <p className="text-xs text-destructive" role="alert">
            {enterError}
          </p>
        ) : null}
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            disabled={entering}
            className="text-destructive hover:bg-destructive/10 hover:text-destructive"
            aria-label={t("library.delete")}
          >
            <Trash2 className="h-3.5 w-3.5" />
            {t("library.delete")}
          </Button>
          <div className="flex-1" />
          <Button onClick={onEnter} disabled={entering}>
            {entering ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : null}
            {study.is_active
              ? t("library.inspector.continue")
              : t("library.inspector.start")}
            <ArrowRight className="h-3.5 w-3.5" />
          </Button>
        </div>
      </footer>
    </aside>
  );
}

function MetaRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-2">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="text-right font-medium">{value}</dd>
    </div>
  );
}

function BookGroup({
  label,
  items,
  accent,
}: {
  label: string;
  items: BookEntry[];
  accent?: boolean;
}) {
  return (
    <div className="space-y-1">
      <div
        className={cn(
          "text-[10px] font-mono uppercase",
          accent ? "text-primary" : "text-muted-foreground",
        )}
      >
        {label}
      </div>
      <ul className="space-y-1">
        {items.map((b) => (
          <li
            key={b.id}
            className="rounded-md border border-border bg-card px-2.5 py-1.5"
          >
            <p className="truncate text-xs font-medium" title={b.title}>
              {b.title}
            </p>
            {b.author ? (
              <p className="truncate text-[11px] text-muted-foreground">
                {b.author}
              </p>
            ) : null}
            {b.role_note ? (
              <p className="truncate text-[11px] text-muted-foreground">
                {b.role_note}
              </p>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  );
}
