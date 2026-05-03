// 챗 메시지 + 스트리밍 상태.
// v0.2부터 chat_messages 테이블에 영속 — 부팅 시 hydrate로 복원.

import { create } from "zustand";

import { api } from "@/lib/api";
import type {
  ChatHistoryMessage,
  ChatMessage,
  Usage,
  ViolationHit,
} from "@/lib/types";

interface ChatStore {
  messages: ChatMessage[];
  /** 진행 중 메시지의 handle → message id 매핑. */
  streamingHandle: string | null;
  streamingMessageId: string | null;

  /** 활성 스터디의 최근 메시지를 백엔드에서 로드. */
  hydrate: (studySlug: string, limit?: number) => Promise<void>;
  addUserMessage: (content: string) => string;
  beginAssistantStream: (handle: string) => string;
  appendChunk: (handle: string, text: string) => void;
  finalizeStream: (handle: string, usage: Usage) => void;
  failStream: (handle: string, message: string, jobId?: number) => void;
  /** 메시지의 job_id를 비움 — 사용자가 재시도 클릭 시 사용. */
  clearJobId: (messageId: string) => void;
  /** 진행 중 어시스턴트 메시지에 검증 위반 의심 hits 첨부 — chat:violation event. */
  attachViolations: (handle: string, violations: ViolationHit[]) => void;
  clear: () => void;
}

function fromHistory(item: ChatHistoryMessage): ChatMessage {
  return {
    id: `srv-${item.id}`,
    // system은 UI에 표시 안 하지만 타입 좁히기 위해 assistant로 매핑.
    role: item.role === "user" ? "user" : "assistant",
    content: item.content,
    created_at: item.created_at,
  };
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

  async hydrate(studySlug, limit) {
    try {
      const items = await api.chatHistory(studySlug, limit ?? null, null);
      set({
        messages: items.map(fromHistory),
        streamingHandle: null,
        streamingMessageId: null,
      });
    } catch (e) {
      // 실패 시 메모리 그대로 — 사용자 입력 진행에 지장 없도록.
      console.error("chatStore.hydrate failed:", e);
    }
  },

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

  failStream(handle, message, jobId) {
    const { streamingHandle, streamingMessageId } = get();
    if (handle !== streamingHandle) return;
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === streamingMessageId
          ? {
              ...m,
              streaming: false,
              error: message,
              job_id: jobId,
            }
          : m,
      ),
      streamingHandle: null,
      streamingMessageId: null,
    }));
  },

  clearJobId(messageId) {
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === messageId ? { ...m, job_id: undefined } : m,
      ),
    }));
  },

  attachViolations(handle, violations) {
    const { streamingHandle, streamingMessageId, messages } = get();
    // chat:violation은 chat:done 직후라 streamingHandle이 *바로 직전 시점* finalizeStream으로 reset됨.
    // 다만 finalizeStream 호출 *전*에 emit될 수 있음 — 두 케이스 모두 커버.
    let targetId: string | null = null;
    if (handle === streamingHandle && streamingMessageId) {
      targetId = streamingMessageId;
    } else {
      // streamingHandle이 이미 reset됨 — 가장 최근 어시스턴트 메시지에 첨부.
      const last = [...messages].reverse().find((m) => m.role === "assistant");
      targetId = last?.id ?? null;
    }
    if (!targetId) return;
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === targetId ? { ...m, violations } : m,
      ),
    }));
  },

  clear: () =>
    set({
      messages: [],
      streamingHandle: null,
      streamingMessageId: null,
    }),
}));
