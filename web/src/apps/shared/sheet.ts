/**
 * 表格数据源的抽象。
 *
 * 渲染管线只依赖这个接口,不关心数据是来自 WASM 还是测试替身 ——
 * 这让 canvas 绘制逻辑可以在 jsdom 里单测,也让将来接入 xlsx 时
 * 视图层一行都不用改。
 */

/** 解析元信息(不含任何单元格内容,可安全打日志)。 */
export interface SheetMeta {
  /** 实际使用的文本编码,如 `UTF-8`、`GBK`。 */
  encoding: string;
  /** 实际使用的分隔符字符。 */
  delimiter: string;
  /** 分隔符的来源:显式指定 / 嗅探得出 / 兜底默认值。 */
  delimiterSource: "explicit" | "sniffed" | "fallback";
  /** 是否有字符无法解码(内容可能不完全准确)。 */
  lossy: boolean;
  /** 行数。 */
  rows: number;
  /** 列数。 */
  cols: number;
  /** 是否因超过行数上限被截断。 */
  truncatedRows: boolean;
  /** 是否因超过列数上限被截断。 */
  truncatedCols: boolean;
  /** 内核解析耗时(毫秒)。 */
  parseMs: number;
}

/**
 * 一块矩形区域内的单元格文本。
 *
 * 用「一个大字符串 + 偏移数组」而不是 `string[][]`:
 * 跨 WASM 边界只需两次拷贝,也避免了每次滚动都产生上千个字符串对象。
 */
export interface CellWindowData {
  /** 区域内单元格文本按行优先首尾相接。 */
  text: string;
  /** 每个单元格在 `text` 中的结束偏移(以 UTF-16 码元计,可直接用于 slice)。 */
  ends: Uint32Array;
  /** 区域行数。 */
  rows: number;
  /** 区域列数。 */
  cols: number;
}

/** 一个公式单元格:0 基下标 + 原始公式文本(含前导 `=`)。 */
export interface CellFormula {
  row: number;
  col: number;
  formula: string;
}

/** 单列过滤规格(与 WASM `FilterSpec` 的紧凑 JSON 对应)。 */
export interface FilterSpec {
  /** 0 基列号。 */
  col: number;
  /** 过滤类型。 */
  kind: "values" | "text" | "number" | "blank";
  /** 运算符:text = contains/notContains/equals/begins/ends;number = eq/ne/gt/ge/lt/le/between。 */
  op?: string;
  /** text 的关键字。 */
  needle?: string;
  /** number 的操作数(between 用 a、b)。 */
  a?: number;
  b?: number;
  /** values 的值集。 */
  values?: string[];
  /** blank:true 只留空白,false 只留非空白。 */
  blank?: boolean;
}

/** 列唯一值枚举结果。 */
export interface UniqueValues {
  values: string[];
  /** 是否因超过上限被截断(还有更多未列出)。 */
  truncated: boolean;
}

/** 单元格视觉样式(xlsx;只读渲染)。 */
export interface CellStyle {
  bold?: boolean;
  italic?: boolean;
  /** 文字色 RRGGBB。 */
  color?: string;
  /** 填充背景 RRGGBB。 */
  fill?: string;
  /** 水平对齐。 */
  align?: "left" | "center" | "right";
}

/** 只读表格句柄。 */
export interface SheetHandle {
  /** 行数(过滤后为可视行数)。 */
  readonly rows: number;
  /** 列数。 */
  readonly cols: number;
  /** 各列建议显示宽度(单位:半角字符数)。 */
  readonly colWidthUnits: Uint32Array;
  /** 取 `[row0, row1) × [col0, col1)` 区域的单元格文本。入参越界会被夹紧。 */
  window(row0: number, row1: number, col0: number, col1: number): CellWindowData;
  /**
   * 若 `(row, col)` 是公式单元格,返回其原始公式(含 `=`);否则 `null`。
   *
   * 单元格里显示的是**计算值**([`window`] 取到的),公式栏用它回显**原始公式** ——
   * 与 Excel 「格显示值、栏显示式」一致。表格无公式时可不实现。
   */
  formula?(row: number, col: number): string | null;
  /** 公式单元格总数(无公式为 0);用于页面提示。 */
  readonly formulaCount?: number;
  /**
   * 可视行 → **底层行号**(0 基)。过滤后可视行是紧凑 `0..V`,行头据此显示原始行号。
   * 未过滤或不支持时可省略(视为恒等)。
   */
  rowLabel?(visualRow: number): number;
  /** 应用列过滤(多列 AND),返回可视行数;`headerRows` 为顶部始终保留的行数。 */
  filter?(specs: FilterSpec[], headerRows: number): number;
  /** 清除过滤,恢复全量。 */
  clearFilter?(): void;
  /**
   * 按某列排序,返回可视行数。`dir` 为 `"asc"`/`"desc"` 排序,`"none"` 取消。
   * 与过滤复合(排序作用在过滤结果之上);顶部 `headerRows` 行固定置顶。
   */
  sort?(col: number, dir: "asc" | "desc" | "none", headerRows: number): number;
  /**
   * 全表查找,返回命中单元格的**可视坐标**(受过滤/排序影响)。
   * `wholeCell` 为整格精确匹配,否则子串;`limit` 上限。
   */
  find?(
    needle: string,
    caseSensitive: boolean,
    wholeCell: boolean,
    limit: number,
  ): { row: number; col: number }[];
  /** 枚举某列唯一值(供值集过滤 UI)。 */
  uniqueValues?(col: number, headerRows: number, limit: number): UniqueValues;
  /** 单元格视觉样式(xlsx);无样式或非 xlsx 返回 null。 */
  cellStyle?(row: number, col: number): CellStyle | null;
  /** 合并单元格区域 `[row0, col0, row1, col1]`(0 基,含首尾)。 */
  merges?: [number, number, number, number][];
  /** 释放底层资源(WASM 线性内存里的表格)。 */
  dispose(): void;
}

/**
 * 把窗口数据摊平成 `string[]`(行优先),供绘制阶段按下标取用。
 *
 * 只在**可见区域发生变化**时调用一次,绘制时直接读数组,
 * 因此每帧不会有字符串切片的开销。
 */
export function flattenWindow(window: CellWindowData): string[] {
  const cells = new Array<string>(window.rows * window.cols);
  let start = 0;
  for (let i = 0; i < cells.length; i += 1) {
    const end = window.ends[i] ?? start;
    cells[i] = start === end ? "" : window.text.slice(start, end);
    start = end;
  }
  return cells;
}
