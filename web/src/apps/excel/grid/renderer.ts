/**
 * 渲染管线的**调度与合成阶段**。
 *
 * 管线共四段,每段职责单一、可独立验证:
 *
 * ```text
 * ① 数据    SheetHandle(WASM 里的表格)+ 可见区域窗口缓存
 *      ↓
 * ② 几何    computeLayout / computeVisibleRange   ← geometry.ts,纯函数
 *      ↓
 * ③ 裁剪    瓦片是否还盖得住可见区域(tile.ts)/ planScrollBlit 脏矩形
 *      ↓
 * ④ 绘制    三张互相独立的 canvas 图层,谁脏画谁
 * ```
 *
 * # 为什么是三张 canvas 而不是一张
 *
 * 表格里三类内容的**变化频率相差两个数量级**:
 *
 * | 图层 | 内容 | 什么时候需要重画 |
 * | --- | --- | --- |
 * | body | 单元格文本 + 网格线(最贵) | 数据/缩放变化,或滚出瓦片余量 |
 * | headers | 行列头(便宜) | 滚动、缩放、选区变化 |
 * | overlay | hover / 选中 / 滚动条(最便宜) | 鼠标一动就变 |
 *
 * 挤在一张画布上,任何一点变化都要把三者全部重画一遍 —— 鼠标划过表格
 * 就会不停地重绘上千个单元格。拆成三张**堆叠的 DOM canvas** 后:
 *
 * - 谁变了只画谁,hover 只碰最上面那张几乎空白的 overlay;
 * - **三张画布的叠加交给浏览器合成器用 GPU 完成**,主线程不再每帧做一次
 *   全视口的 `drawImage` 合成。每层都加了 `will-change: transform`,
 *   会被提升为独立的合成层。
 *
 * # 滚动为什么几乎不用画
 *
 * body 图层是一块比可见区域**四周各大一圈**的瓦片(见 `tile.ts`)。
 * 只要滚动没超出这圈余量,就只改一下 CSS `transform` ——
 * 平移由 GPU 完成,主线程一个像素都不用画。滚出余量才重新锚定瓦片,
 * 且重锚时用位图平移复用重叠部分,只补新露出来的窄带。
 *
 * # 坐标系
 *
 * 渲染器内部统一使用**设备像素**(CSS 像素 × dpr),这样位图平移永远是整像素、
 * 网格线永远压在像素中心;对外的公开方法则一律收发 CSS 像素。
 */

import { flattenWindow, type SheetHandle } from "../../shared/sheet";
import type { Tracer } from "../../shared/logger";
import {
  bodySize,
  clampScroll,
  computeLayout,
  computeVisibleRange,
  colAtOffset,
  hitTest,
  rowAtOffset,
  scrollIntoView,
  scrollFromThumbOffset,
  scrollbarGeometry,
  rangeForContentRect,
  type CellRef,
  type GridLayout,
  type HitTarget,
  type Rect,
  type Scroll,
  type VisibleRange,
  type Viewport,
} from "./geometry";
import {
  cellTextAt,
  paintBody,
  paintEmptyState,
  paintFrozenBody,
  paintHeaders,
  paintOverlay,
  TextFitter,
  type CellTextSource,
  type StyleAt,
  type OverlayImage,
  type OverlayMerge,
  type OverlayChart,
  type OverlaySparkline,
} from "./layers";
import {
  anchorTile,
  tileCovers,
  tileSizeChanged,
  tileSizeFor,
  tileTranslation,
  TILE_MARGIN,
  type Tile,
} from "./tile";

/** 三个图层的名字。 */
export type LayerName = "body" | "headers" | "overlay";

/** 一层画布。 */
export interface Layer {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
}

/**
 * 创建 DOM 元素的工厂。
 *
 * 图层是渲染器自己建出来的(只有它知道要几层、怎么叠),但测试环境里没有
 * 可用的 canvas 上下文,所以把「创建元素」这一步开成注入点。
 */
export type ElementFactory = (tag: "canvas" | "div") => HTMLElement;

/** 滚动位图平移方案。 */
export interface BlitPlan {
  /** 水平位移(设备像素,正数表示内容向左移)。 */
  dx: number;
  /** 垂直位移。 */
  dy: number;
  /** 平移后新露出来、需要重绘的矩形(瓦片局部坐标)。 */
  exposed: Rect[];
}

/**
 * 规划一次位图平移。
 *
 * 返回 `null` 表示「不值得平移」(位移超过整块,平移完还是得全画),
 * 调用方应退回整块重绘。
 */
export function planScrollBlit(prev: Scroll, next: Scroll, area: Viewport): BlitPlan | null {
  const dx = next.x - prev.x;
  const dy = next.y - prev.y;
  if (dx === 0 && dy === 0) return { dx: 0, dy: 0, exposed: [] };
  if (Math.abs(dx) >= area.width || Math.abs(dy) >= area.height) return null;

  const exposed: Rect[] = [];
  if (dx > 0) {
    exposed.push({ x: area.width - dx, y: 0, width: dx, height: area.height });
  } else if (dx < 0) {
    exposed.push({ x: 0, y: 0, width: -dx, height: area.height });
  }
  if (dy > 0) {
    exposed.push({ x: 0, y: area.height - dy, width: area.width, height: dy });
  } else if (dy < 0) {
    exposed.push({ x: 0, y: 0, width: area.width, height: -dy });
  }
  return { dx, dy, exposed };
}

