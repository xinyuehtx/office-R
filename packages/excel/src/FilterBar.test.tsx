import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { FilterBar } from "./FilterBar";
import type { FilterSpec, SheetHandle } from "./sheet";

/** 最小 SheetHandle 桩:只实现 FilterBar 用到的部分。 */
function stubSheet(overrides: Partial<SheetHandle> = {}): SheetHandle {
  return {
    rows: 10,
    cols: 3,
    colWidthUnits: new Uint32Array([5, 5, 5]),
    window: () => ({ text: "", ends: new Uint32Array(), rows: 0, cols: 0 }),
    uniqueValues: () => ({ values: ["北京", "上海"], truncated: false }),
    dispose: () => {},
    ...overrides,
  };
}

function renderBar(props: Partial<Parameters<typeof FilterBar>[0]> = {}) {
  const onApply = vi.fn();
  const onClearAll = vi.fn();
  render(
    <FilterBar
      sheet={stubSheet()}
      activeCol={props.activeCol ?? 0}
      filters={props.filters ?? new Map()}
      headerRows={1}
      onApply={onApply}
      onClearAll={onClearAll}
    />,
  );
  return { onApply, onClearAll };
}

describe("FilterBar", () => {
  it("标题显示当前列名", () => {
    renderBar({ activeCol: 1 });
    expect(screen.getByTestId("filter-bar")).toHaveTextContent("筛选 B 列");
  });

  it("应用文本过滤:回传正确的 spec", async () => {
    const { onApply } = renderBar({ activeCol: 2 });
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("文本关键字"), "北京");
    await user.click(screen.getByTestId("filter-apply"));
    expect(onApply).toHaveBeenCalledWith(2, {
      col: 2,
      kind: "text",
      op: "contains",
      needle: "北京",
    });
  });

  it("空关键字不应用(避免无效过滤)", async () => {
    const { onApply } = renderBar();
    await userEvent.setup().click(screen.getByTestId("filter-apply"));
    expect(onApply).not.toHaveBeenCalled();
  });

  it("应用数值过滤:解析为数字", async () => {
    const { onApply } = renderBar({ activeCol: 1 });
    const user = userEvent.setup();
    await user.selectOptions(screen.getByLabelText("过滤类型"), "number");
    await user.selectOptions(screen.getByLabelText("数值运算"), "gt");
    await user.type(screen.getByLabelText("数值"), "1000");
    await user.click(screen.getByTestId("filter-apply"));
    expect(onApply).toHaveBeenCalledWith(1, {
      col: 1,
      kind: "number",
      op: "gt",
      a: 1000,
      b: 1000,
    });
  });

  it("已生效过滤显示标签,点击可清除该列", async () => {
    const filters = new Map<number, FilterSpec>([
      [0, { col: 0, kind: "text", op: "contains", needle: "x" }],
    ]);
    const { onApply } = renderBar({ activeCol: 0, filters });
    expect(screen.getByTestId("filter-active")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: /A ✕/ }));
    expect(onApply).toHaveBeenCalledWith(0, null);
  });
});
