import { FileUpload } from "../shared/FileUpload";
import { useCsvFile } from "../shared/useCsvFile";
import { SheetCanvas } from "./SheetCanvas";

/** 本期支持的扩展名。xlsx 的表格渲染留待后续切片。 */
const ACCEPT = ".csv,.tsv,.txt";

/** 把字节数格式化成人类可读的大小。 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/**
 * 表格(Excel)页面。
 *
 * 本期范围:**只做 CSV 的只读视图渲染**。
 * 公式求值、数字/日期格式化、图表都不在范围内 —— CSV 本身也不携带这些信息。
 */
export function ExcelPage() {
  const { state, openFile, reset } = useCsvFile();
  const busy = state.status === "reading" || state.status === "parsing";

  return (
    <section className="office-page office-page--sheet" aria-label="表格 · Excel">
      <header className="office-page__header">
        <h2>表格 · Excel</h2>
        <p className="office-page__subtitle">
          上传 CSV 文件,在 canvas 上查看表格视图。解析与列切分由 Rust/WASM 内核完成,
          支持自动识别编码与分隔符。本期仅支持 CSV,不含公式、格式化与图表。
        </p>
      </header>

      <div className="office-page__upload">
        <FileUpload accept={ACCEPT} onFile={openFile} label="上传 CSV 文件" />
        {state.fileName && (
          <span className="office-page__filename">
            {state.fileName}
            <span className="office-page__filesize">{formatBytes(state.fileSize)}</span>
          </span>
        )}
        {state.status === "ready" && (
          <button type="button" className="office-page__link" onClick={reset}>
            关闭
          </button>
        )}
      </div>

      {busy && (
        <div className="office-page__result" data-testid="result">
          <p>{state.status === "reading" ? "正在读取文件…" : "正在解析…"}</p>
        </div>
      )}

      {state.status === "error" && (
        <div className="office-page__result" data-testid="result">
          <p className="office-page__error">打开失败:{state.error}</p>
          <p className="office-page__hint">
            请确认这是一个 CSV 文本文件,然后重新选择文件重试。
            {state.traceId && <>(排查编号 {state.traceId})</>}
          </p>
        </div>
      )}

      {state.status === "idle" && (
        <div className="office-page__result" data-testid="result">
          <p className="office-page__empty">
            尚未选择文件。请上传一个 CSV 文件以查看表格视图。
          </p>
          <ul className="office-page__hint">
            <li>支持 UTF-8 / UTF-16 / GBK 等编码,带不带 BOM 都可以。</li>
            <li>分隔符自动识别:逗号、分号、制表符、竖线。</li>
            <li>滚轮或拖拽平移,Ctrl(⌘)加滚轮以指针为中心缩放,方向键移动选区。</li>
          </ul>
        </div>
      )}

      {state.status === "ready" && state.sheet && state.meta && state.tracer && (
        <>
          <SheetCanvas sheet={state.sheet} tracer={state.tracer} />
          <dl className="office-page__meta" data-testid="sheet-meta">
            <dt>编码</dt>
            <dd>
              {state.meta.encoding}
              {state.meta.lossy && <span className="office-page__warn">(部分字符无法解码)</span>}
            </dd>
            <dt>分隔符</dt>
            <dd>
              {state.meta.delimiter === "\t" ? "制表符" : state.meta.delimiter}
              <span className="office-page__muted">
                {state.meta.delimiterSource === "sniffed"
                  ? "自动识别"
                  : state.meta.delimiterSource === "explicit"
                    ? "手动指定"
                    : "默认值"}
              </span>
            </dd>
            <dt>规模</dt>
            <dd>
              {state.meta.rows.toLocaleString()} 行 × {state.meta.cols.toLocaleString()} 列
              {(state.meta.truncatedRows || state.meta.truncatedCols) && (
                <span className="office-page__warn">已达上限,超出部分未显示</span>
              )}
            </dd>
            {state.metrics && (
              <>
                <dt>耗时</dt>
                <dd className="office-page__muted">
                  读取 {state.metrics.readMs.toFixed(0)} ms · 解析{" "}
                  {state.metrics.kernelMs.toFixed(0)} ms · 合计{" "}
                  {state.metrics.totalMs.toFixed(0)} ms
                  {state.metrics.offMainThread ? "(后台线程解析)" : "(主线程解析)"}
                </dd>
              </>
            )}
          </dl>
        </>
      )}
    </section>
  );
}