/** 渲染统计,用于性能观测。 */
export interface RendererStats {
  /** 累计出帧数。 */
  frames: number;
  /** 最近一帧耗时(毫秒)。 */
  lastFrameMs: number;
  /** 最近若干帧的平均耗时。 */
  avgFrameMs: number;
  /** 首帧耗时(从拿到表格到画完第一帧)。 */
  firstFrameMs: number | null;
  /** 单元格层整块重绘次数。 */
  fullRepaints: number;
  /** 单元格层增量(位图平移 + 补窄带)重绘次数。 */
  incrementalRepaints: number;
  /** **只改 CSS transform、一个像素都没画**的滚动帧数 —— 纯 GPU 平移。 */
  gpuScrolls: number;
  /** 各图层的重绘次数,用来验证「谁变了只画谁」。 */
  layerPaints: Record<LayerName, number>;
  /** 向 WASM 取窗口数据的次数。 */
  windowFetches: number;
}

/** 渲染器构造参数。 */
export interface GridRendererOptions {
  /** 图层挂载的容器(需要是定位上下文)。 */
  container: HTMLElement;
  /** 元素工厂。默认 `document.createElement`。 */
  createElement?: ElementFactory;
  /** 帧调度器。默认 `requestAnimationFrame`;测试里可注入手动触发的版本。 */
  schedule?: (callback: () => void) => number;
  /** 取消帧调度。 */
  cancel?: (handle: number) => void;
  /** 计时函数。 */
  now?: () => number;
  /** 日志。 */
  tracer?: Tracer;
  /** 统计更新回调。 */
  onStats?: (stats: RendererStats) => void;
}

/**
 * 单元格文本窗口在瓦片之外再多缓存的像素数。
 *
 * 缓存范围比瓦片大两圈余量,瓦片重新锚定一次通常仍落在缓存内,
 * 于是能走「位图平移 + 补窄带」而不是整块重绘。
 */
const WINDOW_MARGIN = TILE_MARGIN * 2;

/** 平均帧耗时的滑动窗口大小。 */
const FRAME_SAMPLES = 60;

/** 图层的堆叠顺序:表头压住单元格,覆盖层压住一切。 */
const LAYER_ORDER: LayerName[] = ["body", "headers", "overlay"];

/**
 * canvas 表格渲染器。
 *
 * 生命周期:`new` → `resize` → `setSheet` → (交互) → `destroy`。
 * 所有 `set*` 方法只更新状态并打脏标记,真正的绘制发生在下一个动画帧。
 */
export class GridRenderer {
  private readonly container: HTMLElement;
  private readonly createElement: ElementFactory;
  private readonly schedule: (callback: () => void) => number;
  private readonly cancel: (handle: number) => void;
  private readonly now: () => number;
  private readonly tracer?: Tracer;
  private readonly onStats?: (stats: RendererStats) => void;

  /** body 图层的裁剪容器:瓦片比可见区域大,靠它裁掉溢出表头的部分。 */
  private bodyClip: HTMLElement | null = null;
  private layers: Partial<Record<LayerName, Layer>> = {};
  /** 所有已建出的画布,仅用于卸载时清理(可能包含拿不到上下文的层)。 */
  private readonly canvases: HTMLCanvasElement[] = [];
  /** 位图平移用的暂存层(离屏,不进 DOM)。 */
  private scratch: Layer | null = null;

  private sheet: SheetHandle | null = null;
  /** 可视行 → 行头显示文本;过滤时映射到原始行号,否则即 `可视行 + 1`。 */
  private rowLabelText: (visualRow: number) => string = (r) => String(r + 1);
  /** 冻结的顶部行数 / 左侧列数(0 表示不冻结)。 */
  private frozenRows = 0;
  private frozenCols = 0;
  /** 列宽手动覆盖(基准 px,按列号索引;0/undefined 表示自动)。 */
  private colWidthOverrides: number[] = [];
  private layout: GridLayout = computeLayout({ rows: 0, cols: 0, colWidthUnits: [], zoom: 1 });

  /** 设备像素比。 */
  private dpr = 1;
  /** 视口尺寸(设备像素)。 */
  private viewport: Viewport = { width: 0, height: 0 };
  /** 滚动位置(设备像素,始终为整数)。 */
  private scroll: Scroll = { x: 0, y: 0 };
  /** 用户设定的缩放(CSS 层面的倍率,不含 dpr)。 */
  private zoom = 1;

  private hover: CellRef | null = null;
  private selection: CellRef | null = null;
  /** 内嵌图片(xlsx);已按当前 dpr 预算好尺寸。 */
  private images: OverlayImage[] = [];
  /** 合并单元格区域(xlsx)。 */
  private merges: OverlayMerge[] = [];
  /** 内嵌图表(xlsx)。 */
  private charts: OverlayChart[] = [];
  /** 单元格内迷你图(xlsx)。 */
  private sparklines: OverlaySparkline[] = [];
  private selectionRange: { row0: number; row1: number; col0: number; col1: number } | null = null;

  /** 当前瓦片;`null` 表示还没画过。 */
  private tile: Tile | null = null;
  /** 上一帧应用过的 transform,避免重复写样式。 */
  private appliedTranslation: { x: number; y: number } | null = null;
  /** 单元格文本窗口缓存。 */
  private windowCache: CellTextSource | null = null;

  private layoutDirty = true;
  private dirty: Record<LayerName, boolean> = { body: true, headers: true, overlay: true };
  private frameHandle: number | null = null;
  private destroyed = false;

  private readonly fitter = new TextFitter();
  private readonly frameTimes: number[] = [];
  private sheetSetAt: number | null = null;
  private stats: RendererStats = {
    frames: 0,
    lastFrameMs: 0,
    avgFrameMs: 0,
    firstFrameMs: null,
    fullRepaints: 0,
    incrementalRepaints: 0,
    gpuScrolls: 0,
    layerPaints: { body: 0, headers: 0, overlay: 0 },
    windowFetches: 0,
  };

