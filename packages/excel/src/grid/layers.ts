/**
 * 渲染管线的**绘制阶段**:把几何阶段算好的坐标画到 canvas 上。
 *
 * 这里的函数都只依赖 `CanvasRenderingContext2D` 与纯数据,不读组件状态,
 * 因此可以用一个记录调用的假 ctx 来单测(见 `renderer.test.ts`)。
 *
 * 约定:
 * - 每个函数进入时 `ctx` 的变换是「1 CSS 像素 = 1 单位」,退出时**必须**
 *   恢复到进入时的状态(`save`/`restore` 成对),否则状态会泄漏到下一层;
 * - 1px 的细线画在半像素上(`Math.round(x) + 0.5`),否则会被抗锯齿糊成 2px 灰边。
 */

import { columnLabel, rowLabel } from "./labels";
import {
  bodySize,
  cellScreenRect,
  colAtOffset,
  rowAtOffset,
  scrollbarGeometry,
  type CellRef,
  type GridLayout,
  type Rect,
  type Scroll,
  type VisibleRange,
  type Viewport,
} from "./geometry";
import { COLORS, FONT_FAMILY, SCROLLBAR_SIZE } from "./theme";
import { drawChartInRect } from "@tengxiaohyx/office-shared";
import type { CellStyle } from "../sheet";

/** 每格视觉样式查询(xlsx);返回 null 表示默认样式。 */
export type StyleAt = (row: number, col: number) => CellStyle | null | undefined;

/** `#RRGGBB` 归一化;非 6 位十六进制返回 null。 */
function cssHex(hex: string | undefined): string | null {
  return hex && /^[0-9a-fA-F]{6}$/.test(hex) ? `#${hex}` : null;
}

/** 绘制单元格文本所需的数据源。 */
export interface CellTextSource {
  /** 当前缓存的可见区域。 */
  range: VisibleRange;
  /** 行优先摊平的单元格文本,长度 = (row1-row0) × (col1-col0)。 */
  cells: string[];
}

/** 从缓存里取单元格文本;不在缓存范围内时返回空串。 */
export function cellTextAt(source: CellTextSource, row: number, col: number): string {
  const { range, cells } = source;
  if (row < range.row0 || row >= range.row1 || col < range.col0 || col >= range.col1) {
    return "";
  }
  const width = range.col1 - range.col0;
  return cells[(row - range.row0) * width + (col - range.col0)] ?? "";
}

/**
 * 文本裁剪缓存:`原文 + 可用宽度` → 裁剪后的显示文本。
 *
 * `measureText` 不便宜,而滚动时同一批文本会被反复测量,所以缓存起来。
 * 超过上限就整体清空 —— 简单粗暴,但可见文本数量有限,不会频繁触发。
 */
const CLIP_CACHE_LIMIT = 8_000;

export class TextFitter {
  private cache = new Map<string, string>();
  private font = "";

  /** 切换字体(缩放导致字号变化)时必须清空缓存:测量结果全都变了。 */
  setFont(font: string): void {
    if (this.font !== font) {
      this.font = font;
      this.cache.clear();
    }
  }

  /**
   * 把文本裁剪到 `maxWidth` 以内,超出时以省略号结尾。
   *
   * 先做一次「明显放得下」的快速判断,避开绝大多数 `measureText` 调用 ——
   * 这是滚动流畅度的关键:整屏文字每帧都测量会直接掉帧。
   */
  fit(ctx: CanvasRenderingContext2D, text: string, maxWidth: number, fontSize: number): string {
    if (text === "" || maxWidth <= 0) return "";

    // 单个字符最宽不超过字号(全角字符约等于字号宽),据此快速放行
    if (text.length * fontSize <= maxWidth) return text;

    const key = `${maxWidth.toFixed(1)}\u0000${text}`;
    const cached = this.cache.get(key);
    if (cached !== undefined) return cached;

    let result: string;
    if (ctx.measureText(text).width <= maxWidth) {
      result = text;
    } else {
      const ellipsis = "…";
      const ellipsisWidth = ctx.measureText(ellipsis).width;
      const budget = maxWidth - ellipsisWidth;
      if (budget <= 0) {
        result = "";
      } else {
        // 二分找最长的可放下前缀
        let lo = 0;
        let hi = text.length;
        while (lo < hi) {
          const mid = (lo + hi + 1) >> 1;
          if (ctx.measureText(text.slice(0, mid)).width <= budget) lo = mid;
          else hi = mid - 1;
        }
        result = lo === 0 ? ellipsis : text.slice(0, lo) + ellipsis;
      }
    }

    if (this.cache.size > CLIP_CACHE_LIMIT) this.cache.clear();
    this.cache.set(key, result);
    return result;
  }

