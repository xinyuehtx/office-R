import { describe, it, expect } from "vitest";
import {
  anchoredScroll,
  bodySize,
  cellRect,
  cellScreenRect,
  clampScroll,
  colAtOffset,
  computeLayout,
  computeVisibleRange,
  hitTest,
  maxScroll,
  rangeForContentRect,
  rectContains,
  rowAtOffset,
  scrollFromThumbOffset,
  scrollIntoView,
  scrollbarGeometry,
  type GridLayout,
} from "./geometry";
import { MIN_COL_WIDTH } from "./theme";

/** 构造一个便于心算的布局:4 列、每列都取到最小宽度 48px。 */
function layoutOf(rows = 100, cols = 4, zoom = 1): GridLayout {
  return computeLayout({
    rows,
    cols,
    // 1 个半角字符 → 7 + 12 = 19px,会被 MIN_COL_WIDTH(48) 兜底
    colWidthUnits: new Uint32Array(cols).fill(1),
    zoom,
  });
}

const VIEWPORT = { width: 400, height: 300 };

describe("computeLayout", () => {
  it("列偏移是递增前缀和,末位等于总宽", () => {
    const layout = layoutOf(10, 3);
    expect(Array.from(layout.colOffsets)).toEqual([0, 48, 96, 144]);
    expect(layout.totalWidth).toBe(144);
  });

  it("总高度 = 行数 × 行高", () => {
    const layout = layoutOf(10, 3);
    expect(layout.totalHeight).toBe(10 * layout.rowHeight);
  });

  it("列宽被夹在上下限之间", () => {
    const layout = computeLayout({
      rows: 1,
      cols: 2,
      colWidthUnits: [0, 10_000],
      zoom: 1,
    });
    expect(layout.colOffsets[1]).toBe(MIN_COL_WIDTH);
    expect(layout.colOffsets[2] - layout.colOffsets[1]).toBe(420);
  });

  it("缩放会等比放大所有尺寸", () => {
    const base = layoutOf(10, 3, 1);
    const zoomed = layoutOf(10, 3, 2);
    expect(zoomed.totalWidth).toBe(base.totalWidth * 2);
    expect(zoomed.rowHeight).toBe(base.rowHeight * 2);
    expect(zoomed.fontSize).toBe(base.fontSize * 2);
    expect(zoomed.headerHeight).toBe(base.headerHeight * 2);
  });

  it("行头宽度随行号位数增加", () => {
    const few = layoutOf(9, 1);
    const many = layoutOf(1_000_000, 1);
    expect(many.headerWidth).toBeGreaterThan(few.headerWidth);
  });

  it("空表格不会产生非法尺寸", () => {
    const layout = computeLayout({ rows: 0, cols: 0, colWidthUnits: [], zoom: 1 });
    expect(layout.totalWidth).toBe(0);
    expect(layout.totalHeight).toBe(0);
    expect(layout.colOffsets.length).toBe(1);
  });

  it("非法缩放值退回 1,不会产生 NaN", () => {
    const layout = computeLayout({ rows: 5, cols: 2, colWidthUnits: [1, 1], zoom: Number.NaN });
    expect(layout.zoom).toBe(1);
    expect(Number.isFinite(layout.totalWidth)).toBe(true);
  });

  it("手动列宽覆盖:指定列用覆盖宽度,其余列仍按自动", () => {
    const layout = computeLayout({
      rows: 1,
      cols: 3,
      colWidthUnits: [0, 0, 0], // 自动各夹到 MIN_COL_WIDTH(48)
      zoom: 1,
      colWidthOverrides: [undefined, 200, 0], // 仅第 1 列覆盖为 200
    });
    expect(layout.colOffsets[1] - layout.colOffsets[0]).toBe(MIN_COL_WIDTH);
    expect(layout.colOffsets[2] - layout.colOffsets[1]).toBe(200);
    expect(layout.colOffsets[3] - layout.colOffsets[2]).toBe(MIN_COL_WIDTH);
  });

  it("手动列宽仍受最小宽度约束", () => {
    const layout = computeLayout({
      rows: 1,
      cols: 1,
      colWidthUnits: [10],
      zoom: 1,
      colWidthOverrides: [5], // 小于 MIN_COL_WIDTH
    });
    expect(layout.colOffsets[1]).toBe(MIN_COL_WIDTH);
  });
});

