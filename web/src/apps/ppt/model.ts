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
}

export interface Slide {
  shapes: Shape[];
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
