# 报告-0010:只读深化(公式函数扩展 / 表格查找 / xlsx 按格 numfmt)

- **关联**:RFC-0004、RFC-0006、[报告-0009](./0009-capability-expansion.md)
- **日期**:2026-08-08
- **状态**:本批已实现并通过验收(单测 + e2e);后续项见文末

在**保持只读**的前提下,补齐用户清单中最高价值、可干净落地的部分。

## 1. 公式函数扩展(#44)

- **TEXT** 接 `numfmt` 内核:按格式码把数值格式化成文本(文本原样返回)。
- **SUBTOTAL**:`function_num` 1-11 / 101-111 分派(AVERAGE/COUNT/COUNTA/MAX/MIN/PRODUCT/STDEV/STDEVP/SUM/VAR/VARP)。
- **工程函数** `engineering.rs`:`DEC2BIN/OCT/HEX`、`BIN/OCT/HEX2DEC`(10 位补码)、`BITAND/OR/XOR/LSHIFT/RSHIFT`。
- **现代动态数组** `dynamic.rs`:`XLOOKUP` / `UNIQUE` / `SORT` / `FILTER`(含内联比较广播掩码)/ `SEQUENCE`,
  返回 `Value::Array`;作**中间值**可用(如 `SUM(UNIQUE(...))`),溢出到相邻格(spill)暂不支持。

## 2. Excel 网格内查找(#45)

- `WasmSheet::find(needle, caseSensitive, wholeCell, limit)` 返回命中**可视坐标**(受过滤/排序影响,搜当前所见)。
- `SheetCanvas`:Ctrl/⌘+F 打开查找栏,输入即时匹配 + `n/总数` + 上一个/下一个(Enter/Shift+Enter)+ 命中选中滚入视野 + Esc 关闭。

## 3. xlsx 按格 numfmt + 合并区解析(#46)

- 自解析 `xl/styles.xml`(`numFmts` 自定义 + `cellXfs` 的 `numFmtId`,含内置 id 子集)与
  `sheetN.xml` 每格 `s` 索引 + `mergeCells`;工作表名→路径经 `workbook.xml` + rels。
- 数值格按 numfmt 码渲染(**百分比 / 千分位 / 货币 / 小数**),日期仍走 ISO 换算。
- 合并区解析进 `XlsxSheet::merges`(渲染待后续)。显示改进经既有管线直达前端。

## 质量门禁

- `cargo test --all` **262** ✅ · `fmt` ✅ · `clippy -D warnings` ✅
- `pnpm test` **199** ✅ · `pnpm e2e` **10/10** ✅(含 Ctrl+F 查找)

## 仍未实现(后续,均保持只读)

- **Excel**:xlsx 字体/填充/边框**视觉样式渲染**、合并区**跨格呈现**(已解析)、内嵌图片/图表/迷你图;
  「粘贴/单元格编辑」与只读定位冲突,**不做**(查找已覆盖其只读价值)。
- **Word**:批注、目录/域(TOC/fields)、文本框/绘图形状、公式对象(OMML)、精确行距/缩进/制表位。
- **PPT**:图表/SmartArt 真实绘制、动画时间线回放、幻灯内表格真实渲染、渐变/图片填充、
  阴影效果、自定义几何 `custGeom`、文本框 autofit。
- **通用**:Word/PPT 缩放控件、打印/导出 PDF、文档内全文查找。