  constructor(options: GridRendererOptions) {
    this.container = options.container;
    this.createElement =
      options.createElement ?? ((tag) => document.createElement(tag) as HTMLElement);
    this.schedule =
      options.schedule ?? ((callback) => globalThis.requestAnimationFrame?.(callback) ?? 0);
    this.cancel = options.cancel ?? ((handle) => globalThis.cancelAnimationFrame?.(handle));
    this.now = options.now ?? (() => performance.now());
    this.tracer = options.tracer;
    this.onStats = options.onStats;
    this.buildLayers();
  }

  // ---------- 图层搭建 ----------

  /**
   * 建立图层结构:
   *
   * ```text
   * container
   *   ├─ div.bodyClip  (定位到单元格区域,overflow:hidden 裁掉瓦片溢出的部分)
   *   │    └─ canvas body     ← 瓦片,靠 transform 平移
   *   ├─ canvas headers       ← 覆盖在上面,固定不动
   *   └─ canvas overlay       ← 最上层,hover/选中/滚动条
   * ```
   */
  private buildLayers(): void {
    const clip = this.createElement("div");
    clip.style.position = "absolute";
    clip.style.overflow = "hidden";
    clip.style.pointerEvents = "none";
    this.bodyClip = clip;
    this.container.appendChild(clip);

    for (const name of LAYER_ORDER) {
      const canvas = this.createElement("canvas") as HTMLCanvasElement;
      canvas.style.position = "absolute";
      canvas.style.left = "0";
      canvas.style.top = "0";
      // 事件由 React 管理的交互层统一接收,画布只负责显示
      canvas.style.pointerEvents = "none";
      if (name === "body") {
        // 只有单元格层会被 transform 平移,所以只给它加 will-change ——
        // 这个属性会把元素常驻提升为合成层(占显存),不该到处乱加。
        canvas.style.willChange = "transform";
      }
      if (canvas.dataset) canvas.dataset.layer = name;

      // 先挂进 DOM 再取上下文:即便拿不到 2D 上下文(极老的浏览器 / 显存不足),
      // 结构也保持完整,只是这一层不参与绘制。
      if (name === "body") {
        this.bodyClip.appendChild(canvas);
      } else {
        this.container.appendChild(canvas);
      }
      this.canvases.push(canvas);

      const ctx = canvas.getContext("2d");
      if (ctx) this.layers[name] = { canvas, ctx };
    }
  }

  // ---------- 状态输入 ----------

  /** 换一份表格数据。传 `null` 表示清空。 */
  setSheet(sheet: SheetHandle | null): void {
    this.sheet = sheet;
    // 行头标签:过滤后可视行是紧凑 0..V,但要显示原始行号
    this.rowLabelText =
      sheet && sheet.rowLabel
        ? (r) => String((sheet.rowLabel as (v: number) => number)(r) + 1)
        : (r) => String(r + 1);
    this.scroll = { x: 0, y: 0 };
    this.selection = sheet && sheet.rows > 0 ? { row: 0, col: 0 } : null;
    this.hover = null;
    this.colWidthOverrides = [];
    this.images = [];
    this.merges = [];
    this.charts = [];
    this.sparklines = [];
    this.windowCache = null;
    this.tile = null;
    this.sheetSetAt = this.now();
    this.stats.firstFrameMs = null;
    this.invalidateLayout();
  }

  /**
   * 数据行集合变化(如应用/清除过滤)后刷新:重读行数、清缓存重画,
   * 但**保留滚动与缩放**(比 setSheet 更轻,不重置视图)。
   */
  refreshRows(): void {
    this.selection = this.sheet && this.sheet.rows > 0 ? this.clampSelection() : null;
    this.windowCache = null;
    this.tile = null;
    this.invalidateLayout();
  }

  /** 把当前选区夹回有效范围(行数变化后用)。 */
  private clampSelection(): CellRef {
    const rows = this.sheet?.rows ?? 0;
    const cols = this.sheet?.cols ?? 0;
    const cur = this.selection ?? { row: 0, col: 0 };
    return {
      row: Math.min(Math.max(0, cur.row), Math.max(0, rows - 1)),
      col: Math.min(Math.max(0, cur.col), Math.max(0, cols - 1)),
    };
  }

  /** 设置冻结:顶部 `rows` 行、左侧 `cols` 列。传 0 取消对应方向的冻结。 */
  setFrozen(rows: number, cols: number): void {
    const r = Math.max(0, Math.floor(rows));
    const c = Math.max(0, Math.floor(cols));
    if (r === this.frozenRows && c === this.frozenCols) return;
    this.frozenRows = r;
    this.frozenCols = c;
    // 冻结改变了 body 画布用途(瓦片 ↔ 全量),重置滚动与瓦片,重算布局
    this.scroll = { x: 0, y: 0 };
    this.tile = null;
    this.windowCache = null;
    this.invalidateLayout();
  }

  /** 当前冻结设置。 */
  getFrozen(): { rows: number; cols: number } {
    return { rows: this.frozenRows, cols: this.frozenCols };
  }

