import { describe, it, expect, beforeEach } from "vitest";
import { GridRenderer, planScrollBlit } from "./renderer";
import { cellTextAt, TextFitter, fontString } from "./layers";
import { TILE_MARGIN } from "./tile";
import { createRecordingContext, createStubLayers } from "../testing/canvasStub";
import { createFixtureSheet, makeGrid } from "../testing/sheetFixture";

describe("planScrollBlit", () => {
  const area = { width: 400, height: 300 };

  it("没有位移时不需要重绘任何区域", () => {
    expect(planScrollBlit({ x: 10, y: 10 }, { x: 10, y: 10 }, area)).toEqual({
      dx: 0,
      dy: 0,
      exposed: [],
    });
  });

  it("向下位移时只暴露底部窄带", () => {
    const plan = planScrollBlit({ x: 0, y: 0 }, { x: 0, y: 30 }, area);
    expect(plan).not.toBeNull();
    expect(plan!.dy).toBe(30);
    expect(plan!.exposed).toEqual([{ x: 0, y: 270, width: 400, height: 30 }]);
  });

  it("向上位移时只暴露顶部窄带", () => {
    const plan = planScrollBlit({ x: 0, y: 100 }, { x: 0, y: 60 }, area);
    expect(plan!.exposed).toEqual([{ x: 0, y: 0, width: 400, height: 40 }]);
  });

  it("向右位移时只暴露右侧窄带", () => {
    const plan = planScrollBlit({ x: 0, y: 0 }, { x: 25, y: 0 }, area);
    expect(plan!.exposed).toEqual([{ x: 375, y: 0, width: 25, height: 300 }]);
  });

  it("斜向位移时暴露两条窄带", () => {
    const plan = planScrollBlit({ x: 0, y: 0 }, { x: 10, y: 20 }, area);
    expect(plan!.exposed).toHaveLength(2);
  });

  it("位移超过整块时放弃平移(返回 null 让调用方整块重绘)", () => {
    expect(planScrollBlit({ x: 0, y: 0 }, { x: 0, y: 300 }, area)).toBeNull();
    expect(planScrollBlit({ x: 0, y: 0 }, { x: 400, y: 0 }, area)).toBeNull();
    expect(planScrollBlit({ x: 0, y: 0 }, { x: 0, y: 5000 }, area)).toBeNull();
  });

  it("暴露区域的面积远小于整块(这就是增量的收益)", () => {
    const plan = planScrollBlit({ x: 0, y: 0 }, { x: 0, y: 24 }, area)!;
    const exposedArea = plan.exposed.reduce((sum, r) => sum + r.width * r.height, 0);
    expect(exposedArea).toBeLessThan(area.width * area.height * 0.1);
  });
});

describe("cellTextAt", () => {
  const source = {
    range: { row0: 10, row1: 13, col0: 2, col1: 4 },
    cells: ["a", "b", "c", "d", "e", "f"],
  };

  it("按行优先取到正确的单元格", () => {
    expect(cellTextAt(source, 10, 2)).toBe("a");
    expect(cellTextAt(source, 10, 3)).toBe("b");
    expect(cellTextAt(source, 12, 3)).toBe("f");
  });

  it("范围外返回空串而不是越界", () => {
    expect(cellTextAt(source, 9, 2)).toBe("");
    expect(cellTextAt(source, 13, 2)).toBe("");
    expect(cellTextAt(source, 10, 1)).toBe("");
    expect(cellTextAt(source, 10, 4)).toBe("");
  });
});

describe("TextFitter", () => {
  let fitter: TextFitter;
  const ctx = createRecordingContext(10); // 每个字符宽 10

  beforeEach(() => {
    fitter = new TextFitter();
    fitter.setFont(fontString(13));
  });

  it("放得下时原样返回", () => {
    expect(fitter.fit(ctx, "abc", 200, 13)).toBe("abc");
  });

  it("放不下时截断并加省略号", () => {
    expect(fitter.fit(ctx, "abcdefgh", 45, 13)).toBe("abc…");
  });

  it("宽度极小时返回空串而不是溢出", () => {
    expect(fitter.fit(ctx, "abcdefgh", 5, 13)).toBe("");
  });

  it("空文本与非法宽度都安全", () => {
    expect(fitter.fit(ctx, "", 100, 13)).toBe("");
    expect(fitter.fit(ctx, "abc", 0, 13)).toBe("");
    expect(fitter.fit(ctx, "abc", -10, 13)).toBe("");
  });

  it("相同入参走缓存,不重复测量", () => {
    fitter.fit(ctx, "abcdefgh", 45, 13);
    const sizeAfterFirst = fitter.size;
    fitter.fit(ctx, "abcdefgh", 45, 13);
    expect(fitter.size).toBe(sizeAfterFirst);
  });

  it("换字体会清空缓存(测量结果全变了)", () => {
    fitter.fit(ctx, "abcdefgh", 45, 13);
    expect(fitter.size).toBeGreaterThan(0);
    fitter.setFont(fontString(26));
    expect(fitter.size).toBe(0);
  });
});

