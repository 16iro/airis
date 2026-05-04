// F1 Library — prototype 100% 충실 (PR 35, D-070).
//
// 카드 디자인: cover gradient(이름 hash로 hue 도출) + 큰 라벨(첫 글자) + 진도 바 + streak/goal placeholder.
// 헤더: 제목 + 검색 버튼(placeholder) + 새 스터디 primary button.
// "새 스터디" → setNewStudyOpen(true) → NewStudyDialog 모달.

import { Plus, Search, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { StudyMeta } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

function deriveCoverHue(slug: string): number {
  let h = 0;
  for (let i = 0; i < slug.length; i++) {
    h = (h * 31 + slug.charCodeAt(i)) >>> 0;
  }
  return h % 360;
}

function deriveCoverLabel(name: string): string {
  const first = name.trim().charAt(0);
  return first || "?";
}

export function Library() {
  const { t } = useTranslation();
  const setNewStudyOpen = useUiStore((s) => s.setNewStudyOpen);

  const list = useStudyStore((s) => s.list);
  const active = useStudyStore((s) => s.active);
  const refreshList = useStudyStore((s) => s.refreshList);
  const select = useStudyStore((s) => s.select);
  const remove = useStudyStore((s) => s.remove);
  const setPage = useUiStore((s) => s.setPage);

  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

  useEffect(() => {
    void refreshList();
  }, [refreshList]);

  async function handleOpen(slug: string) {
    if (slug !== active?.slug) {
      await select(slug);
    }
    setPage("workspace");
  }

  async function handleConfirmDelete() {
    if (!pendingDelete) return;
    await remove(pendingDelete);
    setPendingDelete(null);
  }

  const target = pendingDelete
    ? list.find((s) => s.slug === pendingDelete)
    : null;

  return (
    <div className="flex h-full flex-col bg-background">
      <TopBar />
      <main className="mx-auto w-full max-w-6xl flex-1 overflow-y-auto px-7 py-6">
        <div className="mb-6 flex items-end justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">
              {t("library.title")}
            </h1>
            <p className="mt-1 text-sm text-muted-foreground">
              {list.length} {t("library.subtitle_count")}
            </p>
          </div>
          <div className="flex gap-2">
            <Button variant="outline" disabled>
              <Search size={14} />
              {t("library.search")}
              <span className="ml-1.5 rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                ⌘K
              </span>
            </Button>
            <Button onClick={() => setNewStudyOpen(true)}>
              <Plus size={14} />
              {t("library.new_study")}
            </Button>
          </div>
        </div>

        {list.length === 0 ? (
          <EmptyState onCreate={() => setNewStudyOpen(true)} />
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {list.map((s) => (
              <StudyCard
                key={s.slug}
                study={s}
                onOpen={() => void handleOpen(s.slug)}
                onDelete={() => setPendingDelete(s.slug)}
              />
            ))}
          </div>
        )}
      </main>

      {target ? (
        <DeleteConfirmDialog
          name={target.name}
          onConfirm={() => void handleConfirmDelete()}
          onCancel={() => setPendingDelete(null)}
        />
      ) : null}
    </div>
  );
}

function StudyCard({
  study,
  onOpen,
  onDelete,
}: {
  study: StudyMeta;
  onOpen: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const hue = deriveCoverHue(study.slug);
  const label = deriveCoverLabel(study.name);

  return (
    <div
      className="group relative flex cursor-pointer flex-col gap-2.5 overflow-hidden rounded-xl border border-border bg-card p-4 shadow-sm transition-all hover:-translate-y-0.5 hover:border-primary hover:shadow-md"
      onClick={onOpen}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
    >
      <div
        className="relative flex h-[140px] items-center justify-center overflow-hidden rounded-lg"
        style={{
          background: `linear-gradient(135deg, oklch(0.92 0.08 ${hue}), oklch(0.78 0.14 ${hue}))`,
        }}
      >
        <span
          className="font-mono text-[56px] font-bold opacity-90"
          style={{ color: "white" }}
        >
          {label}
        </span>
        {study.is_active ? (
          <span className="absolute left-2 top-2 rounded-full bg-black/45 px-2 py-0.5 text-[11px] text-white">
            {t("library.active_badge")}
          </span>
        ) : null}
      </div>

      <div>
        <div className="mb-1 line-clamp-2 break-all text-[14px] font-semibold leading-tight">
          {study.name}
        </div>
        <p className="mb-2 text-xs text-muted-foreground">
          {t("library.card_meta_books", { count: study.book_count })} ·{" "}
          {study.last_opened
            ? study.last_opened.slice(0, 10)
            : study.created_at.slice(0, 10)}
        </p>
        <div className="flex items-center gap-2 text-xs">
          <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
            <div className="h-full rounded-full bg-primary" style={{ width: "0%" }} />
          </div>
          <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
            0/0
          </span>
        </div>
      </div>

      <Button
        variant="ghost"
        size="sm"
        className="absolute right-2 top-2 h-7 w-7 p-0 text-destructive opacity-0 transition-opacity hover:bg-destructive/10 group-hover:opacity-100"
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
        aria-label={t("library.delete")}
      >
        <Trash2 className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

function EmptyState({ onCreate }: { onCreate: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border py-16 text-center">
      <h3 className="text-lg font-medium">{t("library.empty_title")}</h3>
      <p className="max-w-sm text-sm text-muted-foreground">
        {t("library.empty_hint")}
      </p>
      <Button onClick={onCreate}>
        <Plus size={16} />
        {t("library.new_study")}
      </Button>
    </div>
  );
}

function DeleteConfirmDialog({
  name,
  onConfirm,
  onCancel,
}: {
  name: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="delete-confirm-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onCancel}
    >
      <Card
        className="w-full max-w-md"
        onClick={(e) => e.stopPropagation()}
      >
        <CardHeader>
          <CardTitle id="delete-confirm-title">
            {t("library.delete_confirm_title")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-sm text-muted-foreground">
            {t("library.delete_confirm_body", { name })}
          </p>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onCancel}>
              {t("library.delete_confirm_cancel")}
            </Button>
            <Button
              variant="default"
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={onConfirm}
            >
              {t("library.delete_confirm_yes")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
