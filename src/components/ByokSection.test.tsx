// v0.4.4 PR 5 (D-095) — BYOK 섹션 단위 테스트.
//
// 검증:
//   * config=null이면 enable 토글 + section_desc만 노출 (provider/model/key 영역 X).
//   * config=Some(...)이면 provider 라디오, 모델 select, 키 입력 영역 모두 렌더.
//   * provider 라디오 클릭 → onChange가 새 provider + 그 provider의 default model로 호출.
//   * 모델 select 변경 → onChange(provider, new model).
//   * 키 입력 + 저장 → api.byokKeySet 호출.
//   * cost 카드 자동 mount 시 byokEstimateCost 호출.
//   * routing status mount 시 devByokRoutingCheck 호출.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ByokSection } from "@/components/ByokSection";
import { api } from "@/lib/api";
import type {
  ByokCostEstimate,
  ByokRoutingResult,
} from "@/lib/types";

vi.mock("@/lib/api", () => ({
  api: {
    byokKeyPresent: vi.fn(),
    byokKeySet: vi.fn(),
    byokKeyDelete: vi.fn(),
    byokEstimateCost: vi.fn(),
    devByokRoutingCheck: vi.fn(),
  },
}));

const mockedApi = vi.mocked(api);

const fakeEstimate: ByokCostEstimate = {
  provider: "voyage",
  model: "voyage-3-lite",
  chunks: 1500,
  avg_tokens_per_chunk: 200,
  usd_estimate: 0.006,
  unit_price_label: "$0.02 / 1M tokens",
};

const fakeRouting: ByokRoutingResult = {
  byok_active: true,
  provider: "voyage",
  model: "voyage-3-lite",
  key_present: true,
  routed_to: "cloud (voyage-3-lite)",
};

