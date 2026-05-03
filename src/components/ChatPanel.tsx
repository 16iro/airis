// 챗 패널 — 입력 + 스트리밍 + 메시지 히스토리.
// Tauri events 구독: chat:chunk·chat:done·chat:error.
// 단축키: Mod+L → 입력 포커스, Mod+Enter → 전송 (App.tsx에서 처리).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { ChatMessage } from "@/components/ChatMessage";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import {
  appErrorMessage,
  isAppError,
  type Usage,
} from "@/lib/types";
import { useChatStore } from "@/store/chatStore";
import { useStudyStore } from "@/store/studyStore";
import { useUiStore } from "@/store/uiStore";

interface ChunkPayload {
  handle: string;
  text: string;
}
interface DonePayload {
  handle: string;
  usage: Usage;
}
interface ErrorPayload {
  handle: string;
  error: { kind: string; message?: string };
  job_id: number | null;
}

interface ChatPanelHandle {
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}

export function ChatPanel({
  registerHandle,
}: {
  registerHandle?: (h: ChatPanelHandle) => void;
}) {
  const { t } = useTranslation();
  const [input, setInput] = useState("");
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  const messages = useChatStore((s) => s.messages);
  const streamingHandle = useChatStore((s) => s.streamingHandle);
  const addUserMessage = useChatStore((s) => s.addUserMessage);
  const beginAssistantStream = useChatStore((s) => s.beginAssistantStream);
  const appendChunk = useChatStore((s) => s.appendChunk);
  const finalizeStream = useChatStore((s) => s.finalizeStream);
  const failStream = useChatStore((s) => s.failStream);
  const setPage = useUiStore((s) => s.setPage);
  const activeStudy = useStudyStore((s) => s.active);

  // 키 보유 여부 확인 (없으면 Settings 안내).
  useEffect(() => {
    api
      .apiKeyPresent("anthropic")
      .then(setHasKey)
      .catch(() => setHasKey(false));
  }, [streamingHandle]); // 키 추가/삭제 후 재확인을 위해 streamingHandle 변화에도 반응.

  // 부모(App)가 단축키 처리에 사용할 input ref 등록.
  useEffect(() => {
    if (registerHandle) registerHandle({ inputRef });
  }, [registerHandle]);

  // Tauri events 구독.
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];

    listen<ChunkPayload>("chat:chunk", (event) => {
      appendChunk(event.payload.handle, event.payload.text);
    }).then((u) => unlisteners.push(u));

    listen<DonePayload>("chat:done", (event) => {
      finalizeStream(event.payload.handle, event.payload.usage);
    }).then((u) => unlisteners.push(u));

    listen<ErrorPayload>("chat:error", (event) => {
      const errMessage =
        event.payload.error.message ?? `(${event.payload.error.kind})`;
      failStream(
        event.payload.handle,
        errMessage,
        event.payload.job_id ?? undefined,
      );
    }).then((u) => unlisteners.push(u));

    return () => {
      for (const u of unlisteners) u();
    };
  }, [appendChunk, finalizeStream, failStream]);

  // 새 메시지 들어올 때 자동 스크롤.
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  async function handleSend() {
    const trimmed = input.trim();
    if (!trimmed || streamingHandle) return;
    if (hasKey === false) {
      setPage("settings");
      return;
    }
    if (!activeStudy) {
      // 부팅 hydration이 끝나기 전 — 사용자가 마구 enter 치는 케이스. 무시.
      return;
    }

    addUserMessage(trimmed);
    setInput("");

    try {
      const { handle } = await api.chatSend(activeStudy.slug, trimmed, null);
      beginAssistantStream(handle);
    } catch (e) {
      const errMessage = isAppError(e)
        ? appErrorMessage(e)
        : String(e);
      // 사용자 메시지 직후라 별도 어시스턴트 메시지를 못 만들었음 → 일단 사용자에게 alert.
      addUserMessageFailure(errMessage);
    }
  }

  function addUserMessageFailure(msg: string) {
    // 간단히 — chatStore에 새 어시스턴트 메시지 추가 후 곧장 fail.
    const handle = `local-fail-${Date.now()}`;
    beginAssistantStream(handle);
    failStream(handle, msg);
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key === "Enter") {
      e.preventDefault();
      void handleSend();
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        {hasKey === false ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
            <h3 className="text-lg font-medium">{t("chat.no_api_key")}</h3>
            <p className="max-w-sm text-sm text-muted-foreground">
              {t("chat.no_api_key_hint")}
            </p>
            <Button onClick={() => setPage("settings")}>
              {t("chat.open_settings")}
            </Button>
          </div>
        ) : messages.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
            <h3 className="text-lg font-medium">{t("chat.empty_title")}</h3>
            <p className="max-w-sm text-sm text-muted-foreground">
              {t("chat.empty_hint")}
            </p>
          </div>
        ) : (
          <div className="divide-y divide-border">
            {messages.map((m) => (
              <ChatMessage key={m.id} message={m} />
            ))}
          </div>
        )}
      </div>
      <div className="border-t border-border p-3">
        <div className="flex items-end gap-2">
          <Textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t("chat.input_placeholder")}
            rows={2}
            disabled={streamingHandle !== null}
            className="flex-1 resize-none font-sans"
          />
          <Button
            onClick={() => void handleSend()}
            disabled={!input.trim() || streamingHandle !== null}
            size="sm"
            aria-label={t("chat.send")}
          >
            <Send size={16} />
          </Button>
        </div>
      </div>
    </div>
  );
}
