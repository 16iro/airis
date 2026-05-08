// ChatPanel 회귀 테스트 (v0.4.4.x followup):
//   §1.1 — 응답 중 send → 취소 버튼 + 입력 disabled
//   §1.2 — Enter 발사 / Shift+Enter 줄바꿈 / IME 조합 중 보호 / Cmd+Enter 호환

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { ChatPanel } from "@/components/ChatPanel";
import { useChatStore } from "@/store/chatStore";

// listen()은 Promise<UnlistenFn>을 반환 — 테스트에선 즉시 noop 해제 함수.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

const chatSendSpy = vi.fn<
  (slug: string, query: string, ctx: string | null) => Promise<{ handle: string }>
>(async () => ({ handle: "h-test" }));
const cancelChatStreamSpy = vi.fn<(handle: string) => Promise<void>>(
  async () => undefined,
);
const apiKeyPresentSpy = vi.fn<(provider: string) => Promise<boolean>>(
  async () => true,
);

vi.mock("@/lib/api", () => ({
  api: {
    chatSend: (slug: string, query: string, ctx: string | null) =>
      chatSendSpy(slug, query, ctx),
    cancelChatStream: (handle: string) => cancelChatStreamSpy(handle),
    apiKeyPresent: (provider: string) => apiKeyPresentSpy(provider),
    memoryDetectTriggers: vi.fn(async () => []),
  },
}));

// 활성 스터디 + 설정 — Enter/취소 분기에 필요한 값만 노출.
vi.mock("@/store/studyStore", () => ({
  useStudyStore: (selector: (s: unknown) => unknown) =>
    selector({
      active: {
        slug: "study-1",
        name: "Study 1",
        language: "ko",
        created_at: "2026-05-08",
        last_opened: null,
        is_active: true,
        book_count: 1,
        session_count: 0,
        thumbnail_path: null,
        description: null,
      },
    }),
}));
vi.mock("@/store/settingsStore", () => ({
  useSettingsStore: (selector: (s: unknown) => unknown) =>
    selector({
      settings: {
        active_provider: "anthropic",
        auth_mode: "cli",
        intervention_level: "off",
        dev_ab_compare: false,
        dev_event_log: false,
      },
    }),
}));
vi.mock("@/store/uiStore", () => ({
  useUiStore: (selector: (s: unknown) => unknown) =>
    selector({
      setSettingsOpen: vi.fn(),
    }),
}));

describe("ChatPanel — Enter/Shift/IME/Cmd 키 라우팅 (§1.2)", () => {
  beforeEach(() => {
    chatSendSpy.mockClear();
    cancelChatStreamSpy.mockClear();
    useChatStore.getState().clear();
  });
  afterEach(() => {
    useChatStore.getState().clear();
  });

  it("Enter 단독은 발사 — chat_send 호출", async () => {
    render(<ChatPanel />);
    const ta = await screen.findByPlaceholderText(/Enter 전송/);
    fireEvent.change(ta, { target: { value: "안녕" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    // chatSend는 비동기지만 fireEvent로 동기 trigger — flush 위해 microtask 대기.
    await Promise.resolve();
    expect(chatSendSpy).toHaveBeenCalledTimes(1);
  });

  it("Shift+Enter 는 줄바꿈 — chat_send 호출 X", async () => {
    render(<ChatPanel />);
    const ta = await screen.findByPlaceholderText(/Enter 전송/);
    fireEvent.change(ta, { target: { value: "안녕" } });
    fireEvent.keyDown(ta, { key: "Enter", shiftKey: true });
    await Promise.resolve();
    expect(chatSendSpy).not.toHaveBeenCalled();
  });

  it("Cmd+Enter 도 호환 발사 (기존 단축키 그대로)", async () => {
    render(<ChatPanel />);
    const ta = await screen.findByPlaceholderText(/Enter 전송/);
    fireEvent.change(ta, { target: { value: "안녕" } });
    fireEvent.keyDown(ta, { key: "Enter", metaKey: true });
    await Promise.resolve();
    expect(chatSendSpy).toHaveBeenCalledTimes(1);
  });

  it("IME 조합 중(Enter)은 발사 X — 한글 조합 확정 보호", async () => {
    render(<ChatPanel />);
    const ta = await screen.findByPlaceholderText(/Enter 전송/);
    fireEvent.change(ta, { target: { value: "ㅇㅏ" } });
    // KeyboardEvent.isComposing은 readonly. fireEvent.keyDown의 init에 박아서 nativeEvent에 반영.
    fireEvent.keyDown(ta, { key: "Enter", isComposing: true });
    await Promise.resolve();
    expect(chatSendSpy).not.toHaveBeenCalled();
  });
});

describe("ChatPanel — 취소 버튼 (§1.1)", () => {
  beforeEach(() => {
    chatSendSpy.mockClear();
    cancelChatStreamSpy.mockClear();
    useChatStore.getState().clear();
  });
  afterEach(() => {
    useChatStore.getState().clear();
  });

  it("streamingHandle이 set이면 textarea disabled + 취소 버튼 노출", () => {
    useChatStore.getState().beginAssistantStream("h-active", "anthropic");
    render(<ChatPanel />);
    const ta = screen.getByPlaceholderText(/Enter 전송/) as HTMLTextAreaElement;
    expect(ta).toBeDisabled();
    const cancelBtn = screen.getByRole("button", { name: /취소/ });
    expect(cancelBtn).toBeInTheDocument();
    // 그리고 send 버튼 자리는 없어야 함.
    expect(screen.queryByRole("button", { name: "보내기" })).not.toBeInTheDocument();
  });

  it("취소 버튼 클릭 → cancel_chat_stream invoke", () => {
    useChatStore.getState().beginAssistantStream("h-target", "anthropic");
    render(<ChatPanel />);
    const cancelBtn = screen.getByRole("button", { name: /취소/ });
    fireEvent.click(cancelBtn);
    expect(cancelChatStreamSpy).toHaveBeenCalledWith("h-target");
  });
});