describe("colAtOffset / rowAtOffset", () => {
  it("二分查到正确的列", () => {
    const layout = layoutOf(10, 4);
    expect(colAtOffset(layout, 0)).toBe(0);
    expect(colAtOffset(layout, 47)).toBe(0);
    expect(colAtOffset(layout, 48)).toBe(1);
    expect(colAtOffset(layout, 100)).toBe(2);
    expect(colAtOffset(layout, 191)).toBe(3);
  });

  it("越界坐标被夹到首末列", () => {
    const layout = layoutOf(10, 4);
    expect(colAtOffset(layout, -50)).toBe(0);
    expect(colAtOffset(layout, 99_999)).toBe(3);
  });

  it("行号由行高整除得到并夹到范围内", () => {
    const layout = layoutOf(10, 4);
    expect(rowAtOffset(layout, 0)).toBe(0);
    expect(rowAtOffset(layout, layout.rowHeight)).toBe(1);
    expect(rowAtOffset(layout, layout.rowHeight * 99)).toBe(9);
    expect(rowAtOffset(layout, -10)).toBe(0);
  });

  it("列数为 0 时不会越界", () => {
    const layout = computeLayout({ rows: 0, cols: 0, colWidthUnits: [], zoom: 1 });
    expect(colAtOffset(layout, 10)).toBe(0);
    expect(rowAtOffset(layout, 10)).toBe(0);
  });
});

describe("computeVisibleRange", () => {
  it("只覆盖视口内的行列", () => {
    const layout = layoutOf(1000, 100);
    const range = computeVisibleRange(layout, VIEWPORT, { x: 0, y: 0 });
    const body = bodySize(layout, VIEWPORT);
    // 可见行数应约等于 body 高度 / 行高,而不是全部 1000 行
    expect(range.row1 - range.row0).toBeLessThanOrEqual(
      Math.ceil(body.height / layout.rowHeight) + 1,
    );
    expect(range.row0).toBe(0);
    expect(range.col0).toBe(0);
    expect(range.col1).toBeLessThan(100);
  });

  it("滚动后可见区域随之平移", () => {
    const layout = layoutOf(1000, 100);
    const scrolled = computeVisibleRange(layout, VIEWPORT, { x: 0, y: layout.rowHeight * 50 });
    expect(scrolled.row0).toBe(50);
  });

  it("overscan 向外扩张但不越界", () => {
    const layout = layoutOf(1000, 100);
    const range = computeVisibleRange(layout, VIEWPORT, { x: 0, y: layout.rowHeight * 50 }, 3);
    expect(range.row0).toBe(47);

    const atTop = computeVisibleRange(layout, VIEWPORT, { x: 0, y: 0 }, 3);
    expect(atTop.row0).toBe(0);
  });

  it("滚到底部时不会超过总行数", () => {
    const layout = layoutOf(20, 4);
    const max = maxScroll(layout, VIEWPORT);
    const range = computeVisibleRange(layout, VIEWPORT, max, 5);
    expect(range.row1).toBe(20);
    expect(range.col1).toBe(4);
  });

  it("空表格返回空区域", () => {
    const layout = computeLayout({ rows: 0, cols: 0, colWidthUnits: [], zoom: 1 });
    expect(computeVisibleRange(layout, VIEWPORT, { x: 0, y: 0 })).toEqual({
      row0: 0,
      row1: 0,
      col0: 0,
      col1: 0,
    });
  });

  it("视口小到放不下表头时返回空区域而不是负数", () => {
    const layout = layoutOf(100, 4);
    const range = computeVisibleRange(layout, { width: 10, height: 10 }, { x: 0, y: 0 });
    expect(range).toEqual({ row0: 0, row1: 0, col0: 0, col1: 0 });
  });
});

