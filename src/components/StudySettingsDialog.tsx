// 스터디 설정 모달 — 라이브러리 인스펙터 footer "설정" 버튼이 띄움 (PR 59).
//
// 정보 (PR 68): 이름·자유 메모 편집 — Save 버튼으로 일괄 갱신
// 표지 (PR 62): 변경/제거
// 주교재: read-only 카드 표시 (변경 불가, 사용자 명시)
// 부교재: list + 추가/삭제. 추가 시 add_sub_book + start_indexing 백엔드 호출
// 데이터 폴더 열기 (PR 68): 푸터의 보조 액션
//
// 학습 목표·마감일 (v0.3.2 A1): Overview.md frontmatter의 stated_goal_chapter / deadline.
// SQLite 컬럼이 아니라 Overview.md 파일 경로라서 정보 섹션과는 별도 Save로 분리.

import { open } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FolderOpen, ImageMinus, ImagePlus, Loader2, Plus, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  BookCard,
  BookForm,
  type BookIndexingStatus,
} from "@/components/book/BookFormCard";
import { inferTitleFromPath } from "@/components/book/bookDraft";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { toast } from "@/lib/toast";
import {
  appErrorMessage,
  isAppError,
  type BookEntry,
  type StudyMeta,
} from "@/lib/types";

