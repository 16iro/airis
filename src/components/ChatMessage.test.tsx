// ChatMessage — user/assistant 분기·markdown·에러·재시도 버튼 노출 조건.

import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

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

// v0.4.1 PR 4 — 칩 클릭 → activeBookStore.jumpTo(...). spy로 호출 검증.
const jumpToSpy = vi.fn();
vi.mock("@/store/activeBookStore", () => ({
  useActiveBookStore: (selector: (s: unknown) => unknown) =>
    selector({
      jumpTo: jumpToSpy,
    }),
}));

// 활성 스터디는 mock 고정값.
vi.mock("@/store/studyStore", () => ({
  useStudyStore: (selector: (s: unknown) => unknown) =>
    selector({
      active: {
        slug: "study-1",
        name: "Study 1",
        language: "ko",
        created_at: "2026-05-06",
        last_opened: null,
        is_active: true,
        book_count: 1,
        session_count: 0,
        thumbnail_path: null,
        description: null,
      },
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

  // v0.4.1 PR 4 — v041_hybrid 컨텍스트면 클릭 가능 [Sx] 칩이 렌더되고, 클릭 시 jumpTo 호출.
  it("v041_hybrid + v041_chunks가 있으면 클릭 가능 [Sx] 칩이 렌더된다", () => {
    jumpToSpy.mockClear();
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "본문에 따르면 [S1].",
          context: {
            kind: "v041_hybrid",
            hits: [
              {
                book_id: "book-1",
                book_title: "Book 1",
                book_role: null,
                section_label: "§Intro",
                section_path: "Ch01/§Intro",
                page: 7,
              },
            ],
            v041_chunks: [
              {
                marker: "S1",
                chunk_id: 42,
                page: 7,
                section_path: "Ch01/§Intro",
              },
            ],
          },
        })}
      />,
    );
    const chip = screen.getByRole("button", { name: /S1/ });
    expect(chip).toBeInTheDocument();
    fireEvent.click(chip);
    expect(jumpToSpy).toHaveBeenCalledWith(
      "study-1",
      "book-1",
      "Ch01/§Intro",
      7,
    );
  });

  // v041_hybrid이지만 v041_chunks가 비면 클릭 칩이 아니라 일반 hits 칩 fallback.
  it("v041_hybrid + v041_chunks 비어있으면 클릭 칩이 렌더되지 않는다", () => {
    jumpToSpy.mockClear();
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "응답",
          context: {
            kind: "v041_hybrid",
            hits: [],
            v041_chunks: [],
          },
        })}
      />,
    );
    expect(screen.queryByRole("button", { name: /S1/ })).not.toBeInTheDocument();
  });
});
