// 스터디 설정 모달 — 라이브러리 인스펙터 footer "설정" 버튼이 띄움 (PR 59).
//
// 주교재: read-only 카드 표시 (변경 불가, 사용자 명시)
// 부교재: list + 추가/삭제. 추가 시 add_sub_book + start_indexing 백엔드 호출.
//
// 학습 목표/마감일/이름 변경은 v0.3.1 carryover 후속 PR.

import { open } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ImageMinus, ImagePlus, Loader2, Plus, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { BookCard, BookForm } from "@/components/book/BookFormCard";
import { inferTitleFromPath } from "@/components/book/bookDraft";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { api } from "@/lib/api";
import {
  appErrorMessage,
  isAppError,
  type BookEntry,
  type StudyMeta,
} from "@/lib/types";

const THUMBNAIL_EXTS = ["png", "jpg", "jpeg", "webp", "gif"];

function thumbnailSrcFor(book: BookEntry): string | null {
  if (!book.thumbnail_path) return null;
  // dockview/asset:// 호환 webview-safe URL.
  return convertFileSrc(book.thumbnail_path);
}

interface Props {
  study: StudyMeta;
  onClose: () => void;
  /** 스터디 메타가 갱신되면 부모(라이브러리)가 list를 다시 불러올 수 있도록 알림. */
  onStudyChange?: (study: StudyMeta) => void;
}

function bookEntryToCardDraft(entry: BookEntry) {
  return {
    id: entry.id,
    path: entry.source_path,
    title: entry.title,
    author: entry.author ?? "",
    roleNote: entry.role_note ?? "",
  };
}