describe("ByokSection", () => {
  beforeEach(() => {
    mockedApi.byokKeyPresent.mockReset();
    mockedApi.byokKeySet.mockReset();
    mockedApi.byokKeyDelete.mockReset();
    mockedApi.byokEstimateCost.mockReset();
    mockedApi.devByokRoutingCheck.mockReset();
    mockedApi.byokKeyPresent.mockResolvedValue(false);
    mockedApi.byokEstimateCost.mockResolvedValue(fakeEstimate);
    mockedApi.devByokRoutingCheck.mockResolvedValue(fakeRouting);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("config=null이면 enable 토글만 노출하고 provider/model 영역은 안 보인다", () => {
    render(<ByokSection config={null} onChange={() => undefined} />);

    // 활성 토글이 false 상태로 노출.
    const toggle = screen.getByRole("switch");
    expect(toggle).toHaveAttribute("aria-checked", "false");

    // provider/model 라벨은 보이지 않는다.
    expect(screen.queryByText(/Voyage AI/i)).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText(/예상 청크 수/),
    ).not.toBeInTheDocument();
  });

  it("config=null에서 토글 클릭 시 onChange(default Voyage cfg)", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<ByokSection config={null} onChange={onChange} />);

    await user.click(screen.getByRole("switch"));
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({
        provider: "voyage",
        model: "voyage-3-lite",
      });
    });
  });

  it("config=Some(voyage)이면 provider 라디오 + 모델 select + 키 입력 + 비용·라우팅 카드 노출", async () => {
    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={() => undefined}
      />,
    );

    // provider 라디오 = voyage / gemini.
    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(2);
    expect(radios[0]).toHaveAttribute("aria-checked", "true"); // voyage
    expect(radios[1]).toHaveAttribute("aria-checked", "false"); // gemini

    // 모델 select.
    const select = screen.getByLabelText(/모델/) as HTMLSelectElement;
    expect(select.value).toBe("voyage-3-lite");

    // 비용 카드 mount → byokEstimateCost 호출 (1500 청크, 200 토큰 default).
    await waitFor(() => {
      expect(mockedApi.byokEstimateCost).toHaveBeenCalledWith(
        "voyage",
        "voyage-3-lite",
        1500,
        200,
      );
    });

    // 라우팅 mount → dev_byok_routing_check 호출.
    await waitFor(() => {
      expect(mockedApi.devByokRoutingCheck).toHaveBeenCalled();
    });
    // 라우팅 결과 표시.
    await waitFor(() => {
      expect(screen.getByText(/cloud \(voyage-3-lite\)/)).toBeInTheDocument();
    });
  });

  it("provider 라디오 클릭 시 onChange(new provider + that provider's default model)", async () => {
    const onChange = vi.fn();
    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={onChange}
      />,
    );

    // gemini 라디오 클릭.
    const radios = screen.getAllByRole("radio");
    fireEvent.click(radios[1]);
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({
        provider: "gemini",
        model: "text-embedding-004",
      });
    });
  });

  it("모델 select 변경 시 onChange(provider, new model)", async () => {
    const onChange = vi.fn();
    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={onChange}
      />,
    );

    const select = screen.getByLabelText(/모델/) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "voyage-3" } });
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({
        provider: "voyage",
        model: "voyage-3",
      });
    });
  });

  it("키 입력 + 저장 시 byokKeySet이 호출되고 입력이 비워진다", async () => {
    mockedApi.byokKeySet.mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={() => undefined}
      />,
    );

    const input = (await screen.findByPlaceholderText("pa-...")) as HTMLInputElement;
    await user.type(input, "pa-xyz123456789");
    const saveBtn = screen.getByRole("button", { name: /저장$/ });
    await user.click(saveBtn);

    await waitFor(() => {
      expect(mockedApi.byokKeySet).toHaveBeenCalledWith("voyage", "pa-xyz123456789");
    });
    expect(input.value).toBe("");
  });

  it("저장된 키 보유 시 삭제 버튼이 노출되고 클릭 시 byokKeyDelete 호출", async () => {
    mockedApi.byokKeyPresent.mockResolvedValue(true);
    mockedApi.byokKeyDelete.mockResolvedValue(undefined);
    const user = userEvent.setup();

    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={() => undefined}
      />,
    );

    const deleteBtn = await screen.findByRole("button", {
      name: /저장된 키 삭제/,
    });
    await user.click(deleteBtn);
    await waitFor(() => {
      expect(mockedApi.byokKeyDelete).toHaveBeenCalledWith("voyage");
    });
  });

  it("byokKeySet 실패 시 에러 메시지를 alert role로 표시", async () => {
    mockedApi.byokKeySet.mockRejectedValue({
      kind: "InvalidInput",
      message: "BYOK 키 형식 오류",
    });
    const user = userEvent.setup();

    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={() => undefined}
      />,
    );

    const input = await screen.findByPlaceholderText("pa-...");
    await user.type(input, "pa-bad-key-123456");
    await user.click(screen.getByRole("button", { name: /저장$/ }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/BYOK 키 형식 오류/);
  });

  it("voyage-3-lite 선택 시 차원 mismatch 경고가 표시된다", () => {
    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={() => undefined}
      />,
    );
    expect(
      screen.getByText(/voyage-3-lite는 512차원/),
    ).toBeInTheDocument();
  });

  it("BYOK 활성 + 키 없음 라우팅이면 경고 alert 표시", async () => {
    mockedApi.devByokRoutingCheck.mockResolvedValue({
      byok_active: true,
      provider: "voyage",
      model: "voyage-3-lite",
      key_present: false,
      routed_to: "fastembed (mE5-small) — BYOK 키 없음, 폴백",
    });

    render(
      <ByokSection
        config={{ provider: "voyage", model: "voyage-3-lite" }}
        onChange={() => undefined}
      />,
    );

    // routing 카드의 경고 메시지.
    await waitFor(() => {
      expect(
        screen.getByText(/BYOK 활성 상태이지만 키가 없어/),
      ).toBeInTheDocument();
    });
  });
});
