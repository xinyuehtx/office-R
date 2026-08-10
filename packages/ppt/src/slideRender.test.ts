import { describe, it, expect, vi } from "vitest";
import { fitScale, drawSlide, applyTransition } from "./slideRender";
import type { Slide } from "./model";

describe("fitScale", () => {
  it("等比缩放并居中", () => {
    // 幻灯 960x540 放进 480x360:scale = min(0.5, 0.667) = 0.5
    const r = fitScale(960, 540, 480, 360);
    expect(r.scale).toBeCloseTo(0.5);
    // 缩放后 480x270,垂直居中偏移 (360-270)/2 = 45
    expect(r.offsetY).toBeCloseTo(45);
    expect(r.offsetX).toBeCloseTo(0);
  });

  it("退化输入返回安全值", () => {
    expect(fitScale(0, 0, 100, 100)).toEqual({ scale: 1, offsetX: 0, offsetY: 0 });
  });
});

/** 记录调用的假 ctx,验证 drawSlide 按形状类型发出正确的绘制调用。 */
function fakeCtx() {
  const calls: string[] = [];
  return {
    calls,
    ctx: {
      fillStyle: "",
      strokeStyle: "",
      font: "",
      textBaseline: "",
      lineWidth: 1,
      beginPath: () => calls.push("beginPath"),
      rect: () => calls.push("rect"),
      clip: () => {},
      fillRect: () => calls.push("fillRect"),
      ellipse: () => calls.push("ellipse"),
      moveTo: () => {},
      lineTo: () => {},
      arcTo: () => {},
      closePath: () => {},
      fill: () => calls.push("fill"),
      stroke: () => calls.push("stroke"),
      strokeRect: () => calls.push("strokeRect"),
      fillText: (t: string) => calls.push(`fillText:${t}`),
      measureText: (t: string) => ({ width: t.length * 8 }),
      setLineDash: () => {},
      createLinearGradient: () => ({ addColorStop: () => {} }),
      save: () => calls.push("save"),
      restore: () => calls.push("restore"),
      translate: () => {},
      rotate: () => calls.push("rotate"),
      scale: () => calls.push("scale"),
    } as unknown as CanvasRenderingContext2D,
  };
}

