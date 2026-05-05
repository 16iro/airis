// F1 Library — prototype 디자인 + 우측 인스펙터 (PR 35, PR 40, D-070).
//
// 카드 싱글 클릭 = setInspectorSlug(slug) — 활성 전환 X. 인스펙터에서 "진입" 클릭해야 활성 전환 + workspace 이동.
// 카드 더블 클릭 = 즉시 진입 (PR 67) — 인스펙터를 거치지 않고 바로 워크스페이스로.
// 다른 카드 클릭 = 인스펙터 콘텐츠 교체. inspectorSlug==null이면 닫힘.

import { convertFileSrc } from "@tauri-apps/api/core";
import { BookOpen, Plus, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { LibraryInspector } from "@/components/LibraryInspector";
import { StudySettingsDialog } from "@/components/StudySettingsDialog";
import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { toast } from "@/lib/toast";
import { appErrorMessage, isAppError, type StudyMeta } from "@/lib/types";
import { cn } from "@/lib/utils";
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
  return name.trim().charAt(0) || "?";
}

export function Library() {
  const { t } = useTranslation();
  const setNewStudyOpen = useUiStore((s) => s.setNewStudyOpen);
  const inspectorSlug = useUiStore((s) => s.inspectorSlug);
  const setInspectorSlug = useUiStore((s) => s.setInspectorSlug);

  const list = useStudyStore((s) => s.list);
  const active = useStudyStore((s) => s.active);
  const refreshList = useStudyStore((s) => s.refreshList);
  const select = useStudyStore((s) => s.select);
  const remove = useStudyStore((s) => s.remove);
  const setPage = useUiStore((s) => s.setPage);

  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [enteringSlug, setEnteringSlug] = useState<string | null>(null);
  const [enterError, setEnterError] = useState<string | null>(null);
  const [settingsSlug, setSettingsSlug] = useState<string | null>(null);
  // v0.3.2 B4: 라이브러리 안에서 이름 즉시 필터. ⌘K가 input에 focus.
  const [query, setQuery] = useState<string>("");
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    void refreshList();
  }, [refreshList]);

  // 라이브러리 떠날 때 인스펙터도 닫음 (다음 진입 시 깨끗한 상태).
  useEffect(() => {
    return () => {
      setInspectorSlug(null);
    };
  }, [setInspectorSlug]);

  // ⌘K (또는 Ctrl+K) — 검색 입력창 focus + 기존 query 전체 선택.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        const el = searchInputRef.current;
        if (el) {
          el.focus();
          el.select();
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const filteredList = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return list;
    return list.filter((s) => s.name.toLowerCase().includes(q));
  }, [list, query]);

  async function handleEnter(slug: string) {
    if (enteringSlug) return;
    setEnteringSlug(slug);
    setEnterError(null);
    try {
      if (slug !== active?.slug) {
        await select(slug);
      }
      setInspectorSlug(null);
      setPage("workspace");
    } catch (e) {
      console.error("handleEnter failed:", e);
      const msg = isAppError(e) ? appErrorMessage(e) : String(e);
      setEnterError(msg);
    } finally {
      setEnteringSlug(null);
    }
  }

  async function handleConfirmDelete() {
    if (!pendingDelete) return;
    const removedName = list.find((s) => s.slug === pendingDelete)?.name ?? pendingDelete;
    try {
      await remove(pendingDelete);
      if (inspectorSlug === pendingDelete) {
        setInspectorSlug(null);
      }
      setPendingDelete(null);
      toast.success(t("library.delete_done", { name: removedName }));
    } catch (e) {
      toast.error(isAppError(e) ? appErrorMessage(e) : String(e));
      setPendingDelete(null);
    }
  }

  const inspectorStudy = inspectorSlug
    ? list.find((s) => s.slug === inspectorSlug) ?? null
    : null;
  const settingsStudy = settingsSlug
    ? list.find((s) => s.slug === settingsSlug) ?? null
    : null;
  const target = pendingDelete
    ? list.find((s) => s.slug === pendingDelete)
    : null;

  return (
    <div className="flex h-full flex-col bg-background">
      <TopBar />
      <main
        className={cn(
          "flex-1 overflow-y-auto px-7 py-6 transition-[padding] duration-200",
          inspectorStudy && "pr-[496px]",
        )}
      >
        <div className="mx-auto w-full max-w-6xl">
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
              <div className="relative">
                <Search
                  size={13}
                  className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground"
                  aria-hidden
                />
                <Input
                  ref={searchInputRef}
                  type="search"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={t("library.search_placeholder")}
                  aria-label={t("library.search")}
                  className="h-9 w-56 pl-7 pr-12"
                />
                <span
                  className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                  aria-hidden
                >
                  ⌘K
                </span>
              </div>
              <Button onClick={() => setNewStudyOpen(true)}>
                <Plus size={14} />
                {t("library.new_study")}
              </Button>
            </div>
          </div>

          {list.length === 0 ? (
            <EmptyState onCreate={() => setNewStudyOpen(true)} />
          ) : filteredList.length === 0 ? (
            <p className="py-12 text-center text-sm text-muted-foreground">
              {t("library.search_no_results")}
            </p>
          ) : (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {filteredList.map((s) => (
                <StudyCard
                  key={s.slug}
                  study={s}
                  selected={inspectorSlug === s.slug}
                  onClick={() => setInspectorSlug(s.slug)}
                  onDoubleClick={() => void handleEnter(s.slug)}
                />
              ))}
            </div>
          )}
        </div>
      </main>

      {inspectorStudy ? (
        <LibraryInspector
          study={inspectorStudy}
          entering={enteringSlug === inspectorStudy.slug}
          enterError={enterError}
          onClose={() => setInspectorSlug(null)}
          onEnter={() => void handleEnter(inspectorStudy.slug)}
          onDelete={() => setPendingDelete(inspectorStudy.slug)}
          onOpenSettings={() => setSettingsSlug(inspectorStudy.slug)}
        />
      ) : null}

      {settingsStudy ? (
        <StudySettingsDialog
          study={settingsStudy}
          onClose={() => setSettingsSlug(null)}
          onStudyChange={() => void refreshList()}
        />
      ) : null}

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
  selected,
  onClick,
  onDoubleClick,
}: {
  study: StudyMeta;
  selected: boolean;
  onClick: () => void;
  onDoubleClick: () => void;
}) {
  const { t } = useTranslation();
  const hue = deriveCoverHue(study.slug);
  const label = deriveCoverLabel(study.name);

  return (
    <div
      className={cn(
        "flex cursor-pointer flex-col gap-2.5 overflow-hidden rounded-xl border bg-card p-4 shadow-sm transition-all hover:-translate-y-0.5 hover:shadow-md",
        selected
          ? "border-primary ring-2 ring-primary/30"
          : "border-border hover:border-primary",
      )}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onDoubleClick();
        } else if (e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
    >
      <div
        className="relative flex h-[140px] items-center justify-center overflow-hidden rounded-lg"
        style={
          study.thumbnail_path
            ? undefined
            : {
                background: `linear-gradient(135deg, oklch(0.92 0.08 ${hue}), oklch(0.78 0.14 ${hue}))`,
              }
        }
      >
        {study.thumbnail_path ? (
          <img
            src={convertFileSrc(study.thumbnail_path)}
            alt={study.name}
            className="h-full w-full object-cover"
            loading="lazy"
          />
        ) : (
          <span
            className="font-mono text-[56px] font-bold opacity-90"
            style={{ color: "white" }}
          >
            {label}
          </span>
        )}
        {study.is_active ? (
          <span
            className="absolute left-2 top-2 inline-flex h-6 w-6 items-center justify-center rounded-full bg-black/45 text-white"
            aria-label={t("library.active_badge")}
            title={t("library.active_badge")}
          >
            <BookOpen className="h-3.5 w-3.5" />
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
