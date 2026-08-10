/**
 * 行列表头的文字标签。
 *
 * 这些字符串每帧都要用到,而 `A`→`AA` 的进位计算与数字转字符串都会产生
 * 新的字符串对象。渲染热路径上「不每帧分配字符串」是硬要求,
 * 所以这里做一层缓存:同一个下标只算一次。
 */

/** 列标签缓存,下标即列号。 */
const columnLabels: string[] = [];

/** 行标签缓存,下标即行号。 */
const rowLabels: string[] = [];

/**
 * 行标签缓存的上限。
 *
 * 百万行的表格如果全缓存,光标签就要占几十 MB;
 * 超过上限就整体清空重来 —— 可见行数有限,重建代价可忽略。
 */
const ROW_LABEL_CACHE_LIMIT = 20_000;

/**
 * 列号 → Excel 风格列名:0 → `A`,25 → `Z`,26 → `AA`。
 *
 * 负数或非整数会被夹到 0。
 */
export function columnLabel(index: number): string {
  const i = Math.max(0, Math.floor(index));
  const cached = columnLabels[i];
  if (cached !== undefined) return cached;

  // 26 进制,但没有「0」这一位,所以每次要先减 1
  let label = "";
  let n = i;
  do {
    label = String.fromCharCode(65 + (n % 26)) + label;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);

  columnLabels[i] = label;
  return label;
}

/** 行号 → 展示用行号(从 1 开始)。 */
export function rowLabel(index: number): string {
  const i = Math.max(0, Math.floor(index));
  const cached = rowLabels[i];
  if (cached !== undefined) return cached;

  if (rowLabels.length > ROW_LABEL_CACHE_LIMIT) {
    rowLabels.length = 0;
  }
  const label = String(i + 1);
  rowLabels[i] = label;
  return label;
}

/** 单元格地址,如 `B3`。用于状态栏与无障碍播报。 */
export function cellAddress(row: number, col: number): string {
  return `${columnLabel(col)}${rowLabel(row)}`;
}
