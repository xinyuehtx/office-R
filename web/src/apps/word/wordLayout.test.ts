import { describe, it, expect } from "vitest";
import { layoutDoc, imageIdsIn } from "./wordLayout";
import { TextMeasurer } from "../shared/textMeasure";
import type { WordModel, Paragraph, Run } from "./model";

/** 无 canvas 环境下,TextMeasurer 用字符数 × 8 兜底,布局也应产出合理结果。 */
const measurer = new TextMeasurer();

function run(text: string, extra: Partial<Run> = {}): Run {
  return {
    kind: "text",
    text,
    bold: false,
    italic: false,
    underline: false,
    size_pt: null,
    color: null,
    ...extra,
  };
}

function para(inlines: Run[], extra: Partial<Paragraph> = {}): Paragraph {
  return { type: "paragraph", heading: null, align: "left", list: null, inlines, ...extra };
}

describe("wordLayout", () => {
  it("空文档只有内边距高度", () => {
    const layout = layoutDoc({ blocks: [] }, 820, measurer);
    expect(layout.items).toHaveLength(0);
    expect(layout.height).toBeGreaterThan(0);
  });

  it("段落产出文本行,标题更高", () => {
    const model: WordModel = {
      blocks: [
        para([run("标题")], { heading: 1 }),
        para([run("正文一段")]),
      ],
    };
    const layout = layoutDoc(model, 820, measurer);
    const lines = layout.items.filter((i) => i.kind === "textline");
    expect(lines.length).toBeGreaterThanOrEqual(2);
    // 标题行更高
    expect(lines[0].kind === "textline" && lines[0].height).toBeGreaterThan(
      (lines[1].kind === "textline" && lines[1].height) || 0,
    );
  });

  it("长段落会折成多行,总高度随之增加", () => {
    const long = "词".repeat(400);
    const layout = layoutDoc({ blocks: [para([run(long)])] }, 820, measurer);
    const lines = layout.items.filter((i) => i.kind === "textline");
    expect(lines.length).toBeGreaterThan(1);
  });

  it("居中对齐:行起始 x 大于左边距", () => {
    const centered = layoutDoc(
      { blocks: [para([run("短")], { align: "center" })] },
      820,
      measurer,
    );
    const left = layoutDoc({ blocks: [para([run("短")])] }, 820, measurer);
    const cx = centered.items.find((i) => i.kind === "textline");
    const lx = left.items.find((i) => i.kind === "textline");
    if (cx?.kind === "textline" && lx?.kind === "textline") {
      expect(cx.segments[0].x).toBeGreaterThan(lx.segments[0].x);
    }
  });

  it("列表项带项目符号前缀", () => {
    const layout = layoutDoc(
      { blocks: [para([run("列表项")], { list: { level: 0, ordered: false, number: null } })] },
      820,
      measurer,
    );
    const line = layout.items.find((i) => i.kind === "textline");
    expect(line?.kind === "textline" && line.segments.some((s) => s.text === "•")).toBe(true);
  });

  it("内联图片产出 image 绘制项", () => {
    const model: WordModel = {
      blocks: [
        {
          type: "paragraph",
          heading: null,
          align: "left",
          list: null,
          inlines: [{ kind: "image", id: "rId1", width_px: 100, height_px: 80 }],
        },
      ],
    };
    const layout = layoutDoc(model, 820, measurer);
    const img = layout.items.find((i) => i.kind === "image");
    expect(img?.kind === "image" && img.id).toBe("rId1");
  });

  it("表格产出边框 rect", () => {
    const model: WordModel = {
      blocks: [
        {
          type: "table",
          rows: [
            {
              cells: [
                { blocks: [para([run("A1")])] },
                { blocks: [para([run("B1")])] },
              ],
            },
          ],
        },
      ],
    };
    const layout = layoutDoc(model, 820, measurer);
    const rects = layout.items.filter((i) => i.kind === "rect");
    expect(rects.length).toBe(2); // 两个单元格边框
  });

  it("imageIdsIn 收集内联图片 id(含表格内)", () => {
    const model: WordModel = {
      blocks: [
        {
          type: "table",
          rows: [
            {
              cells: [
                {
                  blocks: [
                    {
                      type: "paragraph",
                      heading: null,
                      align: "left",
                      list: null,
                      inlines: [{ kind: "image", id: "rIdX", width_px: 10, height_px: 10 }],
                    },
                  ],
                },
              ],
            },
          ],
        },
      ],
    };
    expect(imageIdsIn(model)).toEqual(["rIdX"]);
  });
});