/** 手动触发的帧调度器:测试里出帧的时机必须可控。 */
function manualFrame() {
  const holder: { callback: (() => void) | null } = { callback: null };
  return {
    schedule: (callback: () => void) => {
      holder.callback = callback;
      return 1;
    },
    cancel: () => {
      holder.callback = null;
    },
    flush: () => {
      const callback = holder.callback;
      holder.callback = null;
      callback?.();
    },
    pending: () => holder.callback !== null,
  };
}

/** 组装一个可手动出帧的渲染器。 */
function setupRenderer(rows = 200, cols = 10, cssWidth = 800, cssHeight = 400, dpr = 2) {
  const layers = createStubLayers();
  const frame = manualFrame();
  let clock = 0;

  const renderer = new GridRenderer({
    container: layers.container,
    createElement: layers.createElement,
    schedule: frame.schedule,
    cancel: frame.cancel,
    now: () => {
      clock += 1;
      return clock;
    },
  });

  renderer.resize(cssWidth, cssHeight, dpr);
  const sheet = createFixtureSheet(makeGrid(rows, cols));
  renderer.setSheet(sheet);

  return { renderer, layers, flush: frame.flush, sheet, hasPending: frame.pending };
}

describe("GridRenderer 图层结构", () => {
  it("建出三张堆叠画布,各自带 data-layer 标记", () => {
    const { layers } = setupRenderer();
    for (const name of ["body", "headers", "overlay"] as const) {
      expect(layers.canvas(name), `缺少 ${name} 层`).toBeDefined();
    }
  });

  it("画布都不接收指针事件;只有会平移的单元格层提升为合成层", () => {
    const { layers } = setupRenderer();
    for (const name of ["body", "headers", "overlay"] as const) {
      const canvas = layers.canvas(name)!;
      expect(canvas.style.pointerEvents).toBe("none");
      expect(canvas.style.position).toBe("absolute");
    }
    // will-change 会让元素常驻显存,只该给真正要 transform 的那一层
    expect(layers.canvas("body")!.style.willChange).toBe("transform");
    expect(layers.canvas("headers")!.style.willChange).toBeUndefined();
    expect(layers.canvas("overlay")!.style.willChange).toBeUndefined();
  });

  it("单元格文本画在 body 层,表头画在 headers 层,互不混淆", () => {
    const { layers, flush } = setupRenderer();
    flush();

    expect(layers.layer("body")!.texts()).toContain("r0c0");
    const headerTexts = layers.layer("headers")!.texts();
    expect(headerTexts).toContain("A");
    expect(headerTexts).toContain("1");
    // 表头层不该出现单元格内容,反之亦然
    expect(headerTexts).not.toContain("r0c0");
    expect(layers.layer("body")!.texts()).not.toContain("A");
  });

  it("每层的 save/restore 都成对,状态不会泄漏", () => {
    const { layers, flush } = setupRenderer();
    flush();
    for (const name of ["body", "headers", "overlay"] as const) {
      expect(layers.layer(name)!.saveDepth(), `${name} 层 save/restore 不配对`).toBe(0);
    }
  });

  it("destroy 会移除图层并停止出帧", () => {
    const { renderer, flush, hasPending } = setupRenderer();
    flush();
    const frames = renderer.getStats().frames;

    renderer.destroy();
    renderer.setScroll(0, 100);
    expect(hasPending()).toBe(false);
    flush();
    expect(renderer.getStats().frames).toBe(frames);
  });
});

