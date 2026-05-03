// Memory.md 편집 슬라이드업 패널.
//
// 흐름:
//   1) 활성 스터디 slug로 memory_read → 본문 textarea에 표시 + last fingerprint 보관
//   2) 사용자가 편집 → 저장 시 memory_write
//   3) external_edited=true면 경고 배너 + "다시 불러오기" 버튼

import { Loader2, RefreshCw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError, type MemoryDoc } from "@/lib/types";
import { useStudyStore } from "@/store/studyStore";

interface Props {
  onClose: () => void;
}

export function MemoryEditor({ onClose }: Props) {
  const { t } = useTranslation();
  const activeStudy = useStudyStore((s) => s.active);
  const slug = activeStudy?.slug ?? null;

  const [doc, setDoc] = useState<MemoryDoc | null>(null);
  const [externalEdited, setExternalEdited] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // 활성 스터디 변경 시 자동 reload — async IIFE로 setState를 effect body 동기 호출에서 분리.
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
    return null;
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="memory-title"
      className="fixed inset-0 z-50 flex items-end justify-center bg-black/40"
      onClick={onClose}
    >
      <Card
        className="w-full max-w-3xl rounded-b-none"
        onClick={(e) => e.stopPropagation()}
      >
        <CardHeader>
          <div className="flex items-start justify-between gap-2">
            <div>
              <CardTitle id="memory-title">{t("memory.title")}</CardTitle>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("memory.subtitle")}
              </p>
            </div>
            <div className="flex items-center gap-1">
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
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2"
                onClick={onClose}
                aria-label={t("memory.close")}
              >
                <X size={14} />
              </Button>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          {externalEdited ? (
            <p
              className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
              role="alert"
            >
              {t("memory.external_edit_warning")}
            </p>
          ) : null}

          {loading ? (
            <div className="flex h-40 items-center justify-center">
              <Loader2 className="animate-spin" size={20} />
            </div>
          ) : doc ? (
            <Textarea
              value={doc.body}
              onChange={(e) => setDoc({ ...doc, body: e.target.value })}
              className="h-[60vh] resize-none font-mono text-xs"
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
            <Button onClick={() => void handleSave()} disabled={saving || loading || !doc}>
              {saving ? <Loader2 className="animate-spin" size={14} /> : null}
              {saving ? t("memory.saving") : t("memory.save")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
