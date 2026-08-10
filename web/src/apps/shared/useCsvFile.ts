/**
 * 「选择 CSV 文件 → 解析 → 得到可绘制表格」的状态机。
 *
 * 收敛在 shared 里而不是写在页面组件内:将来 xlsx 切片接进来时,
 * 除了解析入口不同,状态流转、错误处理、耗时统计都可以照用。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { createTracer, type Tracer } from "@tengxiaohyx/office-shared";
import type { SheetHandle, SheetMeta } from "@tengxiaohyx/office-excel";
import { createWorker, parseCsvFile } from "@tengxiaohyx/office-excel";

/** 加载状态。 */
export type CsvStatus = "idle" | "reading" | "parsing" | "ready" | "error";

/** 各阶段耗时,用于状态栏展示与性能核对。 */
export interface CsvMetrics {
  /** 读文件为字节的耗时。 */
  readMs: number;
  /** 解析耗时(含 Worker 往返)。 */
  parseMs: number;
  /** 内核自报的纯解析耗时。 */
  kernelMs: number;
  /** 主线程装配句柄耗时。 */
  assembleMs: number;
  /** 从点击到可绘制的总耗时。 */
  totalMs: number;
  /** 解析是否在 Worker 中完成。 */
  offMainThread: boolean;
}

/** 状态快照。 */
export interface CsvFileState {
  status: CsvStatus;
  fileName: string | null;
  fileSize: number;
  sheet: SheetHandle | null;
  meta: SheetMeta | null;
  /** 面向用户的错误信息(已是中文、可操作)。 */
  error: string | null;
  metrics: CsvMetrics | null;
  /** 本次打开的日志 traceId,便于用户报障时对号。 */
  traceId: string | null;
  tracer: Tracer | null;
}

const INITIAL: CsvFileState = {
  status: "idle",
  fileName: null,
  fileSize: 0,
  sheet: null,
  meta: null,
  error: null,
  metrics: null,
  traceId: null,
  tracer: null,
};

/**
 * 管理 CSV 文件的打开流程。
 *
 * 要点:
 * - 旧表格在换文件时**必须** `dispose()`,否则 WASM 线性内存里的数据不会释放;
 * - 每次打开生成新的 traceId,前端与内核日志据此串联;
 * - 失败时保留文件名并给出可操作的提示,用户可以直接重选文件重试。
 */
export function useCsvFile() {
  const [state, setState] = useState<CsvFileState>(INITIAL);
  const workerRef = useRef<Worker | null>(null);
  /** 只保留最后一次打开的序号,避免快速连选两个文件时旧结果覆盖新结果。 */
  const requestSeq = useRef(0);
  /** 当前持有的表格,用于卸载时释放。 */
  const sheetRef = useRef<SheetHandle | null>(null);

  const getWorker = useCallback(() => {
    if (workerRef.current === null) {
      workerRef.current = createWorker();
    }
    return workerRef.current;
  }, []);

  useEffect(
    () => () => {
      workerRef.current?.terminate();
      workerRef.current = null;
      sheetRef.current?.dispose();
      sheetRef.current = null;
    },
    [],
  );

  const openFile = useCallback(
    async (file: File) => {
      const seq = (requestSeq.current += 1);
      const tracer = createTracer();
      const startedAt = performance.now();

      // 绝不记录文件内容,只记录名字与大小
      tracer.info("file.open", { name: file.name, bytes: file.size });
      setState({
        ...INITIAL,
        status: "reading",
        fileName: file.name,
        fileSize: file.size,
        traceId: tracer.traceId,
        tracer,
      });

      try {
        const readStarted = performance.now();
        const bytes = new Uint8Array(await file.arrayBuffer());
        const readMs = performance.now() - readStarted;
        if (seq !== requestSeq.current) return;

        setState((previous) => ({ ...previous, status: "parsing" }));

        const outcome = await parseCsvFile(bytes, tracer.traceId, tracer, getWorker());
        if (seq !== requestSeq.current) {
          // 已经有更新的请求了,当前结果直接丢弃并释放
          outcome.sheet.dispose();
          return;
        }

        sheetRef.current?.dispose();
        sheetRef.current = outcome.sheet;

        const metrics: CsvMetrics = {
          readMs,
          parseMs: outcome.parseMs,
          kernelMs: outcome.packed.meta.parseMs,
          assembleMs: outcome.assembleMs,
          totalMs: performance.now() - startedAt,
          offMainThread: outcome.offMainThread,
        };
        tracer.info("file.ready", {
          rows: outcome.packed.meta.rows,
          cols: outcome.packed.meta.cols,
          encoding: outcome.packed.meta.encoding,
          delimiter: outcome.packed.meta.delimiter,
          readMs: readMs.toFixed(1),
          parseMs: outcome.parseMs.toFixed(1),
          kernelMs: outcome.packed.meta.parseMs.toFixed(1),
          totalMs: metrics.totalMs.toFixed(1),
          offMainThread: outcome.offMainThread,
        });
        if (outcome.packed.meta.truncatedRows || outcome.packed.meta.truncatedCols) {
          tracer.warn("file.truncated", {
            rows: outcome.packed.meta.rows,
            cols: outcome.packed.meta.cols,
          });
        }
        if (outcome.packed.meta.lossy) {
          tracer.warn("file.lossyDecode", { encoding: outcome.packed.meta.encoding });
        }

        setState({
          status: "ready",
          fileName: file.name,
          fileSize: file.size,
          sheet: outcome.sheet,
          meta: outcome.packed.meta,
          error: null,
          metrics,
          traceId: tracer.traceId,
          tracer,
        });
      } catch (error) {
        if (seq !== requestSeq.current) return;
        const message = error instanceof Error ? error.message : String(error);
        tracer.error("file.failed", { name: file.name, bytes: file.size, reason: message });
        setState({
          ...INITIAL,
          status: "error",
          fileName: file.name,
          fileSize: file.size,
          error: message,
          traceId: tracer.traceId,
          tracer,
        });
      }
    },
    [getWorker],
  );

  const reset = useCallback(() => {
    requestSeq.current += 1;
    sheetRef.current?.dispose();
    sheetRef.current = null;
    setState(INITIAL);
  }, []);

  return { state, openFile, reset };
}
