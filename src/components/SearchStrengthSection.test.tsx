// v0.4.3 PR 1 (D-086) — Settings 모달의 "검색 강도" 라디오 단위 테스트.
//
// 검증:
//   * 디폴트 "balanced"로 진입했을 때 균형 라디오가 selected (aria-checked=true).
//   * "정확"을 클릭하면 onChange가 search_strength=accurate로 호출.
//   * 세 옵션 모두 라벨 + 짧은 설명을 노출.

import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { SearchStrengthSection } from "@/components/SearchStrengthSection";
import type { SearchStrength } from "@/lib/types";

describe("SearchStrengthSection", () => {
  it("balanced selected by default props", () => {
    render(<SearchStrengthSection strength="balanced" onChange={() => undefined} />);
    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(3);
    // 라디오는 fast(0) / balanced(1) / accurate(2) 순서.
    expect(radios[0].getAttribute("aria-checked")).toBe("false");
    expect(radios[1].getAttribute("aria-checked")).toBe("true");
    expect(radios[2].getAttribute("aria-checked")).toBe("false");
  });

  it("clicking accurate fires onChange", () => {
    const onChange = vi.fn<(s: SearchStrength) => void>();
    render(<SearchStrengthSection strength="balanced" onChange={onChange} />);
    // 라디오는 fast/balanced/accurate 순서 — index=2 = accurate.
    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(3);
    fireEvent.click(radios[2]);
    expect(onChange).toHaveBeenCalledWith("accurate");
  });

  it("clicking fast fires onChange with 'fast'", () => {
    const onChange = vi.fn<(s: SearchStrength) => void>();
    render(<SearchStrengthSection strength="balanced" onChange={onChange} />);
    const radios = screen.getAllByRole("radio");
    fireEvent.click(radios[0]);
    expect(onChange).toHaveBeenCalledWith("fast");
  });

  it("renders all three options with descriptions", () => {
    render(<SearchStrengthSection strength="fast" onChange={() => undefined} />);
    // 같은 단어가 라벨·설명·섹션 desc 다수 곳에 등장 가능 → 라디오 3개가 정확히 보이는지로 검증.
    expect(screen.getAllByRole("radio")).toHaveLength(3);
    // 짧은 설명 키워드 — 라벨 텍스트와 충돌하지 않는 unique 토큰 검증.
    expect(screen.getByText(/rewriting 생략/)).toBeDefined();
    expect(screen.getByText(/Haiku 1회 호출/)).toBeDefined();
    expect(screen.getByText(/가상 답변까지/)).toBeDefined();
  });
});
