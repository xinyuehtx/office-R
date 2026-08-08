/**
 * 单张幻灯的 canvas 绘制(纯函数,便于复用:缩略图、主视图、演示模式共用)。
 *
 * 幻灯坐标是「幻灯像素」(EMU÷9525)。绘制时按 `scale` 缩放到目标 canvas,
 * 因此同一份幻灯可在不同尺寸(缩略图/全屏)下渲染。
 */

import type { Slide, Shape, Align } from "./model";
import { FONT_FAMILY } from "../excel/grid/theme";
import { sharedMeasurer } from "../shared/textMeasure";

const DEFAULT_TEXT = "#1f2328";

function colorOf(hex: string | null, fallback: string): string {
  if (hex && /^[0-9a-fA-F]{6}$/.test(hex)) return `#${hex}`;
  return fallback;
}

/** 预设几何 → 绘制路径(覆盖常见几种,其余按矩形)。 */
function fillShapeGeom(
  ctx: CanvasRenderingContext2D,
  geom: string,
  x: number,
  y: number,
  w: number,
  h: number,
) {
  ctx.beginPath();
  switch (geom) {
    case "ellipse":
      ctx.ellipse(x + w / 2, y + h / 2, w / 2, h / 2, 0, 0, Math.PI * 2);
      break;
    case "roundRect": {
      const r = Math.min(w, h) * 0.12;
      ctx.moveTo(x + r, y);
      ctx.arcTo(x + w, y, x + w, y + h, r);
      ctx.arcTo(x + w, y + h, x, y + h, r);
      ctx.arcTo(x, y + h, x, y, r);
      ctx.arcTo(x, y, x + w, y, r);
      ctx.closePath();
      break;
    }
    case "line":
      ctx.moveTo(x, y);
      ctx.lineTo(x + w, y + h);
      break;
    case "triangle":
      ctx.moveTo(x + w / 2, y);
      ctx.lineTo(x + w, y + h);
      ctx.lineTo(x, y + h);
      ctx.closePath();
      break;
    default:
      ctx.rect(x, y, w, h);
  }
}

function alignX(align: Align, left: number, width: number, textWidth: number): number {
  if (align === "center") return left + (width - textWidth) / 2;
  if (align === "right") return left + (width - textWidth);
  return left;
}

/** 在一个形状框内绘制文本(逐段落、逐行折行 + 对齐)。 */
function drawShapeText(
  ctx: CanvasRenderingContext2D,
  shape: Shape,
  scale: number,
) {
  const padding = 6 * scale;
  const left = shape.x * scale + padding;
  const top = shape.y * scale + padding;
  const boxW = shape.width * scale - padding * 2;
  if (boxW <= 0) return;

  let cursorY = top;
  for (const para of shape.paragraphs) {
    // 段落基础字号(取首个 run 或默认 18pt)
    for (const run of para.runs.length ? para.runs : [{ text: "", bold: false, italic: false, size_pt: null, color: null }]) {
      const px = (run.size_pt ?? 18) * (96 / 72) * scale;
      const weight = run.bold ? "bold " : "";
      const italic = run.italic ? "italic " : "";
      const font = `${italic}${weight}${px}px ${FONT_FAMILY}`;
      const lines = sharedMeasurer.wrap(run.text, boxW, font);
      ctx.font = font;
      ctx.fillStyle = colorOf(run.color, DEFAULT_TEXT);
      ctx.textBaseline = "top";
      for (const line of lines) {
        const tw = sharedMeasurer.measure(line, font);
        const lx = alignX(para.align, left, boxW, tw);
        ctx.fillText(line, lx, cursorY);
        cursorY += px * 1.3;
      }
    }
  }
}

/** 内嵌对象占位类型 → 中文标签。 */
function kindLabel(kind: string): string {
  switch (kind) {
    case "chart":
      return "图表";
    case "diagram":
      return "SmartArt";
    case "table":
      return "表格";
    default:
      return kind;
  }
}

