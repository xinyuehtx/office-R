import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Tracer } from "../shared/logger";
import { flattenWindow, type FilterSpec, type SheetHandle } from "../shared/sheet";
import { GridRenderer, type RendererStats } from "./grid/renderer";
import { wheelToScrollDelta, wheelToZoomFactor } from "./grid/input";
import { cellAddress } from "./grid/labels";
import { MAX_ZOOM, MIN_ZOOM } from "./grid/theme";
import { rectContains } from "./grid/geometry";
import { FilterBar } from "./FilterBar";

/** 一行 / 一列的滚轮步进(deltaMode = LINE 时用)。 */
const LINE_HEIGHT = 24;
/** 按下后移动超过这个距离才算「拖拽」,否则算「点击」。 */
const DRAG_THRESHOLD = 3;
/** 顶部作为表头、过滤时始终保留的行数(CSV 常以首行为表头)。 */
const HEADER_ROWS = 1;

interface SheetCanvasProps {
  /** 表格数据。 */
  sheet: SheetHandle;
  /** 日志上下文。 */
  tracer: Tracer;
}

type DragMode =
  | { kind: "none" }
  | { kind: "pan"; lastX: number; lastY: number; moved: boolean; startX: number; startY: number }
  | { kind: "scrollbar"; axis: "x" | "y"; grabOffset: number }
  | { kind: "col-resize"; col: number; startX: number; startWidth: number };

/**
 * canvas 表格视图。
 *
 * 职责划分:
 * - **渲染器**建立并绘制三张堆叠的 canvas(单元格 / 表头 / 覆盖层),
 *   它们都是 `pointer-events: none`,纯粹是像素;
 * - **本组件**只提供一张透明的「交互层」`div`,承接指针与键盘事件、
 *   持有焦点与无障碍语义。
 *
 * 这样 React 不需要知道有几层画布,渲染器也不需要处理事件,两边互不干扰。
 */
