// 챗 메시지 + 스트리밍 상태.
// v0.2부터 chat_messages 테이블에 영속 — 부팅 시 hydrate로 복원.

import { create } from "zustand";

import { api } from "@/lib/api";
import type {
  ChatContextSummary,
  ChatHistoryMessage,
  ChatMessage,
  ChatSession,
  Usage,
  ViolationHit,
} from "@/lib/types";

interface ChatStore {
  messages: ChatMessage[];
  /** 진행 중 메시지의 handle → message id 매핑. */
  streamingHandle: string | null;
  streamingMessageId: string | null;

  // v0.6.x (D-113~D-115) — 세션 분리.
  /** 현재 스터디의 세션 목록 (최근 갱신 순). */
  sessions: ChatSession[];
  /** 활성 세션 id. null이면 아직 세션 없음(첫 전송 시 lazy 생성). */
  activeSessionId: string | null;
  /** 세션 목록 새로고침. */
  refreshSessions: (studySlug: string) => Promise<void>;
  /** "새 대화" — 새 세션 생성 + 활성화 + 메시지 비움. 직전 빈 세션은 정리. */
  newSession: (studySlug: string) => Promise<void>;
  /** 세션 전환 — 직전 빈 세션 정리 + 해당 세션 히스토리 로드. */
  selectSession: (studySlug: string, sessionId: string) => Promise<void>;
  /** 세션 제목 수동 변경. */
  renameSession: (sessionId: string, title: string) => Promise<void>;
  /** 세션 삭제 — 활성 세션이면 최근 세션으로 전환. */
  deleteSession: (studySlug: string, sessionId: string) => Promise<void>;
  /** chat:session_titled 이벤트 — 목록 제목 갱신. */
  applySessionTitle: (sessionId: string, title: string) => void;
  /** 전송 직전 활성 세션 보장 — 없으면 생성하고 id 반환. */
  ensureActiveSession: (studySlug: string) => Promise<string>;

  /** 활성 스터디의 세션 + 최근 세션 메시지를 백엔드에서 로드. */
  hydrate: (studySlug: string, limit?: number) => Promise<void>;
  addUserMessage: (content: string) => string;
  /** v0.4.4.x followup §1.3 — provider는 active_provider id (`anthropic`·`openai`·`gemini`).
   *  호출자가 settings에서 읽어 박아 보낸다. UI는 이 값으로 발신자 라벨·색을 결정. */
  beginAssistantStream: (handle: string, provider?: string | null) => string;
  appendChunk: (handle: string, text: string) => void;
  finalizeStream: (handle: string, usage: Usage) => void;
  failStream: (handle: string, message: string, jobId?: number) => void;
  /** v0.4.4.x followup §1.1 — 진행 중 스트리밍을 사용자가 명시 취소.
   *  invoke('cancel_chat_stream') → backend가 ChildGuard.drop으로 CLI subprocess SIGKILL.
   *  invoke가 끝나면 backend가 chat:error{kind:"ChatCancelled"}을 emit해 listener가
   *  failStream을 호출 — UI는 자연스럽게 "취소됨" 톤으로 렌더. invoke 자체 실패 시엔
   *  방어적으로 즉시 failStream을 호출해 UI가 영원히 streaming 상태에 갇히는 사태를 막는다. */
  cancelStream: (handle: string) => Promise<void>;
  /** 메시지의 job_id를 비움 — 사용자가 재시도 클릭 시 사용. */
  clearJobId: (messageId: string) => void;
  /** 진행 중 어시스턴트 메시지에 검증 위반 의심 hits 첨부 — chat:violation event. */
  attachViolations: (handle: string, violations: ViolationHit[]) => void;
  /** v0.3.2 B1: 진행 중 어시스턴트 메시지에 컨텍스트 요약 첨부 — chat:context event. */
  attachContext: (handle: string, context: ChatContextSummary) => void;
  clear: () => void;
}

function fromHistory(item: ChatHistoryMessage): ChatMessage {
  return {
    id: `srv-${item.id}`,
    // system은 UI에 표시 안 하지만 타입 좁히기 위해 assistant로 매핑.
    role: item.role === "user" ? "user" : "assistant",
    content: item.content,
    context: item.context,
    provider: item.provider ?? inferProviderFromModel(item.model),
    created_at: item.created_at,
  };
}

/** v0.4.4.x followup §1.3 — provider 컬럼이 NULL인 *옛 row* 폴백.
 *  v18 마이그가 백필했지만, 마이그 시점에 model이 NULL이거나 매칭 prefix가 아닌 경우엔
 *  여전히 NULL로 남는다. UI 표시는 가능한 만큼 model prefix로 살려준다. */
