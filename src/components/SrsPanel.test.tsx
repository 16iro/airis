// SrsPanel v0.5 PR 2 (D-099/D-103) — on-demand 카드 생성 버튼·LLM 토글·onboarding 테스트.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

// jsdom 환경에서 localStorage가 완전히 구현돼 있지 않을 수 있어 직접 stub.
const localStorageStore: Record<string, string> = {};
const localStorageMock = {
  getItem: (k: string) => localStorageStore[k] ?? null,
  setItem: (k: string, v: string) => { localStorageStore[k] = v; },
  removeItem: (k: string) => { delete localStorageStore[k]; },
  clear: () => { Object.keys(localStorageStore).forEach((k) => delete localStorageStore[k]); },
};
Object.defineProperty(globalThis, "localStorage", { value: localStorageMock, writable: true });

import { SrsPanel } from "@/components/SrsPanel";

// Tauri event API mock.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

const { srsListDueSpy, srsGenerateBookSpy, srsGenerateWeakPrioritySpy } = vi.hoisted(() => ({
  srsListDueSpy: vi.fn(async () => []),
  srsGenerateBookSpy: vi.fn(async () => undefined),
  srsGenerateWeakPrioritySpy: vi.fn(async () => undefined),
}));

vi.mock("@/lib/api", () => ({
  api: {
    srsListDue: srsListDueSpy,
    srsGenerateBook: srsGenerateBookSpy,
    srsGenerateWeakPriority: srsGenerateWeakPrioritySpy,
    srsAddCard: vi.fn(async () => ({})),
  },
}));

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

vi.mock("@/store/activeBookStore", () => ({
  useActiveBookStore: (selector: (s: unknown) => unknown) =>
    selector({
      content: {
        book_id: "book-1",
        format: "md",
        content: "# Ch01\n내용",
        source_path: "/tmp/test.md",
        indexed: true,
      },
    }),
}));

describe("SrsPanel — v0.5 PR 2 카드 생성", () => {
  beforeEach(() => {
    // onboarding: 이미 봤다고 표시해 Dialog 숨김.
    localStorage.setItem("airis_srs_onboarding_seen", "1");
  });

  afterEach(() => {
    localStorage.removeItem("airis_srs_onboarding_seen");
    vi.clearAllMocks();
  });

  it("'이 책 카드 생성' 버튼이 렌더된다", () => {
    render(<SrsPanel onClose={vi.fn()} />);
    expect(screen.getByRole("button", { name: /이 책 카드 생성/ })).toBeInTheDocument();
  });

  it("'약점 우선 30장' 버튼이 렌더된다", () => {
    render(<SrsPanel onClose={vi.fn()} />);
    expect(screen.getByRole("button", { name: /약점 우선 30장/ })).toBeInTheDocument();
  });

  it("LLM 토글 체크박스가 기본으로 체크됨", () => {
    render(<SrsPanel onClose={vi.fn()} />);
    const toggle = screen.getByRole("checkbox", { name: /LLM 보강/ });
    expect(toggle).toBeChecked();
  });

  it("LLM 토글 해제 시 비용 안내 문구가 사라진다", () => {
    render(<SrsPanel onClose={vi.fn()} />);
    const toggle = screen.getByRole("checkbox", { name: /LLM 보강/ });
    // 처음엔 비용 안내 표시.
    expect(screen.getByText(/토큰 비용/)).toBeInTheDocument();
    fireEvent.click(toggle);
    // 토글 해제 후 비용 안내 사라짐.
    expect(screen.queryByText(/토큰 비용/)).not.toBeInTheDocument();
  });

  it("'이 책 카드 생성' 클릭 시 srsGenerateBook이 호출된다", async () => {
    render(<SrsPanel onClose={vi.fn()} />);
    const btn = screen.getByRole("button", { name: /이 책 카드 생성/ });
    fireEvent.click(btn);
    await vi.waitFor(() => {
      expect(srsGenerateBookSpy).toHaveBeenCalledWith("study-1", "book-1", true);
    });
  });

  it("'약점 우선 30장' 클릭 시 srsGenerateWeakPriority가 호출된다", async () => {
    render(<SrsPanel onClose={vi.fn()} />);
    const btn = screen.getByRole("button", { name: /약점 우선 30장/ });
    fireEvent.click(btn);
    await vi.waitFor(() => {
      expect(srsGenerateWeakPrioritySpy).toHaveBeenCalledWith("study-1", 30, true);
    });
  });
});

describe("SrsPanel — onboarding Dialog", () => {
  afterEach(() => {
    localStorage.removeItem("airis_srs_onboarding_seen");
    vi.clearAllMocks();
  });

  it("localStorage key가 없으면 onboarding 다이얼로그가 노출된다", () => {
    // key 없이 렌더.
    render(<SrsPanel onClose={vi.fn()} />);
    expect(screen.getByText(/SRS 카드를 자동으로 만들 수 있어요/)).toBeInTheDocument();
  });

  it("'알겠어요' 클릭 시 onboarding 다이얼로그가 닫히고 localStorage에 저장된다", () => {
    render(<SrsPanel onClose={vi.fn()} />);
    const okBtn = screen.getByRole("button", { name: /알겠어요/ });
    fireEvent.click(okBtn);
    expect(screen.queryByText(/SRS 카드를 자동으로 만들 수 있어요/)).not.toBeInTheDocument();
    expect(localStorage.getItem("airis_srs_onboarding_seen")).toBe("1");
  });
});
