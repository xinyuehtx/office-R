// office-wasm 的加载与类型封装。
//
// wasm-pack 生成的产物位于 ./pkg(构建产物,不入库)。首次调用前需 await init()。
import init, {
  version as wasmVersion,
  detect as wasmDetect,
  render as wasmRender,
} from "./pkg/office_wasm.js";

/** 识别出的格式,与 Rust 端 `Format` 对应。 */
export type OfficeFormat = "docx" | "xlsx" | "pptx" | "unknown";

/** 渲染结果,与 Rust 端 `RenderResult` 对应。 */
export interface RenderResult {
  format: OfficeFormat;
  format_name: string;
  byte_len: number;
  message: string;
  /** 是否解析成功;false 时 message 为失败原因。 */
  ok: boolean;
}

let initialized: Promise<unknown> | null = null;

/** 确保 WASM 模块已初始化(幂等)。 */
export async function ensureReady(): Promise<void> {
  if (!initialized) {
    initialized = init();
  }
  await initialized;
}

/** 计算内核版本。 */
export async function version(): Promise<string> {
  await ensureReady();
  return wasmVersion();
}

/** 识别文件格式。 */
export async function detect(bytes: Uint8Array): Promise<OfficeFormat> {
  await ensureReady();
  return wasmDetect(bytes) as OfficeFormat;
}

/** 读取并(占位)渲染 office 文件。 */
export async function render(bytes: Uint8Array): Promise<RenderResult> {
  await ensureReady();
  return wasmRender(bytes) as RenderResult;
}