  /** 缓存条目数,供测试断言。 */
  get size(): number {
    return this.cache.size;
  }
}

/** 拼出 canvas 的 font 字符串。字号变化时才需要重新拼,不要每帧拼。 */
export function fontString(fontSize: number, bold = false, italic = false): string {
  return `${italic ? "italic " : ""}${bold ? "600 " : ""}${fontSize}px ${FONT_FAMILY}`;
}

/** 单元格区域的绘制参数。 */
export interface BodyPaintParams {
  layout: GridLayout;
  /** 单元格区域尺寸(CSS 像素)。 */
  body: Viewport;
  scroll: Scroll;
  source: CellTextSource;
  fitter: TextFitter;
  /** 只重绘这块矩形(单元格区域局部坐标);不传则整块重绘。 */
  dirty?: Rect;
  /** 每格视觉样式查询(xlsx);不传则无样式。 */
  styleAt?: StyleAt;
}

/**
 * 绘制单元格区域(网格线 + 文本)。
 *
 * 坐标系:传入的 ctx 原点即单元格区域左上角(不含表头),
 * 由调用方通过 translate 事先摆好。
 */
export function paintBody(ctx: CanvasRenderingContext2D, params: BodyPaintParams): void {
  const { layout, body, scroll, source, fitter, dirty, styleAt } = params;

  const clip: Rect = dirty ?? { x: 0, y: 0, width: body.width, height: body.height };
  if (clip.width <= 0 || clip.height <= 0) return;

  ctx.save();
  ctx.beginPath();
  ctx.rect(clip.x, clip.y, clip.width, clip.height);
  ctx.clip();

  ctx.fillStyle = COLORS.cellBackground;
  ctx.fillRect(clip.x, clip.y, clip.width, clip.height);

  if (layout.rows === 0 || layout.cols === 0) {
    ctx.restore();
    return;
  }

  // 把脏矩形换算成需要重绘的行列范围
  const row0 = rowAtOffset(layout, scroll.y + clip.y);
  const row1 = Math.min(layout.rows, rowAtOffset(layout, scroll.y + clip.y + clip.height - 1) + 1);
  const col0 = colAtOffset(layout, scroll.x + clip.x);
  const col1 = Math.min(layout.cols, colAtOffset(layout, scroll.x + clip.x + clip.width - 1) + 1);

  const font = fontString(layout.fontSize);
  fitter.setFont(font);
  ctx.font = font;
  ctx.textBaseline = "middle";
  ctx.textAlign = "left";

  // 首行底色:CSV 的第一行通常是表头,给个浅底方便对照
  if (row0 === 0) {
    ctx.fillStyle = COLORS.firstRowBackground;
    ctx.fillRect(clip.x, -scroll.y, clip.width, layout.rowHeight);
  }

  // 0) 单元格填充底色(xlsx 样式):在网格线与文本之前铺
  if (styleAt) {
    for (let row = row0; row < row1; row += 1) {
      for (let col = col0; col < col1; col += 1) {
        const fill = cssHex(styleAt(row, col)?.fill);
        if (!fill) continue;
        const left = (layout.colOffsets[col] ?? 0) - scroll.x;
        const width = (layout.colOffsets[col + 1] ?? 0) - (layout.colOffsets[col] ?? 0);
        ctx.fillStyle = fill;
        ctx.fillRect(left, row * layout.rowHeight - scroll.y, width, layout.rowHeight);
      }
    }
  }

  // 1) 网格线:一次 path 画完所有线,比逐条 stroke 快得多
  ctx.strokeStyle = COLORS.gridLine;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let row = row0; row <= row1; row += 1) {
    const y = Math.round(row * layout.rowHeight - scroll.y) + 0.5;
    ctx.moveTo(clip.x, y);
    ctx.lineTo(clip.x + clip.width, y);
  }
  for (let col = col0; col <= col1; col += 1) {
    const x = Math.round((layout.colOffsets[col] ?? 0) - scroll.x) + 0.5;
    ctx.moveTo(x, clip.y);
    ctx.lineTo(x, clip.y + clip.height);
  }
  ctx.stroke();

  // 2) 单元格文本:按列夹一次剪裁区,列内所有单元格共用 ——
  //    比每个单元格 clip 一次省下大量状态切换
  ctx.fillStyle = COLORS.cellText;
  for (let col = col0; col < col1; col += 1) {
    const left = (layout.colOffsets[col] ?? 0) - scroll.x;
    const width = (layout.colOffsets[col + 1] ?? 0) - (layout.colOffsets[col] ?? 0);
    const innerWidth = width - layout.padding * 2;
    if (innerWidth <= 0) continue;

    ctx.save();
    ctx.beginPath();
    ctx.rect(left, clip.y, width, clip.height);
    ctx.clip();

    for (let row = row0; row < row1; row += 1) {
      const raw = cellTextAt(source, row, col);
      if (raw === "") continue;
      // 内嵌换行在单行单元格里不该真的换行,统一显示为空格
      const oneLine = raw.includes("\n") ? raw.replace(/[\r\n]+/g, " ") : raw;
      const style = styleAt?.(row, col);
      // 按样式切换字体/颜色(仅样式格,默认格复用外层设置)
      if (style && (style.bold || style.italic)) {
        ctx.font = fontString(layout.fontSize, style.bold, style.italic);
      }
      const text = fitter.fit(ctx, oneLine, innerWidth, layout.fontSize);
      if (text !== "") {
        const centerY = row * layout.rowHeight - scroll.y + layout.rowHeight / 2;
        const color = cssHex(style?.color);
        ctx.fillStyle = color ?? COLORS.cellText;
        // 对齐:默认左,支持 center/right
        let tx = left + layout.padding;
        if (style?.align === "center" || style?.align === "right") {
          const tw = ctx.measureText(text).width;
          tx =
            style.align === "center"
              ? left + (width - tw) / 2
              : left + width - layout.padding - tw;
        }
        ctx.fillText(text, tx, centerY);
      }
      // 恢复默认字体/色,供下一格
      if (style && (style.bold || style.italic)) {
        ctx.font = font;
      }
      if (style?.color) ctx.fillStyle = COLORS.cellText;
    }
    ctx.restore();
  }

  // 3) 单元格边框(xlsx 样式):覆盖默认网格线;逐边描线
  if (styleAt) {
    for (let row = row0; row < row1; row += 1) {
      for (let col = col0; col < col1; col += 1) {
        const border = styleAt(row, col)?.border;
        if (!border) continue;
        const cl = (layout.colOffsets[col] ?? 0) - scroll.x;
        const cr = (layout.colOffsets[col + 1] ?? 0) - scroll.x;
        const cty = row * layout.rowHeight - scroll.y;
        const cby = cty + layout.rowHeight;
        const side = (
          s: { w: number; color: string } | undefined,
          x1: number,
          y1: number,
          x2: number,
          y2: number,
        ) => {
          if (!s) return;
          ctx.strokeStyle = cssHex(s.color) ?? "#8c959f";
          ctx.lineWidth = s.w;
          ctx.beginPath();
          ctx.moveTo(x1, y1);
          ctx.lineTo(x2, y2);
          ctx.stroke();
        };
        side(border.top, cl, Math.round(cty) + 0.5, cr, Math.round(cty) + 0.5);
        side(border.bottom, cl, Math.round(cby) + 0.5, cr, Math.round(cby) + 0.5);
        side(border.left, Math.round(cl) + 0.5, cty, Math.round(cl) + 0.5, cby);
        side(border.right, Math.round(cr) + 0.5, cty, Math.round(cr) + 0.5, cby);
      }
    }
  }

  ctx.restore();
}

