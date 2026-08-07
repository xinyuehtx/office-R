/**
 * 渲染管线的**几何阶段**:把「数据 + 缩放 + 滚动」换算成像素坐标。
 *
 * 这里全部是**纯函数**,不碰 canvas、不碰 DOM,因此:
 * - 可以独立单测(见 `geometry.test.ts`),坐标算错会被测试挡住;
 * - 绘制阶段只负责「照着算好的坐标画」,职责清晰。
 *
 * 坐标系约定:
 * - **内容坐标**:表格左上角为原点,不含表头,单位 CSS 像素(已含缩放);
 * - **视口坐标**:canvas 左上角为原点,含表头;
 * - 二者关系:`视口 x = 内容 x - scrollX + headerWidth`。
 */

import {
  BASE_CELL_PADDING,
  BASE_CHAR_WIDTH,
  BASE_FONT_SIZE,
  BASE_HEADER_HEIGHT,
  BASE_ROW_HEIGHT,
  MAX_COL_WIDTH,
  MIN_COL_WIDTH,
  MIN_HEADER_WIDTH,
  SCROLLBAR_MIN_THUMB,
  SCROLLBAR_SIZE,
} from "./theme";

/** 视口尺寸(CSS 像素)。 */
export interface Viewport {
  width: number;
  height: number;
}

/** 滚动位置(内容坐标,CSS 像素)。 */
export interface Scroll {
  x: number;
  y: number;
}

/** 单元格引用。 */
export interface CellRef {
  row: number;
  col: number;
}

/** 矩形(左上角 + 尺寸)。 */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** 布局阶段的产物:所有尺寸都已乘上缩放系数。 */
export interface GridLayout {
  rows: number;
  cols: number;
  zoom: number;
  /** 行高(所有行等高 —— CSV 没有行高信息)。 */
  rowHeight: number;
  /** 列头高度。 */
  headerHeight: number;
  /** 行头宽度(随行号位数变化,避免 6 位行号被截断)。 */
  headerWidth: number;
  /** 字号。 */
  fontSize: number;
  /** 单元格左右内边距。 */
  padding: number;
  /**
   * 列偏移前缀和,长度 `cols + 1`。
   * `colOffsets[c]` 是第 c 列左边界的内容坐标,`colOffsets[cols]` 即总宽度。
   */
  colOffsets: Float64Array;
  /** 内容总宽度。 */
  totalWidth: number;
  /** 内容总高度。 */
  totalHeight: number;
}

/** 可见区域(半开区间 `[row0, row1)` × `[col0, col1)`)。 */
export interface VisibleRange {
  row0: number;
  row1: number;
  col0: number;
  col1: number;
}

/** 布局输入。 */
export interface LayoutInput {
  rows: number;
  cols: number;
  /** 各列建议宽度(单位:半角字符数),来自 Rust 内核。 */
  colWidthUnits: ArrayLike<number>;
  zoom: number;
  /**
   * 是否把所有尺寸对齐到整像素。
   *
   * 渲染器在**设备像素**空间里工作,对齐整像素有两个好处:
   * 网格线不会被抗锯齿糊成灰边;滚动时的位图搬运(blit)是整像素平移,
   * 不会重采样发虚。
   */
  snap?: boolean;
}

/**
 * 计算布局。
 *
 * 列宽由内核给的「半角字符数」换算而来,并夹在 [MIN_COL_WIDTH, MAX_COL_WIDTH] 内:
 * 太窄看不清,太宽会让横向滚动变得没完没了。
 */
export function computeLayout({
  rows,
  cols,
  colWidthUnits,
  zoom,
  snap = false,
}: LayoutInput): GridLayout {
  const safeZoom = Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
  const safeCols = Math.max(0, Math.floor(cols));
  const safeRows = Math.max(0, Math.floor(rows));
  const round = snap ? (v: number) => Math.max(1, Math.round(v)) : (v: number) => v;

  const colOffsets = new Float64Array(safeCols + 1);
  for (let c = 0; c < safeCols; c += 1) {
    const units = Number(colWidthUnits[c] ?? 0);
    const raw = units * BASE_CHAR_WIDTH + BASE_CELL_PADDING * 2;
    const clamped = Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, raw));
    colOffsets[c + 1] = colOffsets[c] + round(clamped * safeZoom);
  }

  const rowHeight = round(BASE_ROW_HEIGHT * safeZoom);
  // 行头要放得下最大行号:位数越多越宽
  const digits = Math.max(2, String(Math.max(1, safeRows)).length);
  const headerWidth = round(
    Math.max(
      MIN_HEADER_WIDTH * safeZoom,
      (digits * BASE_CHAR_WIDTH + BASE_CELL_PADDING * 2) * safeZoom,
    ),
  );

  return {
    rows: safeRows,
    cols: safeCols,
    zoom: safeZoom,
    rowHeight,
    headerHeight: round(BASE_HEADER_HEIGHT * safeZoom),
    headerWidth,
    fontSize: BASE_FONT_SIZE * safeZoom,
    padding: BASE_CELL_PADDING * safeZoom,
    colOffsets,
    totalWidth: colOffsets[safeCols] ?? 0,
    totalHeight: rowHeight * safeRows,
  };
}