  /**
   * 视口尺寸变化(CSS 像素)与设备像素比变化。
   *
   * 尺寸换算是**清晰度的关键**:后备像素必须等于「显示尺寸 × dpr」,
   * 否则浏览器会把整块位图重采样一遍,画面整体发虚。麻烦在于容器尺寸常是小数
   * (flex 布局、边框、滚动条),而浏览器缩放与 125% / 150% 显示缩放下 dpr 也是小数,
   * 两者相乘几乎必然不是整数 —— 比如 1151 CSS px × 1.25 = 1438.75。
   *
   * 所以不能让 CSS 自己决定显示尺寸再去凑后备像素,而要**先把后备像素向下取整到
   * 整设备像素,再由它反推显示尺寸**,两边同源就不会错位。
   */
  resize(cssWidth: number, cssHeight: number, dpr: number): void {
    const nextDpr = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
    const width = Math.max(0, Math.floor(cssWidth * nextDpr));
    const height = Math.max(0, Math.floor(cssHeight * nextDpr));
    if (width === this.viewport.width && height === this.viewport.height && nextDpr === this.dpr) {
      return;
    }
    this.dpr = nextDpr;
    this.viewport = { width, height };

    for (const name of ["headers", "overlay"] as const) {
      const layer = this.layers[name];
      if (!layer) continue;
      layer.canvas.width = width;
      layer.canvas.height = height;
      layer.canvas.style.width = `${width / nextDpr}px`;
      layer.canvas.style.height = `${height / nextDpr}px`;
    }
    // 视口变了,瓦片尺寸多半也要变
    this.tile = null;
    this.invalidateLayout();
  }

  /** 设置滚动位置(CSS 像素)。 */
  setScroll(cssX: number, cssY: number): void {
    this.applyScroll({ x: cssX * this.dpr, y: cssY * this.dpr });
  }

  /** 相对滚动(CSS 像素)。 */
  scrollBy(cssDx: number, cssDy: number): void {
    this.applyScroll({ x: this.scroll.x + cssDx * this.dpr, y: this.scroll.y + cssDy * this.dpr });
  }

  /** 当前滚动位置(CSS 像素)。 */
  getScroll(): Scroll {
    return { x: this.scroll.x / this.dpr, y: this.scroll.y / this.dpr };
  }

  /** 当前缩放倍率。 */
  getZoom(): number {
    return this.zoom;
  }

  /** 当前布局(设备像素)。交互层做命中判定时需要。 */
  getLayout(): GridLayout {
    this.ensureLayout();
    return this.layout;
  }

  /** 视口尺寸(设备像素)。 */
  getViewport(): Viewport {
    return this.viewport;
  }

  /** 设备像素比。 */
  getDpr(): number {
    return this.dpr;
  }

  /**
   * 设置缩放,并保持指针下的内容不动。
   *
   * @param zoom 新的缩放倍率
   * @param anchorCss 锚点(CSS 像素,相对画布左上角);不传则以视口中心为锚
   */
  setZoom(zoom: number, anchorCss?: { x: number; y: number }): void {
    this.ensureLayout();
    const previous = this.zoom;
    if (zoom === previous) return;

    const before = this.layout;
    this.zoom = zoom;
    this.layoutDirty = true;
    this.ensureLayout();
    const after = this.layout;

    // 锚点相对单元格区域左上角的偏移(设备像素)
    const anchorX = anchorCss
      ? anchorCss.x * this.dpr - before.headerWidth
      : bodySize(before, this.viewport).width / 2;
    const anchorY = anchorCss
      ? anchorCss.y * this.dpr - before.headerHeight
      : bodySize(before, this.viewport).height / 2;

    // 缩放前后布局的实际比例(取整后未必正好等于 zoom 之比,所以按总宽算)
    const ratioX = before.totalWidth > 0 ? after.totalWidth / before.totalWidth : 1;
    const ratioY = before.totalHeight > 0 ? after.totalHeight / before.totalHeight : 1;

    this.applyScroll({
      x: (this.scroll.x + anchorX) * ratioX - anchorX,
      y: (this.scroll.y + anchorY) * ratioY - anchorY,
    });
    this.tile = null;
    this.invalidateAll();
  }

  /**
   * 设置悬停单元格。
   *
   * **只脏 overlay** —— 这是分层最直接的收益:鼠标划过表格时,
   * 上千个单元格与表头一个都不用重画。
   */
  setHover(cell: CellRef | null): void {
    if (sameCell(this.hover, cell)) return;
    this.hover = cell;
    this.invalidate("overlay");
  }

  /** 设置选中单元格。选区会高亮所在行列表头,所以表头也要跟着更新。 */
  setSelection(cell: CellRef | null): void {
    if (sameCell(this.selection, cell)) return;
    this.selection = cell;
    this.invalidate("overlay");
    this.invalidate("headers");
  }

  /** 设置多格选区(已归一化,含首尾);传 `null` 或单格时退化为普通选区。 */
  setSelectionRange(
    range: { row0: number; row1: number; col0: number; col1: number } | null,
  ): void {
    this.selectionRange = range;
    this.invalidate("overlay");
  }

  /** 设置内嵌图片(xlsx);在覆盖层绘制,随滚动定位。 */
  setImages(images: OverlayImage[]): void {
    this.images = images;
    this.invalidate("overlay");
  }

  /** 设置合并单元格区域(xlsx);在覆盖层合并绘制。 */
  setMerges(merges: OverlayMerge[]): void {
    this.merges = merges;
    this.invalidate("overlay");
  }

  /** 设置内嵌图表(xlsx);在覆盖层绘制。 */
  setCharts(charts: OverlayChart[]): void {
    this.charts = charts;
    this.invalidate("overlay");
  }

  /** 设置单元格内迷你图(xlsx)。 */
  setSparklines(sparklines: OverlaySparkline[]): void {
    this.sparklines = sparklines;
    this.invalidate("overlay");
  }

  /** 某列当前宽度(CSS 像素)。 */
  getColumnWidthCss(col: number): number {
    this.ensureLayout();
    const { colOffsets } = this.layout;
    if (col < 0 || col + 1 >= colOffsets.length) return 0;
    return (colOffsets[col + 1] - colOffsets[col]) / this.dpr;
  }