/** 冻结模式的单元格绘制参数。 */
export interface FrozenBodyPaintParams {
  layout: GridLayout;
  /** 单元格区域尺寸(CSS 像素)。 */
  body: Viewport;
  scroll: Scroll;
  /** 取任意单元格文本(渲染器用分象限的窗口回填)。 */
  getCell: (row: number, col: number) => string;
  fitter: TextFitter;
  /** 可见的滚动行列范围(不含冻结)。 */
  visible: VisibleRange;
}

/**
 * 冻结模式下绘制单元格区域。
 *
 * 把区域分成**四象限**:角(冻结行×冻结列,双向不滚)、顶带(冻结行×滚动列)、
 * 左带(滚动行×冻结列)、主体(都滚)。各象限用自己的滚动量绘制,再叠加冻结分隔线。
 *
 * 这条路径是**每帧全量重绘**(不走瓦片 GPU 平移)—— 冻结是用户主动开启的低频审阅态,
 * 不是 50 万行连续滚动的热路径,全量重绘足够流畅,且实现清晰、无回归风险。
 */
export function paintFrozenBody(ctx: CanvasRenderingContext2D, params: FrozenBodyPaintParams): void {
  const { layout, body, scroll, getCell, fitter, visible } = params;
  const { frozenRows, frozenCols, frozenWidth, frozenHeight, rowHeight } = layout;

  ctx.save();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, body.width, body.height);
  ctx.fillStyle = COLORS.cellBackground;
  ctx.fillRect(0, 0, body.width, body.height);

  const font = fontString(layout.fontSize);
  fitter.setFont(font);
  ctx.font = font;
  ctx.textBaseline = "middle";
  ctx.textAlign = "left";

  // 一个象限:行区间 [r0,r1) × 列区间 [c0,c1),裁剪到 clip,滚动量 (sx, sy)
  const drawQuadrant = (
    r0: number,
    r1: number,
    c0: number,
    c1: number,
    clip: Rect,
    sx: number,
    sy: number,
  ) => {
    if (clip.width <= 0 || clip.height <= 0 || r1 <= r0 || c1 <= c0) return;
    ctx.save();
    ctx.beginPath();
    ctx.rect(clip.x, clip.y, clip.width, clip.height);
    ctx.clip();

    // 网格线
    ctx.strokeStyle = COLORS.gridLine;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let row = r0; row <= r1; row += 1) {
      const y = Math.round(row * rowHeight - sy) + 0.5;
      ctx.moveTo(clip.x, y);
      ctx.lineTo(clip.x + clip.width, y);
    }
    for (let col = c0; col <= c1; col += 1) {
      const x = Math.round((layout.colOffsets[col] ?? 0) - sx) + 0.5;
      ctx.moveTo(x, clip.y);
      ctx.lineTo(x, clip.y + clip.height);
    }
    ctx.stroke();

    // 文本(按列夹剪裁区)
    ctx.fillStyle = COLORS.cellText;
    for (let col = c0; col < c1; col += 1) {
      const left = (layout.colOffsets[col] ?? 0) - sx;
      const width = (layout.colOffsets[col + 1] ?? 0) - (layout.colOffsets[col] ?? 0);
      const innerWidth = width - layout.padding * 2;
      if (innerWidth <= 0) continue;
      ctx.save();
      ctx.beginPath();
      ctx.rect(left, clip.y, width, clip.height);
      ctx.clip();
      for (let row = r0; row < r1; row += 1) {
        const raw = getCell(row, col);
        if (raw === "") continue;
        const oneLine = raw.includes("\n") ? raw.replace(/[\r\n]+/g, " ") : raw;
        const text = fitter.fit(ctx, oneLine, innerWidth, layout.fontSize);
        if (text === "") continue;
        const centerY = row * rowHeight - sy + rowHeight / 2;
        ctx.fillText(text, left + layout.padding, centerY);
      }
      ctx.restore();
    }
    ctx.restore();
  };

  const sRow0 = visible.row0;
  const sRow1 = visible.row1;
  const sCol0 = visible.col0;
  const sCol1 = visible.col1;
  const restW = body.width - frozenWidth;
  const restH = body.height - frozenHeight;

  // 主体(都滚)
  drawQuadrant(
    sRow0,
    sRow1,
    sCol0,
    sCol1,
    { x: frozenWidth, y: frozenHeight, width: restW, height: restH },
    scroll.x,
    scroll.y,
  );
  // 顶带:冻结行 × 滚动列(x 滚、y 不滚)
  drawQuadrant(
    0,
    frozenRows,
    sCol0,
    sCol1,
    { x: frozenWidth, y: 0, width: restW, height: frozenHeight },
    scroll.x,
    0,
  );
  // 左带:滚动行 × 冻结列(x 不滚、y 滚)
  drawQuadrant(
    sRow0,
    sRow1,
    0,
    frozenCols,
    { x: 0, y: frozenHeight, width: frozenWidth, height: restH },
    0,
    scroll.y,
  );
  // 角:冻结行 × 冻结列(都不滚)
  drawQuadrant(
    0,
    frozenRows,
    0,
    frozenCols,
    { x: 0, y: 0, width: frozenWidth, height: frozenHeight },
    0,
    0,
  );

  // 冻结分隔线(略重),提示「这里冻住了」
  if (frozenHeight > 0 || frozenWidth > 0) {
    ctx.strokeStyle = COLORS.headerBorder;
    ctx.lineWidth = 2;
    ctx.beginPath();
    if (frozenHeight > 0) {
      const y = Math.round(frozenHeight) + 0.5;
      ctx.moveTo(0, y);
      ctx.lineTo(body.width, y);
    }
    if (frozenWidth > 0) {
      const x = Math.round(frozenWidth) + 0.5;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, body.height);
    }
    ctx.stroke();
  }

  ctx.restore();
}