describe("drawSlide", () => {
  it("矩形填充形状发出 fill", () => {
    const slide: Slide = {
      shapes: [
        {
          x: 0,
          y: 0,
          width: 100,
          height: 50,
          geom: "rect",
          fill: "FF0000",
          image: null,
          paragraphs: [],
        },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls).toContain("fill");
  });

  it("椭圆形状发出 ellipse 路径", () => {
    const slide: Slide = {
      shapes: [
        { x: 0, y: 0, width: 80, height: 80, geom: "ellipse", fill: "00FF00", image: null, paragraphs: [] },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls).toContain("ellipse");
  });

  it("文本形状发出 fillText", () => {
    const slide: Slide = {
      shapes: [
        {
          x: 0,
          y: 0,
          width: 300,
          height: 60,
          geom: null,
          fill: null,
          image: null,
          paragraphs: [
            { align: "center", runs: [{ text: "标题", bold: true, italic: false, size_pt: 24, color: "0000FF" }] },
          ],
        },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls.some((c) => c.startsWith("fillText:标题"))).toBe(true);
  });

  it("图片未解码时画占位框", () => {
    const slide: Slide = {
      shapes: [
        { x: 0, y: 0, width: 100, height: 100, geom: null, fill: null, image: "rId1", paragraphs: [] },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls).toContain("strokeRect");
  });

  it("图片已解码时调用 drawImage", () => {
    const slide: Slide = {
      shapes: [
        { x: 0, y: 0, width: 100, height: 100, geom: null, fill: null, image: "rId1", paragraphs: [] },
      ],
    };
    const { ctx } = fakeCtx();
    const drawImage = vi.fn();
    (ctx as unknown as { drawImage: unknown }).drawImage = drawImage;
    const img = { complete: true, naturalWidth: 10 } as HTMLImageElement;
    drawSlide(ctx, slide, 1, new Map([["rId1", img]]));
    expect(drawImage).toHaveBeenCalled();
  });

  it("图表占位画虚线框 + 类型标签", () => {
    const slide: Slide = {
      shapes: [
        {
          x: 0,
          y: 0,
          width: 200,
          height: 100,
          geom: null,
          fill: null,
          image: null,
          paragraphs: [],
          placeholder_kind: "chart",
        },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls).toContain("strokeRect");
    expect(calls.some((c) => c.startsWith("fillText:图表"))).toBe(true);
  });

  it("SmartArt 占位标签为 SmartArt", () => {
    const slide: Slide = {
      shapes: [
        {
          x: 0,
          y: 0,
          width: 200,
          height: 100,
          geom: null,
          fill: null,
          image: null,
          paragraphs: [],
          placeholder_kind: "diagram",
        },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls.some((c) => c.startsWith("fillText:SmartArt"))).toBe(true);
  });

  it("内嵌表格绘制网格线与单元格文本", () => {
    const slide: Slide = {
      shapes: [
        {
          x: 0,
          y: 0,
          width: 200,
          height: 100,
          geom: null,
          fill: null,
          image: null,
          paragraphs: [],
          placeholder_kind: "table",
          table: {
            col_widths: [100, 100],
            rows: [
              ["姓名", "分数"],
              ["张三", "88"],
            ],
          },
        },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls).toContain("stroke"); // 网格线
    expect(calls.some((c) => c.startsWith("fillText:姓名"))).toBe(true);
    expect(calls.some((c) => c.startsWith("fillText:88"))).toBe(true);
  });

  it("渐变填充调用 createLinearGradient 并 fill", () => {
    const grad = vi.fn(() => ({ addColorStop: () => {} }));
    const slide: Slide = {
      shapes: [
        {
          x: 0,
          y: 0,
          width: 100,
          height: 50,
          geom: "rect",
          fill: null,
          image: null,
          paragraphs: [],
          gradient: ["FF0000", "0000FF"],
        },
      ],
    };
    const { ctx, calls } = fakeCtx();
    (ctx as unknown as { createLinearGradient: unknown }).createLinearGradient = grad;
    drawSlide(ctx, slide, 1, new Map());
    expect(grad).toHaveBeenCalled();
    expect(calls).toContain("fill");
  });

  it("旋转/翻转形状发出 rotate/scale 变换", () => {    const slide: Slide = {
      shapes: [
        {
          x: 0,
          y: 0,
          width: 100,
          height: 50,
          geom: "rect",
          fill: "FF0000",
          image: null,
          paragraphs: [],
          rotation: 90,
          flip_h: true,
        },
      ],
    };
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls).toContain("rotate");
    expect(calls).toContain("scale");
    expect(calls).toContain("restore");
  });

  it("入场动画:按步过滤未出现的形状", () => {
    const text = (t: string, step: number) => ({
      x: 0,
      y: 0,
      width: 300,
      height: 60,
      geom: null,
      fill: null,
      image: null,
      paragraphs: [
        { align: "left" as const, runs: [{ text: t, bold: false, italic: false, size_pt: 18, color: null }] },
      ],
      appear_step: step,
    });
    const slide: Slide = { shapes: [text("常驻", 0), text("第一步", 1), text("第二步", 2)] };

    const at = (step: number) => {
      const { ctx, calls } = fakeCtx();
      drawSlide(ctx, slide, 1, new Map(), step);
      return calls.filter((c) => c.startsWith("fillText:")).join("|");
    };
    expect(at(0)).toContain("常驻");
    expect(at(0)).not.toContain("第一步");
    expect(at(1)).toContain("第一步");
    expect(at(1)).not.toContain("第二步");
    expect(at(2)).toContain("第二步");
    // 缺省(普通视图)显示全部
    const { ctx, calls } = fakeCtx();
    drawSlide(ctx, slide, 1, new Map());
    expect(calls.join("|")).toContain("第二步");
  });
});

describe("applyTransition", () => {
  /** 记录 globalAlpha / clip / translate 的假 ctx。 */
  function transCtx() {
    const rects: number[][] = [];
    const shifts: number[][] = [];
    const state = { globalAlpha: 1, clipped: false };
    const ctx = {
      get globalAlpha() {
        return state.globalAlpha;
      },
      set globalAlpha(v: number) {
        state.globalAlpha = v;
      },
      beginPath: () => {},
      rect: (x: number, y: number, w: number, h: number) => rects.push([x, y, w, h]),
      clip: () => {
        state.clipped = true;
      },
      translate: (x: number, y: number) => shifts.push([x, y]),
    } as unknown as CanvasRenderingContext2D;
    return { ctx, rects, shifts, state };
  }

  it("fade 按进度设置透明度", () => {
    const { ctx, state } = transCtx();
    applyTransition(ctx, "fade", 0.25, 100, 50);
    expect(state.globalAlpha).toBeCloseTo(0.25);
  });

  it("wipe 自左向右裁剪", () => {
    const { ctx, rects, state } = transCtx();
    applyTransition(ctx, "wipe", 0.5, 100, 50);
    expect(rects[0]).toEqual([0, 0, 50, 50]);
    expect(state.clipped).toBe(true);
  });

  it("push 自右侧位移进入", () => {
    const { ctx, shifts } = transCtx();
    applyTransition(ctx, "push", 0.25, 100, 50);
    expect(shifts[0]).toEqual([75, 0]);
  });

  it("无切换 / 已完成 / 未知类型不做变换", () => {
    for (const [kind, t] of [
      [null, 0.5],
      ["fade", 1],
      ["cut", 0.5],
    ] as [string | null, number][]) {
      const { ctx, rects, shifts, state } = transCtx();
      applyTransition(ctx, kind, t, 100, 50);
      expect(state.globalAlpha).toBe(1);
      expect(state.clipped).toBe(false);
      expect(rects).toHaveLength(0);
      expect(shifts).toHaveLength(0);
    }
  });
});