/** 单元格区域(视口坐标)的尺寸:视口去掉表头之后剩下的部分。 */
export function bodySize(layout: GridLayout, viewport: Viewport): Viewport {
  return {
    width: Math.max(0, viewport.width - layout.headerWidth),
    height: Math.max(0, viewport.height - layout.headerHeight),
  };
}

/** 最大可滚动距离。内容比视口小时为 0(不允许把内容拖出视野)。 */
export function maxScroll(layout: GridLayout, viewport: Viewport): Scroll {
  const body = bodySize(layout, viewport);
  return {
    x: Math.max(0, layout.totalWidth - body.width),
    y: Math.max(0, layout.totalHeight - body.height),
  };
}

/** 把滚动位置夹到合法范围。 */
export function clampScroll(layout: GridLayout, viewport: Viewport, scroll: Scroll): Scroll {
  const max = maxScroll(layout, viewport);
  return {
    x: Math.min(max.x, Math.max(0, Number.isFinite(scroll.x) ? scroll.x : 0)),
    y: Math.min(max.y, Math.max(0, Number.isFinite(scroll.y) ? scroll.y : 0)),
  };
}

/**
 * 二分查找某个内容坐标 x 落在哪一列。
 *
 * 列宽不等,所以不能用除法;前缀和 + 二分是 O(log cols),
 * 比线性扫描在超宽表格(上万列)上快得多。
 * 返回值被夹在 `[0, cols - 1]`;无列时返回 0。
 */