describe("clampScroll / maxScroll", () => {
  it("内容小于视口时不可滚动", () => {
    const layout = layoutOf(2, 2);
    expect(maxScroll(layout, VIEWPORT)).toEqual({ x: 0, y: 0 });
    expect(clampScroll(layout, VIEWPORT, { x: 500, y: 500 })).toEqual({ x: 0, y: 0 });
  });

  it("负数滚动被夹到 0", () => {
    const layout = layoutOf(1000, 100);
    expect(clampScroll(layout, VIEWPORT, { x: -20, y: -20 })).toEqual({ x: 0, y: 0 });
  });

  it("超出末尾的滚动被夹到最大值", () => {
    const layout = layoutOf(1000, 100);
    const max = maxScroll(layout, VIEWPORT);
    expect(clampScroll(layout, VIEWPORT, { x: 1e9, y: 1e9 })).toEqual(max);
  });

  it("NaN 被当作 0", () => {
    const layout = layoutOf(1000, 100);
    expect(clampScroll(layout, VIEWPORT, { x: Number.NaN, y: Number.NaN })).toEqual({ x: 0, y: 0 });
  });
});

describe("hitTest", () => {
  const layout = layoutOf(1000, 100);

  it("命中单元格", () => {
    const hit = hitTest(layout, VIEWPORT, { x: 0, y: 0 }, layout.headerWidth + 10, layout.headerHeight + 10);
    expect(hit).toEqual({ kind: "cell", row: 0, col: 0 });
  });

  it("命中列头与行头", () => {
    expect(hitTest(layout, VIEWPORT, { x: 0, y: 0 }, layout.headerWidth + 60, 5)).toEqual({
      kind: "column-header",
      col: 1,
    });
    expect(hitTest(layout, VIEWPORT, { x: 0, y: 0 }, 5, layout.headerHeight + layout.rowHeight + 1)).toEqual({
      kind: "row-header",
      row: 1,
    });
    expect(hitTest(layout, VIEWPORT, { x: 0, y: 0 }, 5, 5)).toEqual({ kind: "corner" });
  });

  it("滚动后命中判定跟着偏移", () => {
    const scroll = { x: 0, y: layout.rowHeight * 10 };
    const hit = hitTest(layout, VIEWPORT, scroll, layout.headerWidth + 10, layout.headerHeight + 1);
    expect(hit).toEqual({ kind: "cell", row: 10, col: 0 });
  });

  it("缩放后命中判定依然准确", () => {
    const zoomed = layoutOf(1000, 100, 2);
    // 第 3 行第 2 列:内容坐标 (col 1 起点, row 2 起点)
    const x = zoomed.headerWidth + zoomed.colOffsets[1] + 1;
    const y = zoomed.headerHeight + zoomed.rowHeight * 2 + 1;
    expect(hitTest(zoomed, { width: 1200, height: 900 }, { x: 0, y: 0 }, x, y)).toEqual({
      kind: "cell",
      row: 2,
      col: 1,
    });
  });

  it("表格内容之外返回 outside", () => {
    const small = layoutOf(2, 2);
    const hit = hitTest(small, VIEWPORT, { x: 0, y: 0 }, 380, 280);
    expect(hit).toEqual({ kind: "outside" });
  });

  it("视口之外返回 outside", () => {
    expect(hitTest(layout, VIEWPORT, { x: 0, y: 0 }, -5, 10)).toEqual({ kind: "outside" });
  });
});

describe("scrollIntoView", () => {
  const layout = layoutOf(1000, 100);

  it("已完整可见时不动", () => {
    const scroll = { x: 0, y: 0 };
    expect(scrollIntoView(layout, VIEWPORT, scroll, { row: 1, col: 1 })).toEqual(scroll);
  });

  it("目标在下方时向下滚到刚好露出", () => {
    const body = bodySize(layout, VIEWPORT);
    const target = { row: 40, col: 0 };
    const next = scrollIntoView(layout, VIEWPORT, { x: 0, y: 0 }, target);
    expect(next.y).toBe((target.row + 1) * layout.rowHeight - body.height);
  });

  it("目标在上方时向上滚到对齐顶端", () => {
    const next = scrollIntoView(layout, VIEWPORT, { x: 0, y: layout.rowHeight * 50 }, {
      row: 10,
      col: 0,
    });
    expect(next.y).toBe(layout.rowHeight * 10);
  });

  it("横向同理", () => {
    const next = scrollIntoView(layout, VIEWPORT, { x: 0, y: 0 }, { row: 0, col: 30 });
    expect(next.x).toBeGreaterThan(0);
    expect(next.x).toBe(layout.colOffsets[31] - bodySize(layout, VIEWPORT).width);
  });
});

