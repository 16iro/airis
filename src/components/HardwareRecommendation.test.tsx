// v0.4.4 PR 4 (D-094) — 하드웨어 추천 카드 단위 테스트.
//
// 검증:
//   * mount 시 api.devProbeHardware + api.devGetModelRecommendation을 호출해 카드 표시.
//   * 등급별 모델 사이즈 표시 (Conservative=120MB / Balanced=2320MB / Aggressive=2920MB).
//   * "이 추천을 따르기" 클릭 시 onChange(null) 호출.
//   * 수동 라디오 클릭 시 onChange(tier) 호출.
//   * below_minimum 시 경고 배지 노출.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { HardwareRecommendation } from "@/components/HardwareRecommendation";
import { api } from "@/lib/api";
import type { HardwareInfo, RecommendationDetail } from "@/lib/types";

vi.mock("@/lib/api", () => ({
  api: {
    devProbeHardware: vi.fn(),
    devGetModelRecommendation: vi.fn(),
  },
}));

const fakeInfo: HardwareInfo = {
  cpu_cores: 8,
  total_ram_gb: 16.0,
  available_ram_gb: 12.5,
  os: "linux",
  arch: "x86_64",
};

const fakeBalanced: RecommendationDetail = {
  tier: "balanced",
  reason: "RAM 16.0GB 환경에 적합합니다.",
  t1_enabled: true,
  t2_enabled: true,
  t3_enabled: false,
  total_model_size_mb: 2320,
  below_minimum: false,
};

const fakeBelowMin: RecommendationDetail = {
  tier: "conservative",
  reason: "최소 권장 사양(4코어 / 8GB) 미만입니다.",
  t1_enabled: true,
  t2_enabled: false,
  t3_enabled: false,
  total_model_size_mb: 120,
  below_minimum: true,
};

const fakeAggressive: RecommendationDetail = {
  tier: "aggressive",
  reason: "RAM 64.0GB 환경입니다.",
  t1_enabled: true,
  t2_enabled: true,
  t3_enabled: true,
  total_model_size_mb: 2920,
  below_minimum: false,
};

describe("HardwareRecommendation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders hardware info + balanced recommendation", async () => {
    vi.mocked(api.devProbeHardware).mockResolvedValue(fakeInfo);
    vi.mocked(api.devGetModelRecommendation).mockResolvedValue(fakeBalanced);

    render(<HardwareRecommendation override={null} onChange={() => undefined} />);

    await waitFor(() => {
      expect(screen.getByText(/8 코어/)).toBeDefined();
    });
    expect(screen.getByText(/16\.0 GB/)).toBeDefined();
    expect(screen.getByText(/linux · x86_64/)).toBeDefined();
    // Balanced 추천 라벨.
    expect(screen.getAllByText(/균형/).length).toBeGreaterThan(0);
    // 사이즈는 추천 카드 + 라디오 양쪽에서 등장 — 적어도 한 곳에 표시.
    expect(screen.getAllByText(/2320MB/).length).toBeGreaterThan(0);
  });

  it("clicking 'follow recommendation' fires onChange(null)", async () => {
    vi.mocked(api.devProbeHardware).mockResolvedValue(fakeInfo);
    vi.mocked(api.devGetModelRecommendation).mockResolvedValue(fakeBalanced);
    const onChange = vi.fn<(t: "conservative" | "balanced" | "aggressive" | null) => void>();

    // override가 conservative이고 추천이 balanced인 상태 — "이 추천을 따르기" 버튼 활성.
    render(<HardwareRecommendation override="conservative" onChange={onChange} />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /이 추천을 따르기/ })).toBeDefined();
    });
    fireEvent.click(screen.getByRole("button", { name: /이 추천을 따르기/ }));
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith(null);
    });
  });

  it("clicking manual radio fires onChange(tier)", async () => {
    vi.mocked(api.devProbeHardware).mockResolvedValue(fakeInfo);
    vi.mocked(api.devGetModelRecommendation).mockResolvedValue(fakeBalanced);
    const onChange = vi.fn();

    render(<HardwareRecommendation override={null} onChange={onChange} />);

    await waitFor(() => {
      expect(screen.getAllByRole("radio")).toHaveLength(3);
    });
    const radios = screen.getAllByRole("radio");
    // conservative=0, balanced=1, aggressive=2.
    fireEvent.click(radios[2]);
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith("aggressive");
    });
  });

  it("shows below_minimum warning when applicable", async () => {
    vi.mocked(api.devProbeHardware).mockResolvedValue({
      ...fakeInfo,
      cpu_cores: 2,
      total_ram_gb: 4.0,
    });
    vi.mocked(api.devGetModelRecommendation).mockResolvedValue(fakeBelowMin);

    render(<HardwareRecommendation override={null} onChange={() => undefined} />);

    // below_minimum 경고 메시지 — 카드 본체에 inline 표시.
    await waitFor(() => {
      expect(screen.getByText(/주의: 최소 권장 사양 미만/)).toBeDefined();
    });
    // 사이즈는 T1만 = 120MB.
    expect(screen.getAllByText(/120MB/).length).toBeGreaterThan(0);
  });

  it("aggressive recommendation shows T1+T2+T3 size", async () => {
    vi.mocked(api.devProbeHardware).mockResolvedValue({
      ...fakeInfo,
      total_ram_gb: 64.0,
    });
    vi.mocked(api.devGetModelRecommendation).mockResolvedValue(fakeAggressive);

    render(<HardwareRecommendation override={null} onChange={() => undefined} />);

    await waitFor(() => {
      expect(screen.getAllByText(/2920MB/).length).toBeGreaterThan(0);
    });
    // 공격적 라벨.
    expect(screen.getAllByText(/공격적/).length).toBeGreaterThan(0);
  });

  it("displays error when probe fails", async () => {
    vi.mocked(api.devProbeHardware).mockRejectedValue(new Error("probe fail"));
    vi.mocked(api.devGetModelRecommendation).mockResolvedValue(fakeBalanced);

    render(<HardwareRecommendation override={null} onChange={() => undefined} />);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeDefined();
    });
    expect(screen.getByText(/probe fail/)).toBeDefined();
  });
});
