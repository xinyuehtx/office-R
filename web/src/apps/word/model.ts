/**
 * Word 文档模型的 TS 类型(与 Rust `office_core::docx` 的 serde 输出一一对应)。
 * 视图层据此做流式布局与 canvas 渲染。
 */

export type Align = "left" | "center" | "right" | "justify";

/** 修订标记:无 / 插入 / 删除。 */
export type Revision = "none" | "inserted" | "deleted";

export interface Run {
  kind: "text";
  text: string;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  /** 字号(磅);缺省用段落/标题默认。 */
  size_pt: number | null;
  /** RRGGBB(无 #)。 */
  color: string | null;
  /** 修订标记(缺省 none)。 */
  revision?: Revision;
  /** 超链接目标(外部 URL 或 #锚点);非链接为 null/缺省。 */
  link?: string | null;
}

export interface ImageRef {
  kind: "image";
  id: string;
  width_px: number;
  height_px: number;
}

export interface BreakInline {
  kind: "break";
}

export type Inline = Run | ImageRef | BreakInline;

export interface ListItem {
  level: number;
  ordered: boolean;
  /** 有序列表序号(从 1 起);无序为 null。 */
  number: number | null;
}

export interface Paragraph {
  type: "paragraph";
  heading: number | null;
  align: Align;
  list: ListItem | null;
  inlines: Inline[];
  /** 左缩进(像素);缺省 0。 */
  indent_px?: number;
  /** 段前间距(像素);缺省 0。 */
  space_before_px?: number;
  /** 段后间距(像素);缺省 0。 */
  space_after_px?: number;
  /** 行距倍数;缺省用默认行距。 */
  line_pct?: number | null;
}

export interface TableCell {
  blocks: Block[];
}
export interface TableRow {
  cells: TableCell[];
}
export interface Table {
  type: "table";
  rows: TableRow[];
}

export type Block = Paragraph | Table;

export interface WordModel {
  blocks: Block[];
  /** 正文分栏数(默认 1)。 */
  columns?: number;
  /** 页眉块。 */
  header?: Block[];
  /** 页脚块。 */
  footer?: Block[];
  /** 脚注块(渲染在正文末尾)。 */
  footnotes?: Block[];
}

/** 图片资源:id → object URL(由字节 + mime 构造)。 */
export type ImageUrls = Map<string, string>;

/** 加载完成的 Word 文档:模型 + 图片 URL + 释放函数。 */
export interface WordDocument {
  model: WordModel;
  images: ImageUrls;
  /** 释放图片 object URL。 */
  dispose(): void;
}
