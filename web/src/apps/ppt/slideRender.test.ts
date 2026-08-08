import { describe, it, expect, vi } from "vitest";
import { fitScale, drawSlide } from "./slideRender";
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
      ellipse: () => calls.push("ellipse"),
      moveTo: () => {},
      lineTo: () => {},
      arcTo: () => {},
      closePath: () => {},
      fill: () => calls.push("fill"),
      stroke: () => calls.push("stroke"),
      strokeRect: () => calls.push("strokeRect"),
      fillText: (t: string) => calls.push(`fillText:${t}`),
      save: () => {},
      restore: () => {},
      translate: () => {},
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
});