describe("wordLayout · 列表编号与两端对齐", () => {
  it("有序列表前缀用序号", () => {
    const layout = layoutDoc(
      { blocks: [para([run("第一项")], { list: { level: 0, ordered: true, number: 3 } })] },
      820,
      measurer,
    );
    const line = layout.items.find((i) => i.kind === "textline");
    expect(line?.kind === "textline" && line.segments.some((s) => s.text === "3.")).toBe(true);
  });

  it("两端对齐:非末行词间被拉伸(首词起点不左移,行更宽)", () => {
    // 造一段有多个空格的英文,窄宽度强制折行
    const text = "aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp";
    const justified = layoutDoc(
      { blocks: [para([run(text)], { align: "justify" })] },
      300,
      measurer,
    );
    const left = layoutDoc({ blocks: [para([run(text)])] }, 300, measurer);
    const jLines = justified.items.filter((i) => i.kind === "textline");
    const lLines = left.items.filter((i) => i.kind === "textline");
    expect(jLines.length).toBeGreaterThan(1);
    // 非末行:两端对齐的最后一个词右缘应比左对齐更靠右(空白被拉伸)
    const jFirst = jLines[0];
    const lFirst = lLines[0];
    if (jFirst.kind === "textline" && lFirst.kind === "textline") {
      const jLastSeg = jFirst.segments[jFirst.segments.length - 1];
      const lLastSeg = lFirst.segments[lFirst.segments.length - 1];
      expect(jLastSeg.x).toBeGreaterThan(lLastSeg.x);
    }
  });
});

describe("wordLayout · 页眉页脚 + 分栏 + 修订", () => {
  it("页眉/页脚块被布局(含分隔线 rect)", () => {
    const model: WordModel = {
      blocks: [para([run("正文")])],
      header: [para([run("页眉")])],
      footer: [para([run("页脚")])],
    };
    const layout = layoutDoc(model, 820, measurer);
    // CJK 逐字成段,拼接每个 textline 判断包含
    const lineTexts = layout.items
      .filter((i) => i.kind === "textline")
      .map((i) => (i.kind === "textline" ? i.segments.map((s) => s.text).join("") : ""));
    expect(lineTexts.some((t) => t.includes("页眉"))).toBe(true);
    expect(lineTexts.some((t) => t.includes("页脚"))).toBe(true);
    expect(lineTexts.some((t) => t.includes("正文"))).toBe(true);
    // 页眉页脚各一条分隔线(height=0 的 rect)
    expect(layout.items.filter((i) => i.kind === "rect" && i.height === 0).length).toBe(2);
  });

  it("分栏:两列时块分布在不同 x", () => {
    const blocks = Array.from({ length: 8 }, (_, i) => para([run(`段落${i}`)]));
    const layout = layoutDoc({ blocks, columns: 2 }, 820, measurer);
    const xs = new Set(
      layout.items
        .filter((i) => i.kind === "textline")
        .map((i) => (i.kind === "textline" ? Math.round(i.segments[0].x) : 0)),
    );
    // 至少出现两个不同的起始 x(两列)
    expect(xs.size).toBeGreaterThanOrEqual(2);
  });

  it("修订:插入=蓝色,删除=红色+删除线", () => {
    const inserted: Run = { ...run("A"), revision: "inserted" };
    const deleted: Run = { ...run("B"), revision: "deleted" };
    const layout = layoutDoc({ blocks: [para([inserted, deleted])] }, 820, measurer);
    const segs = layout.items.flatMap((i) => (i.kind === "textline" ? i.segments : []));
    const ins = segs.find((s) => s.text === "A");
    const del = segs.find((s) => s.text === "B");
    expect(ins?.color).toBe("#0969da");
    expect(del?.color).toBe("#cf222e");
    expect(del?.strike).toBe(true);
  });

  it("超链接:蓝色 + 下划线", () => {
    const link: Run = { ...run("官网"), link: "https://example.com/" };
    const layout = layoutDoc({ blocks: [para([link])] }, 820, measurer);
    // CJK 逐字成段:取任一非空文本段验证其链接样式
    const seg = layout.items
      .flatMap((i) => (i.kind === "textline" ? i.segments : []))
      .find((s) => s.text.trim().length > 0);
    expect(seg?.color).toBe("#0969da");
    expect(seg?.underline).toBe(true);
  });

  it("脚注:渲染在正文末尾(带分隔线)", () => {
    const layout = layoutDoc(
      { blocks: [para([run("正文")])], footnotes: [para([run("1. 一条脚注")])] },
      820,
      measurer,
    );
    const lineTexts = layout.items
      .filter((i) => i.kind === "textline")
      .map((i) => (i.kind === "textline" ? i.segments.map((s) => s.text).join("") : ""));
    expect(lineTexts.some((t) => t.includes("一条脚注"))).toBe(true);
    // 脚注分隔线为 height=0 的 rect
    expect(layout.items.some((i) => i.kind === "rect" && i.height === 0)).toBe(true);
  });
});