/** 表头绘制参数。 */
export interface HeaderPaintParams {
  layout: GridLayout;
  viewport: Viewport;
  scroll: Scroll;
  range: VisibleRange;
  /** 当前选中单元格,用于高亮所在行列的表头。 */
  active: CellRef | null;
  fitter: TextFitter;
  /**
   * 可视行 → 行头显示文本。过滤后可视行是紧凑的 `0..V`,但行头要显示**原始行号**,
   * 由渲染器按数据源的行映射提供;缺省时按 `可视行 + 1` 显示。
   */
  rowLabelText?: (visualRow: number) => string;
}

/**
 * 绘制固定的行头与列头。
 *
 * 表头每帧都重画:它便宜(几十个标签),而且滚动时标签内容一直在变,
 * 缓存反而更麻烦。真正贵的单元格区域才值得做增量。
 */
export function paintHeaders(ctx: CanvasRenderingContext2D, params: HeaderPaintParams): void {
  const { layout, viewport, scroll, range, active, fitter } = params;
  const rowText = params.rowLabelText ?? rowLabel;
  const { headerWidth, headerHeight, frozenCols, frozenRows, frozenWidth, frozenHeight } = layout;

  ctx.save();
  const font = fontString(layout.fontSize);
  fitter.setFont(font);
  ctx.font = font;
  ctx.textBaseline = "middle";

  // 列头背景
  ctx.fillStyle = COLORS.headerBackground;
  ctx.fillRect(0, 0, viewport.width, headerHeight);
  // 行头背景
  ctx.fillRect(0, 0, headerWidth, viewport.height);

  // 画一段列头标签:col 区间,x 按 frozen 决定是否减滚动;先裁剪到给定区域
  const drawColLabels = (c0: number, c1: number, clipX: number, clipW: number, frozen: boolean) => {
    if (clipW <= 0) return;
    ctx.save();
    ctx.beginPath();
    ctx.rect(clipX, 0, clipW, headerHeight);
    ctx.clip();
    ctx.textAlign = "center";
    for (let col = c0; col < c1; col += 1) {
      const left = headerWidth + (layout.colOffsets[col] ?? 0) - (frozen ? 0 : scroll.x);
      const width = (layout.colOffsets[col + 1] ?? 0) - (layout.colOffsets[col] ?? 0);
      const isActive = active?.col === col;
      if (isActive) {
        ctx.fillStyle = COLORS.headerActiveBackground;
        ctx.fillRect(left, 0, width, headerHeight);
      }
      ctx.fillStyle = isActive ? COLORS.headerActiveText : COLORS.headerText;
      ctx.fillText(columnLabel(col), left + width / 2, headerHeight / 2);
    }
    ctx.restore();
  };

  // 滚动列(裁到冻结列右侧),再画冻结列(固定在最左)
  drawColLabels(
    Math.max(range.col0, frozenCols),
    range.col1,
    headerWidth + frozenWidth,
    viewport.width - headerWidth - frozenWidth,
    false,
  );
  if (frozenCols > 0) {
    drawColLabels(0, frozenCols, headerWidth, frozenWidth, true);
  }

  const drawRowLabels = (r0: number, r1: number, clipY: number, clipH: number, frozen: boolean) => {
    if (clipH <= 0) return;
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, clipY, headerWidth, clipH);
    ctx.clip();
    ctx.textAlign = "right";
    for (let row = r0; row < r1; row += 1) {
      const top = headerHeight + row * layout.rowHeight - (frozen ? 0 : scroll.y);
      const isActive = active?.row === row;
      if (isActive) {
        ctx.fillStyle = COLORS.headerActiveBackground;
        ctx.fillRect(0, top, headerWidth, layout.rowHeight);
      }
      ctx.fillStyle = isActive ? COLORS.headerActiveText : COLORS.headerText;
      ctx.fillText(rowText(row), headerWidth - layout.padding, top + layout.rowHeight / 2);
    }
    ctx.restore();
  };

  drawRowLabels(
    Math.max(range.row0, frozenRows),
    range.row1,
    headerHeight + frozenHeight,
    viewport.height - headerHeight - frozenHeight,
    false,
  );
  if (frozenRows > 0) {
    drawRowLabels(0, frozenRows, headerHeight, frozenHeight, true);
  }

  // 表头分隔线与左上角
  ctx.strokeStyle = COLORS.headerBorder;
  ctx.lineWidth = 1;
  ctx.beginPath();
  const hy = Math.round(headerHeight) + 0.5;
  ctx.moveTo(0, hy);
  ctx.lineTo(viewport.width, hy);
  const hx = Math.round(headerWidth) + 0.5;
  ctx.moveTo(hx, 0);
  ctx.lineTo(hx, viewport.height);
  ctx.stroke();

  ctx.restore();
}