/** 图表/SmartArt/表格:本期只画虚线占位框 + 居中类型标签。 */
function drawPlaceholderKind(
  ctx: CanvasRenderingContext2D,
  kind: string,
  x: number,
  y: number,
  w: number,
  h: number,
  scale: number,
) {
  ctx.save();
  ctx.strokeStyle = "#8c959f";
  ctx.setLineDash([6, 4]);
  ctx.lineWidth = 1;
  ctx.strokeRect(x + 0.5, y + 0.5, w, h);
  ctx.setLineDash([]);
  const px = Math.max(11, 14 * scale);
  ctx.font = `${px}px ${FONT_FAMILY}`;
  ctx.fillStyle = "#57606a";
  ctx.textBaseline = "middle";
  const label = kindLabel(kind);
  const tw = ctx.measureText(label).width;
  ctx.fillText(label, x + (w - tw) / 2, y + h / 2);
  ctx.restore();
}

/** 绘制一张幻灯到 ctx,已按 `scale` 缩放;`images` 按 embed id 提供已解码图片。 */
export function drawSlide(
  ctx: CanvasRenderingContext2D,
  slide: Slide,
  scale: number,
  images: Map<string, HTMLImageElement>,
) {
  for (const shape of slide.shapes) {
    const x = shape.x * scale;
    const y = shape.y * scale;
    const w = shape.width * scale;
    const h = shape.height * scale;

    // 旋转/翻转:绕形状中心做仿射变换
    const rot = shape.rotation ?? 0;
    const needXform = rot !== 0 || shape.flip_h || shape.flip_v;
    if (needXform) {
      ctx.save();
      const cx = x + w / 2;
      const cy = y + h / 2;
      ctx.translate(cx, cy);
      if (rot) ctx.rotate((rot * Math.PI) / 180);
      ctx.scale(shape.flip_h ? -1 : 1, shape.flip_v ? -1 : 1);
      ctx.translate(-cx, -cy);
    }

    // 内嵌对象(图表/SmartArt/表格)占位
    if (shape.placeholder_kind) {
      drawPlaceholderKind(ctx, shape.placeholder_kind, x, y, w, h, scale);
      if (needXform) ctx.restore();
      continue;
    }

    // 图片
    if (shape.image) {
      const img = images.get(shape.image);
      if (img && img.complete && img.naturalWidth > 0) {
        ctx.drawImage(img, x, y, w, h);
      } else {
        ctx.strokeStyle = "#d0d7de";
        ctx.strokeRect(x, y, w, h);
      }
      if (needXform) ctx.restore();
      continue;
    }

    // 自选图形:填充 + 描边
    if (shape.geom && (w > 0 || h > 0)) {
      if (shape.fill) {
        ctx.fillStyle = colorOf(shape.fill, "#ffffff");
        fillShapeGeom(ctx, shape.geom, x, y, w, h);
        if (shape.geom === "line") {
          ctx.strokeStyle = colorOf(shape.fill, "#57606a");
          ctx.stroke();
        } else {
          ctx.fill();
        }
      } else if (shape.paragraphs.length === 0) {
        // 无填充无文本的纯图形:描边提示
        ctx.strokeStyle = "#8c959f";
        fillShapeGeom(ctx, shape.geom, x, y, w, h);
        ctx.stroke();
      }
    }

    // 文本
    if (shape.paragraphs.length > 0) {
      drawShapeText(ctx, shape, scale);
    }

    if (needXform) ctx.restore();
  }
}

/** 计算把幻灯等比放进目标区域的缩放系数与居中偏移。 */
export function fitScale(
  slideW: number,
  slideH: number,
  targetW: number,
  targetH: number,
): { scale: number; offsetX: number; offsetY: number } {
  if (slideW <= 0 || slideH <= 0) return { scale: 1, offsetX: 0, offsetY: 0 };
  const scale = Math.min(targetW / slideW, targetH / slideH);
  const offsetX = (targetW - slideW * scale) / 2;
  const offsetY = (targetH - slideH * scale) / 2;
  return { scale, offsetX, offsetY };
}
