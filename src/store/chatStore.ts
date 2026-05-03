// 챗 메시지 + 스트리밍 상태. v0.1 메모리만 (DB 영속은 v0.2).

import { create } from "zustand";

import type { ChatMessage, Usage } from "@/lib/types";

interface ChatStore {
  messages: ChatMessage[];
  /** 진행 중 메시지의 handle → message id 매핑. */
  streamingHandle: string | null;
  streamingMessageId: string | null;

  addUserMessage: (content: string) => string;
  beginAssistantStream: (handle: string) => string;
  appendChunk: (handle: string, text: string) => void;
  finalizeStream: (handle: string, usage: Usage) => void;
  failStream: (handle: string, message: string) => void;
  clear: () => void;
}

let counter = 0;
function nextId(): string {
  counter += 1;
  return `msg-${Date.now()}-${counter}`;
}

export const useChatStore = create<ChatStore>((set, get) => ({
  messages: [],
  streamingHandle: null,
  streamingMessageId: null,

  addUserMessage(content) {
    const id = nextId();
    const message: ChatMessage = {
      id,
      role: "user",
      content,
      created_at: new Date().toISOString(),
    };
    set((s) => ({ messages: [...s.messages, message] }));
    return id;
  },

  beginAssistantStream(handle) {
    const id = nextId();
    const message: ChatMessage = {
      id,
      role: "assistant",
      content: "",
      streaming: true,
      created_at: new Date().toISOString(),
    };
    set((s) => ({
      messages: [...s.messages, message],
      streamingHandle: handle,
      streamingMessageId: id,
    }));
    return id;
  },

  appendChunk(handle, text) {
    const { streamingHandle, streamingMessageId } = get();
    if (handle !== streamingHandle || !streamingMessageId) return;
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === streamingMessageId
          ? { ...m, content: m.content + text }
          : m,
      ),
    }));
  },

  finalizeStream(handle, _usage) {
    const { streamingHandle, streamingMessageId } = get();
    if (handle !== streamingHandle) return;
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === streamingMessageId ? { ...m, streaming: false } : m,
      ),
      streamingHandle: null,
      streamingMessageId: null,
    }));
  },

  failStream(handle, message) {
    const { streamingHandle, streamingMessageId } = get();
    if (handle !== streamingHandle) return;
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === streamingMessageId
          ? { ...m, streaming: false, error: message }
          : m,
      ),
      streamingHandle: null,
      streamingMessageId: null,
    }));
  },

  clear: () =>
    set({
      messages: [],
      streamingHandle: null,
      streamingMessageId: null,
    }),
}));
