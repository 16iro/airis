// RecallChallengeDialog — v0.5 PR 4 (D-101) 단위 테스트.
//
// 검증:
//   R1-A  weak 모드: 마스킹 텍스트 + 텍스트 입력 + 정답 확인 시 "정답!" 표시.
//   R1-B  weak 모드: 오답 입력 시 "오답" 메시지 + recallRecordAttempt(incorrect) 호출.
//   R1-C  medium 모드: mc4 선택지 4개 렌더 + 정답 선택 시 correct 호출.
//   R1-D  나중에 버튼 / Esc → dismissed 기록.
//   R1-E  unmount 미결 시 dismissed 자동 기록.
//   R1-F  strong 모드: 30s 카운트다운 표시.
//   R1-G  확인 버튼 — 빈 입력 시 disabled.

import { describe, expect, it, vi, afterEach, beforeEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

import { RecallChallengeDialog } from "@/components/RecallChallengeDialog";
import type { RecallChallenge } from "@/lib/types";

// ---------- api mock ----------------------------------------------------------
const recordAttemptSpy = vi.fn(async () => undefined);
vi.mock("@/lib/api", () => ({
  api: {
    recallRecordAttempt: (...args: unknown[]) => recordAttemptSpy(...args),
  },
}));

// ---------- i18n mock ---------------------------------------------------------
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

// ---------- helpers ----------------------------------------------------------
function weakChallenge(): RecallChallenge {
  return {
    trigger_id: "tid-001",
    chunk_id: 42,
    strength: "weak",
    masked_text: "사과는 ___ 색이다.",
    answer: "빨간",
    mc4_options: null,
  };
}

function mediumChallenge(): RecallChallenge {
  return {
    trigger_id: "tid-002",
    chunk_id: 43,
    strength: "medium",
    masked_text: "물의 화학식은?",
    answer: "H2O",
    mc4_options: ["CO2", "H2O", "O2", "NaCl"],
  };
}

function strongChallenge(): RecallChallenge {
  return {
    trigger_id: "tid-003",
    chunk_id: 44,
    strength: "strong",
    masked_text: "지구에서 가장 큰 대륙은?",
    answer: "아시아",
    mc4_options: null,
  };
}

afterEach(() => {
  recordAttemptSpy.mockClear();
  vi.useRealTimers();
});

// ---------- tests ------------------------------------------------------------
describe("RecallChallengeDialog", () => {
  it("R1-A: weak 정답 입력 시 correct 기록 및 정답! 표시", async () => {
    const onClose = vi.fn();
    render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={weakChallenge()}
        onClose={onClose}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "빨간" } });

    const submitBtn = screen.getByText("recall.dialog.submit");
    await act(async () => {
      fireEvent.click(submitBtn);
      await Promise.resolve();
    });

    expect(recordAttemptSpy).toHaveBeenCalledWith(
      "s1",
      42,
      "tid-001",
      "weak",
      "correct",
    );
    expect(screen.getByText("recall.dialog.correct")).toBeTruthy();
  });

  it("R1-B: weak 오답 입력 시 incorrect 기록", async () => {
    const onClose = vi.fn();
    render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={weakChallenge()}
        onClose={onClose}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "파란" } });

    const submitBtn = screen.getByText("recall.dialog.submit");
    await act(async () => {
      fireEvent.click(submitBtn);
      await Promise.resolve();
    });

    expect(recordAttemptSpy).toHaveBeenCalledWith(
      "s1",
      42,
      "tid-001",
      "weak",
      "incorrect",
    );
    expect(screen.getByText("recall.dialog.incorrect")).toBeTruthy();
  });

  it("R1-C: medium 정답 선택지 클릭 시 correct 기록", async () => {
    const onClose = vi.fn();
    render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={mediumChallenge()}
        onClose={onClose}
      />,
    );

    // 선택지 4개 렌더 확인.
    expect(screen.getByText("CO2")).toBeTruthy();
    expect(screen.getByText("H2O")).toBeTruthy();
    expect(screen.getByText("O2")).toBeTruthy();
    expect(screen.getByText("NaCl")).toBeTruthy();

    // 정답 선택.
    fireEvent.click(screen.getByText("H2O"));

    const submitBtn = screen.getByText("recall.dialog.submit");
    await act(async () => {
      fireEvent.click(submitBtn);
      await Promise.resolve();
    });

    expect(recordAttemptSpy).toHaveBeenCalledWith(
      "s1",
      43,
      "tid-002",
      "medium",
      "correct",
    );
  });

  it("R1-D: 나중에 버튼 클릭 시 dismissed 기록", async () => {
    const onClose = vi.fn();
    render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={weakChallenge()}
        onClose={onClose}
      />,
    );

    await act(async () => {
      fireEvent.click(screen.getByText("recall.dialog.dismiss"));
      await Promise.resolve();
    });

    expect(recordAttemptSpy).toHaveBeenCalledWith(
      "s1",
      42,
      "tid-001",
      "weak",
      "dismissed",
    );
    expect(onClose).toHaveBeenCalledWith("dismissed");
  });

  it("R1-D: Esc 키 → dismissed 기록", async () => {
    const onClose = vi.fn();
    render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={weakChallenge()}
        onClose={onClose}
      />,
    );

    await act(async () => {
      fireEvent.keyDown(window, { key: "Escape" });
      await Promise.resolve();
    });

    expect(recordAttemptSpy).toHaveBeenCalledWith(
      "s1",
      42,
      "tid-001",
      "weak",
      "dismissed",
    );
  });

  it("R1-E: unmount 미결 시 dismissed 자동 기록", async () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={weakChallenge()}
        onClose={onClose}
      />,
    );

    await act(async () => {
      unmount();
      await Promise.resolve();
    });

    expect(recordAttemptSpy).toHaveBeenCalledWith(
      "s1",
      42,
      "tid-001",
      "weak",
      "dismissed",
    );
  });

  it("R1-F: strong 모드 — 카운트다운 숫자 표시", () => {
    vi.useFakeTimers();
    const onClose = vi.fn();
    render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={strongChallenge()}
        onClose={onClose}
      />,
    );
    // 초기 30s 표시.
    expect(screen.getByText("30s")).toBeTruthy();
  });

  it("R1-G: 빈 입력 시 확인 버튼 disabled", () => {
    const onClose = vi.fn();
    render(
      <RecallChallengeDialog
        studySlug="s1"
        challenge={weakChallenge()}
        onClose={onClose}
      />,
    );

    const submitBtn = screen.getByText("recall.dialog.submit").closest("button");
    expect(submitBtn).toBeDefined();
    expect(submitBtn?.disabled).toBe(true);
  });
});
