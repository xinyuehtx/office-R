/** xlsx → 多工作表工作簿句柄(样式 / 合并区 / 图片 / 图表 / 列宽 / 冻结 / 迷你图)。 */
import { WasmWorkbook } from "./pkg/office_wasm.js";
import { ensureReady, revokeAll } from "./init";
import { buildSheetHandle } from "./sheet";
import type {
  CellFormula,
  CellStyle,
  SheetChart,
  SheetHandle,
  SheetImage,
  SheetSparkline,
} from "../apps/shared/sheet";

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

  // 媒体(图片)→ object URL,按 media key 索引;整簿共用,dispose 时统一 revoke
  const mediaUrls = new Map<string, string>();
  let sheetNames: string[];
  try {
    sheetNames = wb.sheetNames() as string[];
    const mediaCount = wb.mediaCount();
    for (let i = 0; i < mediaCount; i += 1) {
      const key = wb.mediaKey(i);
      if (!key) continue;
      const mime = wb.mediaMime(i) ?? "application/octet-stream";
      const buf = wb.mediaBytes(i).slice().buffer;
      mediaUrls.set(key, URL.createObjectURL(new Blob([buf], { type: mime })));
    }
  } catch (e) {
    // 构造中途失败:调用方拿不到句柄,也就永远调不到 dispose ——
    // 必须在这里把 WASM 侧工作簿与已建的 object URL 一并释放,否则是永久泄漏。
    revokeAll(mediaUrls.values());
    wb.free();
    throw e;
  }

  return {
    sheetNames,
    openSheet(index: number): SheetHandle {
      const inner = wb.sheet(index);
      const formulas = wb.formulas(index) as CellFormula[];
      const handle = buildSheetHandle(inner, formulas);
      // xlsx 视觉样式 + 合并区
      const styleList = wb.styles(index) as Array<
        { row: number; col: number } & CellStyle
      >;
      const styleMap = new Map<string, CellStyle>();
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
        const img: SheetImage = {
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
      handle.charts = wb.charts(index) as SheetChart[];
      // 列宽(Excel 字符宽度 → CSS px:约 7px/字符 + 5px 边距)+ 冻结窗格
      const cw = wb.colWidths(index) as [number, number][];
      handle.colWidthsPx = cw.map(([c, w]) => [c, Math.round(w * 7 + 5)]);
      handle.freeze = wb.freeze(index) as [number, number];
      handle.sparklines = wb.sparklines(index) as SheetSparkline[];
      return handle;
    },
    dispose() {
      revokeAll(mediaUrls.values());
      wb.free();
    },
  };
}
