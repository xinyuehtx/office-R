import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ExcelPage } from "./ExcelPage";
import * as csvClient from "../../wasm/csvClient";
import { createFixtureSheet, fixtureMeta, makeGrid } from "../../test/sheetFixture";

// 隔离 WASM:这里验证的是页面的状态流转与展示,不是解析本身
vi.mock("../../wasm/csvClient", () => ({
  createWorker: vi.fn(() => null),
  parseCsvFile: vi.fn(),
}));

const parseCsvFile = vi.mocked(csvClient.parseCsvFile);

/** 造一个解析成功的返回值。 */
function successOutcome(
  rows: string[][],
  metaOverrides = {},
  formulas: Record<string, string> = {},
) {
  const sheet = createFixtureSheet(rows, formulas);
  return {
    sheet,
    packed: {
      text: new Uint8Array(),
      cellEnds: new Uint32Array(),
      rowStarts: new Uint32Array(),
      colWidthUnits: sheet.colWidthUnits,
      cols: sheet.cols,
      meta: fixtureMeta({ rows: sheet.rows, cols: sheet.cols, ...metaOverrides }),
      formulas: Object.entries(formulas).map(([key, formula]) => {
        const [row, col] = key.split(",").map(Number);
        return { row, col, formula };
      }),
    },
    offMainThread: true,
    parseMs: 12,
    assembleMs: 3,
  };
}

async function upload(content = "a,b\n1,2\n", name = "demo.csv") {
  const user = userEvent.setup();
  const file = new File([content], name, { type: "text/csv" });
  await user.upload(screen.getByTestId("file-input"), file);
}

describe("ExcelPage", () => {
  beforeEach(() => {
    parseCsvFile.mockReset();
  });

  it("初始展示空状态与使用说明", () => {
    render(<ExcelPage />);
    expect(screen.getByText(/尚未选择文件/)).toBeInTheDocument();
    expect(screen.getByText(/分隔符自动识别/)).toBeInTheDocument();
  });

  it("上传后渲染 canvas 表格视图与元信息", async () => {
    parseCsvFile.mockResolvedValue(successOutcome(makeGrid(20, 4)) as never);
    render(<ExcelPage />);
    await upload();

    // 表格主体是堆叠的 canvas 图层,不是 DOM 表格
    const surface = await screen.findByTestId("sheet-canvas");
    const viewport = surface.parentElement!;
    expect(viewport.querySelector('canvas[data-layer="body"]')).not.toBeNull();
    expect(viewport.querySelector('canvas[data-layer="headers"]')).not.toBeNull();
    expect(viewport.querySelector('canvas[data-layer="overlay"]')).not.toBeNull();
    expect(document.querySelector("table")).toBeNull();

    expect(screen.getByTestId("sheet-meta")).toHaveTextContent("UTF-8");
    expect(screen.getByTestId("sheet-status")).toHaveTextContent("20 行 × 4 列");
  });

  it("交互层压在画布之上,画布本身不接收指针事件", async () => {
    parseCsvFile.mockResolvedValue(successOutcome(makeGrid(20, 4)) as never);
    render(<ExcelPage />);
    await upload();

    const surface = await screen.findByTestId("sheet-canvas");
    const viewport = surface.parentElement!;
    for (const canvas of Array.from(viewport.querySelectorAll("canvas"))) {
      expect((canvas as HTMLCanvasElement).style.pointerEvents).toBe("none");
    }
  });

  it("解析失败时给出中文提示与排查编号,并可重新选择文件", async () => {
    parseCsvFile.mockRejectedValue(new Error("文件内容为空,没有可显示的数据"));
    render(<ExcelPage />);
    await upload("");

    expect(await screen.findByText(/文件内容为空/)).toBeInTheDocument();
    expect(screen.getByText(/重新选择文件重试/)).toBeInTheDocument();
    expect(screen.getByText(/排查编号/)).toBeInTheDocument();

    // 失败之后仍然可以再选一个文件,不需要刷新页面
    parseCsvFile.mockResolvedValue(successOutcome(makeGrid(3, 2)) as never);
    await upload("a,b\n1,2\n", "again.csv");
    expect(await screen.findByTestId("sheet-canvas")).toBeInTheDocument();
  });

  it("行列被截断时给出明确提示", async () => {
    parseCsvFile.mockResolvedValue(
      successOutcome(makeGrid(5, 2), { truncatedRows: true }) as never,
    );
    render(<ExcelPage />);
    await upload();

    expect(await screen.findByText(/已达上限,超出部分未显示/)).toBeInTheDocument();
  });

  it("公式格显示计算值,选中后公式栏回显原始公式,元信息给出公式数", async () => {
    // A1 表头、A2 是公式格,显示计算值 14,原始公式 =B2*C2
    parseCsvFile.mockResolvedValue(
      successOutcome([["小计"], ["14"]], {}, { "1,0": "=B2*C2" }) as never,
    );
    render(<ExcelPage />);
    await upload();
    const canvas = await screen.findByTestId("sheet-canvas");

    // 元信息显示公式数
    expect(screen.getByTestId("sheet-meta")).toHaveTextContent("1 个已求值");

    // 初始选中 A1(表头),公式栏显示其文本,不带公式徽标
    const bar = screen.getByTestId("formula-bar");
    expect(bar).toHaveTextContent("A1");
    expect(bar).toHaveTextContent("小计");

    // 下移到 A2(公式格),公式栏回显原始公式
    canvas.focus();
    await userEvent.setup().keyboard("{ArrowDown}");
    expect(bar).toHaveTextContent("A2");
    expect(bar).toHaveTextContent("=B2*C2");
  });

  it("加载公式示例按钮走同一条打开流程", async () => {
    parseCsvFile.mockResolvedValue(successOutcome(makeGrid(8, 4)) as never);
    render(<ExcelPage />);
    await userEvent.setup().click(screen.getByTestId("load-formula-sample"));
    expect(await screen.findByTestId("sheet-canvas")).toBeInTheDocument();
    // 打开流程确实被触发
    expect(parseCsvFile).toHaveBeenCalledTimes(1);
  });

  it("编码有损时提示内容可能不准确", async () => {
    parseCsvFile.mockResolvedValue(
      successOutcome(makeGrid(5, 2), { lossy: true, encoding: "GBK" }) as never,
    );
    render(<ExcelPage />);
    await upload();

    expect(await screen.findByText(/部分字符无法解码/)).toBeInTheDocument();
  });

  it("关闭后回到空状态", async () => {
    parseCsvFile.mockResolvedValue(successOutcome(makeGrid(5, 2)) as never);
    render(<ExcelPage />);
    await upload();
    await screen.findByTestId("sheet-canvas");

    await userEvent.setup().click(screen.getByRole("button", { name: "关闭" }));
    await waitFor(() => expect(screen.getByText(/尚未选择文件/)).toBeInTheDocument());
  });

  it("换文件时释放上一份表格,避免 WASM 内存泄漏", async () => {
    const first = successOutcome(makeGrid(5, 2));
    parseCsvFile.mockResolvedValue(first as never);
    render(<ExcelPage />);
    await upload("a,b\n1,2\n", "first.csv");
    await screen.findByTestId("sheet-canvas");

    parseCsvFile.mockResolvedValue(successOutcome(makeGrid(7, 3)) as never);
    await upload("a,b,c\n1,2,3\n", "second.csv");
    await waitFor(() => expect(screen.getByTestId("sheet-status")).toHaveTextContent("7 行"));

    expect((first.sheet as { disposed: boolean }).disposed).toBe(true);
  });
});