describe("anchoredScroll", () => {
  it("放大后指针下的内容坐标保持不变", () => {
    const pointer = 120;
    const scroll = 300;
    const next = anchoredScroll(scroll, pointer, 1, 2);
    // 缩放前指针指向内容坐标 (300+120)/1 = 420;缩放后应指向 420*2 = 840
    expect(next + pointer).toBe(840);
  });

  it("缩小同样成立", () => {
    const pointer = 50;
    const scroll = 200;
    const next = anchoredScroll(scroll, pointer, 2, 1);
    expect(next + pointer).toBe((scroll + pointer) / 2);
  });

  it("缩放比例不变时滚动量不变", () => {
    expect(anchoredScroll(123, 45, 1.5, 1.5)).toBeCloseTo(123);
  });
});

describe("scrollbarGeometry", () => {
  it("内容放得下时没有滚动条", () => {
    const layout = layoutOf(2, 2);
    const geometry = scrollbarGeometry(layout, VIEWPORT, { x: 0, y: 0 });
    expect(geometry.vertical).toBeNull();
    expect(geometry.horizontal).toBeNull();
  });

  it("内容超出时滑块长度反映可见比例", () => {
    const layout = layoutOf(1000, 100);
    const geometry = scrollbarGeometry(layout, VIEWPORT, { x: 0, y: 0 });
    expect(geometry.vertical).not.toBeNull();
    const body = bodySize(layout, VIEWPORT);
    expect(geometry.vertical!.thumb.height).toBeLessThan(body.height);
    expect(geometry.vertical!.thumb.y).toBe(layout.headerHeight);
  });

  it("滚到底时滑块贴到轨道末端", () => {
    const layout = layoutOf(1000, 100);
    const max = maxScroll(layout, VIEWPORT);
    const geometry = scrollbarGeometry(layout, VIEWPORT, max);
    const bar = geometry.vertical!;
    expect(bar.thumb.y + bar.thumb.height).toBeCloseTo(bar.track.y + bar.track.height, 5);
  });

  it("拖拽滑块能反推出滚动量", () => {
    const layout = layoutOf(1000, 100);
    const max = maxScroll(layout, VIEWPORT);
    expect(scrollFromThumbOffset(layout, VIEWPORT, "y", 0)).toBe(0);
    expect(scrollFromThumbOffset(layout, VIEWPORT, "y", 1e6)).toBe(max.y);
  });
});

describe("rangeForContentRect", () => {
  const layout = layoutOf(1000, 100);

  it("覆盖矩形内的行列", () => {
    const range = rangeForContentRect(layout, {
      x: 0,
      y: layout.rowHeight * 10,
      width: 96,
      height: layout.rowHeight * 5,
    });
    expect(range.row0).toBe(10);
    expect(range.row1).toBe(15);
    expect(range.col0).toBe(0);
    expect(range.col1).toBe(2);
  });

  it("margin 向外扩张,用于给缓存留冗余", () => {
    const rect = { x: 480, y: layout.rowHeight * 10, width: 96, height: layout.rowHeight * 5 };
    const tight = rangeForContentRect(layout, rect);
    const loose = rangeForContentRect(layout, rect, 200);
    expect(loose.col0).toBeLessThan(tight.col0);
    expect(loose.col1).toBeGreaterThan(tight.col1);
    expect(loose.row0).toBeLessThan(tight.row0);
  });

  it("扩张不会越过表格边界", () => {
    const range = rangeForContentRect(layout, { x: 0, y: 0, width: 96, height: 96 }, 100_000);
    expect(range.row0).toBe(0);
    expect(range.col0).toBe(0);
    expect(range.row1).toBe(1000);
    expect(range.col1).toBe(100);
  });

  it("空表或空矩形返回空区域", () => {
    const empty = computeLayout({ rows: 0, cols: 0, colWidthUnits: [], zoom: 1 });
    expect(rangeForContentRect(empty, { x: 0, y: 0, width: 100, height: 100 })).toEqual({
      row0: 0,
      row1: 0,
      col0: 0,
      col1: 0,
    });
    expect(rangeForContentRect(layout, { x: 0, y: 0, width: 0, height: 0 })).toEqual({
      row0: 0,
      row1: 0,
      col0: 0,
      col1: 0,
    });
  });

  it("比可见区域更大的矩形会取到更多行 —— 瓦片取数正是靠这一点", () => {
    const visible = computeVisibleRange(layout, VIEWPORT, { x: 0, y: 0 });
    const body = bodySize(layout, VIEWPORT);
    const tileRange = rangeForContentRect(layout, {
      x: 0,
      y: 0,
      width: body.width + 512,
      height: body.height + 512,
    });
    expect(tileRange.row1).toBeGreaterThan(visible.row1);
  });
});

