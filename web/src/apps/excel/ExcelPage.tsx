import { useState } from "react";
import { FileUpload } from "@tengxiaohyx/office-shared";
import { useCsvFile } from "../shared/useCsvFile";
import { useXlsxFile } from "../shared/useXlsxFile";
import { createTracer } from "@tengxiaohyx/office-shared";
import { SheetCanvas } from "./SheetCanvas";

/** 支持的扩展名:CSV 家族 + xlsx。 */
const ACCEPT = ".csv,.tsv,.txt,.xlsx";

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

type Mode = "csv" | "xlsx";

/**
 * 表格(Excel)页面。
 *
 * 支持 CSV 与 **.xlsx**:CSV 走 Worker 紧凑缓冲解析,xlsx 用 calamine 一次解析成
 * 多张工作表(自带缓存计算值,不重算)。两者都在 canvas 上渲染,共用同一套渲染管线;
 * 以 `=` 开头的单元格显示计算结果、公式栏回显原始公式,语义对齐 Excel。
 */
export function ExcelPage() {
  const csv = useCsvFile();
  const xlsx = useXlsxFile();
  const [mode, setMode] = useState<Mode | null>(null);

  /** 按扩展名路由到 CSV 或 xlsx 打开流程。 */
  const handleFile = (file: File) => {
    if (file.name.toLowerCase().endsWith(".xlsx")) {
      setMode("xlsx");
      csv.reset();
      void xlsx.openFile(file);
    } else {
      setMode("csv");
      xlsx.reset();
      void csv.openFile(file);
    }
  };

  /** 载入内置公式示例(构造成一个 CSV File 走同一条打开流程)。 */
  const openSample = () => {
    handleFile(new File([FORMULA_SAMPLE], "公式示例.csv", { type: "text/csv" }));
  };

  const closeAll = () => {
    csv.reset();
    xlsx.reset();
    setMode(null);
  };

  const csvState = csv.state;
  const xlsxState = xlsx.state;
  const busy =
    (mode === "csv" && (csvState.status === "reading" || csvState.status === "parsing")) ||
    (mode === "xlsx" && (xlsxState.status === "reading" || xlsxState.status === "parsing"));
  const errorState =
    mode === "csv" && csvState.status === "error"
      ? csvState.error
      : mode === "xlsx" && xlsxState.status === "error"
        ? xlsxState.error
        : null;
  const fileName = mode === "xlsx" ? xlsxState.fileName : csvState.fileName;
  const fileSize = mode === "xlsx" ? xlsxState.fileSize : csvState.fileSize;
  const ready =
    (mode === "csv" && csvState.status === "ready") ||
    (mode === "xlsx" && xlsxState.status === "ready");

  return (
    <section className="office-page office-page--sheet" aria-label="表格 · Excel">
      <header className="office-page__header">
        <h2>表格 · Excel</h2>
        <p className="office-page__subtitle">
          上传 CSV 或 <code>.xlsx</code> 文件,在 canvas 上查看表格视图。解析、列切分与
          <strong>公式求值</strong>都由 Rust/WASM 内核完成。xlsx 支持
          <strong>多工作表</strong>、<strong>单元格样式</strong>(字体/颜色/填充/对齐)、
          <strong>数字格式</strong>、<strong>合并单元格</strong>与<strong>内嵌图片</strong>;
          <strong>过滤 / 排序 / 冻结 / 区域选择复制 / 列宽拖拽 / 查找</strong>一应俱全。
        </p>
      </header>

      <div className="office-page__upload">
        <FileUpload accept={ACCEPT} onFile={handleFile} label="上传 CSV / xlsx 文件" />
        <button
          type="button"
          className="office-page__link"
          onClick={openSample}
          data-testid="load-formula-sample"
        >
          加载公式示例
        </button>
        {fileName && (
          <span className="office-page__filename">
            {fileName}
            <span className="office-page__filesize">{formatBytes(fileSize)}</span>
          </span>
        )}
        {ready && (
          <button type="button" className="office-page__link" onClick={closeAll}>
            关闭
          </button>
        )}
      </div>

      {busy && (
        <div className="office-page__result" data-testid="result">
          <p>正在解析…</p>
        </div>
      )}

      {errorState && (
        <div className="office-page__result" data-testid="result">
          <p className="office-page__error">打开失败:{errorState}</p>
          <p className="office-page__hint">
            请确认文件格式正确,然后重新选择文件重试。
            {mode === "csv" && csvState.traceId && <>(排查编号 {csvState.traceId})</>}
          </p>
        </div>
      )}

      {mode === null && (
        <div className="office-page__result" data-testid="result">
          <p className="office-page__empty">
            尚未选择文件。请上传一个 CSV 或 .xlsx 文件以查看表格视图。
          </p>
          <ul className="office-page__hint">
            <li>CSV:支持 UTF-8 / UTF-16 / GBK 等编码,分隔符自动识别。</li>
            <li>
              xlsx:多工作表,单元格显示计算值 + 数字格式,渲染字体/颜色/填充/对齐、合并单元格、内嵌图片。
            </li>
            <li>
              以 <code>=</code> 开头的单元格按 Excel 公式求值,支持 SUM/IF/VLOOKUP/XLOOKUP/DATE 等
              150+ 函数(含跨表引用、具名区域)。点「加载公式示例」试试。
            </li>
            <li>
              过滤 / 排序 / 冻结行列 / 区域选择 + 复制(Ctrl⌘C)/ 列宽拖拽 / 查找(Ctrl⌘F)。
            </li>
            <li>滚轮或拖拽平移,Ctrl(⌘)加滚轮以指针为中心缩放,方向键移动选区。</li>
          </ul>
        </div>
      )}

      {/* CSV 视图 */}
      {mode === "csv" && csvState.status === "ready" && csvState.sheet && csvState.meta && csvState.tracer && (
        <>
          <SheetCanvas sheet={csvState.sheet} tracer={csvState.tracer} />
          <dl className="office-page__meta" data-testid="sheet-meta">
            <dt>编码</dt>
            <dd>
              {csvState.meta.encoding}
              {csvState.meta.lossy && <span className="office-page__warn">(部分字符无法解码)</span>}
            </dd>
            <dt>分隔符</dt>
            <dd>{csvState.meta.delimiter === "\t" ? "制表符" : csvState.meta.delimiter}</dd>
            <dt>规模</dt>
            <dd>
              {csvState.meta.rows.toLocaleString()} 行 × {csvState.meta.cols.toLocaleString()} 列
              {(csvState.meta.truncatedRows || csvState.meta.truncatedCols) && (
                <span className="office-page__warn">已达上限,超出部分未显示</span>
              )}
            </dd>
            {csvState.sheet.formulaCount ? (
              <>
                <dt>公式</dt>
                <dd>{csvState.sheet.formulaCount.toLocaleString()} 个已求值</dd>
              </>
            ) : null}
          </dl>
        </>
      )}

      {/* xlsx 视图:工作表标签 + 表格 */}
      {mode === "xlsx" && xlsxState.status === "ready" && xlsxState.sheet && (
        <>
          {xlsxState.sheetNames.length > 1 && (
            <div className="sheet-tabs" role="tablist" data-testid="sheet-tabs">
              {xlsxState.sheetNames.map((name, i) => (
                <button
                  key={i}
                  type="button"
                  role="tab"
                  aria-selected={i === xlsxState.activeSheet}
                  className={i === xlsxState.activeSheet ? "sheet-tab sheet-tab--active" : "sheet-tab"}
                  onClick={() => xlsx.selectSheet(i)}
                  data-testid={`sheet-tab-${i}`}
                >
                  {name}
                </button>
              ))}
            </div>
          )}
          <SheetCanvas
            key={xlsxState.activeSheet}
            sheet={xlsxState.sheet}
            tracer={xlsxState.tracer ?? createTracer()}
          />
          <dl className="office-page__meta" data-testid="sheet-meta">
            <dt>工作表</dt>
            <dd>
              {xlsxState.sheetNames[xlsxState.activeSheet]}
              <span className="office-page__muted">
                第 {xlsxState.activeSheet + 1} / {xlsxState.sheetNames.length} 张
              </span>
            </dd>
            <dt>规模</dt>
            <dd>
              {xlsxState.sheet.rows.toLocaleString()} 行 × {xlsxState.sheet.cols.toLocaleString()} 列
            </dd>
            {xlsxState.sheet.formulaCount ? (
              <>
                <dt>公式</dt>
                <dd>{xlsxState.sheet.formulaCount.toLocaleString()} 个(显示缓存计算值)</dd>
              </>
            ) : null}
          </dl>
        </>
      )}
    </section>
  );
}