function inferProviderFromModel(model: string | null): string | null {
  if (!model) return null;
  if (model.startsWith("claude-")) return "anthropic";
  if (
    model.startsWith("gpt-") ||
    model.startsWith("o1-") ||
    model.startsWith("o3-")
  ) {
    return "openai";
  }
  if (model.startsWith("gemini-")) return "gemini";
  return null;
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
  sessions: [],
  activeSessionId: null,

  async hydrate(studySlug, limit) {
    try {
      const sessions = await api.chatSessionsList(studySlug);
      // D-113: 진입 시 가장 최근 세션 이어보기. 없으면 활성 세션 없음(첫 전송 시 lazy 생성).
      const active = sessions[0]?.id ?? null;
      const items = active
        ? await api.chatHistory(studySlug, active, limit ?? null, null)
        : [];
      set({
        sessions,
        activeSessionId: active,
        messages: items.map(fromHistory),
        streamingHandle: null,
        streamingMessageId: null,
      });
    } catch (e) {
      // 실패 시 메모리 그대로 — 사용자 입력 진행에 지장 없도록.
      console.error("chatStore.hydrate failed:", e);
    }
  },

  async refreshSessions(studySlug) {
    try {
      const sessions = await api.chatSessionsList(studySlug);
      set({ sessions });
    } catch (e) {
      console.error("chatStore.refreshSessions failed:", e);
    }
  },

  async newSession(studySlug) {
    // 직전 활성 세션이 비어있으면 정리 (D-113 빈 세션 자동 삭제).
    const prev = get().activeSessionId;
    if (prev) {
      try {
        await api.chatSessionDeleteIfEmpty(prev);
      } catch (e) {
        console.error("chatStore.newSession cleanup failed:", e);
      }
    }
    try {
      const created = await api.chatSessionCreate(studySlug);
      const sessions = await api.chatSessionsList(studySlug);
      set({
        sessions,
        activeSessionId: created.id,
        messages: [],
        streamingHandle: null,
        streamingMessageId: null,
      });
    } catch (e) {
      console.error("chatStore.newSession failed:", e);
    }
  },

  async selectSession(studySlug, sessionId) {
    const prev = get().activeSessionId;
    if (prev && prev !== sessionId) {
      try {
        await api.chatSessionDeleteIfEmpty(prev);
      } catch (e) {
        console.error("chatStore.selectSession cleanup failed:", e);
      }
    }
    try {
      const items = await api.chatHistory(studySlug, sessionId, null, null);
      const sessions = await api.chatSessionsList(studySlug);
      set({
        sessions,
        activeSessionId: sessionId,
        messages: items.map(fromHistory),
        streamingHandle: null,
        streamingMessageId: null,
      });
    } catch (e) {
      console.error("chatStore.selectSession failed:", e);
    }
  },

  async renameSession(sessionId, title) {
    try {
      await api.chatSessionRename(sessionId, title);
      set((s) => ({
        sessions: s.sessions.map((x) =>
          x.id === sessionId ? { ...x, title } : x,
        ),
      }));
    } catch (e) {
      console.error("chatStore.renameSession failed:", e);
    }
  },

  async deleteSession(studySlug, sessionId) {
    try {
      await api.chatSessionDelete(sessionId);
    } catch (e) {
      console.error("chatStore.deleteSession failed:", e);
      return;
    }
    const wasActive = get().activeSessionId === sessionId;
    const sessions = await api.chatSessionsList(studySlug).catch(() => []);
    if (wasActive) {
      const next = sessions[0]?.id ?? null;
      const items = next
        ? await api.chatHistory(studySlug, next, null, null).catch(() => [])
        : [];
      set({
        sessions,
        activeSessionId: next,
        messages: items.map(fromHistory),
      });
    } else {
      set({ sessions });
    }
  },

  applySessionTitle(sessionId, title) {
    set((s) => ({
      sessions: s.sessions.map((x) =>
        x.id === sessionId ? { ...x, title } : x,
      ),
    }));
  },

  async ensureActiveSession(studySlug) {
    const current = get().activeSessionId;
    if (current) return current;
    const created = await api.chatSessionCreate(studySlug);
    const sessions = await api.chatSessionsList(studySlug).catch(() => get().sessions);
    set({ sessions, activeSessionId: created.id });
    return created.id;
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

  beginAssistantStream(handle, provider) {
    const id = nextId();
    const message: ChatMessage = {
      id,
      role: "assistant",
      content: "",
      streaming: true,
      provider: provider ?? null,
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

  async cancelStream(handle) {
    try {
      await api.cancelChatStream(handle);
      // 성공 — backend가 곧 chat:error{kind:"ChatCancelled"} emit. listener가 failStream 처리.
      // 만약 emit이 빠르게 도착하지 않으면 대비해, 짧은 grace 후에도 streamingHandle이
      // 그대로면 방어적으로 failStream 호출 (race 보험).
      const message = "사용자 취소";
      // 약 200ms 후 점검. setTimeout — vitest fake timers 호환 위해 setTimeout 사용.
      window.setTimeout(() => {
        const s = useChatStore.getState();
        if (s.streamingHandle === handle) {
          s.failStream(handle, message);
        }
      }, 200);
    } catch (e) {
      // invoke 자체 실패 — 그래도 사용자 의도는 명확히 반영해야 한다.
      console.error("cancelChatStream invoke failed:", e);
      const message = "사용자 취소";
      get().failStream(handle, message);
    }
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

  attachContext(handle, context) {
    // chat:context는 stream 시작 직전 emit. 프론트는 beginAssistantStream 직후 처리.
    // streamingHandle이 일치하면 streaming 메시지에, 아니면 가장 최근 어시스턴트에 (방어적).
    const { streamingHandle, streamingMessageId, messages } = get();
    let targetId: string | null = null;
    if (handle === streamingHandle && streamingMessageId) {
      targetId = streamingMessageId;
    } else {
      const last = [...messages].reverse().find((m) => m.role === "assistant");
      targetId = last?.id ?? null;
    }
    if (!targetId) return;
    set((s) => ({
      messages: s.messages.map((m) =>
        m.id === targetId ? { ...m, context } : m,
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
