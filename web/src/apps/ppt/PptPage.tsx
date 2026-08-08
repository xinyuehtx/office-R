import { useCallback, useEffect, useRef, useState } from "react";
import { FileUpload } from "../shared/FileUpload";
import { loadPptx } from "../../wasm";
import type { PptDocument } from "./model";
import { imageKey } from "./model";
import { drawSlide, fitScale } from "./slideRender";

type Status = "idle" | "loading" | "ready" | "error";

/**
 * 演示(PowerPoint)页面:上传 .pptx → WASM 解析成幻灯模型 → canvas 渲染。
 *
 * - 主视图把当前幻灯等比缩放铺进画布;左侧缩略图导航;
 * - **演示模式**:全屏、方向键/点击翻页、Esc 退出。
 * 形状(文本框/图片/自选图形)与对齐均按模型绘制。
 */
export function PptPage() {
  const [status, setStatus] = useState<Status>("idle");
  const [fileName, setFileName] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [current, setCurrent] = useState(0);
  const [slideCount, setSlideCount] = useState(0);
  const [presenting, setPresenting] = useState(false);

  const docRef = useRef<PptDocument | null>(null);
  const imagesRef = useRef<Map<string, HTMLImageElement>>(new Map());
  const mainCanvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number | null>(null);

  /** 绘制当前幻灯到主画布(适配 stage 尺寸)。 */
  const draw = useCallback(() => {
    rafRef.current = null;
    const canvas = mainCanvasRef.current;
    const stage = stageRef.current;
    const doc = docRef.current;
    if (!canvas || !stage || !doc) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const pres = doc.presentation;
    const slide = pres.slides[current];
    if (!slide) return;

    const dpr = globalThis.devicePixelRatio || 1;
    const cssW = stage.clientWidth;
    const cssH = stage.clientHeight;
    if (cssW <= 0 || cssH <= 0) return;
    canvas.width = Math.floor(cssW * dpr);
    canvas.height = Math.floor(cssH * dpr);
    canvas.style.width = `${cssW}px`;
    canvas.style.height = `${cssH}px`;

    const { scale, offsetX, offsetY } = fitScale(pres.width_px, pres.height_px, cssW, cssH);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);
    // 幻灯纸面
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(offsetX, offsetY, pres.width_px * scale, pres.height_px * scale);
    ctx.strokeStyle = "#d0d7de";
    ctx.strokeRect(offsetX + 0.5, offsetY + 0.5, pres.width_px * scale, pres.height_px * scale);

    // 该幻灯的图片映射(embed → 已解码 <img>)
    const slideImages = new Map<string, HTMLImageElement>();
    for (const shape of slide.shapes) {
      if (shape.image) {
        const img = imagesRef.current.get(imageKey(current, shape.image));
        if (img) slideImages.set(shape.image, img);
      }
    }

    ctx.save();
    ctx.translate(offsetX, offsetY);
    drawSlide(ctx, slide, scale, slideImages);
    ctx.restore();
  }, [current]);

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
        const doc = await loadPptx(bytes);
        docRef.current = doc;

        // 预加载图片(键含幻灯序号)
        for (const [key, url] of doc.images) {
          const img = new Image();
          img.onload = () => requestDraw();
          img.src = url;
          imagesRef.current.set(key, img);
        }

        setSlideCount(doc.presentation.slides.length);
        setCurrent(0);
        setStatus("ready");
        requestDraw();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      }
    },
    [requestDraw],
  );

  const go = useCallback(
    (delta: number) => {
      setCurrent((c) => Math.min(slideCount - 1, Math.max(0, c + delta)));
    },
    [slideCount],
  );

  // 当前页 / 尺寸 / 就绪 变化重绘(就绪后 canvas 才挂载,需再画一次)
  useEffect(() => {
    if (status === "ready") requestDraw();
  }, [current, presenting, status, slideCount, requestDraw]);

  useEffect(() => {
    if (status !== "ready") return;
    const stage = stageRef.current;
    if (!stage) return;
    requestDraw();
    const ro =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(() => requestDraw()) : null;
    ro?.observe(stage);
    return () => ro?.disconnect();
  }, [status, presenting, requestDraw]);

  // 演示模式:键盘翻页 / Esc 退出
  useEffect(() => {
    if (!presenting) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === " ") {
        e.preventDefault();
        go(1);
      } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
        e.preventDefault();
        go(-1);
      } else if (e.key === "Escape") {
        setPresenting(false);
      }
    };
    globalThis.addEventListener("keydown", onKey);
    return () => globalThis.removeEventListener("keydown", onKey);
  }, [presenting, go]);

  useEffect(
    () => () => {
      docRef.current?.dispose();
      if (rafRef.current !== null) globalThis.cancelAnimationFrame(rafRef.current);
    },
    [],
  );

  const busy = status === "loading";
  const curSlide = docRef.current?.presentation.slides[current];
  const hasAnim = curSlide?.has_animation ?? false;
  const hasTrans = curSlide?.has_transition ?? false;

  return (
    <section className="office-page" aria-label="演示 · PowerPoint">
      <header className="office-page__header">
        <h2>演示 · PowerPoint</h2>
        <p className="office-page__subtitle">
          上传 .pptx 文件,在 canvas 上渲染幻灯:文本框、图片、自选图形与对齐;
          形状旋转/翻转、图表/SmartArt 占位、动画/切换徽标。
          解析在 Rust/WASM 侧完成。支持缩略图导航与全屏演示模式。
        </p>
      </header>

      <div className="office-page__upload">
        <FileUpload accept=".pptx" onFile={openFile} label="上传 .pptx 文件" />
        {fileName && <span className="office-page__filename">{fileName}</span>}
        {status === "ready" && (
          <>
            <span className="office-page__muted" data-testid="ppt-stats">
              {slideCount} 张幻灯片
            </span>
            <button
              type="button"
              className="office-page__link"
              data-testid="ppt-present"
              onClick={() => setPresenting(true)}
            >
              ▶ 演示
            </button>
          </>
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
          <p className="office-page__empty">尚未选择文件。上传一个 .pptx 查看渲染效果。</p>
        </div>
      )}

      {status === "ready" && (
        <div
          className={presenting ? "ppt-layout ppt-layout--present" : "ppt-layout"}
          data-testid="ppt-layout"
        >
          {/* 缩略图导航(演示模式下隐藏) */}
          {!presenting && (
            <ol className="ppt-thumbs">
              {Array.from({ length: slideCount }, (_, i) => (
                <li key={i}>
                  <button
                    type="button"
                    className={i === current ? "ppt-thumb ppt-thumb--active" : "ppt-thumb"}
                    onClick={() => setCurrent(i)}
                    data-testid={`ppt-thumb-${i}`}
                  >
                    {i + 1}
                  </button>
                </li>
              ))}
            </ol>
          )}

          {/* 主视图(演示模式下铺满) */}
          <div className="ppt-main">
            <div
              className="ppt-stage"
              ref={stageRef}
              onClick={presenting ? () => go(1) : undefined}
            >
              <canvas ref={mainCanvasRef} data-testid="ppt-canvas" />
            </div>
            {presenting ? (
              <div className="ppt-present-hint">
                {current + 1} / {slideCount} · 方向键翻页,Esc 退出
                <button
                  type="button"
                  className="office-page__link"
                  data-testid="ppt-exit"
                  onClick={() => setPresenting(false)}
                >
                  退出演示
                </button>
              </div>
            ) : (
              <div className="ppt-nav">
                <button type="button" onClick={() => go(-1)} disabled={current === 0}>
                  ◀ 上一张
                </button>
                <span data-testid="ppt-page">
                  {current + 1} / {slideCount}
                </span>
                {hasTrans && (
                  <span className="ppt-badge" data-testid="ppt-badge-transition" title="含切换效果">
                    切换
                  </span>
                )}
                {hasAnim && (
                  <span className="ppt-badge" data-testid="ppt-badge-animation" title="含动画">
                    动画
                  </span>
                )}
                <button type="button" onClick={() => go(1)} disabled={current >= slideCount - 1}>
                  下一张 ▶
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
