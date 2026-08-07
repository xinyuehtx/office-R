# RFC-0005: 表格视图层 —— 列过滤与冻结行列

- **状态**:草稿
- **作者**:office-R team
- **创建日期**:2026-08-07
- **关联**:Spec-0005、Story-0005、[RFC-0003](./0003-csv-canvas-grid.md)

## 动机(为什么)

表格页已能渲染大表并求值公式,但**只能看全量、只能平铺滚动**。真实使用中两件事最刚需:

- **列过滤**:几十万行里只想看「城市=北京 且 金额>1000」的那些行;
- **冻结行列**:横竖滚动时,表头行 / 首列(名称列)要一直可见,否则滚远了就不知道每列是什么。

本 RFC 在视图层加这两项,并坚持「**重 CPU 在 Rust**」:过滤是一次全表扫描(百万行级),放 WASM;
冻结是纯绘制,放前端渲染器。

## 方案(做什么、怎么做)

### 一、列过滤(filter)

**内核**(`crates/core/src/filter.rs`):给定 `Sheet` + 一组列过滤条件,返回**命中的行下标**。

```rust
pub struct ColumnFilter { pub col: u32, pub predicate: Predicate }
pub enum Predicate {
    Values(Vec<String>),                 // 值集:单元格文本 ∈ 集合(忽略大小写)
    Text { op: TextOp, needle: String }, // 文本:contains/equals/begins/ends/not_contains
    Number { op: NumOp, a: f64, b: f64 },// 数值:= <> > >= < <= between(用 a,b)
    Blank(bool),                         // true=只留空白;false=只留非空白
}
/// header_rows:顶部始终保留的行数(如表头行不参与过滤)。
pub fn filter_rows(sheet: &Sheet, filters: &[ColumnFilter], header_rows: u32) -> Vec<u32>;
```

- 多列条件按 **AND** 组合(与 Excel 一致)。
- 返回的行下标 = 顶部 `header_rows` 行 + 其后所有满足全部条件的数据行,**保持原顺序**。

**跨边界与视图映射**:关键设计是让**渲染器几乎无感**。`WasmSheet` 内部持有一个可选
`row_map: Option<Vec<u32>>`(可视行 → 底层行):

- `filter(specsJson)`:在 WASM 侧算出命中行,存进 `row_map`;
- `clearFilter()`:清空 `row_map`;
- `rows()`:有 map 时返回 `map.len()`,否则原行数;
- `window(r0,r1,c0,c1)`:把可视行 `r0..r1` 逐行经 `row_map` 映射到底层行再取数;
- `rowLabel(visualRow)`:返回该可视行对应的**原始行号**(过滤后行头仍显示原行号,像 Excel)。

因为可视行始终是**连续 `0..V`**,现有渲染几何(等高行、前缀和列宽)**完全复用**,
过滤对渲染器透明。前端 `SheetHandle` 增补 `rowLabel(visual)` 用于行头。

过滤条件从前端以紧凑 JSON 传入(避免为复杂枚举写一堆 wasm-bindgen 绑定):

```json
[{"col":2,"kind":"number","op":">","a":1000,"b":0},
 {"col":0,"kind":"values","values":["北京","上海"]}]
```

### 二、冻结行列(freeze panes)

**渲染器**(`geometry.ts` / `renderer.ts`):支持冻结顶部 `frozenRows` 行、左侧 `frozenCols` 列。
视口被分成四个象限,各自的滚动偏移不同:

```
        │ 冻结列(x 不滚)      滚动列(x 滚)
────────┼────────────────────────────────────
冻结行  │  角(都不滚)         只滚 x
(y 不滚) │
────────┼────────────────────────────────────
滚动行  │  只滚 y               滚 x、y(主体)
(y 滚)  │
```

- 几何:新增 `frozenWidth`(前 `frozenCols` 列宽之和)、`frozenHeight`(`frozenRows*rowHeight`);
  主体可滚区域 = `body - frozen`;`maxScroll` 相应减去冻结带。
- 绘制:单元格层按象限分别绘制(冻结象限用固定偏移),或在主体之上再叠加冻结带的绘制。
  冻结分隔线加一道略重的边,提示「这里冻住了」。
- 命中 / 键盘滚动 / 滚动条:落在冻结带的坐标不参与滚动换算;`scrollIntoView` 目标在冻结带内则不滚动。

UI:工具条按钮「冻结首行 / 冻结首列 / 冻结到选区 / 取消冻结」。

## 取舍与备选方案

- **过滤:行映射 in WasmSheet vs 生成新 Sheet**:选前者 —— 不复制单元格数据(百万行不翻倍内存),
  切换/清除过滤是 O(命中行) 而非重新解析。
- **过滤放 Rust vs JS**:全表扫描是重 CPU,且数据本就在 WASM 线性内存里,放 Rust 天然且快。
- **冻结:四象限独立绘制 vs 多个 DOM/canvas**:沿用「单 body 层 + 变换」的思路,冻结带在同一
  绘制流程里用不同偏移画,避免再引入额外图层与合成复杂度。
- **排序**:本期**不做**(排序要么稳定重排行映射、要么重排数据,交互也更重),留作后续;
  过滤的行映射机制已为将来「排序 = 另一种行映射」预留。

## 影响

- `crates/core`:新增 `filter` 模块(无新依赖)。
- `crates/wasm`:`WasmSheet` 增 `filter/clearFilter/rowLabel` 与行映射;新增按列取「唯一值」用于值集 UI。
- `web/`:`SheetHandle` 增 `rowLabel`;渲染器支持冻结;表格页加过滤菜单与冻结控制。
- 文档:architecture / AGENTS / README 同步。

## 未决问题

- 排序(升/降、多列)。
- 值集 UI 在超大表上的唯一值枚举成本(可上限截断 + 搜索)。
- 过滤与公式的交互:被过滤隐藏的行仍参与公式计算(与 Excel 的 `SUBTOTAL` 区别),本期保持「公式看全量」。
- 冻结与缩放/非整数 dpr 的像素对齐细节。
