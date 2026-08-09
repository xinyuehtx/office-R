/** docx → Word 文档模型 + 图片 URL。 */
import { WasmWordDoc } from "./pkg/office_wasm.js";
import { ensureReady, revokeAll } from "./init";
import type { WordDocument, WordModel } from "../apps/word/model";

/**
 * 解析 docx 字节为 Word 文档模型 + 图片 URL。
 *
 * 图片字节留在 WASM 内存,这里按 id 取出后用 Blob 造 object URL(不走 base64),
 * canvas `drawImage` 可直接用;调用方用完 `dispose()` 释放 URL。
 */
export async function loadDocx(bytes: Uint8Array): Promise<WordDocument> {
  await ensureReady();
  const handle = WasmWordDoc.parse(bytes);
  try {
    const model = handle.model as WordModel;
    const images = new Map<string, string>();
    try {
      const count = handle.imageCount;
      for (let i = 0; i < count; i += 1) {
        const id = handle.imageId(i);
        if (!id) continue;
        const mime = handle.imageMime(i) ?? "application/octet-stream";
        const data = handle.imageBytes(i);
        // 复制成独立 ArrayBuffer,规避 wasm 内存 buffer 的类型不匹配
        const buf = data.slice().buffer;
        const url = URL.createObjectURL(new Blob([buf], { type: mime }));
        images.set(id, url);
      }
    } catch (e) {
      revokeAll(images.values());
      throw e;
    }
    return {
      model,
      images,
      dispose() {
        revokeAll(images.values());
      },
    };
  } finally {
    handle.free();
  }
}
