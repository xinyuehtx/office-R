/**
 * CSV 解析的调用方封装。
 *
 * 对外只暴露一个 `parseCsvFile`,内部决定走 Worker 还是主线程:
 * - **优先 Worker**:解析不阻塞 UI,大文件也能保持界面响应;
 * - **退回主线程**:环境不支持 Worker(如单测的 jsdom、极老的浏览器)时,
 *   功能仍然可用,只是解析期间会卡一下 —— 可用性优先于性能。
 */

import { getLogLevel, type Tracer } from "../apps/shared/logger";
import type { SheetHandle } from "../apps/shared/sheet";
import { parseCsv, sheetFromPacked, type PackedSheetTransfer } from "./index";
import type { CsvWorkerRequest, CsvWorkerResponse } from "./csvWorker";

/** 解析结果与耗时明细。 */
export interface CsvParseOutcome {
  sheet: SheetHandle;
  packed: PackedSheetTransfer;
  /** 是否在 Worker 里完成解析。 */
  offMainThread: boolean;
  /** 解析(含跨线程往返)耗时。 */
  parseMs: number;
  /** 主线程装配表格句柄的耗时。 */
  assembleMs: number;
}

/**
 * Worker **本身**不可用(而非解析失败)。
 *
 * 必须把两者分开:解析失败说明文件有问题,再在主线程重试一次只是白白
 * 又跑一遍(大文件还会把 UI 卡住),而且日志里会出现两条一样的错误;
 * 只有 Worker 起不来时才值得降级重试。
 */
class WorkerUnavailableError extends Error {
  constructor(reason: string) {
    super(reason);
    this.name = "WorkerUnavailableError";
  }
}

/** 创建 Worker;不支持时返回 `null`。 */
function createWorker(): Worker | null {
  if (typeof Worker === "undefined") return null;
  try {
    return new Worker(new URL("./csvWorker.ts", import.meta.url), { type: "module" });
  } catch {
    // 某些沙箱环境会禁用模块 Worker,退回主线程即可
    return null;
  }
}

/** 在 Worker 里解析一次;失败时 reject。 */
function parseInWorker(
  worker: Worker,
  bytes: Uint8Array,
  traceId: string,
  delimiter: number,
): Promise<PackedSheetTransfer> {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      worker.onmessage = null;
      worker.onerror = null;
    };

    worker.onmessage = (event: MessageEvent<CsvWorkerResponse>) => {
      cleanup();
      const data = event.data;
      if (!data.ok) {
        // 内核给出的解析错误:文件的问题,不要再试
        reject(new Error(data.message));
        return;
      }
      resolve({
        text: new Uint8Array(data.text),
        cellEnds: new Uint32Array(data.cellEnds),
        rowStarts: new Uint32Array(data.rowStarts),
        colWidthUnits: new Uint32Array(data.colWidthUnits),
        cols: data.cols,
        meta: data.meta,
      });
    };
    worker.onerror = (event) => {
      cleanup();
      reject(new WorkerUnavailableError(event.message || "解析线程异常退出"));
    };

    // 复制一份字节再转移:调用方持有的 File 缓冲不该被我们弄空
    const buffer = bytes.slice().buffer;
    const request: CsvWorkerRequest = {
      bytes: buffer,
      traceId,
      delimiter,
      logLevel: getLogLevel(),
    };
    try {
      worker.postMessage(request, [buffer]);
    } catch (error) {
      cleanup();
      reject(
        new WorkerUnavailableError(
          error instanceof Error ? error.message : "无法向解析线程发送数据",
        ),
      );
    }
  });
}

/**
 * 解析 CSV 字节流并装配成可绘制的表格句柄。
 *
 * @param worker 复用的 Worker;传 `null` 表示直接在主线程解析
 */
export async function parseCsvFile(
  bytes: Uint8Array,
  traceId: string,
  tracer: Tracer,
  worker: Worker | null,
  delimiter = 0,
): Promise<CsvParseOutcome> {
  const parseStarted = performance.now();
  let packed: PackedSheetTransfer;
  let offMainThread = worker !== null;

  if (worker) {
    try {
      packed = await parseInWorker(worker, bytes, traceId, delimiter);
    } catch (error) {
      if (!(error instanceof WorkerUnavailableError)) {
        throw error; // 解析失败,如实抛给上层展示
      }
      // Worker 起不来:降级到主线程重试一次,
      // 用户看到的是「慢一点但成功」,而不是「打不开」
      tracer.warn("csv.worker.fallback", { reason: error.message });
      offMainThread = false;
      packed = await parseCsv(bytes, traceId, delimiter);
    }
  } else {
    packed = await parseCsv(bytes, traceId, delimiter);
  }

  const parseMs = performance.now() - parseStarted;
  const assembleStarted = performance.now();
  const sheet = await sheetFromPacked(packed);
  const assembleMs = performance.now() - assembleStarted;

  return { sheet, packed, offMainThread, parseMs, assembleMs };
}

export { createWorker };
