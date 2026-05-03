// API 키 입력·저장·삭제 UI.
// 키 *값* 자체는 백엔드에만 흐른다 — UI는 "저장됨/없음" 상태만 다룸 (security.md L116).

import { useEffect, useState } from "react";
import { Eye, EyeOff, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api";
import { appErrorMessage, isAppError, type Provider } from "@/lib/types";

interface Props {
  provider: Provider;
  /** 표시명 (예: "Anthropic"). */
  label: string;
}

type Status = "idle" | "saving" | "deleting";

export function ApiKeyInput({ provider, label }: Props) {
  const [keyInput, setKeyInput] = useState("");
  const [reveal, setReveal] = useState(false);
  const [present, setPresent] = useState<boolean | null>(null);
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState<string | null>(null);

  // 마운트 시 키 존재 여부 조회.
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
        <Label htmlFor={`api-key-${provider}`}>{label} API 키</Label>
        <span
          className={
            present === null
              ? "text-xs text-muted-foreground"
              : present
                ? "text-xs text-foreground"
                : "text-xs text-muted-foreground"
          }
        >
          {present === null ? "확인 중…" : present ? "저장됨 ✓" : "없음"}
        </span>
      </div>

      <div className="flex gap-2">
        <div className="relative flex-1">
          <Input
            id={`api-key-${provider}`}
            type={reveal ? "text" : "password"}
            placeholder="sk-ant-..."
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
            aria-label={reveal ? "키 가리기" : "키 보이기"}
          >
            {reveal ? <EyeOff size={16} /> : <Eye size={16} />}
          </button>
        </div>
        <Button
          onClick={handleSave}
          disabled={!keyInput.trim() || status === "saving"}
        >
          {status === "saving" ? (
            <Loader2 className="animate-spin" />
          ) : null}
          저장
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
          저장된 키 삭제
        </Button>
      ) : null}

      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}

      <p className="text-xs text-muted-foreground">
        키는 OS 키체인에만 저장됩니다 — 디스크 평문 저장 X · 외부 서버 전송 X.
        실제 작동 검증은 첫 챗 호출 시 이루어집니다.
      </p>
    </div>
  );
}
