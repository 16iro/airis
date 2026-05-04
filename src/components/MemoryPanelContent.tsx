// Memory.md 편집 콘텐츠 — SlideupPanel의 Memory 탭 콘텐츠로 사용.
// PR 33 (D-070): 기존 MemoryEditor의 모달 wrapper 제거하고 콘텐츠만 분리.
//
// 흐름:
//   1) 활성 스터디 slug로 memory_read → 본문 textarea + last fingerprint 보관
//   2) 사용자가 편집 → 저장 시 memory_write
//   3) external_edited=true면 경고 배너 + "다시 불러오기" 버튼

import { Loader2, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError, type MemoryDoc } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";

export function MemoryPanelContent() {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const slug = activeStudy?.slug ?? null;

  const [doc, setDoc] = useState<MemoryDoc | null>(null);
  const [externalEdited, setExternalEdited] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!slug) return;
    let cancelled = false;
    void (async () => {
      setLoading(true);
      setError(null);
      try {
        const result = await api.memoryRead(slug);
        if (!cancelled) {
          setDoc(result.doc);
          setExternalEdited(result.external_edited);
        }
      } catch (e) {
        if (!cancelled)
          setError(isAppError(e) ? appErrorMessage(e) : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [slug]);

  async function reload() {
    if (!slug) return;
    setLoading(true);
    setError(null);
    try {
      const result = await api.memoryRead(slug);
      setDoc(result.doc);
      setExternalEdited(result.external_edited);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setLoading(false);
    }
  }

  async function handleSave() {
    if (!doc) return;
    setSaving(true);
    setError(null);
    try {
      const next: MemoryDoc = {
        ...doc,
        updated: new Date().toISOString(),
      };
      await api.memoryWrite(next);
      setDoc(next);
      setExternalEdited(false);
      setSavedAt(next.updated);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setSaving(false);
    }
  }

  if (!slug) {
    return (
      <p className="text-xs text-muted-foreground">
        {t("memory.no_active_study")}
      </p>
    );
  }

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex items-start justify-between gap-2">
        <p className="text-xs text-muted-foreground">{t("memory.subtitle")}</p>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2"
          onClick={() => void reload()}
          aria-label={t("memory.reload")}
          disabled={loading || saving}
        >
          <RefreshCw size={14} />
        </Button>
      </div>

      {externalEdited ? (
        <p
          className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
          role="alert"
        >
          {t("memory.external_edit_warning")}
        </p>
      ) : null}

      {loading ? (
        <div className="flex flex-1 items-center justify-center">
          <Loader2 className="animate-spin" size={20} />
        </div>
      ) : doc ? (
        <Textarea
          value={doc.body}
          onChange={(e) => setDoc({ ...doc, body: e.target.value })}
          className="flex-1 resize-none font-mono text-xs"
          spellCheck={false}
          disabled={saving}
        />
      ) : null}

      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>
          {savedAt
            ? `${t("memory.saved")} (${savedAt.slice(0, 19).replace("T", " ")})`
            : doc?.updated
              ? `${t("memory.updated_label")}: ${doc.updated.slice(0, 19).replace("T", " ")}`
              : ""}
        </span>
        <Button
          size="sm"
          onClick={() => void handleSave()}
          disabled={saving || loading || !doc}
        >
          {saving ? <Loader2 className="animate-spin" size={14} /> : null}
          {saving ? t("memory.saving") : t("memory.save")}
        </Button>
      </div>
    </div>
  );
}
