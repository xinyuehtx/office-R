import { useCallback, useEffect, useRef, useState } from "react";
import { FileUpload } from "../shared/FileUpload";
import { sharedMeasurer } from "../shared/textMeasure";
import { loadDocx } from "../../wasm";
import type { WordDocument } from "./model";
import {
  layoutDoc,
  imageIdsIn,
  findLineMatches,
  type Layout,
  type WordMatch,
} from "./wordLayout";

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
  /**
   * 打开请求的序号:只有最后一次 `openFile` 的结果能落地。
   *
   * 连续选两个文件时,先发的大文件可能后返回 —— 没有这道守卫就会用旧文档覆盖新文档,
   * 且被覆盖的那份永远调不到 `dispose()`,object URL 全部泄漏。
   */
  const openSeqRef = useRef(0);
  /** 内容总高度:撑起滚动条(spacer 高度);canvas 只画可见切片。 */
  const [contentHeight, setContentHeight] = useState(0);
  /** 用户缩放倍率(50%–200%)。 */
  const [zoom, setZoom] = useState(1);
  const zoomRef = useRef(1);
  zoomRef.current = zoom;
  /** 全文查找:是否打开、查询、命中列表、当前命中下标。 */
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [findMatches, setFindMatches] = useState<WordMatch[]>([]);
  const [findIdx, setFindIdx] = useState(0);
  /** 当前命中的行高亮(布局坐标顶端 y 与高度);null 不画。 */
  const activeMatchRef = useRef<WordMatch | null>(null);

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
    const zoom = zoomRef.current;

    // 缩放:内容在布局坐标里,乘以 zoom 后再绘制;滚动量换算回布局空间
    ctx.setTransform(dpr * zoom, 0, 0, dpr * zoom, 0, 0);
    ctx.clearRect(0, 0, cssW / zoom, cssH / zoom);
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, cssW / zoom, cssH / zoom);

    // 布局空间里的滚动量与可见窗口
    const scrollYL = scrollY / zoom;
    const top = scrollYL - OVERSCAN;
    const bottom = scrollYL + cssH / zoom + OVERSCAN;
    ctx.textBaseline = "middle";
    ctx.textAlign = "left";

    // 当前查找命中:行级黄色高亮(在文字之下)
    const match = activeMatchRef.current;
    if (match) {
      ctx.fillStyle = "#fff3a3";
      ctx.fillRect(0, match.y - scrollYL, cssW / zoom, match.height);
    }

    for (const item of layout.items) {
      if (item.kind === "textline") {
        if (item.y + item.height < top || item.y - item.height > bottom) continue;
        const drawY = item.y - scrollYL;
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
        ctx.strokeRect(item.x + 0.5, item.y - scrollYL + 0.5, item.width, item.height);
      } else {
        if (item.y + item.height < top || item.y > bottom) continue;
        const img = imagesRef.current.get(item.id);
        if (img && img.complete && img.naturalWidth > 0) {
          ctx.drawImage(img, item.x, item.y - scrollYL, item.width, item.height);
        } else {
          ctx.strokeStyle = "#d0d7de";
          ctx.strokeRect(item.x + 0.5, item.y - scrollYL + 0.5, item.width, item.height);
        }
      }
    }
  }, []);

  const requestDraw = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = globalThis.requestAnimationFrame(draw);
  }, [draw]);

  /** 滚动到第 i 个命中并高亮该行。 */
  const gotoMatch = useCallback(
    (matches: WordMatch[], i: number) => {
      if (matches.length === 0) {
        activeMatchRef.current = null;
        requestDraw();
        return;
      }
      const idx = ((i % matches.length) + matches.length) % matches.length;
      setFindIdx(idx);
      const m = matches[idx];
      activeMatchRef.current = m;
      const scroller = scrollRef.current;
      if (scroller) {
        // 命中行居中显示(布局坐标 × zoom → 像素)
        const target = m.y * zoomRef.current - scroller.clientHeight / 2 + m.height * zoomRef.current;
        scroller.scrollTop = Math.max(0, target);
      }
      requestDraw();
    },
    [requestDraw],
  );

  /** 运行查找并跳到首个命中。 */
  const runFind = useCallback(
    (query: string) => {
      setFindQuery(query);
      const layout = layoutRef.current;
      const matches = layout ? findLineMatches(layout, query) : [];
      setFindMatches(matches);
      gotoMatch(matches, 0);
    },
    [gotoMatch],
  );

  const openFile = useCallback(
    async (file: File) => {
      const seq = openSeqRef.current + 1;
      openSeqRef.current = seq;
      setStatus("loading");
      setFileName(file.name);
      setError(null);
      docRef.current?.dispose();
      docRef.current = null;
      imagesRef.current = new Map();

      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        const doc = await loadDocx(bytes);
        // 已被更晚的请求取代:立刻释放这份结果,不要覆盖当前文档
        if (seq !== openSeqRef.current) {
          doc.dispose();
          return;
        }
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
        setZoom(1);
        setFindOpen(false);
        setFindQuery("");
        setFindMatches([]);
        activeMatchRef.current = null;
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
        if (seq !== openSeqRef.current) return;
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      }
    },
    [requestDraw],
  );

  // 缩放变化:spacer 高度按 zoom 缩放,重绘
  useEffect(() => {
    if (layoutRef.current) setContentHeight(layoutRef.current.height * zoom);
    requestDraw();
  }, [zoom, requestDraw]);

  // Ctrl/⌘+F 打开查找(就绪时)
  useEffect(() => {
    if (status !== "ready") return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        setFindOpen(true);
      }
    };
    globalThis.addEventListener("keydown", onKey);
    return () => globalThis.removeEventListener("keydown", onKey);
  }, [status]);

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
          对齐、列表、图片、表格与图文混排;分栏、页眉页脚、修订、超链接、脚注、批注、
          缩进/段距/行距。支持缩放与全文查找(Ctrl⌘F)。解析在 Rust/WASM 侧完成,长文档纵向虚拟化。
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
        {status === "ready" && (
          <span className="ppt-zoom" data-testid="word-zoom">
            <button
              type="button"
              data-testid="word-zoom-out"
              onClick={() => setZoom((z) => Math.max(0.5, +(z - 0.25).toFixed(2)))}
              title="缩小"
            >
              −
            </button>
            <button type="button" data-testid="word-zoom-reset" onClick={() => setZoom(1)} title="100%">
              {Math.round(zoom * 100)}%
            </button>
            <button
              type="button"
              data-testid="word-zoom-in"
              onClick={() => setZoom((z) => Math.min(2, +(z + 0.25).toFixed(2)))}
              title="放大"
            >
              +
            </button>
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

      {status === "ready" && findOpen && (
        <div className="sheet__find-bar" data-testid="word-find-bar">
          <input
            type="text"
            className="sheet__find-input"
            data-testid="word-find-input"
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
                activeMatchRef.current = null;
                requestDraw();
              }
            }}
          />
          <span className="sheet__find-count" data-testid="word-find-count">
            {findMatches.length > 0 ? `${findIdx + 1}/${findMatches.length}` : "无匹配"}
          </span>
          <button
            type="button"
            data-testid="word-find-prev"
            disabled={findMatches.length === 0}
            onClick={() => gotoMatch(findMatches, findIdx - 1)}
          >
            ↑
          </button>
          <button
            type="button"
            data-testid="word-find-next"
            disabled={findMatches.length === 0}
            onClick={() => gotoMatch(findMatches, findIdx + 1)}
          >
            ↓
          </button>
          <button
            type="button"
            className="office-page__link"
            data-testid="word-find-close"
            onClick={() => {
              setFindOpen(false);
              activeMatchRef.current = null;
              requestDraw();
            }}
          >
            关闭
          </button>
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
