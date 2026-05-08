// chatStore — chunk append 라이프사이클 (BUG-001/002 회귀 가드).
//
// v0.4.4 PR 1 (D-091): backend가 chunk 1번 emit하면 store도 정확히 1번 append해야 함.
// frontend listener race로 같은 chunk가 N회 들어와도 *streamingHandle 가드*가 핸들 일치
// 시점만 append하므로, 같은 핸들로는 들어온 만큼 누적된다. 본 테스트는 *정상 흐름*에서
// chunk 5건 append → 단일 메시지 1개 + 정확한 누적값 검증. listener 중복 fix는
// ChatPanel useEffect 쪽 책임 (별도 흐름).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useChatStore } from "@/store/chatStore";

const cancelChatStreamSpy = vi.fn<(handle: string) => Promise<void>>(
  async () => undefined,
);

vi.mock("@/lib/api", () => ({
  api: {
    chatHistory: vi.fn(async () => []),
    cancelChatStream: (handle: string) => cancelChatStreamSpy(handle),
  },
}));

describe("chatStore stream lifecycle", () => {
  beforeEach(() => {
    useChatStore.getState().clear();
  });

  afterEach(() => {
    useChatStore.getState().clear();
  });

  it("appends 5 chunks into a single assistant message in order", () => {
    const store = useChatStore.getState();
    store.beginAssistantStream("h-1");
    const chunks = ["안녕", "하세요 ", "16비트", " 레지스터", "입니다."];
    for (const c of chunks) {
      // 매 호출마다 fresh state 읽기 (zustand는 mutate 후 새 snapshot 반환).
      useChatStore.getState().appendChunk("h-1", c);
    }
    const messages = useChatStore.getState().messages;
    const assistantMsgs = messages.filter((m) => m.role === "assistant");
    expect(assistantMsgs).toHaveLength(1);
    expect(assistantMsgs[0]?.content).toBe(
      "안녕하세요 16비트 레지스터입니다.",
    );
  });

  it("ignores chunks whose handle does not match streamingHandle", () => {
    // 가드 동작: 다른 핸들의 stale chunk(=이전 stream의 후행 이벤트 등)는 무시.
    const store = useChatStore.getState();
    store.beginAssistantStream("active-handle");
    useChatStore.getState().appendChunk("stale-handle", "버려야 함");
    useChatStore.getState().appendChunk("active-handle", "정상");
    const list = useChatStore.getState().messages;
    const last = [...list].reverse().find((m) => m.role === "assistant");
    expect(last?.content).toBe("정상");
  });

  it("finalizeStream resets streamingHandle/streamingMessageId", () => {
    const store = useChatStore.getState();
    store.beginAssistantStream("h-x");
    useChatStore.getState().appendChunk("h-x", "hi");
    useChatStore.getState().finalizeStream("h-x", {
      input_tokens: 1,
      output_tokens: 2,
      cache_creation_input_tokens: 0,
      cache_read_input_tokens: 0,
    });
    const s = useChatStore.getState();
    expect(s.streamingHandle).toBeNull();
    expect(s.streamingMessageId).toBeNull();
    const last = [...s.messages].reverse().find((m) => m.role === "assistant");
    expect(last?.streaming).toBe(false);
    expect(last?.content).toBe("hi");
  });

  it("cancelStream invokes backend command with given handle", async () => {
    cancelChatStreamSpy.mockClear();
    const store = useChatStore.getState();
    store.beginAssistantStream("h-cancel");
    await useChatStore.getState().cancelStream("h-cancel");
    expect(cancelChatStreamSpy).toHaveBeenCalledWith("h-cancel");
  });

  it("cancelStream falls back to failStream when invoke rejects", async () => {
    cancelChatStreamSpy.mockClear();
    cancelChatStreamSpy.mockImplementationOnce(async () => {
      throw new Error("invoke boom");
    });
    const store = useChatStore.getState();
    store.beginAssistantStream("h-fail");
    await useChatStore.getState().cancelStream("h-fail");
    const s = useChatStore.getState();
    expect(s.streamingHandle).toBeNull();
    const last = [...s.messages].reverse().find((m) => m.role === "assistant");
    expect(last?.error).toBe("사용자 취소");
    expect(last?.streaming).toBe(false);
  });

  it("appendChunk after finalize is a no-op (guard prevents zombie chunks)", () => {
    // BUG-001/002 회귀: stream 종료 후에도 listener 중복으로 chunk가 더 들어오는 케이스.
    // streamingHandle이 null이라 append가 막혀야 함.
    const store = useChatStore.getState();
    store.beginAssistantStream("h-z");
    useChatStore.getState().appendChunk("h-z", "valid");
    useChatStore.getState().finalizeStream("h-z", {
      input_tokens: 0,
      output_tokens: 0,
      cache_creation_input_tokens: 0,
      cache_read_input_tokens: 0,
    });
    // 종료 후 zombie chunk 도착.
    useChatStore.getState().appendChunk("h-z", "zombie");
    const list = useChatStore.getState().messages;
    const last = [...list].reverse().find((m) => m.role === "assistant");
    expect(last?.content).toBe("valid");
  });
});
