/**
 * 表格数据源的测试替身。
 *
 * 用普通二维数组实现 `SheetHandle`,让渲染管线的测试不必依赖 WASM ——
 * 这也反过来验证了「视图层只依赖接口、不依赖具体实现」这条分层约定。
 */

import type { CellWindowData, SheetHandle, SheetMeta } from "../sheet";

/** 与 Rust 侧 `display_width` 对齐的宽度估算(东亚宽字符算 2)。 */
function displayWidth(text: string): number {
  let width = 0;
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    const wide =
      (code >= 0x1100 && code <= 0x115f) ||
      (code >= 0x2e80 && code <= 0x303e) ||
      (code >= 0x3041 && code <= 0x33ff) ||
      (code >= 0x3400 && code <= 0x4dbf) ||
      (code >= 0x4e00 && code <= 0x9fff) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0xf900 && code <= 0xfaff) ||
      (code >= 0xff00 && code <= 0xff60);
    width += wide ? 2 : 1;
  }
  return width;
}

/** 由二维数组构造一个 `SheetHandle`。可选 `formulas` 提供公式格回显。 */
export function createFixtureSheet(
  rows: string[][],
  formulas: Record<string, string> = {},
): SheetHandle & { disposed: boolean } {
  const rowCount = rows.length;
  const colCount = rows.reduce((max, row) => Math.max(max, row.length), 0);

  const colWidthUnits = new Uint32Array(colCount);
  for (let c = 0; c < colCount; c += 1) {
    let widest = 3;
    for (const row of rows) {
      widest = Math.max(widest, Math.min(60, displayWidth(row[c] ?? "")));
    }
    colWidthUnits[c] = widest;
  }

  const handle = {
    rows: rowCount,
    cols: colCount,
    colWidthUnits,
    disposed: false,
    formulaCount: Object.keys(formulas).length,
    formula(row: number, col: number): string | null {
      return formulas[`${row},${col}`] ?? null;
    },
    window(row0: number, row1: number, col0: number, col1: number): CellWindowData {
      const r0 = Math.min(Math.max(0, row0), rowCount);
      const r1 = Math.min(Math.max(r0, row1), rowCount);
      const c0 = Math.min(Math.max(0, col0), colCount);
      const c1 = Math.min(Math.max(c0, col1), colCount);

      let text = "";
      const ends = new Uint32Array((r1 - r0) * (c1 - c0));
      let i = 0;
      for (let r = r0; r < r1; r += 1) {
        for (let c = c0; c < c1; c += 1) {
          text += rows[r]?.[c] ?? "";
          ends[i] = text.length;
          i += 1;
        }
      }
      return { text, ends, rows: r1 - r0, cols: c1 - c0 };
    },
    dispose() {
      handle.disposed = true;
    },
  };
  return handle;
}

/** 生成 `rows × cols` 的示例数据,单元格内容形如 `r12c3`。 */
export function makeGrid(rows: number, cols: number): string[][] {
  return Array.from({ length: rows }, (_, r) =>
    Array.from({ length: cols }, (_, c) => `r${r}c${c}`),
  );
}

/** 一份可用于断言的元信息。 */
export function fixtureMeta(overrides: Partial<SheetMeta> = {}): SheetMeta {
  return {
    encoding: "UTF-8",
    delimiter: ",",
    delimiterSource: "sniffed",
    lossy: false,
    rows: 0,
    cols: 0,
    truncatedRows: false,
    truncatedCols: false,
    parseMs: 1.5,
    ...overrides,
  };
}
