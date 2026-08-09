# 报告-0009:能力扩展(xlsx 网格 / 表格交互 / 公式跨表·具名 / Word·PPT 增强)

> ⚠️ **本文是带日期的快照**。下文的「仍未实现 / 当前边界」反映的是撰写当时的状态,
> 其中多数条目此后已经交付。**最新状态以 [AGENTS.md](../../AGENTS.md) 为准**。

- **关联**:RFC-0006、RFC-0004、[报告-0008](./0008-office-readonly-polish-2.md)
- **日期**:2026-08-08
- **状态**:已实现并通过验收(单测 + 自动化 e2e)

按优先级补齐四项能力,收敛「Excel 页只吃 CSV」等最违和的缺口。

## 1. .xlsx 接入网格(多工作表 / 值 / 公式)

- `core/xlsx.rs`:calamine 解析 xlsx → 每张工作表一张只读 `Sheet` + 公式清单。
  xlsx **自带缓存计算值**,故**不重算**——显示表直接取缓存值,公式原文经 `worksheet_formula`
  取出供公式栏。日期序列数自实现换算为 `YYYY-MM-DD[ HH:MM:SS]`(不引入 chrono,保 wasm 体积)。
- `wasm`:`WasmWorkbook`(`parse`/`sheetNames`/`sheet(i)`/`formulas(i)`),复用 `WasmSheet`。
- `web`:`loadXlsx` + `useXlsxFile` + `ExcelPage` 按扩展名路由 CSV/xlsx;**多工作表标签**切换;
  与 CSV 走同一渲染 / 公式栏 / 过滤 / 排序管线。

## 2. 表格交互:区域选择 + 复制 + 列宽拖拽 + 排序

- **排序**:`core/filter.rs::sort_rows`(数值感知比较、空值恒靠后、稳定排序,表头置顶),
  与过滤**复合**为同一「可视行→底层行」映射(`WasmSheet` 的 `filter`/`sort` 都走 `rebuild`)。
- **区域选择 + 复制**:渲染器 overlay 支持多格选区高亮(`setSelectionRange`);
  锚点+活动格模型,Shift+点击 / Shift+方向键扩选;Ctrl/⌘+C 复制为 TSV。
- **列宽拖拽**:`computeLayout` 增 `colWidthOverrides`(基准 px,放宽上限、仍受最小约束);
  渲染器 `columnResizeHitTest`/`setColumnWidth`,列头右边界拖拽,`col-resize` 光标。

## 3. 公式补齐:跨工作表引用 + 具名区域 + 多条件聚合

- **跨表引用**:词法加 `!` 与带引号表名 `'My Sheet'`;AST `CrossRef`/`CrossRange`;
  `Workbook` 持副表常量值(`set_sheet_input`),求值 `Sheet!A1` / `SUM(Sheet!A1:B2)`;
  缺失表得 `#REF!`。
- **具名区域**:AST `Name`;`Workbook::define_name(name, RangeRef)`;单格名→标量、区域名→数组;
  `SUM(名)` 等聚合可直接展开;未定义名得 `#NAME?`。
- **机械式扩函数**:新增 `AVERAGEIFS` / `MAXIFS` / `MINIFS`(多条件)。

## 4. Word 超链接 / 脚注 + PPT 组合形状

- **Word 超链接**:`ParagraphChild::Hyperlink` → rId 经 `document_rels` 解析为 URL(锚点 `#anchor`);
  `Run.link` 渲染蓝色 + 下划线。
- **Word 脚注**:`footnotes` part 经 serde 抽取文本,文末「n. 文本」汇总;引用处 `[n]` 标记。
- **PPT 组合形状**:`p:grpSp` 按 `chOff`/`chExt`→`off`/`ext` 缩放平移映射子坐标(支持嵌套)。

## 质量门禁

- `cargo fmt --check` ✅ · `clippy --all-targets -D warnings` ✅ · `cargo test --all` **254**
- `pnpm typecheck` ✅ · `pnpm -C web test`(vitest)**199** ✅ · `pnpm -C web build` ✅
- `pnpm -C web e2e`(Playwright chromium)**9/9** ✅(含 xlsx 多表 / 排序 / 区域选择复制)

## 复现

```bash
cargo test --all
wasm-pack build crates/wasm --target web --out-dir web/src/wasm/pkg --out-name office_wasm
pnpm -C web e2e:fixtures && pnpm -C web e2e
```

## 当前边界(暂不做)

- Excel:单元格样式/合并、按 numfmt 格式码渲染 xlsx(calamine 稳定 API 不直接给每格格式码)、
  `SUBTOTAL` 式「只算可见行」语义。
- 公式:动态数组溢出、engineering/database/cube 类别;具名区域/跨表引用**不参与本表增量脏区**
  (按需求值仍正确)。
- Word:批注、精确行距/缩进。PPT:动画时间线回放、图表/SmartArt 真实绘制、自定义几何。
