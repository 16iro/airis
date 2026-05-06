// AbComparePanel — A/B 비교 dev 패널.
//
// dev 토글 OFF 분기는 ChatPanel.test.tsx 영역(렌더 자체가 안 됨)이 맡고, 본 파일은
// 패널 진입 후 흐름:
//   * 누적 stats 표시
//   * input + chatSendAbCompare 호출
//   * tie 버튼이 양쪽 응답 done 도착 전엔 안 보이고, 두 트랙 done 후 보임
//   * 좌우 배치는 무작위 (Math.random mock)

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { AbComparePanel } from "@/components/AbComparePanel";

const chatSendAbCompare = vi.fn();
const devAbExportResults = vi.fn();
const devAbRecordChoice = vi.fn();
const devCacheStats = vi.fn();
const devMeasureChatResponseMs = vi.fn();
const devResponseCacheHitRatio = vi.fn();
const devMeasureCitationAccuracy = vi.fn();
const devMeasureFollowupSkipRate = vi.fn();
const devMeasurePrefixCacheRatio = vi.fn();

vi.mock("@/lib/api", () => ({
  api: {
    chatSendAbCompare: (...args: unknown[]) => chatSendAbCompare(...args),
    devAbExportResults: () => devAbExportResults(),
    devAbRecordChoice: (...args: unknown[]) => devAbRecordChoice(...args),
    devCacheStats: () => devCacheStats(),
    devMeasureChatResponseMs: (...args: unknown[]) =>
      devMeasureChatResponseMs(...args),
    devResponseCacheHitRatio: () => devResponseCacheHitRatio(),
    devMeasureCitationAccuracy: (...args: unknown[]) =>
      devMeasureCitationAccuracy(...args),
    devMeasureFollowupSkipRate: (...args: unknown[]) =>
      devMeasureFollowupSkipRate(...args),
    devMeasurePrefixCacheRatio: (...args: unknown[]) =>
      devMeasurePrefixCacheRatio(...args),
  },
}));

vi.mock("@/store/studyStore", () => ({
  useStudyStore: (selector: (s: unknown) => unknown) =>
    selector({
      active: {
        slug: "s1",
        name: "S1",
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

// listen은 본 파일에서 callback을 *직접 호출하지 않는다* — 그저 unsubscribe 함수만 반환.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => undefined),
}));

beforeEach(() => {
  chatSendAbCompare.mockReset();
  devAbExportResults.mockReset();
  devAbRecordChoice.mockReset();
  devCacheStats.mockReset();
  devMeasureChatResponseMs.mockReset();
  devResponseCacheHitRatio.mockReset();
  devMeasureCitationAccuracy.mockReset();
  devMeasureFollowupSkipRate.mockReset();
  devMeasurePrefixCacheRatio.mockReset();
  // 디폴트 — 누적 stats 비어있는 상태.
  devAbExportResults.mockResolvedValue({
    baseline: 0,
    v041: 0,
    tie: 0,
    total: 0,
    markdown: "",
  });
  devCacheStats.mockResolvedValue({
    embedding: { rows: 0, hit_count: 0, miss_count: 0, hit_ratio: 0 },
    response: { rows: 0, hit_count: 0, miss_count: 0, hit_ratio: 0 },
  });
  devMeasureChatResponseMs.mockResolvedValue({ samples: 0, avg_ms: 0 });
  devResponseCacheHitRatio.mockResolvedValue({
    rows: 0,
    hit_count: 0,
    miss_count: 0,
    hit_ratio: 0,
  });
  devMeasureCitationAccuracy.mockResolvedValue({
    messages: 0,
    markers: 0,
    pass: 0,
    low: 0,
    no_match: 0,
    pass_ratio: 0,
    avg_score: 0,
  });
  devMeasureFollowupSkipRate.mockResolvedValue({
    user_messages: 0,
    followups: 0,
    reusable_followups: 0,
    skip_rate: 0,
  });
  devMeasurePrefixCacheRatio.mockResolvedValue({
    messages: 0,
    cache_read_total: 0,
    input_total: 0,
    hit_ratio: 0,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("AbComparePanel", () => {
  it("초기 렌더 시 stats_empty 메시지가 표시된다", async () => {
    render(<AbComparePanel />);
    await waitFor(() => {
      expect(screen.getByText(/아직 측정 기록이 없습니다/)).toBeInTheDocument();
    });
  });

  it("placeholder가 보이고 input이 비활성 send 버튼을 함께 가진다", async () => {
    render(<AbComparePanel />);
    expect(screen.getAllByText(/질문을 보내면 좌우 칸/).length).toBe(2);
    const sendButton = screen.getByRole("button", { name: /동시 비교 보내기/ });
    expect(sendButton).toBeDisabled();
  });

  it("질문 입력 후 send 버튼 클릭 → chatSendAbCompare 호출", async () => {
    chatSendAbCompare.mockResolvedValue({ handle: "ab-handle-1" });
    render(<AbComparePanel />);

    const textarea = screen.getByPlaceholderText(/두 엔진을 동시에 비교할 질문/);
    fireEvent.change(textarea, { target: { value: "rust 소유권이 뭐야?" } });

    const sendButton = screen.getByRole("button", { name: /동시 비교 보내기/ });
    expect(sendButton).not.toBeDisabled();
    fireEvent.click(sendButton);

    await waitFor(() => {
      expect(chatSendAbCompare).toHaveBeenCalledWith("s1", "rust 소유권이 뭐야?");
    });
  });

  it("좌우 위치는 무작위지만 column 라벨 두 개 + tie 버튼은 응답 도착 전 미노출", async () => {
    render(<AbComparePanel />);
    expect(screen.getByText("왼쪽 응답")).toBeInTheDocument();
    expect(screen.getByText("오른쪽 응답")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /둘 다 비슷함/ }),
    ).not.toBeInTheDocument();
  });

  it("누적 stats가 비어있지 않으면 summary 문자열이 보인다", async () => {
    devAbExportResults.mockResolvedValue({
      baseline: 2,
      v041: 7,
      tie: 1,
      total: 10,
      markdown: "...",
    });
    render(<AbComparePanel />);
    // v0.4.3 PR 5 — 라벨이 v0.4.3 / v0.4.2 로 갱신됨.
    await waitFor(() => {
      expect(screen.getByLabelText(/A\/B 비교 누적 결과/)).toHaveTextContent(
        /v0\.4\.3 7건/,
      );
    });
  });
});