describe("GridRenderer 分层重绘", () => {
  it("首帧三层都画一次", () => {
    const { renderer, flush } = setupRenderer();
    flush();

    const stats = renderer.getStats();
    expect(stats.frames).toBe(1);
    expect(stats.layerPaints).toEqual({ body: 1, headers: 1, overlay: 1 });
    expect(stats.firstFrameMs).not.toBeNull();
  });

  it("只改 hover 时**只**重画 overlay —— 分层最直接的收益", () => {
    const { renderer, flush } = setupRenderer();
    flush();
    const before = renderer.getStats();

    renderer.setHover({ row: 5, col: 3 });
    flush();
    const after = renderer.getStats();

    expect(after.layerPaints.overlay).toBe(before.layerPaints.overlay + 1);
    expect(after.layerPaints.body).toBe(before.layerPaints.body);
    expect(after.layerPaints.headers).toBe(before.layerPaints.headers);
  });

  it("连续移动鼠标 20 次也不会碰到单元格层", () => {
    const { renderer, flush } = setupRenderer();
    flush();
    const bodyBefore = renderer.getStats().layerPaints.body;

    for (let i = 0; i < 20; i += 1) {
      renderer.setHover({ row: i, col: i % 5 });
      flush();
    }

    expect(renderer.getStats().layerPaints.body).toBe(bodyBefore);
  });

  it("改选区时重画 overlay 与 headers(表头要高亮所在行列),但不动单元格层", () => {
    const { renderer, flush } = setupRenderer();
    flush();
    const before = renderer.getStats();

    renderer.setSelection({ row: 4, col: 2 });
    flush();
    const after = renderer.getStats();

    expect(after.layerPaints.overlay).toBe(before.layerPaints.overlay + 1);
    expect(after.layerPaints.headers).toBe(before.layerPaints.headers + 1);
    expect(after.layerPaints.body).toBe(before.layerPaints.body);
  });

  it("单帧内的多次状态变更只出一帧", () => {
    const { renderer, flush } = setupRenderer();
    flush();

    renderer.setScroll(0, 40);
    renderer.setHover({ row: 3, col: 2 });
    renderer.setSelection({ row: 4, col: 1 });
    flush();

    expect(renderer.getStats().frames).toBe(2);
  });
});

describe("GridRenderer 瓦片与 GPU 平移", () => {
  it("瓦片比可见区域大一圈,并用 transform 定位", () => {
    // 行列都远超视口,两个方向才都会留余量
    const { layers, flush } = setupRenderer(5000, 60);
    flush();

    const body = layers.canvas("body")!;
    const headers = layers.canvas("headers")!;
    expect(body.width).toBeGreaterThan(headers.width);
    expect(body.height).toBeGreaterThan(headers.height);
    expect(body.style.transform).toContain("translate3d");
  });

  it("小幅滚动**一个像素都不画**,只改 transform(纯 GPU 平移)", () => {
    const { renderer, flush } = setupRenderer(5000, 30);
    flush();
    const before = renderer.getStats();

    renderer.scrollBy(0, 24);
    flush();
    const after = renderer.getStats();

    expect(after.gpuScrolls).toBe(before.gpuScrolls + 1);
    expect(after.layerPaints.body).toBe(before.layerPaints.body);
  });

  it("在余量内连续滚动,单元格层始终不重绘", () => {
    const { renderer, layers, flush } = setupRenderer(5000, 30);
    flush();
    const bodyBefore = renderer.getStats().layerPaints.body;
    const drawnBefore = layers.layer("body")!.calls.length;

    // 累计滚动量控制在一圈余量以内
    const step = 8;
    const steps = Math.floor(TILE_MARGIN / step) - 2;
    for (let i = 0; i < steps; i += 1) {
      renderer.scrollBy(0, step / renderer.getDpr());
      flush();
    }

    const stats = renderer.getStats();
    expect(stats.layerPaints.body).toBe(bodyBefore);
    expect(layers.layer("body")!.calls.length).toBe(drawnBefore);
    expect(stats.gpuScrolls).toBeGreaterThanOrEqual(steps);
  });

  it("滚出余量后重新锚定瓦片,并靠位图平移复用重叠部分", () => {
    const { renderer, layers, flush } = setupRenderer(5000, 30);
    flush();
    const bodyBefore = renderer.getStats().layerPaints.body;

    // 瓦片在首帧居中锚定,可用余量约 2×TILE_MARGIN;滚过它才会重锚
    renderer.scrollBy(0, (TILE_MARGIN * 2 + 100) / renderer.getDpr());
    flush();

    const stats = renderer.getStats();
    expect(stats.layerPaints.body).toBe(bodyBefore + 1);
    expect(stats.incrementalRepaints).toBe(1);
    // 重叠部分靠 drawImage 搬过去,而不是全部重画
    expect(layers.layer("body")!.countOf("drawImage")).toBeGreaterThan(0);
  });

  it("跳转过远时退回整块重绘", () => {
    const { renderer, flush } = setupRenderer(5000, 30);
    flush();
    const fullBefore = renderer.getStats().fullRepaints;

    renderer.setScroll(0, 50_000);
    flush();

    expect(renderer.getStats().fullRepaints).toBe(fullBefore + 1);
  });

  it("内容比可见区域小时不留余量,避免白占内存", () => {
    const { layers, flush } = setupRenderer(3, 2);
    flush();
    const body = layers.canvas("body")!;
    const headers = layers.canvas("headers")!;
    expect(body.width).toBeLessThan(headers.width);
  });
});