describe("SheetCanvas 交互", () => {
  beforeEach(() => {
    parseCsvFile.mockReset();
    parseCsvFile.mockResolvedValue(successOutcome(makeGrid(50, 6)) as never);
  });

  it("方向键移动选区,状态栏与无障碍播报同步更新", async () => {
    render(<ExcelPage />);
    await upload();
    const canvas = await screen.findByTestId("sheet-canvas");

    const status = screen.getByTestId("sheet-status");
    expect(status).toHaveTextContent("A1");
    expect(status).toHaveTextContent("r0c0");

    const user = userEvent.setup();
    canvas.focus();
    await user.keyboard("{ArrowDown}{ArrowRight}");

    expect(status).toHaveTextContent("B2");
    expect(status).toHaveTextContent("r1c1");
    // 读屏软件读不到 canvas,靠 live region 播报
    expect(screen.getByRole("status")).toHaveTextContent("B2:r1c1");
  });

  it("选区不会越过表格边界", async () => {
    render(<ExcelPage />);
    await upload();
    const canvas = await screen.findByTestId("sheet-canvas");

    const user = userEvent.setup();
    canvas.focus();
    await user.keyboard("{ArrowUp}{ArrowLeft}");
    expect(screen.getByTestId("sheet-status")).toHaveTextContent("A1");
  });

  it("Ctrl+End 跳到最后一个单元格,Ctrl+Home 回到 A1", async () => {
    render(<ExcelPage />);
    await upload();
    const canvas = await screen.findByTestId("sheet-canvas");

    const user = userEvent.setup();
    canvas.focus();
    await user.keyboard("{Control>}{End}{/Control}");
    expect(screen.getByTestId("sheet-status")).toHaveTextContent("F50");

    await user.keyboard("{Control>}{Home}{/Control}");
    expect(screen.getByTestId("sheet-status")).toHaveTextContent("A1");
  });

  it("Ctrl +/- 调整缩放,Ctrl+0 复位", async () => {
    render(<ExcelPage />);
    await upload();
    const canvas = await screen.findByTestId("sheet-canvas");

    const user = userEvent.setup();
    canvas.focus();
    expect(screen.getByTestId("sheet-status")).toHaveTextContent("缩放 100%");

    await user.keyboard("{Control>}{+}{/Control}");
    expect(screen.getByTestId("sheet-status")).toHaveTextContent("缩放 110%");

    await user.keyboard("{Control>}0{/Control}");
    expect(screen.getByTestId("sheet-status")).toHaveTextContent("缩放 100%");
  });

  it("canvas 带有网格语义与行列数,便于辅助技术识别", async () => {
    render(<ExcelPage />);
    await upload();

    const grid = await screen.findByRole("grid");
    expect(grid).toHaveAttribute("aria-rowcount", "50");
    expect(grid).toHaveAttribute("aria-colcount", "6");
  });
});
