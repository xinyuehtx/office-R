/**
 * Word 文档的**流式布局**:把文档模型排成一串带绝对 y 的绘制项。
 *
 * 纯函数、不碰 canvas(只用注入的测量器),因此可单测。视图层拿到 `Layout` 后:
 * - 记录总高度用于滚动;
 * - **纵向虚拟化**:每帧只绘制与视口相交的项(见 `WordCanvas`)。
 *
 * 覆盖:标题、正文、加粗/斜体/下划线/字号/颜色、段落对齐(左/中/右;两端≈左)、
 * 项目符号/编号列表(缩进 + 前缀)、内联图片(等比缩放)、表格(等宽列 + 边框)、图文混排。
 */

import type { TextMeasurer } from "@tengxiaohyx/office-shared";
import { FONT_FAMILY } from "@tengxiaohyx/office-shared";
import type { Block, Paragraph, Run, WordModel } from "@tengxiaohyx/office-word";

/** 页面内边距与默认排版常量(CSS 像素)。 */
export const PAGE_PADDING = 48;
const BODY_FONT_PX = 15;
const LINE_SPACING = 1.5;
const PARA_GAP = 8;
const LIST_INDENT = 26;
const HEADING_PX: Record<number, number> = { 1: 28, 2: 23, 3: 19, 4: 16, 5: 14, 6: 13 };
const HEADING_GAP = 14;

/** 一段可绘制的文本片段(同一批样式)。 */
export interface TextSegment {
  x: number;
  text: string;
  font: string;
  color: string;
  underline: boolean;
  /** 删除线(修订删除)。 */
  strike?: boolean;
}

/** 绘制项(带绝对 y,供虚拟化裁剪)。 */
export type DrawItem =
  | { kind: "textline"; y: number; height: number; segments: TextSegment[] }
  | { kind: "image"; x: number; y: number; width: number; height: number; id: string }
  | { kind: "rect"; x: number; y: number; width: number; height: number };

/** 布局结果。 */
export interface Layout {
  /** 内容宽度(含内边距的页面宽度)。 */
  width: number;
  /** 内容总高度。 */
  height: number;
  items: DrawItem[];
}

/** 一处全文查找命中:所在行的顶端 y 与行高(布局坐标)。 */
export interface WordMatch {
  y: number;
  height: number;
}

/**
 * 全文查找:返回文本行里(不区分大小写)包含 `query` 的行位置,按文档顺序。
 *
 * CJK 逐字成段,故按整行拼接后再匹配;跨行的匹配不处理(按行粒度,足够定位)。
 */
export function findLineMatches(layout: Layout, query: string): WordMatch[] {
  const q = query.trim().toLowerCase();
  if (q === "") return [];
  const out: WordMatch[] = [];
  for (const item of layout.items) {
    if (item.kind !== "textline") continue;
    const text = item.segments.map((s) => s.text).join("").toLowerCase();
    if (text.includes(q)) {
      out.push({ y: item.y - item.height / 2, height: item.height });
    }
  }
  return out;
}

/** 解析出的一个内联「词」token(用于折行)。 */
type Token =
  | { type: "word"; text: string; run: Run; width: number; font: string }
  | { type: "space"; width: number; run: Run; font: string }
  | { type: "image"; id: string; width: number; height: number }
  | { type: "break" };

const DEFAULT_COLOR = "#1f2328";

function ptToPx(pt: number): number {
  return (pt * 96) / 72;
}

/** 段落基础字号与是否加粗(标题更大更粗)。 */
function paragraphBaseFont(p: Paragraph): { px: number; bold: boolean; gap: number } {
  if (p.heading && HEADING_PX[p.heading]) {
    return { px: HEADING_PX[p.heading], bold: true, gap: HEADING_GAP };
  }
  return { px: BODY_FONT_PX, bold: false, gap: PARA_GAP };
}

function runFont(run: Run, basePx: number, baseBold: boolean): { font: string; px: number } {
  const px = run.size_pt != null ? ptToPx(run.size_pt) : basePx;
  const bold = run.bold || baseBold;
  const italic = run.italic ? "italic " : "";
  const weight = bold ? "bold " : "";
  return { font: `${italic}${weight}${px}px ${FONT_FAMILY}`, px };
}

