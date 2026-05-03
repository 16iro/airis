// F1 Library — 스터디 카드 그리드 + 활성 강조 + 새 스터디 CTA + 삭제 확인.
//
// 라우팅: TopBar의 "라이브러리" 버튼 또는 단축키(Mod+B)로 진입.
// 카드 클릭 = 활성 전환 + 워크스페이스로 자동 이동.

import { Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { cn } from "@/lib/utils";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

export function Library() {
  const { t } = useTranslation();
  const setPage = useUiStore((s) => s.setPage);

  const list = useStudyStore((s) => s.list);
  const active = useStudyStore((s) => s.active);
  const refreshList = useStudyStore((s) => s.refreshList);
  const select = useStudyStore((s) => s.select);
  const remove = useStudyStore((s) => s.remove);

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
      <main className="mx-auto w-full max-w-5xl flex-1 overflow-y-auto px-6 py-8">
        <div className="mb-6 flex items-end justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">
              {t("library.title")}
            </h1>
            <p className="mt-1 text-sm text-muted-foreground">
              {t("library.subtitle")}
            </p>
          </div>
          <Button onClick={() => setPage("new-study")}>
            <Plus size={16} />
            {t("library.new_study")}
          </Button>
        </div>

        {list.length === 0 ? (
          <EmptyState onCreate={() => setPage("new-study")} />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
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
  study: import("@/lib/types").StudyMeta;
  onOpen: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Card
      className={cn(
        "cursor-pointer transition-colors hover:border-primary/60",
        study.is_active && "border-primary bg-primary/5",
      )}
      onClick={onOpen}
    >
      <CardHeader>
        <div className="flex items-start justify-between gap-2">
          <CardTitle className="line-clamp-2 break-all text-base">
            {study.name}
          </CardTitle>
          {study.is_active ? (
            <span className="rounded-full bg-primary px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary-foreground">
              {t("library.active_badge")}
            </span>
          ) : null}
        </div>
        <p className="font-mono text-xs text-muted-foreground">{study.slug}</p>
      </CardHeader>
      <CardContent className="space-y-1 text-xs text-muted-foreground">
        <p>
          {t("library.card_meta_books", { count: study.book_count })}
        </p>
        <p>
          {study.last_opened
            ? t("library.card_meta_last_opened", {
                date: study.last_opened.slice(0, 10),
              })
            : t("library.card_meta_created", {
                date: study.created_at.slice(0, 10),
              })}
        </p>
        <div className="flex justify-end pt-2">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-destructive hover:bg-destructive/10 hover:text-destructive"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
            aria-label={t("library.delete")}
          >
            <Trash2 size={14} />
          </Button>
        </div>
      </CardContent>
    </Card>
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
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
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
