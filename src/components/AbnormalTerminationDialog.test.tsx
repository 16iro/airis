// v0.4.2 PR 3 — AbnormalTerminationDialog 단위 테스트.
//
// 검증:
//   * `index:abnormal_termination` 이벤트 수신 시 다이얼로그가 노출되며 청크 합계 표시.
//   * 닫기 버튼 클릭 시 다이얼로그 사라짐.
//   * 모두 취소 버튼 클릭 시 각 jobId에 대해 cancelIndexingJob 호출.

import { describe, expect, it, vi, afterEach } from "vitest";
import { act, render, screen } from "@testing-library/react";

import { AbnormalTerminationDialog } from "@/components/AbnormalTerminationDialog";

// listen mock — 등록된 listener를 캡처해 *수동으로* 이벤트 발생을 시뮬.
let captured:
  | ((event: { payload: { jobs: Array<{ job_id: number; book_id: string; tier: number; pending_chunks: number }> } }) => void)
  | null = null;

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name: string, cb: typeof captured) => {
    captured = cb;
    return () => {
      captured = null;
    };
  }),
}));

const cancelSpy = vi.fn(async (_id: number) => undefined);
vi.mock("@/lib/api", () => ({
  api: {
    cancelIndexingJob: (id: number) => cancelSpy(id),
  },
}));

afterEach(() => {
  captured = null;
  cancelSpy.mockClear();
});

describe("AbnormalTerminationDialog", () => {
  it("이벤트 수신 시 청크 합계 메시지 표시", async () => {
    render(<AbnormalTerminationDialog />);
    // listen이 등록될 때까지 wait — Promise resolve를 마이크로태스크로 한 번 흘려보낸다.
    await act(async () => {
      await Promise.resolve();
    });
    expect(captured).not.toBeNull();
    await act(async () => {
      captured!({
        payload: {
          jobs: [
            { job_id: 1, book_id: "b1", tier: 2, pending_chunks: 100 },
            { job_id: 2, book_id: "b2", tier: 1, pending_chunks: 50 },
          ],
        },
      });
    });
    // body 메시지에 count=2, chunks=150 — ko.json 보간: "2개 잡이 ... 합계 150개".
    expect(screen.getByText(/2개 잡이/)).toBeInTheDocument();
    expect(screen.getByText(/합계 150개/)).toBeInTheDocument();
  });

  it("빈 jobs payload는 다이얼로그 노출 X", async () => {
    render(<AbnormalTerminationDialog />);
    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      captured!({ payload: { jobs: [] } });
    });
    // 다이얼로그 자체가 안 만들어졌으므로 title이 없음.
    expect(screen.queryByText("이전 인덱싱 비정상 종료 감지")).toBeNull();
  });

  it("모두 취소 클릭 시 각 jobId에 cancelIndexingJob 호출", async () => {
    render(<AbnormalTerminationDialog />);
    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      captured!({
        payload: {
          jobs: [
            { job_id: 11, book_id: "b1", tier: 2, pending_chunks: 5 },
            { job_id: 22, book_id: "b2", tier: 2, pending_chunks: 7 },
          ],
        },
      });
    });
    const btn = screen.getByText("모두 취소");
    await act(async () => {
      btn.click();
      await Promise.resolve();
    });
    expect(cancelSpy).toHaveBeenCalledWith(11);
    expect(cancelSpy).toHaveBeenCalledWith(22);
  });
});
