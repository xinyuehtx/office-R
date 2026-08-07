import { FileUpload } from "../shared/FileUpload";
import { useCsvFile } from "../shared/useCsvFile";
import { SheetCanvas } from "./SheetCanvas";

/** 本期支持的扩展名。xlsx 的表格渲染留待后续切片。 */
const ACCEPT = ".csv,.tsv,.txt";

/**
 * 内置公式示例:以 `=` 开头的单元格会被 Rust/WASM 公式引擎求值,
 * 表格显示计算结果,选中后公式栏回显原始公式。
 */
const FORMULA_SAMPLE = `商品,单价,数量,小计
苹果,3.5,4,=B2*C2
香蕉,2,6,=B3*C3
橙子,4.2,3,=B4*C4
合计,,=SUM(C2:C4),=SUM(D2:D4)
均价,=AVERAGE(B2:B4),,
最高价,=MAX(B2:B4),,
满减,=IF(D5>30,"满30打折","未满30"),,
`;

/** 把字节数格式化成人类可读的大小。 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/**
 * 表格(Excel)页面。
 *
 * 上传 CSV → canvas 表格视图。以 `=` 开头的单元格会被 **Rust/WASM 公式引擎**求值,
 * 显示计算结果(选中后公式栏回显原始公式),语义对齐 Excel。
 */
export function ExcelPage() {
  const { state, openFile, reset } = useCsvFile();
  const busy = state.status === "reading" || state.status === "parsing";

  /** 载入内置公式示例(构造成一个 File 走同一条打开流程)。 */
  const openSample = () => {
    const file = new File([FORMULA_SAMPLE], "公式示例.csv", { type: "text/csv" });
    void openFile(file);
  };

  return (
    <section className="office-page office-page--sheet" aria-label="表格 · Excel">
      <header className="office-page__header">
        <h2>表格 · Excel</h2>
        <p className="office-page__subtitle">
          上传 CSV 文件,在 canvas 上查看表格视图。解析、列切分与**公式求值**都由
          Rust/WASM 内核完成,支持自动识别编码与分隔符。写在单元格里以 <code>=</code>{" "}
          开头的公式(如 <code>=SUM(A1:A10)</code>)会像 Excel 一样算出结果。
        </p>
      </header>

      <div className="office-page__upload">
        <FileUpload accept={ACCEPT} onFile={openFile} label="上传 CSV 文件" />
        <button
          type="button"
          className="office-page__link"
          onClick={openSample}
          data-testid="load-formula-sample"
        >
          加载公式示例
        </button>
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
            <li>
              以 <code>=</code> 开头的单元格按 Excel 公式求值,支持 SUM/IF/VLOOKUP/DATE 等
              140+ 函数。点「加载公式示例」试试。
            </li>
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
            {state.sheet.formulaCount ? (
              <>
                <dt>公式</dt>
                <dd>
                  {state.sheet.formulaCount.toLocaleString()} 个已求值
                  <span className="office-page__muted">选中公式格,公式栏显示原始公式</span>
                </dd>
              </>
            ) : null}
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
