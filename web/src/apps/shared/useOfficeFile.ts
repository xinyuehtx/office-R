import { useCallback, useState } from "react";
import { render, type RenderResult } from "../../wasm";

/** 上传/解析的状态机。 */
export interface OfficeFileState {
  /** 当前选中的文件名。 */
  fileName: string | null;
  /** 是否正在解析。 */
  loading: boolean;
  /** 解析结果(成功时)。 */
  result: RenderResult | null;
  /** 错误信息(失败时)。 */
  error: string | null;
}

const INITIAL: OfficeFileState = {
  fileName: null,
  loading: false,
  result: null,
  error: null,
};

/**
 * 管理「选择文件 → 读取字节 → 调用 WASM 内核渲染」的通用逻辑。
 * 三个页面共用,避免重复。
 */
export function useOfficeFile() {
  const [state, setState] = useState<OfficeFileState>(INITIAL);

  const openFile = useCallback(async (file: File) => {
    setState({ fileName: file.name, loading: true, result: null, error: null });
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const result = await render(bytes);
      setState({ fileName: file.name, loading: false, result, error: null });
    } catch (err) {
      setState({
        fileName: file.name,
        loading: false,
        result: null,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }, []);

  const reset = useCallback(() => setState(INITIAL), []);

  return { state, openFile, reset };
}