describe("cellRect / rectContains", () => {
  it("单元格矩形与列偏移一致", () => {
    const layout = layoutOf(10, 4);
    expect(cellRect(layout, 2, 1)).toEqual({
      x: 48,
      y: 2 * layout.rowHeight,
      width: 48,
      height: layout.rowHeight,
    });
  });

  it("矩形包含判定含边界", () => {
    const rect = { x: 10, y: 10, width: 20, height: 20 };
    expect(rectContains(rect, 10, 10)).toBe(true);
    expect(rectContains(rect, 30, 30)).toBe(true);
    expect(rectContains(rect, 31, 30)).toBe(false);
  });
});

describe("冻结行列(freeze panes)", () => {
  function frozenLayout(rows = 100, cols = 6, fr = 1, fc = 2): GridLayout {
    return computeLayout({
      rows,
      cols,
      colWidthUnits: new Uint32Array(cols).fill(1),
      zoom: 1,
      frozenRows: fr,
      frozenCols: fc,
    });
  }

  it("computeLayout 记录冻结行列与像素跨度", () => {
    const layout = frozenLayout(100, 6, 1, 2);
    expect(layout.frozenRows).toBe(1);
    expect(layout.frozenCols).toBe(2);
    expect(layout.frozenHeight).toBe(layout.rowHeight);
    expect(layout.frozenWidth).toBe(layout.colOffsets[2]);
  });

  it("冻结数量被夹到总行列内", () => {
    const layout = computeLayout({
      rows: 3,
      cols: 2,
      colWidthUnits: new Uint32Array(2).fill(1),
      zoom: 1,
      frozenRows: 99,
      frozenCols: 99,
    });
    expect(layout.frozenRows).toBe(3);
    expect(layout.frozenCols).toBe(2);
  });

  it("cellScreenRect:冻结区不随对应轴滚动,其余减去滚动", () => {
    const layout = frozenLayout(100, 6, 1, 2);
    const scroll = { x: 100, y: 50 };
    // 冻结单元格(0,0):双向不滚,固定在表头右下角
    const frozenCell = cellScreenRect(layout, scroll, 0, 0);
    expect(frozenCell.x).toBe(layout.headerWidth);
    expect(frozenCell.y).toBe(layout.headerHeight);
    // 滚动单元格:两轴都减滚动
    const scrollCell = cellScreenRect(layout, scroll, 5, 4);
    expect(scrollCell.x).toBe(layout.headerWidth + layout.colOffsets[4] - scroll.x);
    expect(scrollCell.y).toBe(layout.headerHeight + 5 * layout.rowHeight - scroll.y);
    // 冻结列 + 滚动行:x 不滚、y 滚
    const leftBand = cellScreenRect(layout, scroll, 5, 1);
    expect(leftBand.x).toBe(layout.headerWidth + layout.colOffsets[1]);
    expect(leftBand.y).toBe(layout.headerHeight + 5 * layout.rowHeight - scroll.y);
  });

  it("hitTest:冻结带内的点映射到冻结行列(忽略滚动)", () => {
    const layout = frozenLayout(100, 6, 1, 2);
    const scroll = { x: 300, y: 200 };
    // 点在冻结列带内(x 落在第 0 列),即便已横向滚动,也应命中第 0 列
    const x = layout.headerWidth + 1;
    const y = layout.headerHeight + layout.frozenHeight + 1;
    const hit = hitTest(layout, VIEWPORT, scroll, x, y);
    expect(hit.kind).toBe("cell");
    if (hit.kind === "cell") expect(hit.col).toBe(0);
  });
});