  /**
   * 命中列头右边界的「调整手柄」:返回可拖拽调整宽度的列号,否则 `null`。
   *
   * 只在列头区(`y < headerHeight`)内、且指针距某列右边界 `tolCss` 像素内时命中。
   */
  columnResizeHitTest(cssX: number, cssY: number, tolCss = 4): number | null {
    this.ensureLayout();
    const layout = this.layout;
    const x = cssX * this.dpr;
    const y = cssY * this.dpr;
    if (y < 0 || y > layout.headerHeight) return null;
    if (x < layout.headerWidth) return null;
    const tol = tolCss * this.dpr;

    const { colOffsets, cols, frozenCols, frozenWidth } = layout;
    // 冻结列:边界固定不随滚动;其余列减去横向滚动
    for (let c = 0; c < cols; c += 1) {
      const isFrozen = c < frozenCols;
      const border = layout.headerWidth + colOffsets[c + 1] - (isFrozen ? 0 : this.scroll.x);
      // 非冻结列的边界不能落进冻结区里(被盖住)
      if (!isFrozen && colOffsets[c + 1] - this.scroll.x < frozenWidth) continue;
      if (Math.abs(x - border) <= tol) return c;
    }
    return null;
  }

  /** 手动设置某列宽度(CSS 像素);传 `undefined` 或 `<=0` 恢复自动宽度。 */
  setColumnWidth(col: number, cssWidth: number | undefined): void {
    if (col < 0) return;
    // 覆盖值存基准 px(未乘缩放),与自动宽度同空间;computeLayout 再乘 zoom*dpr
    const basePx = cssWidth != null && cssWidth > 0 ? cssWidth / this.zoom : 0;
    this.colWidthOverrides[col] = basePx;
    this.tile = null;
    this.windowCache = null;
    this.invalidateLayout();
  }

  /** 当前选中单元格。 */
  getSelection(): CellRef | null {
    return this.selection;
  }

  /** 让选中单元格滚动进视野。 */
  revealSelection(): void {
    if (!this.selection) return;
    this.ensureLayout();
    this.applyScroll(scrollIntoView(this.layout, this.viewport, this.scroll, this.selection));
  }

  /** 命中判定。入参为 CSS 像素(相对画布左上角)。 */
  hitTest(cssX: number, cssY: number): HitTarget {
    this.ensureLayout();
    return hitTest(this.layout, this.viewport, this.scroll, cssX * this.dpr, cssY * this.dpr);
  }

  /** 滚动条几何(CSS 像素),供交互层判断是否点在滑块上。 */
  getScrollbars() {
    this.ensureLayout();
    const geometry = scrollbarGeometry(this.layout, this.viewport, this.scroll);
    const toCss = (rect: Rect): Rect => ({
      x: rect.x / this.dpr,
      y: rect.y / this.dpr,
      width: rect.width / this.dpr,
      height: rect.height / this.dpr,
    });
    return {
      vertical: geometry.vertical
        ? { track: toCss(geometry.vertical.track), thumb: toCss(geometry.vertical.thumb) }
        : null,
      horizontal: geometry.horizontal
        ? { track: toCss(geometry.horizontal.track), thumb: toCss(geometry.horizontal.thumb) }
        : null,
    };
  }

  /**
   * 按滚动条滑块的位置设置滚动量(拖拽滚动条时用)。
   *
   * @param axis 轴
   * @param thumbOffsetCss 滑块顶端 / 左端相对轨道起点的偏移(CSS 像素)
   */
  setScrollFromThumb(axis: "x" | "y", thumbOffsetCss: number): void {
    this.ensureLayout();
    const value = scrollFromThumbOffset(
      this.layout,
      this.viewport,
      axis,
      thumbOffsetCss * this.dpr,
    );
    this.applyScroll(axis === "y" ? { x: this.scroll.x, y: value } : { x: value, y: this.scroll.y });
  }

  /** 当前统计数据。 */
  getStats(): RendererStats {
    return { ...this.stats, layerPaints: { ...this.stats.layerPaints } };
  }

  /** 释放资源。 */
  destroy(): void {
    this.destroyed = true;
    if (this.frameHandle !== null) {
      this.cancel(this.frameHandle);
      this.frameHandle = null;
    }
    for (const canvas of this.canvases) {
      canvas.width = 0;
      canvas.height = 0;
      canvas.remove?.();
    }
    this.canvases.length = 0;
    this.layers = {};
    this.bodyClip?.remove?.();
    this.bodyClip = null;
    this.disposeScratch();
    this.sheet = null;
    this.windowCache = null;
  }

  // ---------- 脏标记与调度 ----------

  /** 布局失效(数据 / 视口 / 缩放变化):三层都要重画。 */
  invalidateLayout(): void {
    this.layoutDirty = true;
    this.invalidateAll();
  }

  /** 三层全脏。 */
  invalidateAll(): void {
    this.tile = null;
    for (const name of LAYER_ORDER) this.dirty[name] = true;
    this.requestFrame();
  }

  /** 标记某一层需要重画。 */
  invalidate(layer: LayerName): void {
    this.dirty[layer] = true;
    this.requestFrame();
  }

  /**
   * 请求出帧。
   *
   * 同一帧内的多次调用只会排一次 —— 「单帧内的多次状态变更合并为一次重绘」
   * 就是靠这个 `frameHandle` 保证的。
   */
  requestFrame(): void {
    if (this.destroyed || this.frameHandle !== null) return;
    this.frameHandle = this.schedule(() => {
      this.frameHandle = null;
      this.renderFrame();
    });
  }

