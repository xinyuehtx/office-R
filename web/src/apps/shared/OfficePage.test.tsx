import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { OfficePage } from "./OfficePage";
import * as wasm from "../../wasm";

// 隔离 WASM 内核:只验证「上传 → 调用内核 → 渲染结果」这一视图逻辑。
vi.mock("../../wasm", () => ({
  render: vi.fn(),
}));

const mockedRender = vi.mocked(wasm.render);

describe("OfficePage 上传与渲染", () => {
  beforeEach(() => {
    mockedRender.mockReset();
  });

  it("初始展示空状态提示", () => {
    render(<OfficePage title="文档 · Word" subtitle="测试" accept=".docx" />);
    expect(screen.getByText(/尚未选择文件/)).toBeInTheDocument();
  });

  it("上传文件后展示内核返回的识别结果", async () => {
    mockedRender.mockResolvedValue({
      format: "docx",
      format_name: "Word 文档",
      byte_len: 1234,
      message: "占位说明",
      ok: true,
    });

    const user = userEvent.setup();
    render(<OfficePage title="文档 · Word" subtitle="测试" accept=".docx" />);

    const file = new File([new Uint8Array([80, 75, 3, 4])], "demo.docx");
    await user.upload(screen.getByTestId("file-input"), file);

    expect(await screen.findByText("Word 文档")).toBeInTheDocument();
    expect(screen.getByText("1234 字节")).toBeInTheDocument();
    expect(screen.getByText("占位说明")).toBeInTheDocument();
    expect(mockedRender).toHaveBeenCalledOnce();
  });

  it("内核抛错时展示错误信息", async () => {
    mockedRender.mockRejectedValue(new Error("boom"));

    const user = userEvent.setup();
    render(<OfficePage title="文档 · Word" subtitle="测试" accept=".docx" />);

    const file = new File([new Uint8Array([1, 2])], "bad.docx");
    await user.upload(screen.getByTestId("file-input"), file);

    expect(await screen.findByText(/解析失败:boom/)).toBeInTheDocument();
  });
});