describe("GridRenderer 视口与像素", () => {
  it("HiDPI:表头层后备像素按 dpr 放大,显示尺寸保持不变", () => {
    const { layers } = setupRenderer(200, 10, 800, 400, 2);
    const headers = layers.canvas("headers")!;
    expect(headers.width).toBe(1600);
    expect(headers.height).toBe(800);
    expect(headers.style.width).toBe("800px");
    expect(headers.style.height).toBe("400px");
  });

  // 后备像素与显示尺寸必须同源换算,否则浏览器会重采样整块位图 → 画面发虚。
  // 容器尺寸常是小数(flex/边框/滚动条),dpr 在浏览器缩放与 125%/150% 显示缩放下
  // 也是小数,两者相乘几乎必然不是整数(1151 × 1.25 = 1438.75)。
  it.each([
    { css: [800, 400], dpr: 1 },
    { css: [800, 400], dpr: 2 },
    { css: [1151, 384], dpr: 1.25 },
    { css: [1151, 384], dpr: 1.5 },
    { css: [1389.5, 631.25], dpr: 1 },
    { css: [1389.5, 631.25], dpr: 2 },
    { css: [1150.4, 383.6], dpr: 1.75 },
    { css: [1000, 500], dpr: 2.2 },
  ])("后备像素与显示尺寸精确对齐(css=$css dpr=$dpr)", ({ css, dpr }) => {
    const { layers, flush } = setupRenderer(500, 20, css[0], css[1], dpr);
    flush();

    for (const name of ["headers", "overlay", "body"] as const) {
      const canvas = layers.canvas(name)!;
      const styleW = parseFloat(canvas.style.width);
      const styleH = parseFloat(canvas.style.height);
      // 关键不变式:显示尺寸 × dpr 正好是后备像素 → 缩放比为 1 → 不重采样
      expect(styleW * dpr, `${name} 宽度未对齐`).toBeCloseTo(canvas.width, 9);
      expect(styleH * dpr, `${name} 高度未对齐`).toBeCloseTo(canvas.height, 9);
      expect(Number.isInteger(canvas.width)).toBe(true);
      expect(Number.isInteger(canvas.height)).toBe(true);
    }

    // 视口层(表头/覆盖)不得超过容器,否则会撑出滚动条
    const headers = layers.canvas("headers")!;
    expect(parseFloat(headers.style.width)).toBeLessThanOrEqual(css[0]);
    expect(parseFloat(headers.style.height)).toBeLessThanOrEqual(css[1]);
  });

  it("视口尺寸变化会重建瓦片并整块重绘,不留残影", () => {
    const { renderer, flush } = setupRenderer();
    flush();
    const fullBefore = renderer.getStats().fullRepaints;

    renderer.resize(1000, 600, 2);
    flush();

    expect(renderer.getStats().fullRepaints).toBe(fullBefore + 1);
  });

  it("滚动被夹在合法范围内", () => {
    const { renderer, flush } = setupRenderer(20, 3);
    flush();

    renderer.setScroll(-100, -100);
    expect(renderer.getScroll()).toEqual({ x: 0, y: 0 });

    renderer.setScroll(1e9, 1e9);
    flush();
    const scroll = renderer.getScroll();
    expect(scroll.x).toBeGreaterThanOrEqual(0);
    expect(scroll.y).toBeGreaterThanOrEqual(0);
  });
});