export function StudySettingsDialog({ study: initialStudy, onClose, onStudyChange }: Props) {
  const [study, setStudy] = useState<StudyMeta>(initialStudy);
  const { t } = useTranslation();
  const [books, setBooks] = useState<BookEntry[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [showSubForm, setShowSubForm] = useState<boolean>(false);
  const [busy, setBusy] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  // 책 list 로드 + study slug 변경 시 갱신.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await api.listBooks(study.slug);
        if (!cancelled) setBooks(list);
      } catch (e) {
        if (!cancelled) setError(isAppError(e) ? appErrorMessage(e) : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [study.slug]);

  // ESC로 닫기 (제출 중엔 무시).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const main = books.find((b) => b.role === "main") ?? null;
  const subs = books.filter((b) => b.role === "sub");

  async function handleAddSub(draft: {
    id: string;
    path: string;
    title: string;
    author: string;
    roleNote: string;
  }) {
    setBusy(true);
    setError(null);
    try {
      const entry = await api.addSubBook(
        study.slug,
        draft.path,
        {
          title: draft.title.trim() || inferTitleFromPath(draft.path),
          author: draft.author.trim() || null,
        },
        draft.roleNote.trim() || null,
      );
      void api.startIndexing(study.slug, entry.id).catch((e) => {
        console.warn("startIndexing failed:", entry.id, e);
      });
      setBooks((prev) => [...prev, entry]);
      setShowSubForm(false);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleRemoveSub(bookId: string) {
    setBusy(true);
    setError(null);
    try {
      await api.removeBook(study.slug, bookId);
      setBooks((prev) => prev.filter((b) => b.id !== bookId));
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleSetThumbnail(bookId: string) {
    if (busy) return;
    const selected = await open({
      multiple: false,
      filters: [{ name: t("study_settings.thumbnail_filter"), extensions: THUMBNAIL_EXTS }],
    });
    if (typeof selected !== "string") return;
    setBusy(true);
    setError(null);
    try {
      const updated = await api.setBookThumbnail(study.slug, bookId, selected);
      setBooks((prev) => prev.map((b) => (b.id === bookId ? updated : b)));
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleClearThumbnail(bookId: string) {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await api.clearBookThumbnail(study.slug, bookId);
      setBooks((prev) => prev.map((b) => (b.id === bookId ? updated : b)));
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleSetStudyThumbnail() {
    if (busy) return;
    const selected = await open({
      multiple: false,
      filters: [{ name: t("study_settings.thumbnail_filter"), extensions: THUMBNAIL_EXTS }],
    });
    if (typeof selected !== "string") return;
    setBusy(true);
    setError(null);
    try {
      const updated = await api.setStudyThumbnail(study.slug, selected);
      setStudy(updated);
      onStudyChange?.(updated);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleClearStudyThumbnail() {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await api.clearStudyThumbnail(study.slug);
      setStudy(updated);
      onStudyChange?.(updated);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  function deriveCoverHue(slug: string): number {
    let h = 0;
    for (let i = 0; i < slug.length; i++) {
      h = (h * 31 + slug.charCodeAt(i)) >>> 0;
    }
    return h % 360;
  }
  const studyHue = deriveCoverHue(study.slug);
  const studyLabel = study.name.trim().charAt(0) || "?";

  function thumbnailMenu(book: BookEntry) {
    return (
      <div className="flex flex-col gap-1">
        <Button
          variant="ghost"
          size="sm"
          className="h-5 w-5 rounded-full bg-card p-0 shadow-sm"
          onClick={() => void handleSetThumbnail(book.id)}
          disabled={busy}
          aria-label={t("study_settings.thumbnail_change")}
          title={t("study_settings.thumbnail_change")}
        >
          <ImagePlus className="h-3 w-3" />
        </Button>
        {book.thumbnail_path ? (
          <Button
            variant="ghost"
            size="sm"
            className="h-5 w-5 rounded-full bg-card p-0 shadow-sm"
            onClick={() => void handleClearThumbnail(book.id)}
            disabled={busy}
            aria-label={t("study_settings.thumbnail_clear")}
            title={t("study_settings.thumbnail_clear")}
          >
            <ImageMinus className="h-3 w-3" />
          </Button>
        ) : null}
      </div>
    );
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="study-settings-title"
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/50 p-4 sm:items-center"
      onClick={() => {
        if (!busy) onClose();
      }}
    >
      <Card
        className="w-full max-w-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-2 border-b border-border px-5 py-3.5">
          <div className="space-y-1">
            <h2 id="study-settings-title" className="text-base font-semibold">
              {t("study_settings.title")}
            </h2>
            <p className="break-all text-xs text-muted-foreground">
              {study.name}
            </p>
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={onClose}
            disabled={busy}
            aria-label={t("common.close")}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div className="space-y-6 px-5 py-4">
          <section className="space-y-2">
            <h3 className="text-sm font-semibold">
              {t("study_settings.cover_label")}
            </h3>
            <p className="text-xs text-muted-foreground">
              {t("study_settings.cover_hint")}
            </p>
            <div className="flex items-start gap-3">
              <div
                className="flex h-[100px] w-[140px] shrink-0 items-center justify-center overflow-hidden rounded-md"
                style={
                  study.thumbnail_path
                    ? undefined
                    : {
                        background: `linear-gradient(135deg, oklch(0.92 0.08 ${studyHue}), oklch(0.78 0.14 ${studyHue}))`,
                      }
                }
              >
                {study.thumbnail_path ? (
                  <img
                    src={convertFileSrc(study.thumbnail_path)}
                    alt={study.name}
                    className="h-full w-full object-cover"
                  />
                ) : (
                  <span className="font-mono text-[40px] font-bold text-white opacity-90">
                    {studyLabel}
                  </span>
                )}
              </div>
              <div className="flex flex-col gap-1.5">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void handleSetStudyThumbnail()}
                  disabled={busy}
                >
                  <ImagePlus className="mr-1 h-3.5 w-3.5" />
                  {t("study_settings.cover_change")}
                </Button>
                {study.thumbnail_path ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => void handleClearStudyThumbnail()}
                    disabled={busy}
                  >
                    <ImageMinus className="mr-1 h-3.5 w-3.5" />
                    {t("study_settings.cover_clear")}
                  </Button>
                ) : null}
              </div>
            </div>
          </section>

          <section className="space-y-2">
            <h3 className="text-sm font-semibold">
              {t("study_settings.main_label")}
            </h3>
            <p className="text-xs text-muted-foreground">
              {t("study_settings.main_hint")}
            </p>
            {loading ? (
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3 w-3 animate-spin" />
                {t("common.loading")}
              </p>
            ) : main ? (
              <BookCard
                book={bookEntryToCardDraft(main)}
                kind="main"
                disabled={busy}
                removable={false}
                thumbnailSrc={thumbnailSrcFor(main)}
                thumbnailAction={thumbnailMenu(main)}
              />
            ) : (
              <p className="text-xs text-muted-foreground">
                {t("study_settings.no_main")}
              </p>
            )}
          </section>

          <section className="space-y-2">
            <h3 className="text-sm font-semibold">
              {t("study_settings.sub_label")}
            </h3>
            <p className="text-xs text-muted-foreground">
              {t("study_settings.sub_hint")}
            </p>
            {subs.length > 0 ? (
              <ul className="space-y-2">
                {subs.map((b) => (
                  <li key={b.id}>
                    <BookCard
                      book={bookEntryToCardDraft(b)}
                      kind="sub"
                      disabled={busy}
                      onRemove={() => void handleRemoveSub(b.id)}
                      thumbnailSrc={thumbnailSrcFor(b)}
                      thumbnailAction={thumbnailMenu(b)}
                    />
                  </li>
                ))}
              </ul>
            ) : null}
            {showSubForm ? (
              <BookForm
                kind="sub"
                disabled={busy}
                onAdd={(draft) => void handleAddSub(draft)}
                onCancel={() => setShowSubForm(false)}
              />
            ) : (
              <Button
                variant="outline"
                onClick={() => setShowSubForm(true)}
                disabled={busy}
              >
                <Plus className="mr-2 h-4 w-4" />
                {t("study_settings.sub_add")}
              </Button>
            )}
          </section>

          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </div>
      </Card>
    </div>
  );
}