function runColor(run: Run): string {
  // 修订用醒目色:插入(蓝)、删除(红);超链接蓝;否则用原色 / 默认
  if (run.revision === "inserted") return "#0969da";
  if (run.revision === "deleted") return "#cf222e";
  if (run.link) return "#0969da";
  if (run.color && /^[0-9a-fA-F]{6}$/.test(run.color)) return `#${run.color}`;
  return DEFAULT_COLOR;
}

/** 把段落的内联元素拆成折行用的 token 序列。 */
function tokenize(
  p: Paragraph,
  basePx: number,
  baseBold: boolean,
  contentWidth: number,
  measurer: TextMeasurer,
): Token[] {
  const tokens: Token[] = [];
  for (const inline of p.inlines) {
    if (inline.kind === "break") {
      tokens.push({ type: "break" });
    } else if (inline.kind === "image") {
      // 等比缩放到内容宽度内
      let w = inline.width_px || 120;
      let h = inline.height_px || 90;
      if (w > contentWidth) {
        const s = contentWidth / w;
        w *= s;
        h *= s;
      }
      tokens.push({ type: "image", id: inline.id, width: w, height: h });
    } else {
      const { font } = runFont(inline, basePx, baseBold);
      // 切词:CJK 逐字(无空格,靠单字换行),西文按词,空白单独成 token。
      // 区间端点用 \u 转义而非字面字符:U+3000(表意空格)写进源码后肉眼与普通
      // 空格无异,ESLint 的 no-irregular-whitespace 正是冲它来的。
      const parts =
        inline.text.match(
          /[\u3000-\u303F\u3400-\u9FFF\uFF00-\uFFEF]|\s+|[^\s\u3000-\u303F\u3400-\u9FFF\uFF00-\uFFEF]+/g,
        ) ?? [];
      for (const part of parts) {
        const width = measurer.measure(part, font);
        if (/^\s+$/.test(part)) {
          tokens.push({ type: "space", width, run: inline, font });
        } else {
          tokens.push({ type: "word", text: part, run: inline, width, font });
        }
      }
    }
  }
  return tokens;
}

/** 单段落布局:把 token 折行,产出绝对 y 的绘制项;返回新的 y。 */
function layoutParagraph(
  p: Paragraph,
  x0: number,
  contentWidth: number,
  y: number,
  measurer: TextMeasurer,
  out: DrawItem[],
): number {
  const base = paragraphBaseFont(p);
  // 段前间距
  y += p.space_before_px ?? 0;
  const extraIndent = p.indent_px ?? 0;
  const listIndent = (p.list ? (p.list.level + 1) * LIST_INDENT : 0) + extraIndent;
  const left = x0 + listIndent;
  const avail = Math.max(10, contentWidth - listIndent);

  const tokens = tokenize(p, base.px, base.bold, avail, measurer);

  // 贪心折行:累积 token 到一行,超宽换行
  type Line = { tokens: Token[]; width: number };
  const lines: Line[] = [];
  let cur: Token[] = [];
  let curW = 0;
  const pushLine = () => {
    // 去掉行尾空白
    while (cur.length && cur[cur.length - 1].type === "space") {
      curW -= (cur.pop() as { width: number }).width;
    }
    lines.push({ tokens: cur, width: curW });
    cur = [];
    curW = 0;
  };
  for (const t of tokens) {
    if (t.type === "break") {
      pushLine();
      continue;
    }
    const tw = t.width;
    if (curW + tw > avail && cur.length > 0 && t.type !== "space") {
      pushLine();
    }
    // 行首不放空白
    if (t.type === "space" && cur.length === 0) continue;
    cur.push(t);
    curW += tw;
  }
  pushLine();

  // 列表前缀(画在第一行左侧):有序用「序号.」,无序用项目符号
  const bulletFont = `${base.px}px ${FONT_FAMILY}`;
  const prefix = p.list
    ? p.list.ordered
      ? `${p.list.number ?? 1}.`
      : "•"
    : "";

  for (let li = 0; li < lines.length; li += 1) {
    const line = lines[li];
    // 行高:该行最大字号 × 行距;空行按基础字号
    let maxPx = base.px;
    let maxImgH = 0;
    for (const t of line.tokens) {
      if (t.type === "word" || t.type === "space") {
        const m = t.font.match(/(\d+(?:\.\d+)?)px/);
        if (m) maxPx = Math.max(maxPx, parseFloat(m[1]));
      } else if (t.type === "image") {
        maxImgH = Math.max(maxImgH, t.height);
      }
    }
    const textH = maxPx * (p.line_pct ?? LINE_SPACING);
    const lineH = Math.max(textH, maxImgH + 4);
    const baselineY = y + lineH / 2;

    // 对齐:计算起始 x
    let startX = left;
    if (p.align === "center") startX = left + (avail - line.width) / 2;
    else if (p.align === "right") startX = left + (avail - line.width);

    // 两端对齐:非末行把富余宽度均摊到词间空白(末行左对齐,与 Word 一致)
    const isLastLine = li === lines.length - 1;
    let spaceStretch = 0;
    if (p.align === "justify" && !isLastLine) {
      const spaceCount = line.tokens.filter((t) => t.type === "space").length;
      if (spaceCount > 0 && line.width < avail) {
        spaceStretch = (avail - line.width) / spaceCount;
      }
    }

    // 列表符号只在首行
    const segments: TextSegment[] = [];
    if (prefix && li === 0) {
      segments.push({
        x: x0 + listIndent - LIST_INDENT + 4,
        text: prefix,
        font: bulletFont,
        color: DEFAULT_COLOR,
        underline: false,
      });
    }

    let cx = Math.max(left, startX);
    for (const t of line.tokens) {
      if (t.type === "image") {
        out.push({
          kind: "image",
          id: t.id,
          x: cx,
          y: y + (lineH - t.height) / 2,
          width: t.width,
          height: t.height,
        });
        cx += t.width;
      } else if (t.type === "word") {
        segments.push({
          x: cx,
          text: t.text,
          font: t.font,
          color: runColor(t.run),
          underline: t.run.underline || !!t.run.link,
          strike: t.run.revision === "deleted",
        });
        cx += t.width;
      } else if (t.type === "space") {
        cx += t.width + spaceStretch; // 空白(两端对齐时含拉伸)
      }
    }
    if (segments.length > 0) {
      out.push({ kind: "textline", y: baselineY, height: lineH, segments });
    }
    y += lineH;
  }

  return y + base.gap + (p.space_after_px ?? 0);
}

