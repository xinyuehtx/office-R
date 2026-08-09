/**
 * 表格句柄:CSV 的紧凑缓冲解析 + `WasmSheet` → `SheetHandle` 的装配。
 *
 * CSV 与 xlsx 共用这一层 —— 两者的差异只在如何得到 `WasmSheet`,
 * 取窗口 / 过滤 / 排序 / 查找 / 公式回显的行为完全一致。
 */
import { parseCsvPacked, WasmSheet } from "./pkg/office_wasm.js";
import { ensureReady } from "./init";
import type {
  CellFormula,
  CellWindowData,
  FilterSpec,
  SheetHandle,
  SheetMeta,
  UniqueValues,
} from "../apps/shared/sheet";

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
export function buildSheetHandle(inner: WasmSheet, formulas: CellFormula[]): SheetHandle {
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