// PR 62 스터디 표지용. 책 썸네일은 PR 63에서 사용자 임의 변경 폐지 — PDF 자동만 유지.
const STUDY_COVER_EXTS = ["png", "jpg", "jpeg", "webp", "gif"];

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

  // 정보 편집 (PR 68) — 입력 폼은 ephemeral, Save 시 백엔드 호출.
  const [nameDraft, setNameDraft] = useState<string>(study.name);
  const [descDraft, setDescDraft] = useState<string>(study.description ?? "");
  const infoDirty =
    nameDraft.trim() !== study.name.trim() ||
    descDraft.trim() !== (study.description ?? "").trim();

  // 학습 목표·마감일 (v0.3.2 A1) — Overview.md에서 읽어 ephemeral state로 보관.
  const [goalChapterSaved, setGoalChapterSaved] = useState<string>("");
  const [deadlineSaved, setDeadlineSaved] = useState<string>("");
  const [goalChapterDraft, setGoalChapterDraft] = useState<string>("");
  const [deadlineDraft, setDeadlineDraft] = useState<string>("");
  const [goalLoading, setGoalLoading] = useState<boolean>(true);
  const goalDirty =
    goalChapterDraft.trim() !== goalChapterSaved.trim() ||
    deadlineDraft.trim() !== deadlineSaved.trim();

  // 인덱싱 진행률 (v0.3.2 A3) — index:progress 이벤트 수신해 책별로 누적.
  // bookId → { percent, step }. step="done"이면 indexed_at 갱신을 위해 list 재조회.
  const [progressMap, setProgressMap] = useState<
    Record<string, { percent: number; step: string }>
  >({});

  // v0.4.1 PR 4 — 책별 *명시* 재인덱싱 진행 중인 book_id set.
  const [reindexingIds, setReindexingIds] = useState<Set<string>>(new Set());

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

  // index:progress 구독 — 다이얼로그 lifetime 동안만. 100% 도착 시 indexed_at 갱신을 위해 list 재조회.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    void listen<{ book_id: string; percent: number; current_step: string }>(
      "index:progress",
      (e) => {
        if (cancelled) return;
        const { book_id, percent, current_step } = e.payload;
        setProgressMap((prev) => ({
          ...prev,
          [book_id]: { percent, step: current_step },
        }));
        if (percent >= 100) {
          void api
            .listBooks(study.slug)
            .then((list) => {
              if (!cancelled) setBooks(list);
            })
            .catch((err) => {
              console.warn("listBooks refresh after index done failed:", err);
            });
        }
      },
    ).then((u) => {
      unlisten = u;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [study.slug]);

  // 학습 목표·마감일 로드 (study slug 변경 시 갱신).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const overview = await api.studyOverviewRead(study.slug);
        if (cancelled) return;
        setGoalChapterSaved(overview.stated_goal_chapter);
        setDeadlineSaved(overview.deadline);
        setGoalChapterDraft(overview.stated_goal_chapter);
        setDeadlineDraft(overview.deadline);
      } catch (e) {
        if (!cancelled) {
          console.warn("studyOverviewRead failed:", e);
          setError(t("study_settings.goal_load_failed"));
        }
      } finally {
        if (!cancelled) setGoalLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [study.slug, t]);

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

  function bookIndexingStatus(book: BookEntry): BookIndexingStatus {
    if (book.indexed_at) return { state: "done" };
    const prog = progressMap[book.id];
    if (prog) {
      if (prog.percent >= 100) return { state: "done" };
      return { state: "indexing", percent: prog.percent, step: prog.step };
    }
    return { state: "pending" };
  }

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

  async function handleReindex(bookId: string) {
    if (reindexingIds.has(bookId)) return;
    setReindexingIds((prev) => {
      const next = new Set(prev);
      next.add(bookId);
      return next;
    });
    setError(null);
    try {
      await api.reindexBook(study.slug, bookId);
      // 진행률은 index:progress 이벤트가 이미 다이얼로그에서 누적 중. 완료 후 list 재조회.
      const list = await api.listBooks(study.slug);
      setBooks(list);
      toast.success(t("books.reindex_ok"));
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setReindexingIds((prev) => {
        const next = new Set(prev);
        next.delete(bookId);
        return next;
      });
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

  async function handleSetStudyThumbnail() {
    if (busy) return;
    const selected = await open({
      multiple: false,
      filters: [{ name: t("study_settings.cover_filter"), extensions: STUDY_COVER_EXTS }],
    });
    if (typeof selected !== "string") return;
    setBusy(true);
    setError(null);
    try {
      const updated = await api.setStudyThumbnail(study.slug, selected);
      setStudy(updated);
      onStudyChange?.(updated);
      toast.success(t("study_settings.cover_change_ok"));
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
      toast.success(t("study_settings.cover_clear_ok"));
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleSaveInfo() {
    if (busy || !infoDirty) return;
    if (!nameDraft.trim()) {
      setError(t("study_settings.info_name_empty"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const updated = await api.updateStudyInfo(
        study.slug,
        nameDraft.trim(),
        descDraft.trim() || null,
      );
      setStudy(updated);
      setNameDraft(updated.name);
      setDescDraft(updated.description ?? "");
      onStudyChange?.(updated);
      toast.success(t("study_settings.info_save_ok"));
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleSaveGoal() {
    if (busy || !goalDirty) return;
    setBusy(true);
    setError(null);
    try {
      const overview = await api.studyOverviewWriteMeta(
        study.slug,
        goalChapterDraft.trim(),
        deadlineDraft.trim(),
      );
      setGoalChapterSaved(overview.stated_goal_chapter);
      setDeadlineSaved(overview.deadline);
      setGoalChapterDraft(overview.stated_goal_chapter);
      setDeadlineDraft(overview.deadline);
      toast.success(t("study_settings.goal_save_ok"));
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleOpenFolder() {
    if (busy) return;
    setError(null);
    try {
      await api.openStudyFolder(study.slug);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
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
          <h2 id="study-settings-title" className="text-base font-semibold">
            {t("study_settings.title")}
          </h2>
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
          <section className="space-y-3">
            <h3 className="text-sm font-semibold">
              {t("study_settings.info_label")}
            </h3>
            <div className="space-y-1.5">
              <Label htmlFor="study-info-name" className="text-xs">
                {t("study_settings.info_name_label")}
              </Label>
              <Input
                id="study-info-name"
                value={nameDraft}
                onChange={(e) => setNameDraft(e.target.value)}
                placeholder={t("study_settings.info_name_placeholder")}
                disabled={busy}
                maxLength={80}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="study-info-desc" className="text-xs">
                {t("study_settings.info_desc_label")}
              </Label>
              <Textarea
                id="study-info-desc"
                value={descDraft}
                onChange={(e) => setDescDraft(e.target.value)}
                placeholder={t("study_settings.info_desc_placeholder")}
                disabled={busy}
                rows={3}
              />
              <p className="text-xs text-muted-foreground">
                {t("study_settings.info_desc_hint")}
              </p>
            </div>
            <div className="flex justify-end">
              <Button
                size="sm"
                onClick={() => void handleSaveInfo()}
                disabled={busy || !infoDirty}
              >
                {t("study_settings.info_save")}
              </Button>
            </div>
          </section>

          <section className="space-y-3">
            <h3 className="text-sm font-semibold">
              {t("study_settings.goal_label")}
            </h3>
            <p className="text-xs text-muted-foreground">
              {t("study_settings.goal_hint")}
            </p>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label htmlFor="study-goal-chapter" className="text-xs">
                  {t("study_settings.goal_chapter_label")}
                </Label>
                <Input
                  id="study-goal-chapter"
                  value={goalChapterDraft}
                  onChange={(e) => setGoalChapterDraft(e.target.value)}
                  placeholder={t("study_settings.goal_chapter_placeholder")}
                  disabled={busy || goalLoading}
                  maxLength={120}
                />
                <p className="text-xs text-muted-foreground">
                  {t("study_settings.goal_chapter_hint")}
                </p>
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="study-goal-deadline" className="text-xs">
                  {t("study_settings.goal_deadline_label")}
                </Label>
                <Input
                  id="study-goal-deadline"
                  type="date"
                  value={deadlineDraft}
                  onChange={(e) => setDeadlineDraft(e.target.value)}
                  disabled={busy || goalLoading}
                />
                <p className="text-xs text-muted-foreground">
                  {t("study_settings.goal_deadline_hint")}
                </p>
              </div>
            </div>
            <div className="flex justify-end">
              <Button
                size="sm"
                onClick={() => void handleSaveGoal()}
                disabled={busy || goalLoading || !goalDirty}
              >
                {t("study_settings.goal_save")}
              </Button>
            </div>
          </section>

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
                onReindex={() => void handleReindex(main.id)}
                reindexing={reindexingIds.has(main.id)}
                fileFormat={main.file_format}
                thumbnailSrc={main.thumbnail_path ? convertFileSrc(main.thumbnail_path) : null}
                indexingStatus={bookIndexingStatus(main)}
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
                      onReindex={() => void handleReindex(b.id)}
                      reindexing={reindexingIds.has(b.id)}
                      fileFormat={b.file_format}
                      thumbnailSrc={b.thumbnail_path ? convertFileSrc(b.thumbnail_path) : null}
                      indexingStatus={bookIndexingStatus(b)}
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

          <div className="flex justify-end border-t border-border pt-3">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void handleOpenFolder()}
              disabled={busy}
            >
              <FolderOpen className="mr-1 h-3.5 w-3.5" />
              {t("study_settings.open_folder")}
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
}