  // ---------- 管线实现 ----------

  /** 当前表格的每格样式查询(xlsx);无样式源时返回 undefined。 */
  private styleAt(): StyleAt | undefined {
    const sheet = this.sheet;
    if (!sheet || typeof sheet.cellStyle !== "function") return undefined;
    return (r, c) => sheet.cellStyle!(r, c);
  }

  private applyScroll(next: Scroll): void {
    this.ensureLayout();
    // 取整到设备像素:位图平移与 transform 都必须是整像素,否则会重采样发虚
    const clamped = clampScroll(this.layout, this.viewport, {
      x: Math.round(next.x),
      y: Math.round(next.y),
    });
    if (clamped.x === this.scroll.x && clamped.y === this.scroll.y) return;
    this.scroll = clamped;
    // 滚动会改变表头标签与选区/滚动条位置;body 层是否要重画由瓦片决定
    this.dirty.headers = true;
    this.dirty.overlay = true;
    this.requestFrame();
  }

  private ensureLayout(): void {
    if (!this.layoutDirty) return;
    this.layout = computeLayout({
      rows: this.sheet?.rows ?? 0,
      cols: this.sheet?.cols ?? 0,
      colWidthUnits: this.sheet?.colWidthUnits ?? [],
      // 内部按设备像素工作,所以把 dpr 并进缩放系数
      zoom: this.zoom * this.dpr,
      snap: true,
      frozenRows: this.frozenRows,
      frozenCols: this.frozenCols,
      colWidthOverrides: this.colWidthOverrides,
    });
    this.layoutDirty = false;
    this.scroll = clampScroll(this.layout, this.viewport, this.scroll);
  }

  /**
   * 保证窗口缓存覆盖住 `needed`;不够就按 `fetch` 取一次。
   *
   * 两个范围刻意分开:**判断是否命中**只看瓦片本身(`needed`),
   * **真去取**则取一个更大的范围(`fetch`)。这样瓦片重新锚定一次
   * 通常仍在缓存内,能走增量重绘;若把 `fetch` 当作命中条件,
   * 缓存就会永远差一点、每次重锚都要回 WASM 取数。
   */
  private ensureWindow(needed: VisibleRange, fetch: VisibleRange): void {
    if (!this.sheet) {
      this.windowCache = null;
      return;
    }
    const cached = this.windowCache?.range;
    if (
      cached &&
      cached.row0 <= needed.row0 &&
      cached.row1 >= needed.row1 &&
      cached.col0 <= needed.col0 &&
      cached.col1 >= needed.col1
    ) {
      return;
    }

    const data = this.sheet.window(fetch.row0, fetch.row1, fetch.col0, fetch.col1);
    this.windowCache = { range: fetch, cells: flattenWindow(data) };
    this.stats.windowFetches += 1;
    // 内容变了,瓦片里的旧像素不能再用
    this.dirty.body = true;
  }

  private renderFrame(): void {
    if (this.destroyed) return;
    const started = this.now();

    this.ensureLayout();
    const { viewport, layout } = this;
    if (viewport.width <= 0 || viewport.height <= 0) return;

    // 空表:画一句提示就结束,不必走完整管线
    if (!this.sheet || layout.rows === 0 || layout.cols === 0) {
      this.paintEmpty();
      this.recordFrame(started);
      return;
    }

    const body = bodySize(layout, viewport);
    this.positionBodyClip(layout, body);

    // ① 几何:表头要知道可见了哪些行列(单元格取数按瓦片来,见 updateBodyLayer)
    const range = computeVisibleRange(layout, viewport, this.scroll, 1);

    // ② 单元格层:能靠 GPU 平移就不画;冻结时走全量四象限重绘
    if (this.layout.frozenRows > 0 || this.layout.frozenCols > 0) {
      this.renderFrozenBody(body);
    } else {
      this.updateBodyLayer(body);
    }

    // ③ 表头与覆盖层:便宜,但也只在脏的时候画
    if (this.dirty.headers) {
      const layer = this.layers.headers;
      if (layer) {
        layer.ctx.setTransform(1, 0, 0, 1, 0, 0);
        layer.ctx.clearRect(0, 0, viewport.width, viewport.height);
        paintHeaders(layer.ctx, {
          layout,
          viewport,
          scroll: this.scroll,
          range,
          active: this.selection,
          fitter: this.fitter,
          rowLabelText: this.rowLabelText,
        });
        this.stats.layerPaints.headers += 1;
      }
      this.dirty.headers = false;
    }

    if (this.dirty.overlay) {
      const layer = this.layers.overlay;
      if (layer) {
        layer.ctx.setTransform(1, 0, 0, 1, 0, 0);
        layer.ctx.clearRect(0, 0, viewport.width, viewport.height);
        paintOverlay(layer.ctx, {
          layout,
          viewport,
          scroll: this.scroll,
          hover: this.hover,
          selection: this.selection,
          selectionRange: this.selectionRange,
          images: this.images,
          charts: this.charts,
          sparklines: this.sparklines,
          merges: this.merges,
          fontSize: this.layout.fontSize,
          fitter: this.fitter,
        });
        this.stats.layerPaints.overlay += 1;
      }
      this.dirty.overlay = false;
    }

    this.recordFrame(started);
  }

