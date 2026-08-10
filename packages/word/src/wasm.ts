/** Word wasm 后端的加载与封装。 */
import init, { WasmWordDoc, setLogLevel } from "../pkg/office_word_wasm.js";
import { getLogLevel, onLogLevelChange } from "@tengxiaohyx/office-shared";
import type { WordDocument, WordModel } from "./model";

let ready: Promise<unknown> | null = null;

/** 确保本包的 wasm 已初始化(幂等);每个应用包各持一份 wasm。 */
async function ensureReady(): Promise<void> {
  if (!ready) {
    ready = init().then(() => {
      setLogLevel(getLogLevel());
      onLogLevelChange((level) => setLogLevel(level));
    });
  }
  await ready;
}

function revokeAll(urls: Iterable<string>): void {
  for (const url of urls) URL.revokeObjectURL(url);
}

/**
 * 解析 docx 字节为 Word 文档模型 + 图片 URL。
 *
 * 图片字节留在 WASM 内存,按 id 取出后用 Blob 造 object URL;`dispose()` 释放。
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
        const buf = handle.imageBytes(i).slice().buffer;
        images.set(id, URL.createObjectURL(new Blob([buf], { type: mime })));
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
