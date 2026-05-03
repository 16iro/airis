// ChatMessage — user/assistant 분기·markdown·에러·재시도 버튼 노출 조건.

import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import { ChatMessage } from "@/components/ChatMessage";
import type { ChatMessage as ChatMsg } from "@/lib/types";

vi.mock("@/lib/api", () => ({
  api: { retryFailedJob: vi.fn() },
}));

vi.mock("@/store/chatStore", () => ({
  useChatStore: (selector: (s: unknown) => unknown) =>
    selector({
      beginAssistantStream: vi.fn(),
      clearJobId: vi.fn(),
    }),
}));

function makeMessage(overrides: Partial<ChatMsg>): ChatMsg {
  return {
    id: "m1",
    role: "user",
    content: "안녕하세요",
    created_at: new Date().toISOString(),
    ...overrides,
  };
}

describe("ChatMessage", () => {
  it("user 메시지는 본문을 그대로 whitespace-pre-wrap으로 렌더한다", () => {
    render(<ChatMessage message={makeMessage({ role: "user", content: "줄1\n줄2" })} />);
    expect(screen.getByText(/줄1/)).toBeInTheDocument();
    expect(screen.getByText("사용자")).toBeInTheDocument();
  });

  it("assistant 메시지는 markdown으로 렌더된다 (강조 처리됨)", () => {
    render(
      <ChatMessage
        message={makeMessage({ role: "assistant", content: "**굵게** 텍스트" })}
      />,
    );
    // ReactMarkdown이 **굵게**를 <strong>으로 렌더 → 한 노드만 잡히면 됨.
    const strong = screen.getByText("굵게");
    expect(strong.tagName.toLowerCase()).toBe("strong");
    expect(screen.getByText("Claude")).toBeInTheDocument();
  });

  it("streaming=true면 '응답 생성 중…' 표시", () => {
    render(
      <ChatMessage
        message={makeMessage({ role: "assistant", content: "", streaming: true })}
      />,
    );
    expect(screen.getByText(/응답 생성 중/)).toBeInTheDocument();
  });

  it("error만 있고 job_id가 없으면 재시도 버튼이 안 나온다", () => {
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "",
          error: "네트워크 오류",
        })}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("네트워크 오류");
    expect(screen.queryByRole("button", { name: /다시 시도/ })).not.toBeInTheDocument();
  });

  it("error + job_id가 함께 있으면 재시도 버튼이 노출된다", () => {
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "",
          error: "네트워크 오류",
          job_id: 42,
        })}
      />,
    );
    expect(screen.getByRole("button", { name: /다시 시도/ })).toBeInTheDocument();
  });
});
