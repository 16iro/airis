// ApiKeyInput — 키 보유 여부 표시·저장·삭제 흐름.
// `@/lib/api`를 mock해 Tauri invoke를 우회. 실제 키체인은 만지지 않는다.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ApiKeyInput } from "@/components/ApiKeyInput";

vi.mock("@/lib/api", () => ({
  api: {
    apiKeyPresent: vi.fn(),
    apiKeySet: vi.fn(),
    apiKeyDelete: vi.fn(),
  },
}));

import { api } from "@/lib/api";

const mockedApi = vi.mocked(api);

describe("ApiKeyInput", () => {
  beforeEach(() => {
    mockedApi.apiKeyPresent.mockReset();
    mockedApi.apiKeySet.mockReset();
    mockedApi.apiKeyDelete.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("키 보유 시 '저장됨 ✓' + 삭제 버튼을 노출한다", async () => {
    mockedApi.apiKeyPresent.mockResolvedValue(true);

    render(<ApiKeyInput provider="anthropic" />);

    expect(await screen.findByText(/저장됨/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /저장된 키 삭제/ }),
    ).toBeInTheDocument();
  });

  it("키 미보유 시 '없음' 표시 + 삭제 버튼은 안 나온다", async () => {
    mockedApi.apiKeyPresent.mockResolvedValue(false);

    render(<ApiKeyInput provider="anthropic" />);

    expect(await screen.findByText(/없음/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /저장된 키 삭제/ }),
    ).not.toBeInTheDocument();
  });

  it("입력이 비어있으면 저장 버튼이 disabled", async () => {
    mockedApi.apiKeyPresent.mockResolvedValue(false);

    render(<ApiKeyInput provider="anthropic" />);
    const saveBtn = await screen.findByRole("button", { name: /저장/ });
    expect(saveBtn).toBeDisabled();
  });

  it("키 입력 후 저장하면 apiKeySet이 호출되고 입력이 비워진다", async () => {
    mockedApi.apiKeyPresent.mockResolvedValue(false);
    mockedApi.apiKeySet.mockResolvedValue(undefined);

    const user = userEvent.setup();
    render(<ApiKeyInput provider="anthropic" />);

    const input = (await screen.findByPlaceholderText("sk-ant-...")) as HTMLInputElement;
    await user.type(input, "sk-ant-test");
    const saveBtn = screen.getByRole("button", { name: /저장/ });
    await user.click(saveBtn);

    await waitFor(() => {
      expect(mockedApi.apiKeySet).toHaveBeenCalledWith("anthropic", "sk-ant-test");
    });
    expect(input.value).toBe("");
  });

  it("저장 실패 시 에러 메시지를 alert role로 표시한다", async () => {
    mockedApi.apiKeyPresent.mockResolvedValue(false);
    mockedApi.apiKeySet.mockRejectedValue({
      kind: "InvalidInput",
      message: "키 형식 오류",
    });

    const user = userEvent.setup();
    render(<ApiKeyInput provider="anthropic" />);

    const input = await screen.findByPlaceholderText("sk-ant-...");
    await user.type(input, "bad");
    await user.click(screen.getByRole("button", { name: /저장/ }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("키 형식 오류");
  });

  it("기본은 password 타입, 토글 버튼 클릭 시 text 타입", async () => {
    mockedApi.apiKeyPresent.mockResolvedValue(false);

    const user = userEvent.setup();
    render(<ApiKeyInput provider="anthropic" />);

    const input = (await screen.findByPlaceholderText("sk-ant-...")) as HTMLInputElement;
    expect(input.type).toBe("password");

    const reveal = screen.getByRole("button", { name: /키 보이기/ });
    await user.click(reveal);
    expect(input.type).toBe("text");
  });
});
