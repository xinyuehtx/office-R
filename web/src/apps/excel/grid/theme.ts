/**
 * 表格视图的视觉与尺寸常量。
 *
 * 全部以 **CSS 像素**为单位、且是 zoom = 1 时的基准值;
 * 实际尺寸由 `geometry.ts` 乘上缩放系数得到。
 * 集中在这里是为了让「改外观」不需要翻遍绘制代码。
 */

/** 基准字号。 */
export const BASE_FONT_SIZE = 13;

/**
 * 基准行高。
 *
 * 24px 在 13px 字号下上下各留约 5px,既紧凑又不至于挤在一起。
 */
export const BASE_ROW_HEIGHT = 24;

/** 单元格左右内边距。 */
export const BASE_CELL_PADDING = 6;

/**
 * 单个半角字符的估算宽度(基准像素)。
 *
 * 内核返回的列宽单位是「半角字符数」,这里把它换算成像素。
 * 取 9 是照着实测的**最坏情况**定的 —— 13px 系统无衬线字体下,
 * 每个半角单位实际占:小写字母 ≈ 6.8px、汉字 ≈ 6.5px(每字 2 单位)、
 * 数字 ≈ 7.8px(最宽的 `0` 为 8.3px)、大写字母 ≈ 8.7px。
 *
 * 为什么宁可估宽:估窄了会让明明放得下的内容被打上省略号,
 * 那是实实在在的信息损失;估宽只是多几像素留白。
 * 极端情况(整列都是 `W`/`M`)仍会裁剪 —— 这时省略号与状态栏的完整值就是兜底。
 */
export const BASE_CHAR_WIDTH = 9;

/** 列宽下限 / 上限(基准像素)。 */
export const MIN_COL_WIDTH = 48;
export const MAX_COL_WIDTH = 420;

/** 列头(A/B/C…)高度。 */
export const BASE_HEADER_HEIGHT = 26;

/** 行头(行号)最小宽度。 */
export const MIN_HEADER_WIDTH = 44;

/** 缩放范围。 */
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 3;

/** 滚动条视觉尺寸。 */
export const SCROLLBAR_SIZE = 10;
export const SCROLLBAR_MIN_THUMB = 24;

/** 配色。与 App.css 的 GitHub 风格保持一致。 */
export const COLORS = {
  /** 单元格区域背景。 */
  cellBackground: "#ffffff",
  /** 网格线。 */
  gridLine: "#e6e8eb",
  /** 单元格文字。 */
  cellText: "#1f2328",
  /** 表头背景。 */
  headerBackground: "#f6f8fa",
  /** 表头文字。 */
  headerText: "#57606a",
  /** 表头分隔线 / 外边框。 */
  headerBorder: "#d0d7de",
  /** 选中所在行列的表头高亮。 */
  headerActiveBackground: "#ddf4ff",
  /** 选中所在行列的表头文字。 */
  headerActiveText: "#0969da",
  /** 首行(通常是表头行)的底色,便于一眼分辨。 */
  firstRowBackground: "#fbfcfd",
  /** 悬停单元格底色。 */
  hoverBackground: "rgba(9, 105, 218, 0.06)",
  /** 选中单元格边框。 */
  selectionBorder: "#0969da",
  /** 选中区域底色。 */
  selectionBackground: "rgba(9, 105, 218, 0.10)",
  /** 滚动条滑块。 */
  scrollbarThumb: "rgba(87, 96, 106, 0.45)",
  /** 滚动条轨道。 */
  scrollbarTrack: "rgba(208, 215, 222, 0.35)",
} as const;

/** 绘制文字用的字体族。与页面正文一致,避免中文回退到难看的字体。 */
export const FONT_FAMILY =
  '-apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", Helvetica, Arial, sans-serif';