/** 表格布局:等宽列,单元格递归布局,行高取单元格最大高。 */
function layoutTable(
  table: { rows: { cells: { blocks: Block[] }[] }[] },
  x0: number,
  contentWidth: number,
  y: number,
  measurer: TextMeasurer,
  out: DrawItem[],
): number {
  const colCount = table.rows.reduce((m, r) => Math.max(m, r.cells.length), 1);
  const colW = contentWidth / colCount;
  const cellPad = 6;

  for (const row of table.rows) {
    const rowTop = y;
    // 先各单元格布局到临时数组,求行高
    const cellItems: DrawItem[][] = [];
    let rowH = 0;
    for (let c = 0; c < colCount; c += 1) {
      const cell = row.cells[c];
      const items: DrawItem[] = [];
      let cy = rowTop + cellPad;
      if (cell) {
        for (const block of cell.blocks) {
          if (block.type === "paragraph") {
            cy = layoutParagraph(block, x0 + c * colW + cellPad, colW - cellPad * 2, cy, measurer, items);
          }
        }
      }
      cellItems.push(items);
      rowH = Math.max(rowH, cy - rowTop + cellPad);
    }
    // 落盘单元格内容 + 边框
    for (let c = 0; c < colCount; c += 1) {
      out.push({ kind: "rect", x: x0 + c * colW, y: rowTop, width: colW, height: rowH });
      out.push(...cellItems[c]);
    }
    y = rowTop + rowH;
  }
  return y + PARA_GAP;
}

/** 把一组块从 (x0,y) 起、按 width 流式布局,返回结束 y。 */
function layoutBlocks(
  blocks: Block[],
  x0: number,
  width: number,
  y: number,
  measurer: TextMeasurer,
  items: DrawItem[],
): number {
  for (const block of blocks) {
    if (block.type === "paragraph") {
      y = layoutParagraph(block, x0, width, y, measurer, items);
    } else if (block.type === "table") {
      y = layoutTable(block, x0, width, y, measurer, items);
    }
  }
  return y;
}

/** 页眉/页脚分隔线与列间距。 */
const HF_GAP = 10;
const COL_GAP = 28;

