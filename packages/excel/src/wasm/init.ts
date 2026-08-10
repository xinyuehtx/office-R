/** Excel/CSV wasm 后端的初始化与共享工具。 */
import init, { setLogLevel } from "../../pkg/office_excel_wasm.js";
import { getLogLevel, onLogLevelChange } from "@tengxiaohyx/office-shared";

let ready: Promise<unknown> | null = null;

/** 确保本包的 wasm 已初始化(幂等)。 */
export async function ensureReady(): Promise<void> {
  if (!ready) {
    ready = init().then(() => {
      setLogLevel(getLogLevel());
      onLogLevelChange((level) => setLogLevel(level));
    });
  }
  await ready;
}

/** 释放一批 object URL(半途失败的加载器也要回收)。 */
export function revokeAll(urls: Iterable<string>): void {
  for (const url of urls) URL.revokeObjectURL(url);
}
