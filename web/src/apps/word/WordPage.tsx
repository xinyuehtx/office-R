import { useCallback, useEffect, useRef, useState } from "react";
import { FileUpload } from "../shared/FileUpload";
import { sharedMeasurer } from "../shared/textMeasure";
import { loadDocx } from "../../wasm";
import type { WordDocument } from "./model";
import { layoutDoc, imageIdsIn, type Layout } from "./wordLayout";

type Status = "idle" | "loading" | "ready" | "error";

/** 页面宽度(文档纸张宽度,CSS 像素)。 */
const PAGE_WIDTH = 820;
/** 虚拟化时视口上下各多绘制的余量。 */
const OVERSCAN = 200;

/**
 * 文档(Word)页面:上传 .docx → WASM 解析成文档模型 → canvas 流式布局渲染。
 *
 * 渲染要点:
 * - 布局在 `wordLayout` 里算一次(依赖测量缓存),得到带绝对 y 的绘制项;
 * - **纵向虚拟化**:滚动时只绘制与视口相交的项,长文档也不卡;
 * - 图片用 object URL 异步解码为 `HTMLImageElement` 后重绘。
 */
export function WordPage() {
  const [status, setStatus] = useState<Status>("idle");
  const [fileName, setFileName] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<{ blocks: number; images: number; heightPx: number } | null>(
    null,
  );

  const scrollRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const docRef = useRef<WordDocument | null>(null);
  const layoutRef = useRef<Layout | null>(null);
  const imagesRef = useRef<Map<string, HTMLImageElement>>(new Map());
  const rafRef = useRef<number | null>(null);
  /** 内容总高度:撑起滚动条(spacer 高度);canvas 只画可见切片。 */
  const [contentHeight, setContentHeight] = useState(0);

  /** 画一帧:只绘制与视口相交的项。 */
  const draw = useCallback(() => {
    rafRef.current = null;
    const canvas = canvasRef.current;
    const scroller = scrollRef.current;
    const layout = layoutRef.current;
    if (!canvas || !scroller || !layout) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = globalThis.devicePixelRatio || 1;
    const cssW = PAGE_WIDTH;
    const cssH = scroller.clientHeight;
    if (canvas.width !== Math.floor(cssW * dpr) || canvas.height !== Math.floor(cssH * dpr)) {
      canvas.width = Math.floor(cssW * dpr);
      canvas.height = Math.floor(cssH * dpr);
      canvas.style.width = `${cssW}px`;
      canvas.style.height = `${cssH}px`;
    }
    const scrollY = scroller.scrollTop;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, cssW, cssH);

    const top = scrollY - OVERSCAN;
    const bottom = scrollY + cssH + OVERSCAN;
    ctx.textBaseline = "middle";
    ctx.textAlign = "left";

    for (const item of layout.items) {
      if (item.kind === "textline") {
        if (item.y + item.height < top || item.y - item.height > bottom) continue;
        const drawY = item.y - scrollY;
        for (const seg of item.segments) {
          ctx.font = seg.font;
          ctx.fillStyle = seg.color;
          ctx.fillText(seg.text, seg.x, drawY);
          if (seg.underline || seg.strike) {
            const w = ctx.measureText(seg.text).width;
            ctx.strokeStyle = seg.color;
            ctx.lineWidth = 1;
            ctx.beginPath();
            // 下划线在基线下方,删除线在中线
            const ly = seg.strike
              ? Math.round(drawY) + 0.5
              : Math.round(drawY + item.height / 3) + 0.5;
            ctx.moveTo(seg.x, ly);
            ctx.lineTo(seg.x + w, ly);
            ctx.stroke();
          }
        }
      } else if (item.kind === "rect") {
        if (item.y + item.height < top || item.y > bottom) continue;
        ctx.strokeStyle = "#d0d7de";
        ctx.lineWidth = 1;
        ctx.strokeRect(item.x + 0.5, item.y - scrollY + 0.5, item.width, item.height);
      } else {
        if (item.y + item.height < top || item.y > bottom) continue;
        const img = imagesRef.current.get(item.id);
        if (img && img.complete && img.naturalWidth > 0) {
          ctx.drawImage(img, item.x, item.y - scrollY, item.width, item.height);
        } else {
          ctx.strokeStyle = "#d0d7de";
          ctx.strokeRect(item.x + 0.5, item.y - scrollY + 0.5, item.width, item.height);
        }
      }
    }
  }, []);

  const requestDraw = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = globalThis.requestAnimationFrame(draw);
  }, [draw]);

  const openFile = useCallback(
    async (file: File) => {
      setStatus("loading");
      setFileName(file.name);
      setError(null);
      docRef.current?.dispose();
      docRef.current = null;
      imagesRef.current = new Map();

      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        const doc = await loadDocx(bytes);
        docRef.current = doc;

        for (const id of imageIdsIn(doc.model)) {
          const url = doc.images.get(id);
          if (!url) continue;
          const img = new Image();
          img.onload = () => requestDraw();
          img.src = url;
          imagesRef.current.set(id, img);
        }

        layoutRef.current = layoutDoc(doc.model, PAGE_WIDTH, sharedMeasurer);
        setContentHeight(layoutRef.current.height);
        setStats({
          blocks: doc.model.blocks.length,
          images: doc.images.size,
          heightPx: Math.round(layoutRef.current.height),
        });
        setStatus("ready");
        if (scrollRef.current) scrollRef.current.scrollTop = 0;
        requestDraw();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      }
    },
    [requestDraw],
  );

  useEffect(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    const onScroll = () => requestDraw();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    const ro =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(() => requestDraw()) : null;
    ro?.observe(scroller);
    return () => {
      scroller.removeEventListener("scroll", onScroll);
      ro?.disconnect();
    };
  }, [requestDraw]);

  useEffect(
    () => () => {
      docRef.current?.dispose();
      if (rafRef.current !== null) globalThis.cancelAnimationFrame(rafRef.current);
    },
    [],
  );

  const busy = status === "loading";

  return (
    <section className="office-page" aria-label="文档 · Word">
      <header className="office-page__header">
        <h2>文档 · Word</h2>
        <p className="office-page__subtitle">
          上传 .docx 文件,在 canvas 上按流式布局渲染:标题、正文、加粗/斜体/颜色、
          对齐、列表、图片、表格与图文混排;分栏、页眉页脚、修订(插入/删除)标记。
          解析在 Rust/WASM 侧完成,长文档纵向虚拟化。
        </p>
      </header>

      <div className="office-page__upload">
        <FileUpload accept=".docx" onFile={openFile} label="上传 .docx 文件" />
        {fileName && <span className="office-page__filename">{fileName}</span>}
        {stats && (
          <span className="office-page__muted" data-testid="word-stats">
            {stats.blocks} 个顶层块 · {stats.images} 张图片 · 高 {stats.heightPx}px
          </span>
        )}
      </div>

      {busy && <div className="office-page__result">正在解析…</div>}
      {status === "error" && (
        <div className="office-page__result">
          <p className="office-page__error">打开失败:{error}</p>
        </div>
      )}
      {status === "idle" && (
        <div className="office-page__result">
          <p className="office-page__empty">尚未选择文件。上传一个 .docx 查看渲染效果。</p>
        </div>
      )}

      <div
        className="word-viewport"
        ref={scrollRef}
        data-testid="word-viewport"
        style={{ display: status === "ready" ? "block" : "none" }}
      >
        {/* spacer 撑起滚动高度;canvas sticky 只画可见切片(纵向虚拟化) */}
        <div className="word-scroll" style={{ height: contentHeight }}>
          <canvas ref={canvasRef} className="word-canvas" data-testid="word-canvas" />
        </div>
      </div>
    </section>
  );
}
