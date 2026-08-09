/**
 * PPT 演示模型的 TS 类型(与 Rust `office_core::pptx` 的 serde 输出对应)。
 */

export type Align = "left" | "center" | "right" | "justify";

export interface Run {
  text: string;
  bold: boolean;
  italic: boolean;
  size_pt: number | null;
  color: string | null;
}

export interface Para {
  align: Align;
  runs: Run[];
}

export interface Shape {
  x: number;
  y: number;
  width: number;
  height: number;
  /** 预设几何(rect/ellipse/…);文本框/图片为 null。 */
  geom: string | null;
  /** 填充色 RRGGBB。 */
  fill: string | null;
  /** 图片 embed id(在其幻灯内唯一)。 */
  image: string | null;
  paragraphs: Para[];
  /** 旋转角度(度,顺时针)。 */
  rotation?: number;
  /** 水平/垂直翻转。 */
  flip_h?: boolean;
  flip_v?: boolean;
  /** 内嵌对象占位类型:"chart" / "diagram" / "table";普通形状为 null。 */
  placeholder_kind?: string | null;
  /** 渐变填充两端色 [首, 末](RRGGBB);无渐变为 null。上→下线性渐变。 */
  gradient?: [string, string] | null;
  /** 内嵌表格;非表格为 null。 */
  table?: SlideTable | null;
  /** 内嵌图表;非图表为 null。 */
  chart?: SlideChart | null;
}

/** 幻灯内图表(与 Excel 图表同构)。 */
export interface SlideChart {
  kind: string;
  series: number[][];
  categories: string[];
  title?: string | null;
}

/** 幻灯内表格。 */
export interface SlideTable {
  /** 各列宽(像素)。 */
  col_widths: number[];
  /** 行 → 各单元格纯文本。 */
  rows: string[][];
}

export interface Slide {
  shapes: Shape[];
  /** 是否含动画(p:timing)。 */
  has_animation?: boolean;
  /** 是否含切换效果(p:transition)。 */
  has_transition?: boolean;
}

export interface Presentation {
  width_px: number;
  height_px: number;
  slides: Slide[];
}

/** 图片键:`${slideIndex}|${embedId}` → object URL。 */
export type SlideImageUrls = Map<string, string>;

export interface PptDocument {
  presentation: Presentation;
  images: SlideImageUrls;
  dispose(): void;
}

/** 构造图片键。 */
export function imageKey(slideIndex: number, embed: string): string {
  return `${slideIndex}|${embed}`;
}
