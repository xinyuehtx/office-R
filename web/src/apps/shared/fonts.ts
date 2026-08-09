/**
 * 字体常量。
 *
 * 独立成文件是为了切断一条方向错误的依赖:此前 `FONT_FAMILY` 住在
 * `apps/excel/grid/theme.ts` 里,于是 Word 的布局与 PPT 的绘制都要
 * `import ... from "../excel/grid/theme"` —— 这是三个应用之间**仅有的**两条
 * 实质耦合,也会让 word/ppt 包必须依赖 excel 包。
 *
 * 它本质是文本度量的关注点(与 `textMeasure.ts` 同层),不是表格网格的主题。
 */

/** 绘制文字用的字体族。与页面正文一致,避免中文回退到难看的字体。 */
export const FONT_FAMILY =
  '-apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", Helvetica, Arial, sans-serif';
