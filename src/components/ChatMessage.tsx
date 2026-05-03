// 한 챗 메시지 — 사용자/어시스턴트 + 스트리밍·에러 상태.

import { Loader2, User, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import type { ChatMessage as ChatMsg } from "@/lib/types";

interface Props {
  message: ChatMsg;
}

export function ChatMessage({ message }: Props) {
  const { t } = useTranslation();
  const isUser = message.role === "user";

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
            <ReactMarkdown remarkPlugins={[remarkGfm]}>
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
        {message.error ? (
          <div
            className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
            role="alert"
          >
            <span className="font-medium">{t("chat.error_prefix")}</span>:{" "}
            {message.error}
          </div>
        ) : null}
      </div>
    </div>
  );
}
