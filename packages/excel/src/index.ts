//! `@tengxiaohyx/office-excel`:Excel (.xlsx) 与 CSV 表格查看。
//
// 数据层:表格句柄契约(SheetHandle 等)、xlsx / CSV 的 wasm 加载器与 Worker 客户端。
// 视图层(SheetCanvas / grid / 页面)在 Phase 7 迁入。
export * from "./sheet";
export { loadXlsx, type XlsxWorkbookHandle } from "./wasm/xlsx";
export { nowSerial, parseCsv, sheetFromPacked, type PackedSheetTransfer } from "./wasm/sheet";
export { parseCsvFile, createWorker, type CsvParseOutcome } from "./wasm/csvClient";
