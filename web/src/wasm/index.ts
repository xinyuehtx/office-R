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
  WasmWorkbook,
  WasmWordDoc,
  WasmPresentation,
} from "./pkg/office_wasm.js";
import { getLogLevel, onLogLevelChange } from "../apps/shared/logger";
import { imageKey } from "../apps/ppt/model";
import type {
  CellFormula,
  CellWindowData,
  FilterSpec,
  SheetHandle,
  SheetMeta,
  UniqueValues,
} from "../apps/shared/sheet";

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
  /** 公式单元格清单(可跨线程结构化克隆的小数组)。无公式时为空。 */
  formulas: CellFormula[];
}

/**
 * 当前时刻的 Excel 序列数,注入给公式 `TODAY`/`NOW`。
 *
 * Excel 用「1899-12-30 以来的天数 + 当天时间比例」表示时间,1970-01-01 = 25569。
 * 这里换算到**本地时区**,让 `TODAY()` 与用户日历一致。
 */
export function nowSerial(): number {
  const now = new Date();
  const localMs = now.getTime() - now.getTimezoneOffset() * 60000;
  return localMs / 86_400_000 + 25569;
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
  const packed = parseCsvPacked(bytes, traceId, delimiter, nowSerial());
  try {
    // 先取元信息与公式,再把缓冲「移出」—— take* 之后缓冲就空了
    const meta = packed.meta as SheetMeta;
    const formulas = packed.formulas as CellFormula[];
    return {
      meta,
      formulas,
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
  return buildSheetHandle(inner, packed.formulas);
}

/**
 * 把一个 `WasmSheet` + 公式清单封装成渲染管线依赖的 `SheetHandle`。
 *
 * CSV(`sheetFromPacked`)与 xlsx(`loadXlsx`)共用:两者的差异只在如何得到
 * `WasmSheet`,取窗口 / 过滤 / 公式回显的行为完全一致。
 */
function buildSheetHandle(inner: WasmSheet, formulas: CellFormula[]): SheetHandle {
  // 公式清单转成 Map,按 "row,col" 键 O(1) 查询,供公式栏回显
  const formulaMap = new Map<string, string>();
  for (const f of formulas) {
    formulaMap.set(`${f.row},${f.col}`, f.formula);
  }

  // 注意:rows/cols 用 getter 实时读取 —— 过滤会改变可视行数,
  // 渲染器 refreshRows() 时要拿到最新值。
  return {
    get rows() {
      return inner.rows;
    },
    get cols() {
      return inner.cols;
    },
    colWidthUnits: inner.colWidthUnits(),
    formulaCount: formulaMap.size,
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
    formula(row: number, col: number): string | null {
      return formulaMap.get(`${row},${col}`) ?? null;
    },
    rowLabel(visualRow: number): number {
      return inner.rowLabel(visualRow);
    },
    filter(specs: FilterSpec[], headerRows: number): number {
      return inner.filter(specs, headerRows);
    },
    clearFilter(): void {
      inner.clearFilter();
    },
    sort(col: number, dir: "asc" | "desc" | "none", headerRows: number): number {
      return inner.sort(col, dir, headerRows);
    },
    find(needle: string, caseSensitive: boolean, wholeCell: boolean, limit: number) {
      return inner.find(needle, caseSensitive, wholeCell, limit) as {
        row: number;
        col: number;
      }[];
    },
    uniqueValues(col: number, headerRows: number, limit: number): UniqueValues {
      return inner.uniqueValues(col, headerRows, limit) as UniqueValues;
    },
    dispose() {
      inner.free();
    },
  };
}

/** 一个已打开的 xlsx 工作簿:工作表名 + 按需取某表的句柄。 */
export interface XlsxWorkbookHandle {
  /** 各工作表名(按原始顺序)。 */
  sheetNames: string[];
  /** 取第 `i` 张工作表为可绘制句柄;调用方用完 `dispose()`。 */
  openSheet(index: number): SheetHandle;
  /** 释放工作簿(其下已取出的表句柄需各自 dispose)。 */
  dispose(): void;
}

/**
 * 解析 xlsx 字节为工作簿句柄(多工作表)。
 *
 * xlsx 自带缓存计算值,内核不重算;每张表按需取出为 `SheetHandle`,与 CSV 走同一渲染管线。
 */
export async function loadXlsx(bytes: Uint8Array): Promise<XlsxWorkbookHandle> {
  await ensureReady();
  const wb = WasmWorkbook.parse(bytes);
  const sheetNames = wb.sheetNames() as string[];

  // 媒体(图片)→ object URL,按 media key 索引;整簿共用,dispose 时统一 revoke
  const mediaUrls = new Map<string, string>();
  const mediaCount = wb.mediaCount();
  for (let i = 0; i < mediaCount; i += 1) {
    const key = wb.mediaKey(i);
    if (!key) continue;
    const mime = wb.mediaMime(i) ?? "application/octet-stream";
    const buf = wb.mediaBytes(i).slice().buffer;
    mediaUrls.set(key, URL.createObjectURL(new Blob([buf], { type: mime })));
  }

  return {
    sheetNames,
    openSheet(index: number): SheetHandle {
      const inner = wb.sheet(index);
      const formulas = wb.formulas(index) as CellFormula[];
      const handle = buildSheetHandle(inner, formulas);
      // xlsx 视觉样式 + 合并区
      const styleList = wb.styles(index) as Array<
        { row: number; col: number } & import("../apps/shared/sheet").CellStyle
      >;
      const styleMap = new Map<string, import("../apps/shared/sheet").CellStyle>();
      for (const s of styleList) {
        styleMap.set(`${s.row},${s.col}`, {
          bold: s.bold,
          italic: s.italic,
          color: s.color,
          fill: s.fill,
          align: s.align,
          border: s.border,
        });
      }
      handle.cellStyle = (r, c) => styleMap.get(`${r},${c}`) ?? null;
      handle.merges = wb.merges(index) as [number, number, number, number][];
      // 内嵌图片:锚点 + object URL
      const anchors = wb.images(index) as Array<{
        mediaKey: string;
        fromRow: number;
        fromCol: number;
        toRow?: number;
        toCol?: number;
        extW?: number;
        extH?: number;
      }>;
      handle.images = anchors.flatMap((a) => {
        const url = mediaUrls.get(a.mediaKey);
        if (!url) return [];
        const img: import("../apps/shared/sheet").SheetImage = {
          fromRow: a.fromRow,
          fromCol: a.fromCol,
          url,
        };
        if (a.toRow !== undefined) img.toRow = a.toRow;
        if (a.toCol !== undefined) img.toCol = a.toCol;
        if (a.extW !== undefined) img.extW = a.extW;
        if (a.extH !== undefined) img.extH = a.extH;
        return [img];
      });
      // 内嵌图表
      handle.charts = wb.charts(index) as import("../apps/shared/sheet").SheetChart[];
      // 列宽(Excel 字符宽度 → CSS px:约 7px/字符 + 5px 边距)+ 冻结窗格
      const cw = wb.colWidths(index) as [number, number][];
      handle.colWidthsPx = cw.map(([c, w]) => [c, Math.round(w * 7 + 5)]);
      handle.freeze = wb.freeze(index) as [number, number];
      handle.sparklines = wb.sparklines(index) as import("../apps/shared/sheet").SheetSparkline[];
      return handle;
    },
    dispose() {
      for (const url of mediaUrls.values()) URL.revokeObjectURL(url);
      wb.free();
    },
  };
}

/**
 * 解析 docx 字节为 Word 文档模型 + 图片 URL。
 *
 * 图片字节留在 WASM 内存,这里按 id 取出后用 Blob 造 object URL(不走 base64),
 * canvas `drawImage` 可直接用;调用方用完 `dispose()` 释放 URL。
 */
export async function loadDocx(bytes: Uint8Array): Promise<import("../apps/word/model").WordDocument> {
  await ensureReady();
  const handle = WasmWordDoc.parse(bytes);
  try {
    const model = handle.model as import("../apps/word/model").WordModel;
    const images = new Map<string, string>();
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
    return {
      model,
      images,
      dispose() {
        for (const url of images.values()) URL.revokeObjectURL(url);
      },
    };
  } finally {
    handle.free();
  }
}

/**
 * 解析 pptx 字节为演示模型 + 图片 URL(按 幻灯序号|embed 键)。
 */
export async function loadPptx(bytes: Uint8Array): Promise<import("../apps/ppt/model").PptDocument> {
  await ensureReady();
  const handle = WasmPresentation.parse(bytes);
  try {
    const presentation = handle.model as import("../apps/ppt/model").Presentation;
    const images = new Map<string, string>();
    const count = handle.imageCount;
    for (let i = 0; i < count; i += 1) {
      const embed = handle.imageEmbed(i);
      if (!embed) continue;
      const slide = handle.imageSlide(i);
      const mime = handle.imageMime(i) ?? "application/octet-stream";
      const buf = handle.imageBytes(i).slice().buffer;
      images.set(imageKey(slide, embed), URL.createObjectURL(new Blob([buf], { type: mime })));
    }
    return {
      presentation,
      images,
      dispose() {
        for (const url of images.values()) URL.revokeObjectURL(url);
      },
    };
  } finally {
    handle.free();
  }
}
