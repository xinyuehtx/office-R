/**
 * WASM 模块的初始化与共享工具。
 *
 * 这一层刻意只放「与具体格式无关」的东西 —— 三个应用的加载器(`sheet` / `xlsx` /
 * `word` / `ppt`)各自依赖它,但彼此不相往来。这个边界是为了让三个应用将来能拆成
 * 独立的 npm 包:届时 `initialized` 这个模块级单例会按包各有一份(每个包一份 wasm)。
 */
import init, { setLogLevel as wasmSetLogLevel } from "./pkg/office_wasm.js";
import { getLogLevel, onLogLevelChange } from "../apps/shared/logger";

let initialized: Promise<unknown> | null = null;

/** 确保 WASM 模块已初始化(幂等)。 */
export async function ensureReady(): Promise<void> {
  if (!initialized) {
    initialized = init().then(() => {
      // 让 WASM 侧与前端用同一个日志级别,两边输出才能串起来看
      wasmSetLogLevel(getLogLevel());
      onLogLevelChange((level) => wasmSetLogLevel(level));
    });
  }
  await initialized;
}

/**
 * 释放一批 object URL。
 *
 * 半途失败的加载函数必须调它:局部 `images` Map 随异常一起被丢弃后,
 * 调用方拿不到句柄、也就永远调不到 `dispose()`,已建的 URL 会挂到页面关闭。
 */
export function revokeAll(urls: Iterable<string>): void {
  for (const url of urls) URL.revokeObjectURL(url);
}
