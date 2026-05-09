// ReportsPage — v0.5 PR 5 (D-102) 단위 테스트.
//
// 검증:
//   R5-A  활성 스터디 없으면 no_active_study 메시지 표시.
//   R5-B  활성 스터디 있으면 title 표시.
//   R5-C  SelfRatingForm — eligible=true 시 점수 입력 폼 표시.
//   R5-D  SelfRatingForm — eligible=false 시 not_eligible 메시지 표시.
//   R5-E  BatchReviewQueue — 신뢰도 낮은 facts가 있으면 체크박스 목록 표시.
//   R5-F  BatchReviewQueue — 데이터 없으면 empty 메시지 표시.

import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import type { Fact } from "@/lib/types";

// i18n mock.
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, _opts?: unknown) => key,
  }),
}));

// studyStore — 기본 활성 스터디 있음.
let mockActiveStudy: { slug: string; name: string } | null = { slug: "test-study", name: "테스트 스터디" };
vi.mock("@/store/studyStore", () => ({
  useStudyStore: (selector: (s: unknown) => unknown) =>
    selector({ active: mockActiveStudy }),
}));

// settingsStore mock.
vi.mock("@/store/settingsStore", () => ({
  useSettingsStore: (selector: (s: unknown) => unknown) =>
    selector({
      settings: {
        learning_dev_panel_enabled: false,
        learning_self_rating_log: [],
        first_run_at: null,
      },
    }),
}));

// TopBar mock — 복잡한 TopBar 의존성 제거.
vi.mock("@/components/TopBar", () => ({
  TopBar: () => <div data-testid="topbar" />,
}));

// MemoryPanelContent mock — 이미 별도 테스트 파일에서 검증됨.
vi.mock("@/components/MemoryPanelContent", () => ({
  MemoryPanelContent: ({ mode }: { mode?: string }) => (
    <div data-testid="memory-panel-content" data-mode={mode} />
  ),
}));

const mockLearningSelfRatingEligible = vi.fn();
const mockLearningSelfRatingRecord = vi.fn();
const mockMemoryFactsRecent = vi.fn();
const mockLearningAcceptanceMetrics = vi.fn();

vi.mock("@/lib/api", () => ({
  api: {
    learningSelfRatingEligible: () => mockLearningSelfRatingEligible(),
    learningSelfRatingRecord: (score: number) => mockLearningSelfRatingRecord(score),
    memoryFactsRecent: (...args: unknown[]) => mockMemoryFactsRecent(...args),
    learningAcceptanceMetrics: (...args: unknown[]) => mockLearningAcceptanceMetrics(...args),
  },
}));

function makeFact(overrides: Partial<Fact>): Fact {
  return {
    id: 1,
    study_id: "test-study",
    kind: "preference",
    content: "낮은 신뢰도 항목",
    source: "trigger",
    confidence: 0.3,
    status: "active",
    created_at: 1000,
    updated_at: 1000,
    ...overrides,
  };
}

// ReportsPage를 동적으로 import (vi.mock 이후에 import해야 mock이 적용됨).
const { ReportsPage } = await import("@/pages/ReportsPage");

describe("ReportsPage", () => {
  it("R5-A: 활성 스터디 없으면 no_active_study 메시지 표시", () => {
    mockActiveStudy = null;
    render(<ReportsPage />);
    expect(screen.queryByText("reports.no_active_study")).toBeTruthy();
    mockActiveStudy = { slug: "test-study", name: "테스트 스터디" };
  });

  it("R5-B: 활성 스터디 있으면 title 표시", async () => {
    mockLearningSelfRatingEligible.mockResolvedValue(false);
    mockMemoryFactsRecent.mockResolvedValue([]);
    render(<ReportsPage />);
    await waitFor(() => {
      expect(screen.queryByText("reports.title")).toBeTruthy();
    });
  });

  it("R5-C: SelfRatingForm — eligible=true 시 점수 입력 표시", async () => {
    mockLearningSelfRatingEligible.mockResolvedValue(true);
    mockMemoryFactsRecent.mockResolvedValue([]);
    render(<ReportsPage />);
    await waitFor(() => {
      // 폼 제출 버튼.
      expect(screen.queryByText("reports.self_rating.submit")).toBeTruthy();
    });
  });

  it("R5-D: SelfRatingForm — eligible=false 시 not_eligible 메시지 표시", async () => {
    mockLearningSelfRatingEligible.mockResolvedValue(false);
    mockMemoryFactsRecent.mockResolvedValue([]);
    render(<ReportsPage />);
    await waitFor(() => {
      expect(screen.queryByText("reports.self_rating.not_eligible")).toBeTruthy();
    });
  });

  it("R5-E: BatchReviewQueue — 낮은 신뢰도 facts 체크박스 목록 표시", async () => {
    mockLearningSelfRatingEligible.mockResolvedValue(false);
    mockMemoryFactsRecent.mockResolvedValue([
      makeFact({ id: 1, content: "아이템 A", confidence: 0.3 }),
      makeFact({ id: 2, content: "아이템 B", confidence: 0.2 }),
    ]);
    render(<ReportsPage />);
    await waitFor(() => {
      expect(screen.queryByText("아이템 A")).toBeTruthy();
      expect(screen.queryByText("아이템 B")).toBeTruthy();
    });
  });

  it("R5-F: BatchReviewQueue — 낮은 신뢰도 facts 없으면 empty 메시지", async () => {
    mockLearningSelfRatingEligible.mockResolvedValue(false);
    // 모두 high confidence → 필터 후 0건.
    mockMemoryFactsRecent.mockResolvedValue([
      makeFact({ id: 1, content: "높은 신뢰도", confidence: 0.9 }),
    ]);
    render(<ReportsPage />);
    await waitFor(() => {
      expect(screen.queryByText("reports.batch_review.empty")).toBeTruthy();
    });
  });

  it("R5-G: MemoryPanelContent가 editable 모드로 렌더됨", async () => {
    mockLearningSelfRatingEligible.mockResolvedValue(false);
    mockMemoryFactsRecent.mockResolvedValue([]);
    render(<ReportsPage />);
    await waitFor(() => {
      const panel = screen.queryByTestId("memory-panel-content");
      expect(panel).toBeTruthy();
      expect(panel?.getAttribute("data-mode")).toBe("editable");
    });
  });
});
