// API 키 입력·저장·삭제 UI.
// 키 *값* 자체는 백엔드에만 흐른다 — UI는 "저장됨/없음" 상태만 다룸 (security.md L116).

import { useEffect, useState } from "react";
import { Eye, EyeOff, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError, type Provider } from "@/lib/types";

interface Props {
  provider: Provider;
}

type Status = "idle" | "saving" | "deleting";

export function ApiKeyInput({ provider }: Props) {
  const { t } = useTranslation();
  const [keyInput, setKeyInput] = useState("");
  const [reveal, setReveal] = useState(false);
  const [present, setPresent] = useState<boolean | null>(null);
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .apiKeyPresent(provider)
      .then(setPresent)
      .catch(() => setPresent(false));
  }, [provider]);

  async function handleSave() {
    if (!keyInput.trim()) return;
    setStatus("saving");
    setError(null);
    try {
      await api.apiKeySet(provider, keyInput);
      setKeyInput("");
      setPresent(true);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setStatus("idle");
    }
  }

  async function handleDelete() {
    setStatus("deleting");
    setError(null);
    try {
      await api.apiKeyDelete(provider);
      setPresent(false);
    } catch (e) {
      setError(isAppError(e) ? appErrorMessage(e) : String(e));
    } finally {
      setStatus("idle");
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <Label htmlFor={`api-key-${provider}`}>
          {t("settings.api_key.label")}
        </Label>
        <span className="text-xs text-muted-foreground">
          {present === null
            ? t("common.checking")
            : present
              ? t("settings.api_key.saved")
              : t("settings.api_key.missing")}
        </span>
      </div>

      <div className="flex gap-2">
        <div className="relative flex-1">
          <Input
            id={`api-key-${provider}`}
            type={reveal ? "text" : "password"}
            placeholder={t("settings.api_key.input_placeholder")}
            value={keyInput}
            onChange={(e) => setKeyInput(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            className="pr-10 font-mono"
          />
          <button
            type="button"
            onClick={() => setReveal((v) => !v)}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            aria-label={
              reveal
                ? t("settings.api_key.hide")
                : t("settings.api_key.reveal")
            }
          >
            {reveal ? <EyeOff size={16} /> : <Eye size={16} />}
          </button>
        </div>
        <Button
          onClick={handleSave}
          disabled={!keyInput.trim() || status === "saving"}
        >
          {status === "saving" ? <Loader2 className="animate-spin" /> : null}
          {t("common.save")}
        </Button>
      </div>

      {present ? (
        <Button
          variant="outline"
          onClick={handleDelete}
          disabled={status === "deleting"}
          className="text-destructive hover:text-destructive"
        >
          {status === "deleting" ? <Loader2 className="animate-spin" /> : null}
          {t("settings.api_key.delete_confirm")}
        </Button>
      ) : null}

      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}

      <p className="text-xs text-muted-foreground">
        {t("settings.api_key.footer_note")}
      </p>
    </div>
  );
}
