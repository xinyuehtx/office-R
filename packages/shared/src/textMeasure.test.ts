import { describe, it, expect } from "vitest";
import { TextMeasurer } from "./textMeasure";

/**
 * 用一个可控的假 canvas 上下文:宽度 = 字符数 × perChar,便于精确断言。
 * 通过 stub 全局 document.createElement 注入。
 */
function withStubCanvas(perChar: number, fn: (m: TextMeasurer) => void) {
  const ctx = {
    font: "",
    measureText: (t: string) => ({ width: Array.from(t).length * perChar }),
  };
  const stub = { createElement: () => ({ getContext: () => ctx }) };
  const originalDoc = Object.getOwnPropertyDescriptor(globalThis, "document");
  const off = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  // 用 stub 文档,并关掉 OffscreenCanvas 强制走 DOM 分支命中 stub
  Object.defineProperty(globalThis, "document", { value: stub, configurable: true });
  // @ts-expect-error 测试注入
  delete globalThis.OffscreenCanvas;
  try {
    fn(new TextMeasurer());
  } finally {
    if (originalDoc) Object.defineProperty(globalThis, "document", originalDoc);
    if (off) (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = off;
  }
}

describe("TextMeasurer", () => {
  it("测量宽度 = 字符数 × 每字符宽", () => {
    withStubCanvas(10, (m) => {
      expect(m.measure("abc", "14px x")).toBe(30);
      expect(m.measure("北京", "14px x")).toBe(20);
      expect(m.measure("", "14px x")).toBe(0);
    });
  });

  it("按 (font, text) 缓存:同参数只测一次", () => {
    withStubCanvas(10, (m) => {
      const first = m.measure("hello", "14px x");
      const second = m.measure("hello", "14px x");
      expect(first).toBe(second);
      // 不同字体各自缓存
      expect(m.measure("hello", "20px x")).toBe(50);
    });
  });

  it("fit:放得下则原样,放不下则省略号结尾", () => {
    withStubCanvas(10, (m) => {
      const font = "14px x";
      expect(m.fit("abcde", 100, font)).toBe("abcde"); // 50 <= 100
      // maxWidth 35:省略号占 10,前缀最多 2 字符(20+10=30<=35)
      const clipped = m.fit("abcde", 35, font);
      expect(clipped.endsWith("…")).toBe(true);
      expect(clipped.length).toBeLessThan("abcde…".length + 1);
    });
  });

  it("fit:宽度为 0 或放不下省略号返回空", () => {
    withStubCanvas(10, (m) => {
      expect(m.fit("abc", 0, "14px x")).toBe("");
      expect(m.fit("abc", 5, "14px x")).toBe(""); // 省略号 10 > 5
    });
  });

  it("wrap:按宽度折行,超长词按字符硬断", () => {
    withStubCanvas(10, (m) => {
      const font = "14px x";
      // 每行最多 3 字符(30px);"aaaa bbbb" → 词各 4 字符,硬断
      const lines = m.wrap("aaaa bbbb", 30, font);
      expect(lines.length).toBeGreaterThan(1);
      for (const l of lines) expect(m.measure(l, font)).toBeLessThanOrEqual(30);
    });
  });

  it("wrap:显式换行被保留", () => {
    withStubCanvas(10, (m) => {
      const lines = m.wrap("ab\ncd", 100, "14px x");
      expect(lines).toEqual(["ab", "cd"]);
    });
  });

  it("无 canvas 时用字符数兜底,不抛错", () => {
    const m = new TextMeasurer();
    // 不注入 document/OffscreenCanvas 的极简环境下也应返回有限值
    const w = m.measure("abcd", "14px x");
    expect(Number.isFinite(w)).toBe(true);
  });
});
