import { useCallback, useEffect, useRef, useState } from "react";
import { FileUpload } from "@tengxiaohyx/office-shared";
import { loadPptx } from "@tengxiaohyx/office-ppt";
import type { PptDocument } from "@tengxiaohyx/office-ppt";
import { imageKey } from "@tengxiaohyx/office-ppt";
import { drawSlide, fitScale, applyTransition, TRANSITION_MS } from "./slideRender";

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
  /** 用户缩放倍率(在适配缩放之上叠加);演示模式恒为适配。 */
  const [zoom, setZoom] = useState(1);
  /** 演示模式的入场动画步号(0 = 仅显示无动画的形状)。 */
  const [step, setStep] = useState(0);
  const stepRef = useRef(0);
  stepRef.current = step;
  /** 进行中的切换效果:类型 + 起始时间戳。 */
  const transRef = useRef<{ kind: string | null; start: number } | null>(null);

  const docRef = useRef<PptDocument | null>(null);
  const imagesRef = useRef<Map<string, HTMLImageElement>>(new Map());
  const mainCanvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number | null>(null);
  /**
   * 打开请求的序号:只有最后一次 `openFile` 的结果能落地。
   *
   * 连续选两个文件时,先发的大文件可能后返回 —— 没有这道守卫就会用旧文档覆盖新文档,
   * 且被覆盖的那份永远调不到 `dispose()`,object URL 全部泄漏。
   */
  const openSeqRef = useRef(0);

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

    const fit = fitScale(pres.width_px, pres.height_px, cssW, cssH);
    // 演示模式恒用适配缩放;普通视图叠加用户缩放并居中(超出则对称裁切)
    const scale = presenting ? fit.scale : fit.scale * zoom;
    const offsetX = (cssW - pres.width_px * scale) / 2;
    const offsetY = (cssH - pres.height_px * scale) / 2;
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
    // 切换效果:演示模式换页时按进度做淡入/揭开/推入
    const trans = transRef.current;
    let animating = false;
    if (trans) {
      const t = (performance.now() - trans.start) / TRANSITION_MS;
      if (t >= 1) {
        transRef.current = null;
      } else {
        animating = true;
        applyTransition(ctx, trans.kind, t, pres.width_px * scale, pres.height_px * scale);
      }
    }
    // 入场动画:演示模式按当前步显示;普通视图显示全部
    drawSlide(
      ctx,
      slide,
      scale,
      slideImages,
      presenting ? stepRef.current : Number.POSITIVE_INFINITY,
    );
    ctx.restore();

    // 动画未结束 → 继续出帧
    if (animating) {
      rafRef.current = globalThis.requestAnimationFrame(draw);
    }
  }, [current, zoom, presenting]);

  const requestDraw = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = globalThis.requestAnimationFrame(draw);
  }, [draw]);

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
        const doc = await loadPptx(bytes);
        // 已被更晚的请求取代:立刻释放这份结果,不要覆盖当前文档
        if (seq !== openSeqRef.current) {
          doc.dispose();
          return;
        }
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
        setZoom(1);
        setStep(0);
        stepRef.current = 0;
        setStatus("ready");
        requestDraw();
      } catch (e) {
        if (seq !== openSeqRef.current) return;
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      }
    },
    [requestDraw],
  );

  /** 换页:演示模式下带切换效果,并把入场步复位。 */
  const go = useCallback(
    (delta: number) => {
      setCurrent((c) => {
        const next = Math.min(slideCount - 1, Math.max(0, c + delta));
        if (next !== c) {
          setStep(0);
          stepRef.current = 0;
          if (presenting) {
            const kind = docRef.current?.presentation.slides[next]?.transition ?? null;
            if (kind) transRef.current = { kind, start: performance.now() };
          }
        }
        return next;
      });
    },
    [slideCount, presenting],
  );

  /**
   * 演示模式的「下一步」:先播完本页入场动画(逐步显示),再翻到下一页。
   * 这与 PowerPoint 的点击行为一致。
   */
  const advance = useCallback(
    (delta: number) => {
      if (!presenting) {
        go(delta);
        return;
      }
      const slide = docRef.current?.presentation.slides[current];
      const total = slide?.build_steps ?? 0;
      if (delta > 0) {
        if (stepRef.current < total) {
          setStep((s) => s + 1);
          return;
        }
        go(1);
      } else {
        if (stepRef.current > 0) {
          setStep((s) => Math.max(0, s - 1));
          return;
        }
        go(-1);
      }
    },
    [presenting, current, go],
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

  // 演示模式:键盘翻页(先播入场动画,再翻页)/ Esc 退出
  useEffect(() => {
    if (!presenting) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === " ") {
        e.preventDefault();
        advance(1);
      } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
        e.preventDefault();
        advance(-1);
      } else if (e.key === "Escape") {
        setPresenting(false);
      }
    };
    globalThis.addEventListener("keydown", onKey);
    return () => globalThis.removeEventListener("keydown", onKey);
  }, [presenting, advance]);

  // 入场步变化:重绘
  useEffect(() => {
    if (status === "ready") requestDraw();
  }, [step, status, requestDraw]);

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
  /** 本页入场动画总步数(0 = 无分步)。 */
  const buildTotal = curSlide?.build_steps ?? 0;

  return (
    <section className="office-page" aria-label="演示 · PowerPoint">
      <header className="office-page__header">
        <h2>演示 · PowerPoint</h2>
        <p className="office-page__subtitle">
          上传 .pptx 文件,在 canvas 上渲染幻灯:文本框、图片、自选图形与对齐;
          形状旋转/翻转、渐变填充、内嵌表格与图表真实绘制、SmartArt 占位、动画/切换徽标。
          解析在 Rust/WASM 侧完成。支持缩放、缩略图导航与全屏演示模式
          ——演示时点击/方向键逐步播放入场动画,换页按切换效果淡入/揭开/推入。
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
              onClick={() => {
                setStep(0);
                stepRef.current = 0;
                const kind = docRef.current?.presentation.slides[current]?.transition ?? null;
                if (kind) transRef.current = { kind, start: performance.now() };
                setPresenting(true);
              }}
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
              onClick={presenting ? () => advance(1) : undefined}
            >
              <canvas ref={mainCanvasRef} data-testid="ppt-canvas" />
            </div>
            {presenting ? (
              <div className="ppt-present-hint">
                {current + 1} / {slideCount}
                {buildTotal > 0 && (
                  <span data-testid="ppt-build-step">
                    {" "}
                    · 动画 {Math.min(step, buildTotal)}/{buildTotal}
                  </span>
                )}{" "}
                · 方向键/点击播放,Esc 退出
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
                <span className="ppt-zoom" data-testid="ppt-zoom">
                  <button
                    type="button"
                    data-testid="ppt-zoom-out"
                    onClick={() => setZoom((z) => Math.max(0.25, +(z - 0.25).toFixed(2)))}
                    title="缩小"
                  >
                    −
                  </button>
                  <button
                    type="button"
                    onClick={() => setZoom(1)}
                    title="适配"
                    data-testid="ppt-zoom-reset"
                  >
                    {Math.round(zoom * 100)}%
                  </button>
                  <button
                    type="button"
                    data-testid="ppt-zoom-in"
                    onClick={() => setZoom((z) => Math.min(4, +(z + 0.25).toFixed(2)))}
                    title="放大"
                  >
                    +
                  </button>
                </span>
              </div>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
