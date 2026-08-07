import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Tracer } from "../shared/logger";
import type { SheetHandle } from "../shared/sheet";
import { GridRenderer, type RendererStats } from "./grid/renderer";
import { wheelToScrollDelta, wheelToZoomFactor } from "./grid/input";
import { cellAddress } from "./grid/labels";
import { MAX_ZOOM, MIN_ZOOM } from "./grid/theme";
import { rectContains } from "./grid/geometry";

/** 一行 / 一列的滚轮步进(deltaMode = LINE 时用)。 */
const LINE_HEIGHT = 24;
/** 按下后移动超过这个距离才算「拖拽」,否则算「点击」。 */
const DRAG_THRESHOLD = 3;

interface SheetCanvasProps {
  /** 表格数据。 */
  sheet: SheetHandle;
  /** 日志上下文。 */
  tracer: Tracer;
}

type DragMode =
  | { kind: "none" }
  | { kind: "pan"; lastX: number; lastY: number; moved: boolean; startX: number; startY: number }
  | { kind: "scrollbar"; axis: "x" | "y"; grabOffset: number };

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
  const [zoom, setZoom] = useState(1);
  const [stats, setStats] = useState<RendererStats | null>(null);
  const selectionRef = useRef(selection);
  selectionRef.current = selection;

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

  // 换数据:重置视图状态
  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    renderer.setSheet(sheet);
    setSelection({ row: 0, col: 0 });
    setZoom(renderer.getZoom());
    renderer.requestFrame();
  }, [sheet]);

  // 选区变化同步给渲染器
  useEffect(() => {
    rendererRef.current?.setSelection(sheet.rows > 0 ? selection : null);
  }, [selection, sheet]);

  const clampCell = useCallback(
    (row: number, col: number) => ({
      row: Math.min(Math.max(0, row), Math.max(0, sheet.rows - 1)),
      col: Math.min(Math.max(0, col), Math.max(0, sheet.cols - 1)),
    }),
    [sheet],
  );

  /** 移动选区并保证它可见。 */
  const moveSelection = useCallback(
    (deltaRow: number, deltaCol: number, absolute?: { row?: number; col?: number }) => {
      const renderer = rendererRef.current;
      const current = selectionRef.current;
      const next = clampCell(
        absolute?.row ?? current.row + deltaRow,
        absolute?.col ?? current.col + deltaCol,
      );
      setSelection(next);
      if (renderer) {
        renderer.setSelection(next);
        renderer.revealSelection();
      }
    },
    [clampCell],
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

    // 2) 否则进入「可能是拖拽平移,也可能只是点击选中」的状态
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

    // 空闲状态下更新 hover —— 只重画覆盖层,代价很低
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
      if (hit.kind === "cell") {
        moveSelection(0, 0, { row: hit.row, col: hit.col });
      } else if (hit.kind === "row-header") {
        moveSelection(0, 0, { row: hit.row, col: 0 });
      } else if (hit.kind === "column-header") {
        moveSelection(0, 0, { row: 0, col: hit.col });
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

    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        moveSelection(1, 0);
        break;
      case "ArrowUp":
        event.preventDefault();
        moveSelection(-1, 0);
        break;
      case "ArrowRight":
        event.preventDefault();
        moveSelection(0, 1);
        break;
      case "ArrowLeft":
        event.preventDefault();
        moveSelection(0, -1);
        break;
      case "PageDown":
        event.preventDefault();
        moveSelection(pageRows, 0);
        break;
      case "PageUp":
        event.preventDefault();
        moveSelection(-pageRows, 0);
        break;
      case "Home":
        event.preventDefault();
        moveSelection(0, 0, { col: 0 });
        break;
      case "End":
        event.preventDefault();
        moveSelection(0, 0, { col: sheet.cols - 1 });
        break;
      default:
        break;
    }
  };

  const address = cellAddress(selection.row, selection.col);
  const fps = stats && stats.avgFrameMs > 0 ? Math.min(60, Math.round(1000 / stats.avgFrameMs)) : null;

  return (
    <div className="sheet">
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