export function colAtOffset(layout: GridLayout, x: number): number {
  const { colOffsets, cols } = layout;
  if (cols === 0) return 0;
  if (x <= 0) return 0;
  if (x >= colOffsets[cols]) return cols - 1;

  let lo = 0;
  let hi = cols - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (colOffsets[mid] <= x) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

/** 某个内容坐标 y 落在哪一行。行等高,直接除。 */
export function rowAtOffset(layout: GridLayout, y: number): number {
  if (layout.rows === 0 || layout.rowHeight <= 0) return 0;
  const row = Math.floor(y / layout.rowHeight);
  return Math.min(layout.rows - 1, Math.max(0, row));
}

/**
 * 计算可见区域 —— 渲染管线的**裁剪阶段**。
 *
 * `overscan` 是向外多取的行列数:滚动时先画好边缘一点点,
 * 避免快速滚动时边上出现空白。
 */
export function computeVisibleRange(
  layout: GridLayout,
  viewport: Viewport,
  scroll: Scroll,
  overscan = 0,
): VisibleRange {
  const body = bodySize(layout, viewport);
  if (layout.rows === 0 || layout.cols === 0 || body.width <= 0 || body.height <= 0) {
    return { row0: 0, row1: 0, col0: 0, col1: 0 };
  }

  const row0 = Math.max(0, rowAtOffset(layout, scroll.y) - overscan);
  const row1 = Math.min(
    layout.rows,
    rowAtOffset(layout, scroll.y + body.height - 1) + 1 + overscan,
  );
  const col0 = Math.max(0, colAtOffset(layout, scroll.x) - overscan);
  const col1 = Math.min(
    layout.cols,
    colAtOffset(layout, scroll.x + body.width - 1) + 1 + overscan,
  );

  return { row0, row1, col0, col1 };
}

/**
 * 覆盖某个**内容矩形**的行列范围。
 *
 * 与 [`computeVisibleRange`] 的区别:那个算的是「视口里能看到什么」,
 * 这个算的是「这块内容矩形里有什么」—— 单元格层的瓦片比视口大一圈,
 * 取数必须按瓦片来,否则瓦片边缘那圈会画成空白,滚动露出来就穿帮了。
 *
 * @param margin 向外扩张的像素数,用于给缓存留冗余
 */
export function rangeForContentRect(
  layout: GridLayout,
  rect: Rect,
  margin = 0,
): VisibleRange {
  if (layout.rows === 0 || layout.cols === 0 || rect.width <= 0 || rect.height <= 0) {
    return { row0: 0, row1: 0, col0: 0, col1: 0 };
  }
  const left = rect.x - margin;
  const top = rect.y - margin;
  const right = rect.x + rect.width + margin;
  const bottom = rect.y + rect.height + margin;

  return {
    row0: rowAtOffset(layout, Math.max(0, top)),
    row1: Math.min(layout.rows, rowAtOffset(layout, Math.max(0, bottom - 1)) + 1),
    col0: colAtOffset(layout, Math.max(0, left)),
    col1: Math.min(layout.cols, colAtOffset(layout, Math.max(0, right - 1)) + 1),
  };
}

/** 单元格在**内容坐标**下的矩形。 */
export function cellRect(layout: GridLayout, row: number, col: number): Rect {
  return {
    x: layout.colOffsets[col] ?? 0,
    y: row * layout.rowHeight,
    width: (layout.colOffsets[col + 1] ?? 0) - (layout.colOffsets[col] ?? 0),
    height: layout.rowHeight,
  };
}

/** 视口中命中的目标区域。 */
export type HitTarget =
  | { kind: "cell"; row: number; col: number }
  | { kind: "column-header"; col: number }
  | { kind: "row-header"; row: number }
  | { kind: "corner" }
  | { kind: "outside" };

/**
 * 命中判定:视口坐标 →(单元格 / 表头 / 左上角)。
 *
 * 缩放后 `layout` 里的尺寸已经是缩放过的,所以这里不需要再乘 zoom ——
 * 「缩放后命中判定还准」正是靠这一点保证的。
 */
export function hitTest(
  layout: GridLayout,
  viewport: Viewport,
  scroll: Scroll,
  x: number,
  y: number,
): HitTarget {
  if (x < 0 || y < 0 || x > viewport.width || y > viewport.height) {
    return { kind: "outside" };
  }
  const inRowHeader = x < layout.headerWidth;
  const inColHeader = y < layout.headerHeight;

  if (inRowHeader && inColHeader) return { kind: "corner" };

  if (inColHeader) {
    if (layout.cols === 0) return { kind: "outside" };
    const contentX = x - layout.headerWidth + scroll.x;
    if (contentX >= layout.totalWidth) return { kind: "outside" };
    return { kind: "column-header", col: colAtOffset(layout, contentX) };
  }

  if (inRowHeader) {
    if (layout.rows === 0) return { kind: "outside" };
    const contentY = y - layout.headerHeight + scroll.y;
    if (contentY >= layout.totalHeight) return { kind: "outside" };
    return { kind: "row-header", row: rowAtOffset(layout, contentY) };
  }

  const contentX = x - layout.headerWidth + scroll.x;
  const contentY = y - layout.headerHeight + scroll.y;
  if (
    layout.rows === 0 ||
    layout.cols === 0 ||
    contentX >= layout.totalWidth ||
    contentY >= layout.totalHeight
  ) {
    return { kind: "outside" };
  }
  return {
    kind: "cell",
    row: rowAtOffset(layout, contentY),
    col: colAtOffset(layout, contentX),
  };
}

/**
 * 让某个单元格滚动进视野所需的新滚动位置(键盘移动选区时用)。
 *
 * 只在必要时滚动:单元格已经完整可见就原地不动,避免视图跳来跳去。
 */
export function scrollIntoView(
  layout: GridLayout,
  viewport: Viewport,
  scroll: Scroll,
  cell: CellRef,
): Scroll {
  const body = bodySize(layout, viewport);
  let { x, y } = scroll;
  const left = layout.colOffsets[cell.col] ?? 0;
  const right = layout.colOffsets[cell.col + 1] ?? left;
  if (left < x) x = left;
  else if (right > x + body.width) x = right - body.width;

  const top = cell.row * layout.rowHeight;
  const bottom = top + layout.rowHeight;
  if (top < y) y = top;
  else if (bottom > y + body.height) y = bottom - body.height;

  return clampScroll(layout, viewport, { x, y });
}

/**
 * 以指针为锚点缩放后的滚动位置。
 *
 * 目标是「指针下的那个单元格纹丝不动」:
 * 指针处的内容坐标 `u = (scroll + p) / zoom` 在缩放前后应保持不变,
 * 于是 `scroll' = u * zoom' - p`。
 *
 * @param scroll 缩放前的滚动量(单轴)
 * @param pointer 指针相对**单元格区域左上角**的偏移(单轴)
 */
export function anchoredScroll(
  scroll: number,
  pointer: number,
  oldZoom: number,
  newZoom: number,
): number {
  if (oldZoom <= 0) return scroll;
  return ((scroll + pointer) * newZoom) / oldZoom - pointer;
}

/** 滚动条的几何信息(视口坐标)。不需要时为 `null`。 */
export interface ScrollbarGeometry {
  vertical: { track: Rect; thumb: Rect } | null;
  horizontal: { track: Rect; thumb: Rect } | null;
}

/**
 * 计算自绘滚动条的位置。
 *
 * canvas 里没有原生滚动条,自己画一条既能提示「还有多少内容」,
 * 又能支持拖拽 —— 拖拽是鼠标用户最直觉的长距离滚动方式。
 */
export function scrollbarGeometry(
  layout: GridLayout,
  viewport: Viewport,
  scroll: Scroll,
): ScrollbarGeometry {
  const body = bodySize(layout, viewport);
  const max = maxScroll(layout, viewport);

  const vertical =
    max.y > 0 && body.height > SCROLLBAR_MIN_THUMB
      ? (() => {
          const trackY = layout.headerHeight;
          const trackH = body.height;
          const ratio = body.height / layout.totalHeight;
          const thumbH = Math.max(SCROLLBAR_MIN_THUMB, trackH * ratio);
          const thumbY = trackY + ((trackH - thumbH) * scroll.y) / max.y;
          return {
            track: {
              x: viewport.width - SCROLLBAR_SIZE,
              y: trackY,
              width: SCROLLBAR_SIZE,
              height: trackH,
            },
            thumb: {
              x: viewport.width - SCROLLBAR_SIZE,
              y: thumbY,
              width: SCROLLBAR_SIZE,
              height: thumbH,
            },
          };
        })()
      : null;

  const horizontal =
    max.x > 0 && body.width > SCROLLBAR_MIN_THUMB
      ? (() => {
          const trackX = layout.headerWidth;
          const trackW = body.width;
          const ratio = body.width / layout.totalWidth;
          const thumbW = Math.max(SCROLLBAR_MIN_THUMB, trackW * ratio);
          const thumbX = trackX + ((trackW - thumbW) * scroll.x) / max.x;
          return {
            track: {
              x: trackX,
              y: viewport.height - SCROLLBAR_SIZE,
              width: trackW,
              height: SCROLLBAR_SIZE,
            },
            thumb: {
              x: thumbX,
              y: viewport.height - SCROLLBAR_SIZE,
              width: thumbW,
              height: SCROLLBAR_SIZE,
            },
          };
        })()
      : null;

  return { vertical, horizontal };
}

/**
 * 由滑块沿轨道的偏移量反推滚动量(拖拽滚动条时用)。
 *
 * @param thumbOffset 滑块**顶端 / 左端**相对轨道起点的偏移(CSS 像素)
 */
export function scrollFromThumbOffset(
  layout: GridLayout,
  viewport: Viewport,
  axis: "x" | "y",
  thumbOffset: number,
): number {
  const body = bodySize(layout, viewport);
  const max = maxScroll(layout, viewport);
  const geometry = scrollbarGeometry(layout, viewport, { x: 0, y: 0 });
  const bar = axis === "y" ? geometry.vertical : geometry.horizontal;
  if (!bar) return 0;

  const trackLength = axis === "y" ? body.height : body.width;
  const thumbLength = axis === "y" ? bar.thumb.height : bar.thumb.width;
  // 滑块只能在「轨道长度 - 滑块长度」这段距离里移动
  const travel = trackLength - thumbLength;
  if (travel <= 0) return 0;

  const ratio = thumbOffset / travel;
  return Math.min(max[axis], Math.max(0, ratio * max[axis]));
}

/** 判断点是否落在矩形内。 */
export function rectContains(rect: Rect, x: number, y: number): boolean {
  return x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height;
}
