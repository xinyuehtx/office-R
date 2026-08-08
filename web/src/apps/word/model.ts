/**
 * Word 文档模型的 TS 类型(与 Rust `office_core::docx` 的 serde 输出一一对应)。
 * 视图层据此做流式布局与 canvas 渲染。
 */

export type Align = "left" | "center" | "right" | "justify";

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
