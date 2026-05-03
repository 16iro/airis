// 한 챗 메시지 — 사용자/어시스턴트 + 스트리밍·에러·재시도·위반·인용 마커.

import { AlertTriangle, Loader2, RotateCcw, Sparkles, User } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import {
  appErrorMessage,
  isAppError,
  type ChatMessage as ChatMsg,
} from "@/lib/types";
import { useChatStore } from "@/store/chatStore";

// `[1]`·`[2]` 등 인용 마커를 *시각적으로 강조* — clickable 점프는 v0.3+ (백엔드 mapping 필요).
const CITATION_RE = /\[(\d{1,2})\]/g;
const CITATION_COMPONENTS: Components = {
  p: ({ children, ...rest }) => (
    <p {...rest}>{renderWithCitations(children)}</p>
  ),
  li: ({ children, ...rest }) => (
    <li {...rest}>{renderWithCitations(children)}</li>
  ),
};

function renderWithCitations(children: React.ReactNode): React.ReactNode {
  if (typeof children === "string") {
    return splitOnCitations(children);
  }
  if (Array.isArray(children)) {
    return children.map((c, i) =>
      typeof c === "string" ? (
        <span key={i}>{splitOnCitations(c)}</span>
      ) : (
        <span key={i}>{c}</span>
      ),
    );
  }
  return children;
}

function splitOnCitations(text: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  let last = 0;
  let key = 0;
  for (const m of text.matchAll(CITATION_RE)) {
    const start = m.index ?? 0;
    if (start > last) out.push(text.slice(last, start));
    out.push(
      <span
        key={`cit-${key++}`}
        className="inline-flex items-center justify-center rounded-md bg-primary/15 px-1.5 py-0 text-[10px] font-semibold text-primary"
        title={`인용 #${m[1]}`}
      >
        {m[0]}
      </span>,
    );
    last = start + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

interface Props {
  message: ChatMsg;
}

export function ChatMessage({ message }: Props) {
  const { t } = useTranslation();
  const isUser = message.role === "user";
  const [retrying, setRetrying] = useState(false);

  const beginAssistantStream = useChatStore((s) => s.beginAssistantStream);
  const clearJobId = useChatStore((s) => s.clearJobId);

  async function handleRetry() {
    if (!message.job_id || retrying) return;
    setRetrying(true);
    try {
      const { handle } = await api.retryFailedJob(message.job_id);
      // 새 어시스턴트 메시지 시작.
      beginAssistantStream(handle);
      // 기존 에러 메시지의 job_id 비움 → 재시도 버튼 사라짐.
      clearJobId(message.id);
    } catch (e) {
      // 재시도 호출 자체가 실패한 경우 — 그대로 둠 (큐는 그대로).
      const msg = isAppError(e) ? appErrorMessage(e) : String(e);
      console.error("retry failed:", msg);
    } finally {
      setRetrying(false);
    }
  }

  return (
    <div className="flex gap-3 px-4 py-3">
      <div
        className={
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-full " +
          (isUser ? "bg-primary text-primary-foreground" : "bg-accent")
        }
      >
        {isUser ? <User size={16} /> : <Sparkles size={16} />}
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-1 text-xs font-medium text-muted-foreground">
          {isUser ? t("chat.you") : t("chat.assistant")}
        </div>
        {isUser ? (
          <div className="whitespace-pre-wrap text-sm">{message.content}</div>
        ) : (
          <div className="markdown-body text-sm">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={CITATION_COMPONENTS}
            >
              {message.content || (message.streaming ? "…" : "")}
            </ReactMarkdown>
          </div>
        )}
        {message.streaming ? (
          <div className="mt-1 flex items-center gap-1 text-xs text-muted-foreground">
            <Loader2 size={12} className="animate-spin" />
            {t("chat.streaming")}
          </div>
        ) : null}
        {message.violations && message.violations.length > 0 ? (
          <div
            className="mt-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
            role="alert"
          >
            <div className="mb-1 flex items-center gap-1 font-medium">
              <AlertTriangle size={12} />
              {t("chat.violation_title")}
            </div>
            <p className="text-[11px] opacity-80">{t("chat.violation_hint")}</p>
            <ul className="mt-1 space-y-0.5">
              {message.violations.map((v, i) => (
                <li key={i} className="text-[11px]">
                  · {v.forbidden}
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        {message.error ? (
          <div
            className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
            role="alert"
          >
            <div>
              <span className="font-medium">{t("chat.error_prefix")}</span>:{" "}
              {message.error}
            </div>
            {message.job_id ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => void handleRetry()}
                disabled={retrying}
                className="mt-2 h-7 px-2 text-xs"
              >
                {retrying ? (
                  <Loader2 size={12} className="animate-spin" />
                ) : (
                  <RotateCcw size={12} />
                )}
                {t("chat.retry")}
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
