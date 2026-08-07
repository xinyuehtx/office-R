# Spec-0005: 视图层 —— 列过滤与冻结行列

- **关联**:RFC-0005、Story-0005
- **状态**:列过滤 已实现;冻结行列 计划中

## 1. 列过滤(已实现)

### 内核 `filter_rows(sheet, filters, header_rows) -> Vec<u32>`

- 返回命中行下标:顶部 `header_rows` 行始终保留,其后数据行需满足**全部**条件(AND)。
- 谓词:
  - `Values`:单元格文本(忽略大小写)∈ 集合;
  - `Text{op, needle}`:contains / notContains / equals / begins / ends(忽略大小写);
  - `Number{op, a, b}`:= ≠ > ≥ < ≤ between;非数值单元格仅满足 `≠`;
  - `Blank(bool)`:只留空白 / 只留非空白。
- `column_unique_values(sheet, col, header_rows, limit) -> (Vec<String>, truncated)`:值集 UI 用。

**验收**(见 `filter.rs` 单测):
- 无条件 → 全部行;`header_rows` 行恒保留。
- `金额>1000`、`between(800,1500)`、`城市=北京`、`Values{上海,深圳}`、`Blank`、多列 AND 均正确。
- 文本忽略大小写;唯一值去重且按上限截断。

### WASM `WasmSheet`(行映射)

- `row_map: Option<Vec<u32>>`;`rows()` 返回可视行数;`window(可视行区间…)` 经映射取底层行;
  `rowLabel(visual)` 返回底层行号;`filter(specsJson, header_rows)`、`clearFilter()`、
  `uniqueValues(col, header_rows, limit)`。
- **可视行始终连续 `0..V`**,渲染几何完全复用。

### 前端

- `SheetHandle` 增 `rowLabel/filter/clearFilter/uniqueValues`;`rows` 用实时 getter。
- 渲染器 `refreshRows()`:行集变化后重读行数、清缓存重画,**保留滚动/缩放**;行头经
  `rowLabelText` 显示**原始行号**。
- `FilterBar`:作用于当前选中列,选类型→填条件→应用;生效过滤以标签列出,可单独/整体清除。

**验收**(见 `FilterBar.test.tsx` + 浏览器):
- 选中列后标题显示列名;应用文本/数值过滤回传正确 spec;空输入不应用;
- 生效过滤显示标签、可清除;
- 端到端:公式示例选 B 列「数值 > 3」→ 可视行由 8 减到 5(表头 + 4 行),行头显示原始行号。

## 2. 冻结行列(计划中)

- 冻结顶部 `frozenRows` 行、左侧 `frozenCols` 列:四象限独立滚动偏移。
- 几何:`GridLayout` 增 `frozenRows/frozenCols` 及像素跨度;`bodySize/maxScroll/computeVisibleRange/
  hitTest/scrollbarGeometry/scrollIntoView` 相应扣除冻结带。
- 绘制:冻结象限用固定偏移;冻结分隔线加重;overlay/headers 冻结带不随滚动移动。
- UI:冻结首行 / 冻结首列 / 冻结到选区 / 取消冻结。

## 3. 非目标(本期)

- 排序;过滤下 `SUBTOTAL` 式「只算可见行」的公式语义(本期公式仍看全量)。