  /** 空表:清掉三层,在最上层写一句提示。 */
  private paintEmpty(): void {
    for (const name of LAYER_ORDER) {
      const layer = this.layers[name];
      if (!layer) continue;
      layer.ctx.setTransform(1, 0, 0, 1, 0, 0);
      layer.ctx.clearRect(0, 0, this.viewport.width, this.viewport.height);
    }
    const top = this.layers.overlay;
    if (top) {
      paintEmptyState(top.ctx, this.viewport, "没有可显示的数据", 13 * this.dpr);
      this.stats.layerPaints.overlay += 1;
    }
    for (const name of LAYER_ORDER) this.dirty[name] = false;
  }

  /** 把 body 的裁剪容器摆到单元格区域(表头右下方)。 */
  private positionBodyClip(layout: GridLayout, body: Viewport): void {
    const clip = this.bodyClip;
    if (!clip) return;
    clip.style.left = `${layout.headerWidth / this.dpr}px`;
    clip.style.top = `${layout.headerHeight / this.dpr}px`;
    clip.style.width = `${body.width / this.dpr}px`;
    clip.style.height = `${body.height / this.dpr}px`;
  }

  /**
   * 冻结模式的单元格层:body 画布铺满单元格区域,四象限全量重绘(见 `paintFrozenBody`)。
   *
   * 不走瓦片/GPU 平移 —— 冻结是低频审阅态,全量重绘足够流畅且无回归风险。
   * 从 sheet 按四个象限分别取窗口(都被视口夹小),`getCell` 按坐标选对应窗口。
   */
  private renderFrozenBody(body: Viewport): void {
    const layer = this.layers.body;
    if (!layer || !this.sheet) return;
    const layout = this.layout;

    // body 画布铺满单元格区域(丢弃瓦片语义)
    if (layer.canvas.width !== body.width || layer.canvas.height !== body.height) {
      layer.canvas.width = body.width;
      layer.canvas.height = body.height;
    }
    layer.canvas.style.width = `${body.width / this.dpr}px`;
    layer.canvas.style.height = `${body.height / this.dpr}px`;
    layer.canvas.style.transform = "translate3d(0,0,0)";
    this.tile = null;

    // 可见的滚动行列范围(在冻结带之后)
    const sRow0 = Math.min(
      layout.rows,
      Math.max(layout.frozenRows, rowAtOffset(layout, this.scroll.y + layout.frozenHeight)),
    );
    const sRow1 = Math.min(layout.rows, rowAtOffset(layout, this.scroll.y + body.height - 1) + 1);
    const sCol0 = Math.min(
      layout.cols,
      Math.max(layout.frozenCols, colAtOffset(layout, this.scroll.x + layout.frozenWidth)),
    );
    const sCol1 = Math.min(layout.cols, colAtOffset(layout, this.scroll.x + body.width - 1) + 1);

    // 四象限窗口(都被视口夹小),合成 getCell
    const fetch = (r0: number, r1: number, c0: number, c1: number): CellTextSource | null => {
      if (r1 <= r0 || c1 <= c0) return null;
      const data = this.sheet!.window(r0, r1, c0, c1);
      this.stats.windowFetches += 1;
      return { range: { row0: r0, row1: r1, col0: c0, col1: c1 }, cells: flattenWindow(data) };
    };
    const main = fetch(sRow0, sRow1, sCol0, sCol1);
    const top = fetch(0, layout.frozenRows, sCol0, sCol1);
    const left = fetch(sRow0, sRow1, 0, layout.frozenCols);
    const corner = fetch(0, layout.frozenRows, 0, layout.frozenCols);
    const sources = [main, top, left, corner].filter((s): s is CellTextSource => s !== null);
    const getCell = (row: number, col: number): string => {
      for (const s of sources) {
        if (row >= s.range.row0 && row < s.range.row1 && col >= s.range.col0 && col < s.range.col1) {
          return cellTextAt(s, row, col);
        }
      }
      return "";
    };

    paintFrozenBody(layer.ctx, {
      layout,
      body,
      scroll: this.scroll,
      getCell,
      fitter: this.fitter,
      visible: { row0: sRow0, row1: sRow1, col0: sCol0, col1: sCol1 },
    });
    this.stats.layerPaints.body += 1;
    this.stats.fullRepaints += 1;
    this.dirty.body = false;
  }

  /**
   * 更新单元格层 —— 管线里唯一真正昂贵的一步,因此能不画就不画。
   *
   * 三种情况:
   * 1. 瓦片还盖得住可见区域且内容没变 → **只改 transform,由 GPU 平移**;
   * 2. 滚出了瓦片余量 → 重新锚定,并用位图平移复用重叠部分,只补新露出的窄带;
   * 3. 数据/布局变了 → 整块重绘。
   */
  private updateBodyLayer(body: Viewport): void {
    const layer = this.layers.body;
    if (!layer) return;

    const content = { width: this.layout.totalWidth, height: this.layout.totalHeight };
    const size = tileSizeFor(body, content);

    // 瓦片尺寸变了(视口/缩放/数据规模变化)就必须重建画布
    if (tileSizeChanged(this.tile, size)) {
      layer.canvas.width = size.width;
      layer.canvas.height = size.height;
      layer.canvas.style.width = `${size.width / this.dpr}px`;
      layer.canvas.style.height = `${size.height / this.dpr}px`;
      this.tile = null;
      this.dirty.body = true;
    }

    const covered = this.tile !== null && tileCovers(this.tile, this.scroll, body, content);
    // 先定下这一帧要用哪块瓦片,再据此取数 —— 顺序反了就会漏取瓦片边缘的单元格
    const target = covered && this.tile ? this.tile : anchorTile(this.scroll, body, size, content);
    this.ensureWindow(this.rangeForTile(target, 0), this.rangeForTile(target, WINDOW_MARGIN));

    if (!this.dirty.body && covered) {
      // 情况 1:一个像素都不用画
      this.applyTileTransform();
      this.stats.gpuScrolls += 1;
      return;
    }
    if (!this.windowCache) return;

    const previous = this.tile;
    // 情况 2:内容没变、只是瓦片挪了位置 → 复用重叠区
    const plan =
      !this.dirty.body && previous
        ? planScrollBlit(
            { x: previous.originX, y: previous.originY },
            { x: target.originX, y: target.originY },
            size,
          )
        : null;

    if (plan && plan.exposed.length > 0) {
      this.blitTile(layer, size, plan, target);
      this.stats.incrementalRepaints += 1;
    } else if (plan) {
      // 位置没变也没脏,理论上不该走到这里,兜底不做事
      this.tile = target;
    } else {
      layer.ctx.setTransform(1, 0, 0, 1, 0, 0);
      layer.ctx.clearRect(0, 0, size.width, size.height);
      this.tile = target;
      paintBody(layer.ctx, {
        layout: this.layout,
        body: size,
        scroll: { x: target.originX, y: target.originY },
        source: this.windowCache,
        fitter: this.fitter,
        styleAt: this.styleAt(),
      });
      this.stats.fullRepaints += 1;
    }

    this.stats.layerPaints.body += 1;
    this.dirty.body = false;
    this.applyTileTransform();
  }

