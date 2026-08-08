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