/** 覆盖层里绘制的一张图片:锚点单元格 + 已解码位图 + 尺寸(设备像素,oneCell 用)。 */
export interface OverlayImage {
  fromRow: number;
  fromCol: number;
  toRow?: number;
  toCol?: number;
  /** oneCellAnchor 尺寸(设备像素);twoCell 用 from/to 单元格跨度。 */
  extWDev?: number;
  extHDev?: number;
  image: CanvasImageSource & { complete?: boolean; naturalWidth?: number };
}

/** 覆盖层绘制参数(hover / 选中 / 滚动条 / 内嵌图片)。 */
export interface OverlayPaintParams {
  layout: GridLayout;
  viewport: Viewport;
  scroll: Scroll;
  hover: CellRef | null;
  selection: CellRef | null;
  /** 多格选区(已归一化,含首尾)。存在且非单格时,整块高亮 + 外框。 */
  selectionRange?: { row0: number; row1: number; col0: number; col1: number } | null;
  /** 内嵌图片(xlsx);在选区之下绘制。 */
  images?: OverlayImage[];
  /** 内嵌图表(xlsx);在图片之上、选区之下绘制。 */
  charts?: OverlayChart[];
  /** 单元格内迷你图(xlsx)。 */
  sparklines?: OverlaySparkline[];
  /** 合并单元格区域(xlsx);盖住内部网格线 + 跨区绘制左上角文本。 */
  merges?: OverlayMerge[];
  /** 字号(设备像素);合并区文本用。 */
  fontSize?: number;
  /** 文本裁剪器;合并区文本用。 */
  fitter?: TextFitter;
}

