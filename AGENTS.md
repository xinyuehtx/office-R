# AGENTS.md

> 本文件是 office-R 项目的**单一事实来源(single source of truth)**,面向所有人类与 AI 协作者。
> 描述项目定位、架构、开发命令、工作流与约定。修改架构或流程时,请同步更新本文件。

## 项目简介

office-R 是一个**统一的 Office 三件套应用**(文档 Word / 表格 Excel / 演示 PowerPoint):

- **视图层 = Web**:React + Vite + TypeScript(使用 **pnpm** 管理)。
- **计算内核 = Rust**:编译为 **WASM**,在浏览器内识别与(未来)解析 office 文件。
- **部署**:纯静态产物 → **GitHub Pages**,无需服务器。

当前能力:

- **表格页(本期重点)**:上传 **CSV** → WASM 解析(编码探测 / 分隔符嗅探 / 列切分)→ **canvas 表格视图**,支持滚动、缩放、键盘与拖拽。视图由**三张堆叠的 canvas** 组成(单元格 / 表头 / 覆盖层),叠加交给浏览器合成器用 GPU 完成;单元格层是超出视口一圈的瓦片,多数滚动帧只改 CSS transform、主线程零绘制。50 万行流畅浏览。见 [RFC-0003](./docs/rfcs/0003-csv-canvas-grid.md)。
- **表格数据源:CSV + .xlsx**:CSV 走 Worker 紧凑缓冲解析;**.xlsx** 由 `office-excel`(calamine)解析成**多工作表**(显示缓存计算值、公式栏回显原文、日期序列数自换算),`WasmWorkbook` 暴露、`ExcelPage` 按扩展名路由并提供**工作表标签切换**。
- **列过滤 + 排序 + 冻结行列 + 数字格式化 + 区域选择/复制/列宽拖拽(视图层)**:列过滤(文本/数值/值集/空白,多列 AND)与**排序**(数值感知、空值靠后、稳定)复合在 `office-core` 的 `filter.rs`,`WasmSheet` 持「可视行→底层行」映射(可视行连续 `0..V`,渲染几何复用);**冻结行列**在 `GridLayout` 记录冻结跨度,冻结时走四象限全量重绘隔离路径;**区域选择**(Shift+点击/方向键)+ **复制 TSV**(Ctrl/⌘+C)+ **列宽拖拽**(`colWidthOverrides`)+ **查找**(Ctrl/⌘+F,受过滤/排序影响);**数字格式化** `office-core` 的 `numfmt.rs`。见 [RFC-0005](./docs/rfcs/0005-view-filter-freeze.md) / [RFC-0006](./docs/rfcs/0006-word-excel-ppt-readonly.md)。
- **公式计算引擎(Rust 侧)**:以 `=` 开头的单元格按 **Excel 语义**求值(词法 → 语法 → 求值 + 值层 `Workbook`),对齐 Excel 的运算符优先级、错误值(`#DIV/0!` 等)、类型强制与循环检测,内置 **160+ 函数**(math/stats/logical/text/date/lookup/info/financial/engineering/dynamic;准确数量以 `formula::functions::count()` 为准,别在文档里写死),含**跨工作表引用** `Sheet!A1`、**具名区域** `define_name`。之上是**计算管线**:依赖图(前驱/后继)+ 脏区跟踪 + 拓扑序增量重算(计算合并)+ 循环更新策略(默认 `#REF!`,可开启 Jacobi 迭代计算)。表格页显示计算值、公式栏回显原始公式。参考 HyperFormula / Univer 的函数目录,Rust 自研。见 [RFC-0004](./docs/rfcs/0004-formula-engine.md)。
- **文档页(Word 只读)**:`office-word` 用 docx-rs 读路径抽出平面化模型(段落/标题/对齐/**有序无序列表(查 numbering.xml)**/图片/表格/**分栏 sectPr、页眉页脚、修订 ins/del**),`office-word` 流式布局 + sticky canvas 纵向虚拟化渲染(文字/加粗斜体/颜色/对齐含**两端对齐**/列表/图文混排/表格/**分栏贪心分配、页眉页脚分隔线、修订着色+删除线**)。见 [RFC-0006](./docs/rfcs/0006-word-excel-ppt-readonly.md)。
- **演示页(PPT 只读)**:`office-ppt` 用 zip+quick-xml 直接解析 PresentationML → 幻灯模型(文本框/图片/自选图形/对齐,EMU÷9525,**占位符继承版式/母版几何**、**主题 schemeClr**、**旋转/翻转**、**母版文本默认样式继承**、**组合形状/渐变/内嵌表格/内嵌图表**、**graphicFrame SmartArt 占位**、**timing/transition 动画时间线**(`clickEffect` → 形状 `appear_step` + `build_steps`,切换类型 `transition`)),`office-ppt` canvas 渲染(含旋转仿射、真实表格/图表、SmartArt 虚线占位、动画/切换徽标) + 缩略图导航 + 缩放 + **全屏演示模式**(逐步播放入场动画 + 切换效果)。见 [RFC-0006](./docs/rfcs/0006-word-excel-ppt-readonly.md)。
- **Excel 数字格式化(numfmt)**:支持四段 `正;负;零;文本`、占位符/千分位/百分比/货币/小数,以及**颜色码 `[Red]`、条件段 `[>=100]`、分数 `?/?`**。
- **wasm 瘦身**:`docx-rs` 主依赖 `default-features=false`(image 特性移到 dev-dependency),WASM 体积 −38%。
- **共享文本测量**:`office-shared` 的 `textMeasure.ts` 参考 pretext(canvas measureText + `font→segment` 两级缓存 + 字体加载失效 + 二分裁剪 + 折行),三页共用。

解析失败优雅降级:错误类型清晰(`thiserror`)、提示中文可操作、可重新选择文件重试。

## 架构

分层与数据流详见 [docs/architecture.md](./docs/architecture.md)。核心分层:

**monorepo**:三个应用(Word/Excel/PPT)各自是可独立使用的单元 —— 一个解析 crate
+ 一个 wasm cdylib + 一个 npm 包。`web/` 只是把三者组合的演示壳。见 [RFC-0007](./docs/rfcs/0007-monorepo-split.md)。

```
packages/{word,excel,ppt}  ──wasm-bindgen──▶  crates/{word,excel,ppt}-wasm  ──▶  crates/{word,excel,ppt} 解析
       (npm 包,各含 pkg/)                        (三个独立 cdylib)              ──▶  crates/core(仅 excel 依赖)
```

- `crates/core`(`office-core`):表格内核,**不含任何格式解析依赖**,可原生单测。
- `crates/ooxml`(`office-ooxml`):chart + OPC/XML 共享原语,xlsx/pptx/word 共用。
- `crates/{word,excel,ppt}`:三个格式解析 crate;**word/ppt 不依赖 core**,只有 excel 依赖。
- `crates/{word,excel,ppt}-wasm` + `crates/wasm-log`:每应用一个 cdylib,三份独立 wasm。
- `packages/*`:pnpm workspace 前端包(`@tengxiaohyx/office-{shared,word,excel,ppt}`),`exports` 直指源码 `.ts`。
- `web/`:演示壳,`import { WordPage/ExcelPage/PptPage }` 自三个包。

**重 CPU 逻辑一律在 Rust/WASM 侧**:文件解析、列切分、列宽度量都在 `office-core`;
前端只负责「取可见区域 + 绘制」。CSV 解析跑在 Web Worker 里,主线程不被阻塞。

**核心依赖**(选型理由见 [RFC-0002](./docs/rfcs/0002-core-dependencies.md) 与 [RFC-0003](./docs/rfcs/0003-csv-canvas-grid.md)):
`calamine`(xlsx)、`docx-rs`(docx)、`zip`(仅 `deflate`)+ `quick-xml`(pptx)、
`csv` + `encoding_rs` + `chardetng`(CSV 解析与编码)、`thiserror`。
**公式引擎零新增依赖**:词法/语法/求值/函数库全部只用标准库,不给 WASM 体积加负担。
**不引入 tokio**:浏览器 wasm 下 tokio 基本不可用,office 解析为同步 CPU 密集型。

## 目录结构

```
crates/core/        office-core 表格内核(format/sheet/csv/formula/filter/numfmt/serial/limits)
  src/formula/      公式引擎:token/parser/ast/eval + functions/(math/stats/logical/…)
crates/ooxml/       office-ooxml:chart + OPC/XML 共享原语(local/attr/mime_of/emu/rels)
crates/word/        office-word:docx.rs(docx-rs 读路径 → 平面化模型)
crates/excel/       office-excel:xlsx.rs(calamine + 自解析样式/合并/图片/图表/迷你图)
crates/ppt/         office-ppt:pptx.rs(zip+quick-xml → 幻灯模型)
crates/{word,excel,ppt}-wasm/  三个独立 cdylib(WasmWordDoc / WasmSheet+WasmWorkbook / WasmPresentation)
crates/wasm-log/    三个 cdylib 共用的分级日志桥
crates/xtask/       e2e 夹具生成 / 漂移校验
packages/shared/    @tengxiaohyx/office-shared:logger/textMeasure/chartDraw/FileUpload/fonts/page.css/testing
packages/word/      @tengxiaohyx/office-word:model + wasm 加载器 + WordPage + wordLayout + pkg/
packages/excel/     @tengxiaohyx/office-excel:SheetHandle + grid(geometry/tile/layers/renderer)+ SheetCanvas + csvWorker + pkg/
packages/ppt/       @tengxiaohyx/office-ppt:model + wasm 加载器 + PptPage + slideRender + pkg/
web/                演示壳:App.tsx Tab + main + 外壳 CSS + App.test
  e2e/              Playwright 浏览器 e2e(word/excel/ppt spec + fixtures)
docs/               workflow / architecture / rfcs / specs / stories / reports
.github/workflows/  ci.yml(CI)、deploy-pages.yml(Pages 部署)
```

## 开发环境

- Rust(stable,含 `wasm32-unknown-unknown` target,见 `rust-toolchain.toml`)
- `wasm-pack`:`curl -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh`
  (注:`cargo install wasm-pack` 在旧版 cargo 上会因 edition2024 失败,请用上面的预编译安装脚本)
- Node ≥ 22、pnpm ≥ 10

## 常用命令

```bash
# Rust 内核
cargo test --all                 # 单元测试
cargo fmt --all                  # 格式化
cargo clippy --all-targets -- -D warnings

# 构建三份 WASM(每应用一份,产物落到各自包的 pkg/)
for a in word excel ppt; do \
  wasm-pack build crates/$a-wasm --target web --out-dir ../../packages/$a/pkg --out-name office_${a}_wasm; done

# Web / 包(在仓库根运行,pnpm 工作区)
pnpm install
pnpm typecheck                   # 所有包(pnpm -r --parallel)
pnpm lint                        # 全仓(根 eslint.config.js,--max-warnings 23)
pnpm test                        # vitest workspace(所有包一次跑)
pnpm -C web dev                  # 演示站本地开发
pnpm -C web build                # 演示站产物在 web/dist
pnpm e2e:fixtures                # 生成 e2e 夹具(需 Rust)
pnpm -C web e2e                  # 浏览器 e2e(Playwright chromium)
```

> 首次运行前需先构建三份 WASM 生成 `packages/{word,excel,ppt}/pkg`,否则前端无法解析引用。
>
> **改动某个格式的公共数据结构后必须重新构建对应的 wasm**(如改 xlsx 类型 →
> `crates/excel-wasm`):前端引用的是 `pkg/` 里的二进制产物,忘记重建会让前端拿着旧内核跑,
> 症状通常是「内容错位」这类很难定位的问题。

## 工作流(SDD + TDD)

完整流程见 [docs/workflow.md](./docs/workflow.md)。要点:

1. 需求 → 写 **RFC**(`docs/rfcs/`)供评审。
2. 写 **Spec / Story / 失败测试**(先行)。
3. 按 **SDD + TDD** 实现,让测试变绿。
4. **CI 全绿** → 合并 `main` → **自动部署 Pages**。

### 提交前检查清单(强制)

- [ ] `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test` 通过
- [ ] `pnpm typecheck && pnpm lint && pnpm test && pnpm -C web build` 通过(根级,覆盖所有包)
- [ ] **更新项目网站**:功能变化已反映到对应包,`pnpm -C web dev` 验证可用
- [ ] **更新架构文档**:变化已同步到 [docs/architecture.md](./docs/architecture.md) 与本文件
- [ ] 相关 RFC / Spec / Story 已同步

> 「每次提交前更新项目网站与架构文档」是硬性要求:代码、部署页面、架构描述三者不得脱节。

## 代码约定

- **Rust**:遵循 `rustfmt` 默认风格;公共 API 写文档注释(`///`);`office-core` 不引入浏览器/OS 依赖。
- **TypeScript**:严格模式;组件用函数式;共享逻辑抽到 `@tengxiaohyx/office-shared`。
- **注释与文档以中文为主**,与团队沟通语言一致;注释解释**为什么**这么做,而不是复述代码。
- 单元测试与被测代码就近(Rust 用 `#[cfg(test)]`,前端用同目录 `*.test.tsx`)。
- **日志**:统一走 `office-shared` 的 `logger.ts`(前端)与 `office-wasm-log`(内核),
  两侧格式一致、共用 traceId;**绝不打印用户文件内容**。禁止把裸 `console.log` 留在代码里。

## 提交规范

- 从 `main` 切功能分支,PR 合并回 `main`。
- 提交信息简洁的祈使句,一次提交聚焦一件事。
