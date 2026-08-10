/**
 * 「选择 xlsx 文件 → 解析 → 多工作表可绘制」的状态机。
 *
 * 与 {@link useCsvFile} 并列:CSV 走 Worker 紧凑缓冲,xlsx 自带缓存计算值、
 * 在主线程一次解析成多张表,这里管理工作表切换与句柄释放。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { createTracer, type Tracer } from "@tengxiaohyx/office-shared";
import type { SheetHandle } from "./sheet";
import { loadXlsx, type XlsxWorkbookHandle } from "./wasm/xlsx";

/** 加载状态。 */
export type XlsxStatus = "idle" | "reading" | "parsing" | "ready" | "error";

/** 状态快照。 */
export interface XlsxFileState {
  status: XlsxStatus;
  fileName: string | null;
  fileSize: number;
  /** 工作表名(按原始顺序)。 */
  sheetNames: string[];
  /** 当前工作表下标。 */
  activeSheet: number;
  /** 当前工作表句柄。 */
  sheet: SheetHandle | null;
  error: string | null;
  tracer: Tracer | null;
}

const INITIAL: XlsxFileState = {
  status: "idle",
  fileName: null,
  fileSize: 0,
  sheetNames: [],
  activeSheet: 0,
  sheet: null,
  error: null,
  tracer: null,
};

/**
 * 管理 xlsx 文件打开与工作表切换。
 *
 * 要点:换文件 / 换表时旧句柄**必须** `dispose()`(WASM 线性内存里的表不会自动释放);
 * 工作簿本体在换文件或卸载时 `dispose()`。
 */
export function useXlsxFile() {
  const [state, setState] = useState<XlsxFileState>(INITIAL);
  const wbRef = useRef<XlsxWorkbookHandle | null>(null);
  const sheetRef = useRef<SheetHandle | null>(null);
  const requestSeq = useRef(0);

  const disposeAll = useCallback(() => {
    sheetRef.current?.dispose();
    sheetRef.current = null;
    wbRef.current?.dispose();
    wbRef.current = null;
  }, []);

  useEffect(() => () => disposeAll(), [disposeAll]);

  const openFile = useCallback(
    async (file: File) => {
      const seq = (requestSeq.current += 1);
      const tracer = createTracer();
      tracer.info("xlsx.open", { name: file.name, bytes: file.size });
      setState({ ...INITIAL, status: "reading", fileName: file.name, fileSize: file.size, tracer });

      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        if (seq !== requestSeq.current) return;
        setState((p) => ({ ...p, status: "parsing" }));

        const wb = await loadXlsx(bytes);
        if (seq !== requestSeq.current) {
          wb.dispose();
          return;
        }
        disposeAll();
        wbRef.current = wb;
        const sheet = wb.openSheet(0);
        sheetRef.current = sheet;
        tracer.info("xlsx.ready", { sheets: wb.sheetNames.length });

        setState({
          status: "ready",
          fileName: file.name,
          fileSize: file.size,
          sheetNames: wb.sheetNames,
          activeSheet: 0,
          sheet,
          error: null,
          tracer,
        });
      } catch (error) {
        if (seq !== requestSeq.current) return;
        const message = error instanceof Error ? error.message : String(error);
        tracer.error("xlsx.failed", { name: file.name, reason: message });
        setState({ ...INITIAL, status: "error", fileName: file.name, fileSize: file.size, error: message, tracer });
      }
    },
    [disposeAll],
  );

  /** 切换到第 `index` 张工作表。 */
  const selectSheet = useCallback((index: number) => {
    const wb = wbRef.current;
    if (!wb) return;
    setState((prev) => {
      if (prev.status !== "ready" || index === prev.activeSheet) return prev;
      sheetRef.current?.dispose();
      const sheet = wb.openSheet(index);
      sheetRef.current = sheet;
      return { ...prev, activeSheet: index, sheet };
    });
  }, []);

  const reset = useCallback(() => {
    requestSeq.current += 1;
    disposeAll();
    setState(INITIAL);
  }, [disposeAll]);

  return { state, openFile, selectSheet, reset };
}