/** 合并单元格区域 + 左上角文本(设备像素坐标由 cellScreenRect 求)。 */
export interface OverlayMerge {
  row0: number;
  col0: number;
  row1: number;
  col1: number;
  text: string;
}

/** 单元格内迷你图(xlsx)。 */
export interface OverlaySparkline {
  row: number;
  col: number;
  kind: string;
  values: number[];
}

/** 内嵌图表(xlsx);锚定单元格区域 + 系列数据。 */
export interface OverlayChart {
  fromRow: number;
  fromCol: number;
  toRow?: number;
  toCol?: number;
  kind: string;
  title?: string;
  series: number[][];
  categories: string[];
}

/**
 * 绘制交互反馈层。
 *
 * 单独一层的意义:鼠标移动时只要重画这一层(加上便宜的表头),
 * 昂贵的单元格层原封不动 —— 这就是「hover 不掉帧」的原因。
 */
export function paintOverlay(ctx: CanvasRenderingContext2D, params: OverlayPaintParams): void {
  const { layout, viewport, scroll, hover, selection, selectionRange, images, charts, merges, sparklines } =
    params;
  const body = bodySize(layout, viewport);
  if (body.width <= 0 || body.height <= 0) return;

  ctx.save();
  ctx.beginPath();
  ctx.rect(layout.headerWidth, layout.headerHeight, body.width, body.height);
  ctx.clip();

  // 合并单元格(在图片/选区之下):用白底盖住区域内部网格线与重复文本,
  // 再跨区重绘左上角文本 + 外框 —— 视觉上「合并」为一格。
  if (merges && merges.length > 0 && params.fitter && params.fontSize) {
    const font = fontString(params.fontSize);
    params.fitter.setFont(font);
    ctx.font = font;
    ctx.textBaseline = "middle";
    ctx.textAlign = "left";
    const pad = layout.padding;
    for (const m of merges) {
      const tl = cellScreenRect(layout, scroll, m.row0, m.col0);
      const br = cellScreenRect(layout, scroll, m.row1, m.col1);
      const x = tl.x;
      const y = tl.y;
      const w = br.x + br.width - tl.x;
      const h = br.y + br.height - tl.y;
      if (w <= 0 || h <= 0) continue;
      ctx.fillStyle = COLORS.cellBackground;
      ctx.fillRect(x, y, w, h);
      if (m.text) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(x, y, w, h);
        ctx.clip();
        ctx.fillStyle = COLORS.cellText;
        const text = params.fitter.fit(ctx, m.text.replace(/[\r\n]+/g, " "), w - pad * 2, params.fontSize);
        ctx.fillText(text, x + pad, y + h / 2);
        ctx.restore();
      }
      ctx.strokeStyle = COLORS.gridLine;
      ctx.lineWidth = 1;
      ctx.strokeRect(Math.round(x) + 0.5, Math.round(y) + 0.5, Math.round(w), Math.round(h));
    }
  }

  // 内嵌图片(在 hover/选区之下):按锚点单元格定位,尺寸取 from→to 跨度或 ext
  if (images && images.length > 0) {
    for (const im of images) {
      if (im.image.complete === false || im.image.naturalWidth === 0) continue;
      const tl = cellScreenRect(layout, scroll, im.fromRow, im.fromCol);
      let w = im.extWDev ?? 0;
      let h = im.extHDev ?? 0;
      if (im.toRow !== undefined && im.toCol !== undefined) {
        const br = cellScreenRect(layout, scroll, im.toRow, im.toCol);
        w = br.x - tl.x;
        h = br.y - tl.y;
      }
      if (w > 0 && h > 0) {
        try {
          ctx.drawImage(im.image, tl.x, tl.y, w, h);
        } catch {
          // 尚未解码完成等情况下忽略
        }
      }
    }
  }

  // 内嵌图表:按锚点区域绘制简单柱/线/饼图
  if (charts && charts.length > 0 && params.fontSize) {
    for (const ch of charts) {
      const tl = cellScreenRect(layout, scroll, ch.fromRow, ch.fromCol);
      let w = layout.rowHeight * 8;
      let h = layout.rowHeight * 6;
      if (ch.toRow !== undefined && ch.toCol !== undefined) {
        const br = cellScreenRect(layout, scroll, ch.toRow, ch.toCol);
        w = br.x - tl.x;
        h = br.y - tl.y;
      }
      if (w > 8 && h > 8) drawChartInRect(ctx, ch, tl.x, tl.y, w, h, params.fontSize, FONT_FAMILY);
    }
  }

  // 单元格内迷你图:在宿主单元格内画折线/柱
  if (sparklines && sparklines.length > 0) {
    const inset = 2;
    for (const sp of sparklines) {
      if (sp.values.length === 0) continue;
      const cell = cellScreenRect(layout, scroll, sp.row, sp.col);
      const px = cell.x + inset;
      const py = cell.y + inset;
      const pw = cell.width - inset * 2;
      const ph = cell.height - inset * 2;
      if (pw <= 2 || ph <= 2) continue;
      // 用循环而非 Math.min(...values):展开运算符在数组超过约 12 万元素时会抛
      // RangeError,而这里位于 overlay 层 —— 每帧、每次 hover 都会重抛,网格将彻底不出图。
      let min = sp.values[0];
      let max = sp.values[0];
      for (let i = 1; i < sp.values.length; i += 1) {
        const v = sp.values[i];
        if (v < min) min = v;
        if (v > max) max = v;
      }
      const span = max - min || 1;
      const norm = (v: number) => (v - min) / span;
      ctx.save();
      ctx.beginPath();
      ctx.rect(cell.x, cell.y, cell.width, cell.height);
      ctx.clip();
      if (sp.kind === "column" || sp.kind === "stacked") {
        const n = sp.values.length;
        const bw = pw / n;
        for (let i = 0; i < n; i += 1) {
          const bh = Math.max(1, norm(sp.values[i]) * ph);
          ctx.fillStyle = "#4c78a8";
          ctx.fillRect(px + i * bw, py + ph - bh, Math.max(1, bw - 1), bh);
        }
      } else {
        ctx.strokeStyle = "#4c78a8";
        ctx.lineWidth = 1;
        ctx.beginPath();
        sp.values.forEach((v, i) => {
          const x = px + (sp.values.length <= 1 ? 0 : (i / (sp.values.length - 1)) * pw);
          const y = py + ph - norm(v) * ph;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        });
        ctx.stroke();
      }
      ctx.restore();
    }
  }

  // 坐标系:视口原点。用 cellScreenRect 定位,天然处理冻结(冻结轴不减滚动)。
  if (hover && !(selection && hover.row === selection.row && hover.col === selection.col)) {
    const rect = cellScreenRect(layout, scroll, hover.row, hover.col);
    ctx.fillStyle = COLORS.hoverBackground;
    ctx.fillRect(rect.x, rect.y, rect.width, rect.height);
  }

  // 多格选区:整块淡色高亮 + 外框(活动格另画实心边框)
  const isMulti =
    selectionRange &&
    (selectionRange.row0 !== selectionRange.row1 || selectionRange.col0 !== selectionRange.col1);
  if (isMulti && selectionRange) {
    const tl = cellScreenRect(layout, scroll, selectionRange.row0, selectionRange.col0);
    const br = cellScreenRect(layout, scroll, selectionRange.row1, selectionRange.col1);
    const x = tl.x;
    const y = tl.y;
    const w = br.x + br.width - tl.x;
    const h = br.y + br.height - tl.y;
    ctx.fillStyle = COLORS.selectionBackground;
    ctx.fillRect(x, y, w, h);
    ctx.strokeStyle = COLORS.selectionBorder;
    ctx.lineWidth = 2;
    ctx.strokeRect(x + 1, y + 1, w - 2, h - 2);
  }

  if (selection) {
    const rect = cellScreenRect(layout, scroll, selection.row, selection.col);
    if (!isMulti) {
      ctx.fillStyle = COLORS.selectionBackground;
      ctx.fillRect(rect.x, rect.y, rect.width, rect.height);
    }
    ctx.strokeStyle = COLORS.selectionBorder;
    ctx.lineWidth = 2;
    // 边框画在格子内侧,避免被相邻格子盖住一半
    ctx.strokeRect(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
  }

  ctx.restore();

  paintScrollbars(ctx, layout, viewport, scroll);
}

