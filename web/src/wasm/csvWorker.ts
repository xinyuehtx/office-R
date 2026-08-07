/// <reference lib="webworker" />
/**
 * CSV 解析 Worker。
 *
 * 解析 200MB 的 CSV 要几百毫秒到数秒,放在主线程上会**整页冻住** ——
 * 连「正在解析…」的转圈都不会动。所以解析全程在这个 Worker 里跑,
 * 结果以可转移的 `ArrayBuffer` 交回主线程,主线程只负责装配与绘制。
 */

import init, { parseCsvPacked, setLogLevel } from "./pkg/office_wasm.js";
import type { LogLevel } from "../apps/shared/logger";
import type { CellFormula, SheetMeta } from "../apps/shared/sheet";

/** 主线程 → Worker 的请求。 */
export interface CsvWorkerRequest {
  /** 文件字节。会被转移进 Worker,主线程不再持有。 */
  bytes: ArrayBuffer;
  /** 日志 traceId,用于把两侧日志串起来。 */
  traceId: string;
  /** 分隔符字符码;0 表示自动嗅探。 */
  delimiter: number;
  /** 日志级别,与主线程保持一致。 */
  logLevel: LogLevel;
  /** 当前时刻的 Excel 序列数,注入给公式 TODAY/NOW。 */
  nowSerial: number;
}

/** Worker → 主线程的响应。 */
export type CsvWorkerResponse =
  | {
      ok: true;
      text: ArrayBuffer;
      cellEnds: ArrayBuffer;
      rowStarts: ArrayBuffer;
      colWidthUnits: ArrayBuffer;
      cols: number;
      meta: SheetMeta;
      /** 公式单元格清单(结构化克隆,数据量小)。 */
      formulas: CellFormula[];
    }
  | { ok: false; message: string };

let ready: Promise<unknown> | null = null;

function ensureReady(): Promise<unknown> {
  if (!ready) ready = init();
  return ready;
}

self.onmessage = async (event: MessageEvent<CsvWorkerRequest>) => {
  const { bytes, traceId, delimiter, logLevel, nowSerial } = event.data;
  try {
    await ensureReady();
    setLogLevel(logLevel);

    const packed = parseCsvPacked(new Uint8Array(bytes), traceId, delimiter, nowSerial);
    try {
      const meta = packed.meta as SheetMeta;
      const formulas = packed.formulas as CellFormula[];
      const text = packed.takeText();
      const cellEnds = packed.takeCellEnds();
      const rowStarts = packed.takeRowStarts();
      const colWidthUnits = packed.takeColWidthUnits();
      const response: CsvWorkerResponse = {
        ok: true,
        cols: packed.cols,
        meta,
        formulas,
        text: text.buffer as ArrayBuffer,
        cellEnds: cellEnds.buffer as ArrayBuffer,
        rowStarts: rowStarts.buffer as ArrayBuffer,
        colWidthUnits: colWidthUnits.buffer as ArrayBuffer,
      };
      // 第二个参数是转移列表:这些 ArrayBuffer 直接过户,不产生拷贝
      self.postMessage(response, [
        response.text,
        response.cellEnds,
        response.rowStarts,
        response.colWidthUnits,
      ]);
    } finally {
      packed.free();
    }
  } catch (error) {
    const response: CsvWorkerResponse = {
      ok: false,
      message: error instanceof Error ? error.message : String(error),
    };
    self.postMessage(response);
  }
};