  /**
   * 瓦片需要哪些行列。
   *
   * @param margin 向外扩张的像素数。判断缓存是否命中时传 0(只要盖住瓦片就够),
   *   真去取数时传 [`WINDOW_MARGIN`] 留出冗余,好让下一次重锚还能命中。
   */
  private rangeForTile(tile: Tile, margin: number): VisibleRange {
    return rangeForContentRect(
      this.layout,
      { x: tile.originX, y: tile.originY, width: tile.width, height: tile.height },
      margin,
    );
  }

  /** 位图平移 + 补窄带。用离屏暂存层做乒乓,避免同一张画布自我覆盖。 */
  private blitTile(layer: Layer, size: Viewport, plan: BlitPlan, next: Tile): void {
    const scratch = this.ensureScratch(size);
    if (!scratch) {
      // 拿不到暂存层就退回整块重绘,功能不受影响
      layer.ctx.setTransform(1, 0, 0, 1, 0, 0);
      layer.ctx.clearRect(0, 0, size.width, size.height);
      this.tile = next;
      paintBody(layer.ctx, {
        layout: this.layout,
        body: size,
        scroll: { x: next.originX, y: next.originY },
        source: this.windowCache!,
        fitter: this.fitter,
        styleAt: this.styleAt(),
      });
      this.stats.fullRepaints += 1;
      return;
    }

    scratch.ctx.setTransform(1, 0, 0, 1, 0, 0);
    scratch.ctx.clearRect(0, 0, size.width, size.height);
    scratch.ctx.drawImage(layer.canvas, -plan.dx, -plan.dy);
    layer.ctx.setTransform(1, 0, 0, 1, 0, 0);
    layer.ctx.clearRect(0, 0, size.width, size.height);
    layer.ctx.drawImage(scratch.canvas, 0, 0);

    this.tile = next;
    for (const rect of plan.exposed) {
      paintBody(layer.ctx, {
        layout: this.layout,
        body: size,
        scroll: { x: next.originX, y: next.originY },
        source: this.windowCache!,
        fitter: this.fitter,
        dirty: rect,
        styleAt: this.styleAt(),
      });
    }
  }

  /** 把瓦片相对可见区域的偏移写进 CSS transform,交给合成器。 */
  private applyTileTransform(): void {
    const layer = this.layers.body;
    if (!layer || !this.tile) return;
    const { x, y } = tileTranslation(this.tile, this.scroll);
    if (this.appliedTranslation && this.appliedTranslation.x === x && this.appliedTranslation.y === y) {
      return;
    }
    this.appliedTranslation = { x, y };
    // translate3d 而非 translate:显式提示走 GPU 合成路径
    layer.canvas.style.transform = `translate3d(${x / this.dpr}px, ${y / this.dpr}px, 0)`;
  }

  private ensureScratch(size: Viewport): Layer | null {
    if (this.scratch && this.scratch.canvas.width === size.width && this.scratch.canvas.height === size.height) {
      return this.scratch;
    }
    this.disposeScratch();
    const canvas = this.createElement("canvas") as HTMLCanvasElement;
    canvas.width = size.width;
    canvas.height = size.height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    this.scratch = { canvas, ctx };
    return this.scratch;
  }

  private disposeScratch(): void {
    if (this.scratch) {
      this.scratch.canvas.width = 0;
      this.scratch.canvas.height = 0;
    }
    this.scratch = null;
  }

  private recordFrame(started: number): void {
    const elapsed = this.now() - started;
    this.stats.frames += 1;
    this.stats.lastFrameMs = elapsed;
    this.frameTimes.push(elapsed);
    if (this.frameTimes.length > FRAME_SAMPLES) this.frameTimes.shift();
    let total = 0;
    for (const time of this.frameTimes) total += time;
    this.stats.avgFrameMs = total / this.frameTimes.length;

    if (this.stats.firstFrameMs === null && this.sheetSetAt !== null && this.sheet) {
      this.stats.firstFrameMs = this.now() - this.sheetSetAt;
      this.tracer?.info("sheet.firstFrame", {
        ms: this.stats.firstFrameMs.toFixed(1),
        rows: this.sheet.rows,
        cols: this.sheet.cols,
        paintMs: elapsed.toFixed(1),
      });
    }
    this.onStats?.(this.getStats());
  }
}

function sameCell(a: CellRef | null, b: CellRef | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.row === b.row && a.col === b.col;
}
