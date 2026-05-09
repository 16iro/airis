// ChatMessage — user/assistant 분기·markdown·에러·재시도 버튼 노출 조건.

import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { ChatMessage } from "@/components/ChatMessage";
import type { ChatMessage as ChatMsg } from "@/lib/types";

const { srsGenerateChunkSpy } = vi.hoisted(() => ({
  srsGenerateChunkSpy: vi.fn().mockResolvedValue({ inserted: [1], skipped: [] }),
}));

vi.mock("@/lib/api", () => ({
  api: {
    retryFailedJob: vi.fn(),
    srsGenerateChunk: srsGenerateChunkSpy,
  },
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
        message={makeMessage({
          role: "assistant",
          content: "**굵게** 텍스트",
          // v0.4.4.x followup §1.3 — provider가 명시되어야 "Claude" 라벨 출력.
          provider: "anthropic",
        })}
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

  // v0.4.4.x followup §1.3 — assistant 메시지에 provider별 라벨·강조 색이 붙는지.
  it("provider=anthropic이면 Claude 라벨 + sky 톤", () => {
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "응답",
          provider: "anthropic",
        })}
      />,
    );
    const label = screen.getByText("Claude");
    expect(label.className).toMatch(/text-sky/);
  });

  it("provider=openai면 ChatGPT 라벨 + lime 톤", () => {
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "응답",
          provider: "openai",
        })}
      />,
    );
    const label = screen.getByText("ChatGPT");
    expect(label.className).toMatch(/text-lime/);
  });

  it("provider=gemini면 Gemini 라벨 + orange 톤", () => {
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "응답",
          provider: "gemini",
        })}
      />,
    );
    const label = screen.getByText("Gemini");
    expect(label.className).toMatch(/text-orange/);
  });

  it("provider가 NULL이면 'Assistant' 폴백 (옛 row 호환)", () => {
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "응답",
          provider: null,
        })}
      />,
    );
    expect(screen.getByText("Assistant")).toBeInTheDocument();
  });

  // v0.5 PR 2 (D-099) — citation chip ⚡ 버튼이 노출되고 클릭 시 srsGenerateChunk 호출.
  it("v041_hybrid 칩에 ⚡ 버튼이 노출되고 클릭 시 srsGenerateChunk가 호출된다", async () => {
    srsGenerateChunkSpy.mockClear();
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
              { marker: "S1", chunk_id: 55, page: 7, section_path: "Ch01/§Intro" },
            ],
          },
        })}
      />,
    );
    const zapBtn = screen.getByRole("button", { name: /이 단락으로 카드 만들기/ });
    expect(zapBtn).toBeInTheDocument();
    fireEvent.click(zapBtn);
    // 비동기 처리 대기.
    await vi.waitFor(() => {
      expect(srsGenerateChunkSpy).toHaveBeenCalledWith("study-1", 55, true);
    });
  });

  // v0.4.3 PR 4 (D-090) — 의심 인용 칩이 경고 톤(노란색)으로 렌더되는지.
  it("citation_scores의 verdict가 low/no_match인 [Sx] 칩이 경고 톤으로 렌더된다", () => {
    jumpToSpy.mockClear();
    render(
      <ChatMessage
        message={makeMessage({
          role: "assistant",
          content: "본문에 따르면 [S1] [S2].",
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
              {
                book_id: "book-1",
                book_title: "Book 1",
                book_role: null,
                section_label: "§Other",
                section_path: "Ch02/§Other",
                page: 13,
              },
            ],
            v041_chunks: [
              { marker: "S1", chunk_id: 42, page: 7, section_path: "Ch01/§Intro" },
              { marker: "S2", chunk_id: 99, page: 13, section_path: "Ch02/§Other" },
            ],
            citation_scores: [
              { source_idx: 1, score: 0.8, verdict: "pass" },
              { source_idx: 2, score: 0.2, verdict: "no_match" },
            ],
          },
        })}
      />,
    );
    const passChip = screen.getByRole("button", { name: /S1/ });
    const lowChip = screen.getByRole("button", { name: /S2/ });
    // pass 칩 = primary 톤 (border-primary), 의심 칩 = amber 톤 (border-amber-500).
    expect(passChip.className).toContain("border-primary/30");
    expect(lowChip.className).toContain("border-amber-500/60");
    // hover title — 의심 칩에 안내 문구.
    expect(lowChip).toHaveAttribute(
      "title",
      "출처와 매칭 점수가 낮습니다. 인용이 자료와 일치하는지 직접 확인하세요.",
    );
  });
});