describe("GridRenderer 数据与缩放", () => {
  it("只绘制可见区域:窗口取数远小于总行数", () => {
    const { renderer, layers, flush } = setupRenderer(100_000, 20);
    flush();

    const drawn = layers.layer("body")!.texts().length;
    expect(drawn).toBeGreaterThan(0);
    expect(drawn).toBeLessThan(3_000);
    expect(renderer.getStats().windowFetches).toBe(1);
  });

  it("连续小幅滚动不会每次都回 WASM 取数(overscan 缓存生效)", () => {
    const { renderer, flush } = setupRenderer(100_000, 20);
    flush();
    const fetchesAfterFirst = renderer.getStats().windowFetches;

    renderer.scrollBy(0, 4);
    flush();

    expect(renderer.getStats().windowFetches).toBe(fetchesAfterFirst);
  });

  it("缩放以指针为锚点:锚点下的内容坐标保持不变", () => {
    const { renderer, flush } = setupRenderer(1000, 30);
    flush();
    renderer.setScroll(200, 200);
    flush();

    const layoutBefore = renderer.getLayout();
    const scrollBefore = renderer.getScroll();
    const anchor = { x: 400, y: 200 };
    const anchorDeviceX = anchor.x * renderer.getDpr() - layoutBefore.headerWidth;
    const ratioBefore =
      (scrollBefore.x * renderer.getDpr() + anchorDeviceX) / layoutBefore.totalWidth;

    renderer.setZoom(2, anchor);
    flush();

    const layoutAfter = renderer.getLayout();
    const scrollAfter = renderer.getScroll();
    const ratioAfter =
      (scrollAfter.x * renderer.getDpr() + anchorDeviceX) / layoutAfter.totalWidth;

    expect(ratioAfter).toBeCloseTo(ratioBefore, 2);
    expect(renderer.getZoom()).toBe(2);
  });

  it("缩放后命中判定依然指向同一个单元格", () => {
    const { renderer, flush } = setupRenderer(1000, 30);
    flush();

    const layout = renderer.getLayout();
    const dpr = renderer.getDpr();
    const cssX = (layout.headerWidth + layout.colOffsets[1] + 5) / dpr;
    const cssY = (layout.headerHeight + layout.rowHeight * 2 + 5) / dpr;
    expect(renderer.hitTest(cssX, cssY)).toEqual({ kind: "cell", row: 2, col: 1 });

    renderer.setZoom(1.5);
    renderer.setScroll(0, 0);
    flush();
    const zoomed = renderer.getLayout();
    const zoomedX = (zoomed.headerWidth + zoomed.colOffsets[1] + 5) / dpr;
    const zoomedY = (zoomed.headerHeight + zoomed.rowHeight * 2 + 5) / dpr;
    expect(renderer.hitTest(zoomedX, zoomedY)).toEqual({ kind: "cell", row: 2, col: 1 });
    expect(renderer.hitTest(cssX, cssY)).toEqual({ kind: "cell", row: 1, col: 0 });
  });

  it("换数据后回到左上角,选区重置到 A1", () => {
    const { renderer, flush } = setupRenderer();
    flush();
    renderer.setScroll(300, 300);
    renderer.setSelection({ row: 9, col: 4 });
    flush();

    renderer.setSheet(createFixtureSheet(makeGrid(5, 2)));
    flush();

    expect(renderer.getScroll()).toEqual({ x: 0, y: 0 });
    expect(renderer.getSelection()).toEqual({ row: 0, col: 0 });
  });

  it("空表格画占位画面而不是崩溃", () => {
    const layers = createStubLayers();
    const frame = manualFrame();
    const renderer = new GridRenderer({
      container: layers.container,
      createElement: layers.createElement,
      schedule: frame.schedule,
    });
    renderer.resize(400, 300, 1);
    renderer.setSheet(createFixtureSheet([]));
    frame.flush();

    expect(layers.layer("overlay")!.texts()).toContain("没有可显示的数据");
    expect(renderer.getStats().frames).toBe(1);
  });

  it("超长文本被裁剪,不会溢出到相邻列", () => {
    const layers = createStubLayers();
    const frame = manualFrame();
    const renderer = new GridRenderer({
      container: layers.container,
      createElement: layers.createElement,
      schedule: frame.schedule,
    });
    renderer.resize(800, 400, 1);
    renderer.setSheet(createFixtureSheet([["x".repeat(5_000), "第二列"]]));
    frame.flush();

    const drawn = layers.layer("body")!.texts();
    const longest = drawn.reduce((a, b) => (a.length > b.length ? a : b), "");
    expect(longest.length).toBeLessThan(200);
    expect(longest.endsWith("…")).toBe(true);
  });

  it("单元格内嵌换行显示为单行", () => {
    const layers = createStubLayers();
    const frame = manualFrame();
    const renderer = new GridRenderer({
      container: layers.container,
      createElement: layers.createElement,
      schedule: frame.schedule,
    });
    renderer.resize(800, 400, 1);
    renderer.setSheet(createFixtureSheet([["第一行\n第二行"]]));
    frame.flush();

    expect(layers.layer("body")!.texts().some((text) => text.includes("\n"))).toBe(false);
  });
});