/** 对整篇文档做流式布局(支持分栏、页眉、页脚)。 */
export function layoutDoc(model: WordModel, pageWidth: number, measurer: TextMeasurer): Layout {
  const contentWidth = Math.max(40, pageWidth - PAGE_PADDING * 2);
  const x0 = PAGE_PADDING;
  const items: DrawItem[] = [];
  let y = PAGE_PADDING;

  // 页眉:顶部块 + 一条分隔线
  const header = model.header ?? [];
  if (header.length > 0) {
    y = layoutBlocks(header, x0, contentWidth, y, measurer, items);
    y += HF_GAP;
    items.push({ kind: "rect", x: x0, y, width: contentWidth, height: 0 });
    y += HF_GAP;
  }

  // 正文:分栏(columns>1 时按等宽多列平铺,列满则回到顶继续下一列)
  const columns = Math.max(1, Math.floor(model.columns ?? 1));
  if (columns <= 1) {
    y = layoutBlocks(model.blocks, x0, contentWidth, y, measurer, items);
  } else {
    const colWidth = (contentWidth - COL_GAP * (columns - 1)) / columns;
    // 先按单列量出每块高度,用「贪心装箱」把块分配到各列,尽量等高
    const bodyTop = y;
    // 估算总高:先在临时缓冲里单列布局,拿到每块高度
    const heights = blockHeights(model.blocks, colWidth, measurer);
    const total = heights.reduce((a, b) => a + b, 0);
    const target = total / columns;
    let col = 0;
    let colY = bodyTop;
    let acc = 0;
    let maxBottom = bodyTop;
    for (let i = 0; i < model.blocks.length; i += 1) {
      // 换列:累计超过目标且还有后续列
      if (acc > 0 && acc + heights[i] / 2 > target && col < columns - 1) {
        col += 1;
        colY = bodyTop;
        acc = 0;
      }
      const cx = x0 + col * (colWidth + COL_GAP);
      colY = layoutBlocks([model.blocks[i]], cx, colWidth, colY, measurer, items);
      acc += heights[i];
      maxBottom = Math.max(maxBottom, colY);
    }
    y = maxBottom;
  }

  // 页脚:一条分隔线 + 底部块
  const footer = model.footer ?? [];
  if (footer.length > 0) {
    y += HF_GAP;
    items.push({ kind: "rect", x: x0, y, width: contentWidth, height: 0 });
    y += HF_GAP;
    y = layoutBlocks(footer, x0, contentWidth, y, measurer, items);
  }

  // 脚注:一条分隔线 + 各条脚注(渲染在正文末尾)
  const footnotes = model.footnotes ?? [];
  if (footnotes.length > 0) {
    y += HF_GAP;
    items.push({ kind: "rect", x: x0, y, width: contentWidth / 3, height: 0 });
    y += HF_GAP;
    y = layoutBlocks(footnotes, x0, contentWidth, y, measurer, items);
  }

  // 批注:一条分隔线 + 各条批注(渲染在正文末尾)
  const comments = model.comments ?? [];
  if (comments.length > 0) {
    y += HF_GAP;
    items.push({ kind: "rect", x: x0, y, width: contentWidth / 3, height: 0 });
    y += HF_GAP;
    y = layoutBlocks(comments, x0, contentWidth, y, measurer, items);
  }

  return { width: pageWidth, height: y + PAGE_PADDING, items };
}

/** 单列布局量取每个块的高度(用于分栏装箱),不产出绘制项。 */
function blockHeights(blocks: Block[], width: number, measurer: TextMeasurer): number[] {
  return blocks.map((b) => {
    const tmp: DrawItem[] = [];
    const end = layoutBlocks([b], 0, width, 0, measurer, tmp);
    return end;
  });
}

/** 收集布局里用到的图片 id(用于预加载)。 */
export function imageIdsIn(model: WordModel): string[] {
  const ids = new Set<string>();
  const walk = (blocks: Block[]) => {
    for (const b of blocks) {
      if (b.type === "paragraph") {
        for (const inl of b.inlines) if (inl.kind === "image") ids.add(inl.id);
      } else if (b.type === "table") {
        for (const r of b.rows) for (const c of r.cells) walk(c.blocks);
      }
    }
  };
  walk(model.blocks);
  walk(model.header ?? []);
  walk(model.footer ?? []);
  walk(model.footnotes ?? []);
  walk(model.comments ?? []);
  return [...ids];
}
