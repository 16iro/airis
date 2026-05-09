// MetacogAlertToast — v0.5 PR 3 (D-100) 단위 테스트.
//
// 검증:
//   D2-A  metacog:alert 이벤트 수신 시 toast.info 호출 (signal_types 조합 포함).
//   D2-B  이벤트 수신 시 각 signal_id에 대해 interventionSignalDismiss 호출.
//   D2-C  컴포넌트 언마운트(cancelled) 시 unlisten 호출 — listener 누수 방지.
//   D2-D  컴포넌트는 null 렌더 — DOM에 추가 요소 없음.

import { describe, expect, it, vi, afterEach } from "vitest";
import { act, render } from "@testing-library/react";

import type { MetacogAlert } from "@/lib/types";
import { MetacogAlertToast } from "@/components/MetacogAlertToast";

// ---------- listen mock -------------------------------------------------------
// BUG-002 패턴: Promise<UnlistenFn>을 반환.
// captured: 등록된 이벤트 핸들러를 테스트에서 수동 호출.
// unlisten: unmount 시 호출 여부 확인용 spy.

type ListenCallback = (event: { payload: MetacogAlert }) => void;
let captured: ListenCallback | null = null;
const unlistenSpy = vi.fn();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name: string, cb: ListenCallback) => {
    captured = cb;
    return unlistenSpy;
  }),
}));

// ---------- api mock ----------------------------------------------------------
const dismissSpy = vi.fn(async (_id: number) => undefined);
vi.mock("@/lib/api", () => ({
  api: {
    interventionSignalDismiss: (id: number) => dismissSpy(id),
  },
}));

// ---------- toast mock --------------------------------------------------------
const toastInfoSpy = vi.fn();
vi.mock("@/lib/toast", () => ({
  toast: {
    info: (msg: string) => toastInfoSpy(msg),
  },
}));

afterEach(() => {
  captured = null;
  unlistenSpy.mockClear();
  dismissSpy.mockClear();
  toastInfoSpy.mockClear();
});

/** 컴포넌트 마운트 후 listen Promise가 resolve될 때까지 microtask 진행. */
async function mountAndFlush() {
  const result = render(<MetacogAlertToast />);
  await act(async () => {
    await Promise.resolve();
  });
  return result;
}

/** 이벤트 payload를 캡처된 listener에 dispatch. */
async function fireEvent(payload: MetacogAlert) {
  await act(async () => {
    captured!({ payload });
  });
}

describe("MetacogAlertToast", () => {
  it("D2-D: 컴포넌트 렌더 출력 없음 (null)", async () => {
    const { container } = await mountAndFlush();
    // Sonner Toaster가 없으므로 toast DOM은 없고, 컨테이너는 빈 div만.
    expect(container.firstChild).toBeNull();
  });

  it("D2-A: metacog:alert 수신 시 toast.info 호출", async () => {
    await mountAndFlush();
    expect(captured).not.toBeNull();

    await fireEvent({
      signal_types: ["progress_recall_gap", "self_report_low"],
      signal_ids: [1, 2],
    });

    expect(toastInfoSpy).toHaveBeenCalledOnce();
    // ko.json: title = "능력 착각 신호", combo = "진도-회상 격차 + 자기보고-실제 격차 동시 발화"
    const msg: string = toastInfoSpy.mock.calls[0][0] as string;
    expect(msg).toContain("능력 착각 신호");
    expect(msg).toContain("진도-회상 격차");
    expect(msg).toContain("자기보고-실제 격차");
  });

  it("D2-B: 이벤트 수신 시 각 signal_id에 interventionSignalDismiss 호출", async () => {
    await mountAndFlush();
    await fireEvent({
      signal_types: ["repeat_search", "progress_recall_gap"],
      signal_ids: [10, 20],
    });

    expect(dismissSpy).toHaveBeenCalledTimes(2);
    expect(dismissSpy).toHaveBeenCalledWith(10);
    expect(dismissSpy).toHaveBeenCalledWith(20);
  });

  it("D2-C: 언마운트 시 unlisten 호출 (cancelled + listener 해제)", async () => {
    // RTL cleanup이 이전 테스트 컴포넌트를 unmount → unlistenSpy를 호출할 수 있으므로
    // 마운트 직전 spy 초기화 후 명시적 unmount 호출 횟수만 검증.
    unlistenSpy.mockClear();
    const { unmount } = await mountAndFlush();
    const countBefore = unlistenSpy.mock.calls.length;

    unmount();

    // unmount 후 최소 1회 추가 호출됨 (cleanup effect → unlisten).
    expect(unlistenSpy.mock.calls.length).toBeGreaterThan(countBefore);
  });

  it("D2-C: 언마운트 후 도착한 이벤트는 toast·dismiss 미호출 (cancelled guard)", async () => {
    const savedCaptured = captured; // 마운트 전은 null
    const { unmount } = await mountAndFlush();
    const liveCallback = captured!;

    unmount();

    // 언마운트 후 이벤트 도착 시뮬.
    await act(async () => {
      liveCallback({ payload: { signal_types: ["self_report_low"], signal_ids: [99] } });
    });

    expect(toastInfoSpy).not.toHaveBeenCalled();
    expect(dismissSpy).not.toHaveBeenCalled();

    void savedCaptured; // lint suppress
  });

  it("signal_types 단일 항목도 정상 표시", async () => {
    await mountAndFlush();
    await fireEvent({
      signal_types: ["repeat_search"],
      signal_ids: [5],
    });

    expect(toastInfoSpy).toHaveBeenCalledOnce();
    const msg: string = toastInfoSpy.mock.calls[0][0] as string;
    expect(msg).toContain("같은 검색 반복");
  });

  it("signal_ids 비어 있으면 dismiss 미호출, toast는 호출", async () => {
    await mountAndFlush();
    await fireEvent({
      signal_types: ["short_dwell"],
      signal_ids: [],
    });

    expect(toastInfoSpy).toHaveBeenCalledOnce();
    expect(dismissSpy).not.toHaveBeenCalled();
  });
});
