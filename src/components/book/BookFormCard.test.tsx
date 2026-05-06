// v0.4.2 PR 3 — BookCard 일시정지/재개/취소 버튼 + pause_reason 라벨 단위 테스트.
//
// 검증:
//   * `indexing` + jobId 있을 때 일시정지 버튼이 노출되고 클릭 시 onPauseIndexing 호출.
//   * `paused` 상태일 때 재개 버튼이 노출되고 pause_reason 라벨(예: "사용자 일시정지")이 함께 표시.
//   * `paused` 상태일 때 취소 버튼이 함께 노출.
//
// i18n은 vitest.setup.ts에서 ko.json 로드.

import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { BookCard } from "@/components/book/BookFormCard";

const draft = {
  id: "b-test",
  path: "/x/y.md",
  title: "테스트 책",
  author: "",
  roleNote: "",
};

describe("BookCard 일시정지/재개/취소", () => {
  it("indexing 상태 + jobId면 일시정지 버튼 노출 + onPauseIndexing 호출", () => {
    const onPause = vi.fn();
    render(
      <BookCard
        book={draft}
        kind="main"
        disabled={false}
        removable={false}
        indexingStatus={{ state: "indexing", percent: 50, step: "embed_t2", jobId: 42 }}
        onPauseIndexing={onPause}
      />,
    );
    const btn = screen.getByLabelText("일시정지");
    fireEvent.click(btn);
    expect(onPause).toHaveBeenCalledWith(42);
  });

  it("paused 상태면 재개 버튼 노출 + pause_reason 라벨 표시", () => {
    const onResume = vi.fn();
    render(
      <BookCard
        book={draft}
        kind="main"
        disabled={false}
        removable={false}
        indexingStatus={{
          state: "paused",
          percent: 50,
          jobId: 99,
          pauseReason: "battery_low",
        }}
        onResumeIndexing={onResume}
      />,
    );
    // pause_reason 라벨이 한국어로 표시되는지 — ko.json의 pause_reason.battery_low.
    expect(screen.getByText(/배터리 부족 자동 일시정지/)).toBeInTheDocument();

    const btn = screen.getByLabelText("재개");
    fireEvent.click(btn);
    expect(onResume).toHaveBeenCalledWith(99);
  });

  it("user pause는 '사용자 일시정지' 라벨", () => {
    render(
      <BookCard
        book={draft}
        kind="main"
        disabled={false}
        removable={false}
        indexingStatus={{
          state: "paused",
          percent: 30,
          jobId: 7,
          pauseReason: "user",
        }}
      />,
    );
    expect(screen.getByText(/사용자 일시정지/)).toBeInTheDocument();
  });

  it("paused 상태에서 취소 버튼 노출 + onCancelIndexing 호출", () => {
    const onCancel = vi.fn();
    render(
      <BookCard
        book={draft}
        kind="main"
        disabled={false}
        removable={false}
        indexingStatus={{
          state: "paused",
          percent: 60,
          jobId: 13,
          pauseReason: "user",
        }}
        onCancelIndexing={onCancel}
      />,
    );
    const btn = screen.getByLabelText("취소");
    fireEvent.click(btn);
    expect(onCancel).toHaveBeenCalledWith(13);
  });

  it("done 상태면 일시정지/재개/취소 버튼 모두 미노출", () => {
    render(
      <BookCard
        book={draft}
        kind="main"
        disabled={false}
        removable={false}
        indexingStatus={{ state: "done" }}
        onPauseIndexing={vi.fn()}
        onResumeIndexing={vi.fn()}
        onCancelIndexing={vi.fn()}
      />,
    );
    expect(screen.queryByLabelText("일시정지")).toBeNull();
    expect(screen.queryByLabelText("재개")).toBeNull();
    expect(screen.queryByLabelText("취소")).toBeNull();
  });
});