/** 绘制自绘滚动条。canvas 里没有原生滚动条,只能自己画。 */
function paintScrollbars(
  ctx: CanvasRenderingContext2D,
  layout: GridLayout,
  viewport: Viewport,
  scroll: Scroll,
): void {
  const geometry = scrollbarGeometry(layout, viewport, scroll);
  ctx.save();
  for (const bar of [geometry.vertical, geometry.horizontal]) {
    if (!bar) continue;
    ctx.fillStyle = COLORS.scrollbarTrack;
    ctx.fillRect(bar.track.x, bar.track.y, bar.track.width, bar.track.height);
    ctx.fillStyle = COLORS.scrollbarThumb;
    const radius = SCROLLBAR_SIZE / 2;
    roundRect(ctx, bar.thumb, radius);
    ctx.fill();
  }
  ctx.restore();
}

/** 圆角矩形路径。滚动条滑块用圆头看起来更像原生滚动条。 */
function roundRect(ctx: CanvasRenderingContext2D, rect: Rect, radius: number): void {
  const r = Math.min(radius, rect.width / 2, rect.height / 2);
  ctx.beginPath();
  ctx.moveTo(rect.x + r, rect.y);
  ctx.lineTo(rect.x + rect.width - r, rect.y);
  ctx.arcTo(rect.x + rect.width, rect.y, rect.x + rect.width, rect.y + r, r);
  ctx.lineTo(rect.x + rect.width, rect.y + rect.height - r);
  ctx.arcTo(rect.x + rect.width, rect.y + rect.height, rect.x + rect.width - r, rect.y + rect.height, r);
  ctx.lineTo(rect.x + r, rect.y + rect.height);
  ctx.arcTo(rect.x, rect.y + rect.height, rect.x, rect.y + rect.height - r, r);
  ctx.lineTo(rect.x, rect.y + r);
  ctx.arcTo(rect.x, rect.y, rect.x + r, rect.y, r);
  ctx.closePath();
}

/** 没有数据时的占位画面。 */
export function paintEmptyState(
  ctx: CanvasRenderingContext2D,
  viewport: Viewport,
  message: string,
  fontSize = 13,
): void {
  ctx.save();
  ctx.fillStyle = COLORS.cellBackground;
  ctx.fillRect(0, 0, viewport.width, viewport.height);
  if (message !== "") {
    ctx.fillStyle = COLORS.headerText;
    ctx.font = fontString(fontSize);
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(message, viewport.width / 2, viewport.height / 2);
  }
  ctx.restore();
}
