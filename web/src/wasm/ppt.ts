/** pptx → 演示模型 + 图片 URL。 */
import { WasmPresentation } from "./pkg/office_wasm.js";
import { ensureReady, revokeAll } from "./init";
import { imageKey, type PptDocument, type Presentation } from "../apps/ppt/model";

/**
 * 解析 pptx 字节为演示模型 + 图片 URL(按 幻灯序号|embed 键)。
 */
export async function loadPptx(bytes: Uint8Array): Promise<PptDocument> {
  await ensureReady();
  const handle = WasmPresentation.parse(bytes);
  try {
    const presentation = handle.model as Presentation;
    const images = new Map<string, string>();
    try {
      const count = handle.imageCount;
      for (let i = 0; i < count; i += 1) {
        const embed = handle.imageEmbed(i);
        if (!embed) continue;
        const slide = handle.imageSlide(i);
        const mime = handle.imageMime(i) ?? "application/octet-stream";
        const buf = handle.imageBytes(i).slice().buffer;
        images.set(imageKey(slide, embed), URL.createObjectURL(new Blob([buf], { type: mime })));
      }
    } catch (e) {
      revokeAll(images.values());
      throw e;
    }
    return {
      presentation,
      images,
      dispose() {
        revokeAll(images.values());
      },
    };
  } finally {
    handle.free();
  }
}