export function SheetCanvas({ sheet, tracer }: SheetCanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<GridRenderer | null>(null);
  const dragRef = useRef<DragMode>({ kind: "none" });

  const [selection, setSelection] = useState({ row: 0, col: 0 });
  /** 选区锚点:范围选择从 anchor 拉到 selection(活动格)。 */
  const [anchor, setAnchor] = useState({ row: 0, col: 0 });
  const [zoom, setZoom] = useState(1);
  const [stats, setStats] = useState<RendererStats | null>(null);
  const [filters, setFilters] = useState<Map<number, FilterSpec>>(new Map());
  const [frozen, setFrozen] = useState({ rows: 0, cols: 0 });
  /** 当前排序:列 + 方向;null 表示未排序。 */
  const [sort, setSort] = useState<{ col: number; dir: "asc" | "desc" } | null>(null);
  /** 复制反馈(短暂显示「已复制 R×C」)。 */
  const [copied, setCopied] = useState<string | null>(null);
  /** 查找:是否打开、查询串、命中列表、当前命中下标。 */
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [findMatches, setFindMatches] = useState<{ row: number; col: number }[]>([]);
  const [findIdx, setFindIdx] = useState(0);
  const canFind = typeof sheet.find === "function";
  const selectionRef = useRef(selection);
  selectionRef.current = selection;
  const anchorRef = useRef(anchor);
  anchorRef.current = anchor;

  /** 归一化选区(含首尾)。 */
  const range = useMemo(
    () => ({
      row0: Math.min(anchor.row, selection.row),
      row1: Math.max(anchor.row, selection.row),
      col0: Math.min(anchor.col, selection.col),
      col1: Math.max(anchor.col, selection.col),
    }),
    [anchor, selection],
  );
  const isMultiSelection = range.row0 !== range.row1 || range.col0 !== range.col1;

  /** 该数据源是否支持过滤(WASM 句柄支持,测试替身可能不支持)。 */
  const canFilter = typeof sheet.filter === "function";
  const canSort = typeof sheet.sort === "function";

  /** 选中单元格的文本,用于状态栏与无障碍播报。 */
  const selectedText = useMemo(() => {
    if (sheet.rows === 0) return "";
    const window = sheet.window(selection.row, selection.row + 1, selection.col, selection.col + 1);
    return window.text;
  }, [sheet, selection]);

  /** 选中单元格的原始公式(若是公式格);单元格里显示的是计算值。 */
  const selectedFormula = useMemo(
    () => sheet.formula?.(selection.row, selection.col) ?? null,
    [sheet, selection],
  );

  // 建立渲染器,并把容器尺寸 / 设备像素比变化喂给它
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const renderer = new GridRenderer({
      container,
      tracer,
      onStats: setStats,
    });
    rendererRef.current = renderer;

    const sync = () => {
      const rect = container.getBoundingClientRect();
      renderer.resize(rect.width, rect.height, globalThis.devicePixelRatio || 1);
      renderer.requestFrame();
    };
    sync();

    // 容器驱动尺寸:布局怎么变,画布就怎么变,不需要页面手动通知
    const observer =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(sync) : null;
    observer?.observe(container);

    // 把窗口拖到不同 dpr 的显示器上时,也要重新按新的像素比出图
    const media =
      typeof matchMedia === "function"
        ? matchMedia(`(resolution: ${globalThis.devicePixelRatio || 1}dppx)`)
        : null;
    media?.addEventListener?.("change", sync);
    globalThis.addEventListener?.("resize", sync);

    return () => {
      observer?.disconnect();
      media?.removeEventListener?.("change", sync);
      globalThis.removeEventListener?.("resize", sync);
      renderer.destroy();
      rendererRef.current = null;
    };
  }, [tracer]);

  // 换数据:重置视图状态与过滤
  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    renderer.setSheet(sheet);
    setSelection({ row: 0, col: 0 });
    setAnchor({ row: 0, col: 0 });
    setFilters(new Map());
    setSort(null);
    setFindOpen(false);
    setFindQuery("");
    setFindMatches([]);
    renderer.setSelectionRange(null);
    setZoom(renderer.getZoom());
    renderer.requestFrame();

    // 内嵌图片(xlsx):解码位图,按 dpr 预算 ext,推给渲染器覆盖层
    const imgs = sheet.images ?? [];
    if (imgs.length > 0) {
      const dpr = renderer.getDpr();
      const overlay = imgs.map((im) => {
        const image = new Image();
        image.onload = () => rendererRef.current?.requestFrame();
        image.src = im.url;
        return {
          fromRow: im.fromRow,
          fromCol: im.fromCol,
          toRow: im.toRow,
          toCol: im.toCol,
          extWDev: im.extW !== undefined ? im.extW * dpr : undefined,
          extHDev: im.extH !== undefined ? im.extH * dpr : undefined,
          image,
        };
      });
      renderer.setImages(overlay);
    } else {
      renderer.setImages([]);
    }

    // 内嵌图表(xlsx):直接透传给覆盖层
    renderer.setCharts(sheet.charts ?? []);
    // 单元格内迷你图(xlsx)
    renderer.setSparklines(sheet.sparklines ?? []);

    // xlsx 列宽覆盖(冻结由下方专门的 effect 处理)
    for (const [col, px] of sheet.colWidthsPx ?? []) {
      renderer.setColumnWidth(col, px);
    }

    // 合并单元格(xlsx):取左上角文本,推给覆盖层跨区绘制
    const mg = sheet.merges ?? [];
    if (mg.length > 0) {
      renderer.setMerges(
        mg.map(([r0, c0, r1, c1]) => {
          const win = sheet.window(r0, r0 + 1, c0, c0 + 1);
          return { row0: r0, col0: c0, row1: r1, col1: c1, text: win.text };
        }),
      );
    } else {
      renderer.setMerges([]);
    }
  }, [sheet]);

  // 冻结变化:推给渲染器(重置滚动、切换到四象限绘制)
  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    renderer.setFrozen(frozen.rows, frozen.cols);
    setSelection((prev) => ({ ...prev }));
    renderer.requestFrame();
  }, [frozen]);

  // 换数据时冻结:xlsx 有冻结窗格则应用,否则清零
  useEffect(() => {
    const f = sheet.freeze;
    setFrozen(f && (f[0] > 0 || f[1] > 0) ? { rows: f[0], cols: f[1] } : { rows: 0, cols: 0 });
  }, [sheet]);

  // 过滤变化:在 WASM 侧重算命中行,再让渲染器按新行集刷新(保留滚动/缩放)
  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer || !canFilter) return;
    const specs = [...filters.values()];
    if (specs.length === 0) sheet.clearFilter?.();
    else sheet.filter?.(specs, HEADER_ROWS);
    renderer.refreshRows();
    setSelection((prev) => ({ row: 0, col: Math.min(prev.col, Math.max(0, sheet.cols - 1)) }));
    renderer.requestFrame();
  }, [filters, sheet, canFilter]);

  // 选区变化同步给渲染器
  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    renderer.setSelection(sheet.rows > 0 ? selection : null);
    renderer.setSelectionRange(sheet.rows > 0 ? range : null);
  }, [selection, range, sheet]);

  const clampCell = useCallback(
    (row: number, col: number) => ({
      row: Math.min(Math.max(0, row), Math.max(0, sheet.rows - 1)),
      col: Math.min(Math.max(0, col), Math.max(0, sheet.cols - 1)),
    }),
    [sheet],
  );

  /** 移动 / 扩展选区并保证活动格可见。`extend` 为真时保持锚点(范围选择)。 */
  const moveSelection = useCallback(
    (
      deltaRow: number,
      deltaCol: number,
      absolute?: { row?: number; col?: number },
      extend?: boolean,
    ) => {
      const renderer = rendererRef.current;
      const current = selectionRef.current;
      const next = clampCell(
        absolute?.row ?? current.row + deltaRow,
        absolute?.col ?? current.col + deltaCol,
      );
      setSelection(next);
      const nextAnchor = extend ? anchorRef.current : next;
      if (!extend) setAnchor(next);
      if (renderer) {
        renderer.setSelection(next);
        renderer.setSelectionRange({
          row0: Math.min(nextAnchor.row, next.row),
          row1: Math.max(nextAnchor.row, next.row),
          col0: Math.min(nextAnchor.col, next.col),
          col1: Math.max(nextAnchor.col, next.col),
        });
        renderer.revealSelection();
      }
    },
    [clampCell],
  );

  /** 复制当前选区到剪贴板(TSV,行以换行、列以制表符分隔)。 */
  const copySelection = useCallback(async () => {
    const r = {
      row0: Math.min(anchorRef.current.row, selectionRef.current.row),
      row1: Math.max(anchorRef.current.row, selectionRef.current.row),
      col0: Math.min(anchorRef.current.col, selectionRef.current.col),
      col1: Math.max(anchorRef.current.col, selectionRef.current.col),
    };
    if (sheet.rows === 0) return;
    const win = sheet.window(r.row0, r.row1 + 1, r.col0, r.col1 + 1);
    const cells = flattenWindow(win);
    const lines: string[] = [];
    for (let i = 0; i < win.rows; i += 1) {
      lines.push(cells.slice(i * win.cols, (i + 1) * win.cols).join("\t"));
    }
    const tsv = lines.join("\n");
    try {
      await navigator.clipboard?.writeText?.(tsv);
      setCopied(`已复制 ${win.rows}×${win.cols}`);
      tracer.info("sheet.copy", { rows: win.rows, cols: win.cols });
    } catch {
      // 剪贴板不可用(权限/环境)时静默:选区仍在,用户可重试
      setCopied("复制失败");
    }
    globalThis.setTimeout?.(() => setCopied(null), 1500);
  }, [sheet, tracer]);

  /** 对某列按方向排序(`"none"` 取消),并把选区带回数据首行。 */
  const sortColumn = useCallback(
    (col: number, dir: "asc" | "desc" | "none") => {
      if (!canSort) return;
      const renderer = rendererRef.current;
      sheet.sort?.(col, dir, HEADER_ROWS);
      setSort(dir === "none" ? null : { col, dir });
      renderer?.refreshRows();
      const r = Math.min(HEADER_ROWS, Math.max(0, sheet.rows - 1));
      setSelection({ row: r, col });
      setAnchor({ row: r, col });
      renderer?.requestFrame();
    },
    [canSort, sheet],
  );

  /** 跳到第 i 个命中:选中该格并滚入视野。 */
  const gotoMatch = useCallback(
    (matches: { row: number; col: number }[], i: number) => {
      if (matches.length === 0) return;
      const idx = ((i % matches.length) + matches.length) % matches.length;
      setFindIdx(idx);
      const m = matches[idx];
      const renderer = rendererRef.current;
      setSelection(m);
      setAnchor(m);
      renderer?.setSelection(m);
      renderer?.setSelectionRange(m ? { row0: m.row, row1: m.row, col0: m.col, col1: m.col } : null);
      renderer?.revealSelection();
    },
    [],
  );

  /** 运行查找并跳到第一个命中。 */
  const runFind = useCallback(
    (query: string) => {
      setFindQuery(query);
      if (!canFind || query === "") {
        setFindMatches([]);
        setFindIdx(0);
        return;
      }
      const matches = sheet.find?.(query, false, false, 5000) ?? [];
      setFindMatches(matches);
      gotoMatch(matches, 0);
    },
    [canFind, sheet, gotoMatch],
  );

  const applyZoom = useCallback((next: number, anchor?: { x: number; y: number }) => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    const clamped = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, next));
    if (clamped === renderer.getZoom()) return;
    renderer.setZoom(clamped, anchor);
    setZoom(clamped);
  }, []);

  // 滚轮:滚动 + Ctrl/⌘ 缩放。必须用非被动监听器才能 preventDefault,
  // 否则浏览器会把 ⌘+滚轮 当成整页缩放。
  useEffect(() => {
    const surface = surfaceRef.current;
    if (!surface) return;

    const onWheel = (event: WheelEvent) => {
      const renderer = rendererRef.current;
      if (!renderer) return;
      event.preventDefault();

      const context = { lineHeight: LINE_HEIGHT, pageHeight: surface.clientHeight };

      if (event.ctrlKey || event.metaKey) {
        const rect = surface.getBoundingClientRect();
        applyZoom(renderer.getZoom() * wheelToZoomFactor(event, context), {
          x: event.clientX - rect.left,
          y: event.clientY - rect.top,
        });
        return;
      }

      // Shift + 滚轮横滚、触控板双向滚动都在这个纯函数里处理
      const { dx, dy } = wheelToScrollDelta(event, context);
      renderer.scrollBy(dx, dy);
    };

    surface.addEventListener("wheel", onWheel, { passive: false });
    return () => surface.removeEventListener("wheel", onWheel);
  }, [applyZoom]);

  const pointerPosition = (event: React.PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    event.currentTarget.focus();
    const { x, y } = pointerPosition(event);

    // 1) 先看是不是按在自绘滚动条的滑块上
    const bars = renderer.getScrollbars();
    for (const [axis, bar] of [
      ["y", bars.vertical],
      ["x", bars.horizontal],
    ] as const) {
      if (bar && rectContains(bar.thumb, x, y)) {
        const grabOffset = axis === "y" ? y - bar.thumb.y : x - bar.thumb.x;
        dragRef.current = { kind: "scrollbar", axis, grabOffset };
        event.currentTarget.setPointerCapture(event.pointerId);
        return;
      }
    }

    // 2) 列头右边界:进入列宽拖拽
    const resizeCol = renderer.columnResizeHitTest(x, y);
    if (resizeCol !== null) {
      dragRef.current = {
        kind: "col-resize",
        col: resizeCol,
        startX: x,
        startWidth: renderer.getColumnWidthCss(resizeCol),
      };
      event.currentTarget.style.cursor = "col-resize";
      event.currentTarget.setPointerCapture(event.pointerId);
      return;
    }

    // 3) 否则进入「可能是拖拽平移,也可能只是点击选中」的状态
    dragRef.current = { kind: "pan", lastX: x, lastY: y, startX: x, startY: y, moved: false };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    const { x, y } = pointerPosition(event);
    const drag = dragRef.current;

    if (drag.kind === "scrollbar") {
      const bars = renderer.getScrollbars();
      const bar = drag.axis === "y" ? bars.vertical : bars.horizontal;
      if (bar) {
        const trackStart = drag.axis === "y" ? bar.track.y : bar.track.x;
        const offset = (drag.axis === "y" ? y : x) - trackStart - drag.grabOffset;
        renderer.setScrollFromThumb(drag.axis, offset);
      }
      return;
    }

    if (drag.kind === "col-resize") {
      const next = Math.max(24, drag.startWidth + (x - drag.startX));
      renderer.setColumnWidth(drag.col, next);
      return;
    }

    if (drag.kind === "pan") {
      const dx = x - drag.lastX;
      const dy = y - drag.lastY;
      const movedFar =
        Math.abs(x - drag.startX) > DRAG_THRESHOLD || Math.abs(y - drag.startY) > DRAG_THRESHOLD;
      if (movedFar) {
        // 拖拽方向与内容移动方向相反,像抓着纸往一边推
        renderer.scrollBy(-dx, -dy);
        event.currentTarget.style.cursor = "grabbing";
        dragRef.current = { ...drag, lastX: x, lastY: y, moved: true };
      }
      return;
    }

    // 空闲状态下更新 hover —— 只重画覆盖层,代价很低。
    // 悬停到列头右边界时给出 col-resize 光标提示。
    const overBorder = renderer.columnResizeHitTest(x, y) !== null;
    event.currentTarget.style.cursor = overBorder ? "col-resize" : "";
    const hit = renderer.hitTest(x, y);
    renderer.setHover(hit.kind === "cell" ? { row: hit.row, col: hit.col } : null);
  };

  const onPointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    const renderer = rendererRef.current;
    const drag = dragRef.current;
    dragRef.current = { kind: "none" };
    event.currentTarget.style.cursor = "";
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    if (!renderer) return;

    // 没怎么动 = 用户想选中这个单元格,而不是拖动视图
    if (drag.kind === "pan" && !drag.moved) {
      const { x, y } = pointerPosition(event);
      const hit = renderer.hitTest(x, y);
      const extend = event.shiftKey;
      if (hit.kind === "cell") {
        moveSelection(0, 0, { row: hit.row, col: hit.col }, extend);
      } else if (hit.kind === "row-header") {
        moveSelection(0, 0, { row: hit.row, col: 0 }, extend);
      } else if (hit.kind === "column-header") {
        moveSelection(0, 0, { row: 0, col: hit.col }, extend);
      }
    }
  };

  const onPointerLeave = () => {
    rendererRef.current?.setHover(null);
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    const layout = renderer.getLayout();
    const pageRows = Math.max(
      1,
      Math.floor((renderer.getViewport().height - layout.headerHeight) / layout.rowHeight),
    );
    const zoomStep = 0.1;

    if (event.ctrlKey || event.metaKey) {
      switch (event.key) {
        case "c":
        case "C":
          event.preventDefault();
          void copySelection();
          return;
        case "f":
        case "F":
          event.preventDefault();
          setFindOpen(true);
          return;
        case "=":
        case "+":
          event.preventDefault();
          applyZoom(renderer.getZoom() + zoomStep);
          return;
        case "-":
          event.preventDefault();
          applyZoom(renderer.getZoom() - zoomStep);
          return;
        case "0":
          event.preventDefault();
          applyZoom(1);
          return;
        case "Home":
          event.preventDefault();
          moveSelection(0, 0, { row: 0, col: 0 });
          return;
        case "End":
          event.preventDefault();
          moveSelection(0, 0, { row: sheet.rows - 1, col: sheet.cols - 1 });
          return;
        default:
          return;
      }
    }

    // Shift + 方向键:扩展选区(保持锚点)
    const extend = event.shiftKey;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        moveSelection(1, 0, undefined, extend);
        break;
      case "ArrowUp":
        event.preventDefault();
        moveSelection(-1, 0, undefined, extend);
        break;
      case "ArrowRight":
        event.preventDefault();
        moveSelection(0, 1, undefined, extend);
        break;
      case "ArrowLeft":
        event.preventDefault();
        moveSelection(0, -1, undefined, extend);
        break;
      case "PageDown":
        event.preventDefault();
        moveSelection(pageRows, 0, undefined, extend);
        break;
      case "PageUp":
        event.preventDefault();
        moveSelection(-pageRows, 0, undefined, extend);
        break;
      case "Home":
        event.preventDefault();
        moveSelection(0, 0, { col: 0 }, extend);
        break;
      case "End":
        event.preventDefault();
        moveSelection(0, 0, { col: sheet.cols - 1 }, extend);
        break;
      default:
        break;
    }
  };

  const setColumnFilter = useCallback((col: number, spec: FilterSpec | null) => {
    setFilters((prev) => {
      const next = new Map(prev);
      if (spec) next.set(col, spec);
      else next.delete(col);
      return next;
    });
  }, []);

  const clearAllFilters = useCallback(() => setFilters(new Map()), []);

  const address = cellAddress(selection.row, selection.col);
  const fps = stats && stats.avgFrameMs > 0 ? Math.min(60, Math.round(1000 / stats.avgFrameMs)) : null;

  return (
    <div className="sheet">
      {canFind && findOpen && (
        <div className="sheet__find-bar" data-testid="find-bar">
          <input
            type="text"
            className="sheet__find-input"
            data-testid="find-input"
            placeholder="查找…"
            autoFocus
            value={findQuery}
            onChange={(e) => runFind(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                gotoMatch(findMatches, findIdx + (e.shiftKey ? -1 : 1));
              } else if (e.key === "Escape") {
                e.preventDefault();
                setFindOpen(false);
              }
            }}
          />
          <span className="sheet__find-count" data-testid="find-count">
            {findMatches.length > 0 ? `${findIdx + 1}/${findMatches.length}` : "无匹配"}
          </span>
          <button
            type="button"
            data-testid="find-prev"
            disabled={findMatches.length === 0}
            onClick={() => gotoMatch(findMatches, findIdx - 1)}
            title="上一个"
          >
            ↑
          </button>
          <button
            type="button"
            data-testid="find-next"
            disabled={findMatches.length === 0}
            onClick={() => gotoMatch(findMatches, findIdx + 1)}
            title="下一个"
          >
            ↓
          </button>
          <button
            type="button"
            className="sheet__filter-link"
            data-testid="find-close"
            onClick={() => setFindOpen(false)}
          >
            关闭
          </button>
        </div>
      )}
      {canFilter && (
        <FilterBar
          sheet={sheet}
          activeCol={selection.col}
          filters={filters}
          headerRows={HEADER_ROWS}
          onApply={setColumnFilter}
          onClearAll={clearAllFilters}
        />
      )}

      <div className="sheet__freeze-bar" data-testid="freeze-bar">
        <span className="sheet__freeze-title">冻结</span>
        <button type="button" onClick={() => setFrozen({ rows: 1, cols: frozen.cols })}>
          冻结首行
        </button>
        <button type="button" onClick={() => setFrozen({ rows: frozen.rows, cols: 1 })}>
          冻结首列
        </button>
        <button
          type="button"
          data-testid="freeze-to-selection"
          onClick={() => setFrozen({ rows: selection.row, cols: selection.col })}
          title="冻结选中单元格左上方的所有行列"
        >
          冻结到选区
        </button>
        {(frozen.rows > 0 || frozen.cols > 0) && (
          <>
            <span className="sheet__freeze-state" data-testid="freeze-state">
              已冻结 {frozen.rows} 行 / {frozen.cols} 列
            </span>
            <button
              type="button"
              className="sheet__filter-link"
              data-testid="freeze-clear"
              onClick={() => setFrozen({ rows: 0, cols: 0 })}
            >
              取消冻结
            </button>
          </>
        )}
      </div>

      {/* 排序 + 复制工具条(作用于当前列 / 当前选区) */}
      <div className="sheet__tools-bar" data-testid="tools-bar">
        {canSort && (
          <>
            <span className="sheet__freeze-title">
              排序 {cellAddress(0, selection.col).replace(/\d+/, "")} 列
            </span>
            <button
              type="button"
              data-testid="sort-asc"
              className={sort?.col === selection.col && sort.dir === "asc" ? "is-active" : ""}
              onClick={() => sortColumn(selection.col, "asc")}
              title="升序"
            >
              ↑ 升序
            </button>
            <button
              type="button"
              data-testid="sort-desc"
              className={sort?.col === selection.col && sort.dir === "desc" ? "is-active" : ""}
              onClick={() => sortColumn(selection.col, "desc")}
              title="降序"
            >
              ↓ 降序
            </button>
            {sort && (
              <button
                type="button"
                className="sheet__filter-link"
                data-testid="sort-clear"
                onClick={() => sortColumn(0, "none")}
              >
                取消排序
              </button>
            )}
          </>
        )}
        <span className="sheet__tools-spacer" />
        <button type="button" data-testid="copy-selection" onClick={() => void copySelection()}>
          复制选区
        </button>
        {isMultiSelection && (
          <span className="sheet__muted" data-testid="selection-size">
            {range.row1 - range.row0 + 1}×{range.col1 - range.col0 + 1}
          </span>
        )}
        {copied && (
          <span className="sheet__copied" data-testid="copied-toast">
            {copied}
          </span>
        )}
      </div>

      {/* 公式栏:左侧显示当前地址,右侧显示原始公式(公式格)或计算值 */}
      <div className="sheet__formula-bar" data-testid="formula-bar">
        <span className="sheet__formula-address">{address}</span>
        <span className="sheet__formula-input" title={selectedFormula ?? selectedText}>
          {selectedFormula ? (
            <>
              <span className="sheet__formula-badge" aria-label="公式">
                ƒ
              </span>
              {selectedFormula}
            </>
          ) : (
            selectedText || <span className="sheet__formula-empty">(空)</span>
          )}
        </span>
      </div>

      {/* 三张堆叠画布由渲染器插入这里;它们只负责像素,不接收事件 */}
      <div className="sheet__viewport" ref={containerRef}>
        <div
          ref={surfaceRef}
          className="sheet__surface"
          tabIndex={0}
          role="grid"
          aria-label={`表格,共 ${sheet.rows} 行 ${sheet.cols} 列。用方向键移动,Ctrl 加滚轮缩放。`}
          aria-rowcount={sheet.rows}
          aria-colcount={sheet.cols}
          data-testid="sheet-canvas"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerLeave={onPointerLeave}
          onKeyDown={onKeyDown}
        />
      </div>

      {/* canvas 里的内容读屏软件读不到,这里补一个只对辅助技术可见的播报区 */}
      <div className="sheet__live" role="status" aria-live="polite">
        {`${address}:${selectedText || "空"}`}
      </div>

      <div className="sheet__status" data-testid="sheet-status">
        <span className="sheet__status-cell">
          <strong>{address}</strong>
          <span className="sheet__status-value" title={selectedText}>
            {selectedText || "(空)"}
          </span>
        </span>
        <span className="sheet__status-spacer" />
        <span>
          {sheet.rows.toLocaleString()} 行 × {sheet.cols.toLocaleString()} 列
        </span>
        <span>缩放 {Math.round(zoom * 100)}%</span>
        {fps !== null && (
          <span title={`平均每帧 ${stats?.avgFrameMs.toFixed(2)} ms`}>{fps} FPS</span>
        )}
      </div>
    </div>
  );
}
