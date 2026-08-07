// office-wasm 的加载与类型封装。
//
// wasm-pack 生成的产物位于 ./pkg(构建产物,不入库)。首次调用前需 await ensureReady()。
import init, {
  version as wasmVersion,
  detect as wasmDetect,
  render as wasmRender,
  setLogLevel as wasmSetLogLevel,
  parseCsvPacked,
  WasmSheet,
} from "./pkg/office_wasm.js";
import { getLogLevel, onLogLevelChange } from "../apps/shared/logger";
import type { CellWindowData, SheetHandle, SheetMeta } from "../apps/shared/sheet";

/** 识别出的格式,与 Rust 端 `Format` 对应。 */
export type OfficeFormat = "docx" | "xlsx" | "pptx" | "csv" | "unknown";

/** 渲染结果,与 Rust 端 `RenderResult` 对应。 */
export interface RenderResult {
  format: OfficeFormat;
  format_name: string;
  byte_len: number;
  message: string;
  /** 是否解析成功;false 时 message 为失败原因。 */
  ok: boolean;
}

/**
 * 表格的紧凑传输表示。
 *
 * 各字段都是连续缓冲,可作为 `ArrayBuffer` 在 Worker 与主线程之间
 * **转移**(而非拷贝),这是大文件不卡的关键之一。
 */
export interface PackedSheetTransfer {
  text: Uint8Array;
  cellEnds: Uint32Array;
  rowStarts: Uint32Array;
  colWidthUnits: Uint32Array;
  cols: number;
  meta: SheetMeta;
}

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

/** 读取 office 文件并产出摘要(docx / xlsx / pptx 页面使用)。 */
export async function render(bytes: Uint8Array): Promise<RenderResult> {
  await ensureReady();
  return wasmRender(bytes) as RenderResult;
}

/**
 * 解析 CSV,产出可跨线程转移的紧凑缓冲。
 *
 * @param delimiter 分隔符字符码;0 表示自动嗅探
 */
export async function parseCsv(
  bytes: Uint8Array,
  traceId: string,
  delimiter = 0,
): Promise<PackedSheetTransfer> {
  await ensureReady();
  const packed = parseCsvPacked(bytes, traceId, delimiter);
  try {
    // 先取元信息,再把缓冲「移出」—— take* 之后缓冲就空了
    const meta = packed.meta as SheetMeta;
    return {
      meta,
      cols: packed.cols,
      text: packed.takeText(),
      cellEnds: packed.takeCellEnds(),
      rowStarts: packed.takeRowStarts(),
      colWidthUnits: packed.takeColWidthUnits(),
    };
  } finally {
    packed.free();
  }
}

/**
 * 把紧凑缓冲装配成主线程可同步取数的表格句柄。
 *
 * 表格内容随后一直留在 WASM 线性内存里:每帧取可见区域是一次同步调用,
 * 不需要等 Promise,因此不会掉帧。
 */
export async function sheetFromPacked(packed: PackedSheetTransfer): Promise<SheetHandle> {
  await ensureReady();
  const inner = WasmSheet.fromPacked(
    packed.text,
    packed.cellEnds,
    packed.rowStarts,
    packed.cols,
    packed.colWidthUnits,
  );

  return {
    rows: inner.rows,
    cols: inner.cols,
    colWidthUnits: inner.colWidthUnits(),
    window(row0: number, row1: number, col0: number, col1: number): CellWindowData {
      const window = inner.window(row0, row1, col0, col1);
      try {
        // 顺序要紧:rows/cols 是普通 getter,takeText 之后文本就被移走了
        const rows = window.rows;
        const cols = window.cols;
        return { rows, cols, text: window.takeText(), ends: window.takeEnds() };
      } finally {
        window.free();
      }
    },
    dispose() {
      inner.free();
    },
  };
}
