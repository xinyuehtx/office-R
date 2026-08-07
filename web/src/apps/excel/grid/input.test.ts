import { describe, it, expect } from "vitest";
import { wheelToScrollDelta, wheelToZoomFactor, type WheelLike } from "./input";

const CONTEXT = { lineHeight: 24, pageHeight: 400 };

/** 造一个滚轮事件。默认是最常见的「像素模式、无修饰键」。 */
function wheel(overrides: Partial<WheelLike>): WheelLike {
  return { deltaX: 0, deltaY: 0, deltaMode: 0, shiftKey: false, ...overrides };
}

describe("wheelToScrollDelta", () => {
  it("纵向滚轮只产生纵向滚动", () => {
    expect(wheelToScrollDelta(wheel({ deltaY: 120 }), CONTEXT)).toEqual({ dx: 0, dy: 120 });
  });

  it("触控板同时给出两轴时直接透传,实现双向滚动", () => {
    expect(wheelToScrollDelta(wheel({ deltaX: -40, deltaY: 15 }), CONTEXT)).toEqual({
      dx: -40,
      dy: 15,
    });
  });

  it("Shift + 滚轮把纵向增量转成横向 —— 鼠标用户唯一的横滚方式", () => {
    expect(wheelToScrollDelta(wheel({ deltaY: 120, shiftKey: true }), CONTEXT)).toEqual({
      dx: 120,
      dy: 0,
    });
  });

  it("Shift 反向滚动同样有效", () => {
    expect(wheelToScrollDelta(wheel({ deltaY: -90, shiftKey: true }), CONTEXT)).toEqual({
      dx: -90,
      dy: 0,
    });
  });

  it("平台已经给出 deltaX 时不再二次转换,避免横滚两倍", () => {
    // 部分平台/浏览器会自己把 Shift+滚轮 变成 deltaX
    expect(wheelToScrollDelta(wheel({ deltaX: 120, deltaY: 0, shiftKey: true }), CONTEXT)).toEqual({
      dx: 120,
      dy: 0,
    });
  });

  it("行模式按行高换算", () => {
    expect(wheelToScrollDelta(wheel({ deltaY: 3, deltaMode: 1 }), CONTEXT)).toEqual({
      dx: 0,
      dy: 72,
    });
  });

  it("页模式按一屏高度换算", () => {
    expect(wheelToScrollDelta(wheel({ deltaY: 1, deltaMode: 2 }), CONTEXT)).toEqual({
      dx: 0,
      dy: 400,
    });
  });

  it("行模式下的 Shift 横滚也按行高换算", () => {
    expect(wheelToScrollDelta(wheel({ deltaY: 2, deltaMode: 1, shiftKey: true }), CONTEXT)).toEqual({
      dx: 48,
      dy: 0,
    });
  });

  it("没有增量时不产生滚动", () => {
    expect(wheelToScrollDelta(wheel({}), CONTEXT)).toEqual({ dx: 0, dy: 0 });
  });
});

describe("wheelToZoomFactor", () => {
  it("向上滚放大,向下滚缩小", () => {
    expect(wheelToZoomFactor(wheel({ deltaY: -100 }), CONTEXT)).toBeGreaterThan(1);
    expect(wheelToZoomFactor(wheel({ deltaY: 100 }), CONTEXT)).toBeLessThan(1);
  });

  it("放大再缩小同样距离能回到原点(指数换算的意义)", () => {
    const inFactor = wheelToZoomFactor(wheel({ deltaY: -240 }), CONTEXT);
    const outFactor = wheelToZoomFactor(wheel({ deltaY: 240 }), CONTEXT);
    expect(inFactor * outFactor).toBeCloseTo(1, 10);
  });

  it("无增量时倍率为 1", () => {
    expect(wheelToZoomFactor(wheel({ deltaY: 0 }), CONTEXT)).toBe(1);
  });
});
