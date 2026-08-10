# 架构文档

> 本文件描述 office-R 的整体架构。**每次提交前若架构或目录发生变化,必须同步更新本文件。**

## 定位

office-R 是一个**统一的 Office 三件套应用**(文档 / 表格 / 演示),特点:

- **视图层 = Web**:React + Vite + TypeScript,表格视图用 **canvas** 绘制。
- **计算内核 = Rust**:编译为 **WASM**,在浏览器内识别与解析 office 文件。
- **纯静态部署**:产物为静态资源,部署到 GitHub Pages,无需后端服务器。

## 分层

这是一个 **monorepo**:三个应用(Word / Excel / PPT)各自是可独立使用的单元 ——
一个 Rust 解析 crate + 一个 wasm cdylib + 一个 npm 包。演示站 `web/` 只是把三者
组合起来的壳。拆分动机与依赖测绘见 [RFC-0007](./rfcs/0007-monorepo-split.md)。

```
演示站 web/(React + Vite,只有壳:App.tsx Tab + main + 外壳 CSS)
  └─ import { WordPage / ExcelPage / PptPage } from 三个 npm 包

npm 包(packages/,exports 直指源码 .ts,pnpm workspace)
  ├─ @tengxiaohyx/office-shared   logger / textMeasure / chartDraw / FileUpload
  │                               / fonts / page.css / testing(三应用共用叶子)
  ├─ @tengxiaohyx/office-word     model + wasm 加载器 + WordPage + 布局
  ├─ @tengxiaohyx/office-excel    SheetHandle + grid 渲染管线 + SheetCanvas
  │                               + CSV/xlsx wasm 加载器 + csvWorker
  └─ @tengxiaohyx/office-ppt      model + wasm 加载器 + PptPage + slideRender
        │  每个应用包内含自己的 pkg/(wasm-pack 产物,不入库)
        ▼ wasm-bindgen
wasm cdylib(crates/,每应用一份独立产物)
  ├─ office-word-wasm    WasmWordDoc          → office-word
  ├─ office-excel-wasm   WasmSheet/WasmWorkbook/parseCsvPacked → office-excel + office-core
  ├─ office-ppt-wasm     WasmPresentation     → office-ppt
  └─ office-wasm-log     三个 cdylib 共用的 console 日志桥(格式一致)
        │  纯 Rust 调用
        ▼
解析 crate(crates/)
  ├─ office-word    docx.rs  → docx-rs      (不依赖 office-core)
  ├─ office-excel   xlsx.rs  → calamine     (依赖 office-core + office-ooxml)
  ├─ office-ppt     pptx.rs               (只依赖 office-ooxml)
  ├─ office-ooxml   chart + OPC/XML 共享原语(local/attr/mime_of/emu/rels)
  └─ office-core    表格内核:sheet / csv / formula / numfmt / serial /
                    limits / filter / format(无任何格式解析依赖)
```

**关键设计**:
- **依赖是无环 DAG,且严格隔离**:`office-core` 不含 calamine/docx-rs/quick-xml/zip;
  Word 与 PPT **不依赖** `office-core`(它们与表格内核无关);只有 Excel 需要内核。
  `cargo tree` 可逐条验证。
- **每应用一份 wasm**:只装 `@tengxiaohyx/office-excel` 的消费方不会拿到 docx-rs
  或 pptx 的字节(word 0.68M / excel 0.83M / ppt 0.25M,拆分前单体是 1.20M)。
- **CSV 与 xlsx 同在 office-excel-wasm**:`WasmWorkbook::sheet(i) -> WasmSheet`
  要求两者在同一个 wasm 模块里(wasm-bindgen 类型不能跨模块实例传递)。
- `office-core` 平台无关、可原生 `cargo test`;wasm 层只做类型转换与跨边界搬运。

## 核心依赖

| 用途 | crate | 许可证 | 备注 |
| --- | --- | --- | --- |
| xlsx 读取 | `calamine` | MIT | 纯 Rust,已验证 wasm 可用 |
| docx 读取 | `docx-rs` | MIT | 官方支持 WebAssembly |
| pptx / 容器 / XML | `zip`(仅 `deflate`)+ `quick-xml` | MIT | `zip` 关默认 features 以兼容 wasm |
| **CSV 解析** | `csv` | Unlicense OR MIT | Rust 事实标准;错误带行号 |
| **编码解码** | `encoding_rs` | (Apache-2.0 OR MIT) AND BSD-3-Clause | Firefox 同款,覆盖 GBK/Big5 等 |
| **编码探测** | `chardetng` | Apache-2.0 OR MIT | 与 `encoding_rs` 配套 |
| 错误 | `thiserror` | MIT OR Apache-2.0 | 轻量错误定义 |

选型对比与理由见 [RFC-0002](./rfcs/0002-core-dependencies.md) 与
[RFC-0003](./rfcs/0003-csv-canvas-grid.md)。

> **异步**:主目标是浏览器 wasm,tokio 在此基本不可用,故**不引入 tokio**;
> office 解析是同步 CPU 密集型任务,并发靠 Web Worker 解决。

## 数据流一:文档解析(Word / PPT / xlsx)

1. 用户在页面选择 `.docx/.xlsx/.pptx`,读为 `Uint8Array`;
2. 调用对应的 WASM 入口(`WasmWordDoc.parse` / `WasmWorkbook.parse` /
   `WasmPresentation.parse`),内核解析出**可直接渲染的模型**;
3. 图片字节留在 WASM 内存,前端按 id 取出造 object URL(不走 base64),
   canvas `drawImage` 直接用;句柄 `dispose()` 时统一 revoke,
   加载**中途失败也会 revoke 已建的 URL**(否则调用方拿不到句柄就永远泄漏);
4. 页面按模型绘制(Word 流式布局 + 虚拟化;PPT `fitScale` + `drawSlide`;
   xlsx 复用下面的表格渲染管线)。

> 早期还有一条「识别 → 产出摘要文本」的轻量路径(`core::render` +
> `shared/OfficePage`)。三页都长出真实渲染器后它就没有调用方了,已整链删除。

## 数据流二:表格渲染(CSV / xlsx)

```
① 选择文件            useCsvFile:生成 traceId,读为字节
② Worker 解析         csvWorker → parseCsvPacked
                      解码(BOM/UTF-8/GBK…)→ 嗅探分隔符 → csv crate 切分
                      → 紧凑存储 → 逐列度量显示宽度            ← 重 CPU,全在 Rust
②' 公式求值(如有)    若含 `=` 单元格 → 建 Workbook → 求值 → 公式格
                      换成计算值(显示表),公式原文单独回传   ← 无公式则零开销
③ 零拷贝转移          postMessage(transfer):text / cellEnds /
                      rowStarts / colWidthUnits 四个 ArrayBuffer 直接过户
                      + formulas(公式原文,数据量小,结构化克隆)
④ 主线程装配          WasmSheet.fromPacked → 表格常驻 WASM 线性内存
⑤ 绘制                GridRenderer 每帧只取「可见矩形」的单元格并绘制
                      公式栏用 SheetHandle.formula(r,c) 回显选中格的原始公式
⑥ 过滤(可选)        WasmSheet.filter 在内核侧扫描 → 存「可视行→底层行」映射;
                      可视行连续 0..V,渲染几何复用,行头经 rowLabel 显示原始行号
```

### 列过滤(视图层,见 [RFC-0005](./rfcs/0005-view-filter-freeze.md))

重扫描在 `crates/core/src/filter.rs`(文本/数值/值集/空白,多列 AND)。关键设计:`WasmSheet`
内部持「可视行 → 底层行」映射,`rows()`/`window()` 据此重映射,**可视行始终连续 `0..V`**,
所以渲染器的几何(等高行、列前缀和)**完全复用**,过滤对渲染器透明 —— 只有行头标签改为经
`rowLabel` 显示原始行号。前端 `FilterBar` 收集条件,`renderer.refreshRows()` 保留滚动/缩放刷新。
**冻结行列**(见 [RFC-0006](./rfcs/0006-word-excel-ppt-readonly.md)):`GridLayout` 记录
`frozenRows/Cols` 与像素跨度;冻结时走**四象限全量重绘**隔离路径(不碰 50 万行滚动的瓦片
热路径),`cellScreenRect`/`hitTest`/表头/覆盖层均冻结感知。数字格式化在 `core/numfmt.rs`。

**为什么分两段**:解析必须离开主线程(否则大文件冻住 UI),而绘制取数必须同步
(否则掉帧)。两段之间只有「移出 wasm 堆」和「移入 wasm 堆」两次必要拷贝,
表格内容不会以 JS 字符串数组的形式整体存在。

### Sheet 的紧凑表示

不用 `Vec<Vec<String>>`(每格一次堆分配),而是:

```
text:       所有单元格文本按行优先首尾相接(一次大块分配)
cell_ends:  每个单元格的结束偏移        每格仅 4 字节开销
row_starts: 每行首个单元格的下标
```

窗口取数返回「一个大字符串 + 偏移数组」,偏移以 **UTF-16 码元**计,
让 JS 侧可以直接 `slice`(用字节偏移会让含中文的单元格全部错位)。

## canvas 渲染管线

位于 `packages/excel/src/grid/`,分四段,每段可独立验证:

```
① 数据    SheetHandle + 按瓦片取数的窗口缓存(比瓦片再大两圈,便于增量复用)
② 几何    geometry.ts —— 纯函数:列偏移前缀和、二分定位、可见区域、
          命中判定、滚动条几何、指针锚点缩放      ← 全部有单测
③ 裁剪    tile.ts —— 瓦片是否还盖得住可见区域;planScrollBlit 脏矩形
④ 绘制    layers.ts —— paintBody / paintHeaders / paintOverlay
```

### 三张堆叠的 canvas + GPU 合成

表格里三类内容的**变化频率相差两个数量级**,所以拆成三张独立的 DOM canvas
(而不是一张画布反复重画):

| 图层 | z | 内容 | 何时重画 |
| --- | --- | --- | --- |
| `body` | 1 | 单元格文本 + 网格线(**最贵**) | 数据/缩放变化,或滚出瓦片余量 |
| `headers` | 2 | 行列头(便宜) | 滚动、缩放、选区变化 |
| `overlay` | 3 | hover / 选中 / 滚动条(最便宜) | 鼠标一动就变 |
| `surface` | 10 | 透明 `div`,承接事件与无障碍语义 | — |

```
.sheet__viewport (contain: layout paint)
  ├─ div(裁剪容器,定位到单元格区域,overflow:hidden)
  │    └─ canvas[data-layer=body]     ← 瓦片,靠 CSS transform 平移
  ├─ canvas[data-layer=headers]
  ├─ canvas[data-layer=overlay]
  └─ div.sheet__surface               ← pointer/keyboard 都打在这里
```

三张画布的**叠加交给浏览器合成器(GPU)**,主线程不再每帧做一次全视口的
`drawImage` 合成。画布一律 `pointer-events: none`;`will-change: transform`
只加在真正会平移的 `body` 层(这个属性会让元素常驻显存,不该到处加)。

### 滚动为什么几乎不用画

`body` 是一块**比可见区域四周各大 256 设备像素**的瓦片(见 `tile.ts`):

- 滚动没超出这圈余量 → 只改 `transform: translate3d(...)`,**主线程零绘制**;
- 滚出余量 → 重新锚定瓦片(可见区域居中),用位图平移复用重叠部分,只补新露出的窄带;
- 跳转过远 / 数据变化 → 整块重绘。

代价是画布内存变大(多出边距那一圈),换来绝大多数滚动帧的绘制开销归零。
瓦片原点与滚动量都取整到整设备像素,平移不会重采样发虚。

### 其它

- **单帧合并**:所有 `set*` 只打脏标记,由一次 `requestAnimationFrame` 统一出帧;
- **设备像素坐标**:渲染器内部统一按 CSS 像素 × dpr 工作,网格线永远压在像素中心;
  画布的后备像素与显示尺寸**同源换算**(先把后备像素向下取整到整设备像素,
  再反推显示尺寸),这样非整数 dpr(浏览器缩放、125%/150% 显示缩放)下
  位图也不会被重采样发虚;
- **取数按瓦片而非可见区域**:瓦片比视口大,若按可见区域取数,瓦片边缘那圈会画成
  空白,一平移就穿帮;
- **输入换算**:滚轮的 `deltaMode`、触控板双轴、Shift 横滚、Ctrl 缩放都收在
  `grid/input.ts` 的纯函数里,可单测。

DOM 只承载:容器、交互层、状态栏,以及供读屏软件播报当前单元格的 live region。
**没有一个单元格是 DOM 拼出来的。**

## 可观测性

前端 `office-shared` 的 `logger.ts` 与 `office-wasm-log` **同格式**输出:

```
[office-R][web ][info][a1b2c3] file.open name=demo.csv bytes=41672736
[office-R][wasm][info][a1b2c3] csv.parse.ok bytes=41672736 rows=500001 cols=12 ms=451.0
[office-R][web ][info][a1b2c3] sheet.firstFrame ms=2.8 rows=500001 cols=12
```

同一次打开共用 `traceId`,两侧日志可串联。级别 `debug/info/warn/error/off`,
默认开发 `info` / 生产 `warn`,可用 `?logLevel=debug` 或
`localStorage.officeR.logLevel` 调整;级别变化会同步给 WASM 侧。
**日志绝不打印单元格内容**(`Sheet` 的 `Debug` 也只输出维度)。

## 公式引擎(`crates/core/src/formula/`)

一条经典**解释器管线** + 一个独立的**值/公式层**,语义对齐 Excel(详见 [RFC-0004](./rfcs/0004-formula-engine.md)):

```
公式文本 "=SUM(A1:A3)*2"
  token.rs   词法:数字/字符串/布尔/错误/引用/运算符/函数
  parser.rs  语法:Pratt 优先级爬升(: > 一元 - > % > ^ > * / > + - > & > 比较)→ AST
  eval.rs    求值:错误传播 + 类型强制 + 范围展开;Workbook 值层承载字面量/公式,
             按需求值 + 记忆化缓存 + 循环检测(环 → #REF!,不 panic/不死循环)
  graph.rs   依赖图:前驱提取 + 范围包含判定 + 拓扑排序(Kahn)
  functions/ 可扩展注册表:math/stats/logical/text/datetime/lookup/info/financial/
             engineering/dynamic(实测函数数以 `formula::functions::count()` 为准)
```

**计算管线**(编辑后不全表重算):`set_input` 只更新受影响的依赖图边,并把「该格 + 传递后继」
标记为**脏区**;`recalculate()` 对脏区子图拓扑排序(前驱在前),喂入干净值后按序求值,
每个脏格**只算一次**(计算合并)。环(循环引用)默认得 `#REF!`,或 `set_iterative` 开启
迭代至收敛(`epsilon`)/上限(`max_iter`),迭代法可选 Jacobi / Gauss–Seidel。
`precedents/dependents/dirty_cells` 暴露依赖路径与脏区供审查。范围不展开成边:建**列索引**
(列 → 覆盖它的窄范围公式,宽范围回退)+ 脏区上按包含判定,编辑时只查该列候选,避免大范围爆炸。

**关键取舍**:

- **错误是一等值**(`#DIV/0!` 等沿链传播),不是逐层 `Result` 中断 —— `IFERROR` 才可能实现。
- 函数拿到**未求值的 AST 参数** + 求值器句柄:`IF` 能短路,聚合函数能遍历范围而不物化。
- **补齐更多 Excel 函数是机械式的**:在类别文件写实现、在其 `register` 插一行,不碰求值器。
- `Sheet` 保持**只读纯文本**;公式引擎是它之上独立的值层,与下方「扩展边界」的方向一致。
- `TODAY`/`NOW` 的「当前时间」由外部注入(前端传 `Date` 换算的序列数),core 不读系统时钟。

CSV 表格页的落地:含 `=` 单元格的文件由 `formula::evaluate_sheet` 求值,网格显示**计算值**、
公式栏回显**原始公式**(与 Excel「格显示值、栏显示式」一致)。引擎层还支持**跨工作表引用**
(`Sheet!A1`,读另一张常量表)与**具名区域**(`define_name`);`.xlsx` 多表数据据此互相引用。
**已补齐**:动态数组(FILTER/SORT/UNIQUE/SEQUENCE/XLOOKUP,`dynamic.rs`)与
`engineering.rs` 类别。**仍非目标**:database/cube 类别、跨工作簿引用。

## 健壮性:解析资源预算(`crates/core/src/limits.rs`)

只读查看器面对的是**任意来源**的文件,其中一部分畸形、少数是刻意构造的。
在 WASM 里这件事的代价比原生程序高一档:一次 panic / OOM 会让整个模块 trap,
而前端缓存了 init promise —— **用户必须刷新页面才能再打开任何文件**。

CSV 路径一开始就有 `DEFAULT_MAX_BYTES / MAX_ROWS / MAX_COLS`;OOXML 与公式引擎
此前一个上限都没有。现在统一在 `limits.rs`,在五处入口钳制:

| 位置 | 触发形态 | 不钳制的后果 |
| --- | --- | --- |
| `xlsx::build_sheet` | 一个 `<c r="XFD1048576"/>` | 循环压进 172 亿个空串 |
| `xlsx::expand_sqref` | `sqref="A1:XFD1048576"`(「全选 + 条件格式」的**真实**产物) | 逐格展开 ≈137 GB |
| `formula::eval::flatten_range` | `=SUM(A1:XFD1048576)` | `rows()*cols()` 在 u32 上回绕成 0,再 push 到内存耗尽 |
| `formula::parser::parse_expr` | `=((((…1…))))` | 递归下降吃光原生栈(求值侧的 `MAX_DEPTH` 来得太晚) |
| `dynamic::sequence` | `=SEQUENCE(100000,100000)` | `as usize` 在 wasm32 上截断,循环到 OOM |

原则:**超限给可读错误或截断,绝不 panic**。取值宽到不影响真实文件,窄到不至于耗尽内存。

日期换算同理收口到 `serial.rs`:此前 `numfmt` / `xlsx` / `formula::datetime` 各有一份
Hinnant 算法,而 `xlsx` 那份用朴素的 `serial - 25569`、**不复刻 1900 闰年 bug** ——
同一个工作簿里走 numfmt 格式码和走 `Data::DateTime` 的日期在 1900 年初会差一天。

## 开发工具(`crates/xtask`)

`cargo run -p xtask -- fixtures [--check] <目录>` 生成或校验浏览器 e2e 夹具。

夹具**入库**(clone 即可跑 e2e;`git bisect` 时拿到的是那个 commit 的夹具),
CI 的 `rust` job 用 `--check` 逐字节比对,保证它们不与生成器脱节 ——
三个生成器都是确定性的(zip 条目时间戳固定),所以字节比对可行。

此前这套逻辑挂在三个 `#[ignore]` 测试上,由 npm script 触发再从 `/tmp` 拷贝:
`cargo test <过滤器>` 匹配不到测试时退出码仍是 0,后面的 `cp` 会拷走上次遗留的旧文件,
**拿着陈旧夹具跑出绿灯**。搬进 xtask 后失败即非零退出,且 `docx-rs` 的 `image` 特性
从 `crates/core` 的 dev-dependencies 移到这里,`cargo test` 不再为它编译一套位图解码。

## 扩展边界(叠层原则)

`Sheet` 只承载**纯文本单元格**。下表原本是「本期刻意不实现」的清单,
其中多数已按各自的**叠层**方式落地 —— 保留此表是为了记录**加在哪里**的原则,
而不是记录还没做什么:

| 能力 | 应该加在哪 | 不应该怎么做 |
| --- | --- | --- |
| 值类型 / 数字与日期格式化 | ✅ 已实现:`numfmt.rs` 独立格式层 + `serial.rs` 日历,绘制时查表 | ❌ 把格式化塞进 `Sheet` 或在绘制代码里判断内容像不像日期 |
| 公式求值 | ✅ 已实现:独立的 `formula` 模块([值层 `Workbook`])求值,产出显示表喂给 `Sheet` | ❌ 让渲染管线感知公式 |
| 图表 | ✅ 已实现:`office-ooxml` 的 chart.rs 解析(xlsx/pptx 共用)+ `web/shared/chartDraw.ts` 绘制 | ❌ 混进表格渲染管线 |
| xlsx 表格视图 | ✅ 已实现:`xlsx.rs` 产出 `Sheet`,视图层复用 `SheetHandle` 接口 | ❌ 为 xlsx 再写一套渲染 |
| 解析资源预算 | `limits.rs` 统一常量,在各解析入口钳制 | ❌ 各模块各拍一个魔数,或干脆不设上限 |

## Word / PPT 只读渲染(`crates/{word,ppt}/src/` + `packages/{word,ppt}`)

见 [RFC-0006](./rfcs/0006-word-excel-ppt-readonly.md)。两条与表格并列的渲染管线,同样「重 CPU 在
Rust、canvas 虚拟化 + 多级缓存」:

- **Word**:`docx.rs` 用 `docx-rs` 读路径抽出平面化模型(段落/run/标题/对齐/列表/内联图片/表格、
  **分栏 `sectPr`、页眉页脚、修订 ins/del 标记**);`office-word` 的 `wordLayout` 做**流式布局**产出带绝对 y
  的绘制项(分栏为贪心分配、页眉页脚各带分隔线、修订插入蓝色/删除红色+删除线),`WordPage` 用
  **sticky canvas + spacer** 纵向虚拟化(只画视口内的项)。字号/颜色经 serde 读 docx-rs 私有字段;图片字节 → Blob object URL。
- **PPT**:`pptx.rs` 直接用 `zip + quick-xml` 解析 PresentationML(尺寸/顺序 → rels → spTree 形状/图片,
  **旋转/翻转 `xfrm@rot/flipH/flipV`、母版 `txStyles` 文本默认样式继承、组合形状 `grpSp`、渐变 `gradFill`、
  内嵌表格 `a:tbl`、内嵌图表 `c:chart`(复用 `office-ooxml` 的 chart)、`graphicFrame` SmartArt 占位、
  `timing`/`transition` 动画时间线**),用元素名栈区分 spPr 填充与 rPr 颜色,EMU÷9525;
  动画把 `mainSeq` 里每个 `clickEffect` 记为一步,`spTgt@spid` 经 `cNvPr@id` 落到形状的 `appear_step`;
  `office-ppt` 的 `slideRender` 按 `fitScale` 等比铺进画布(形状几何/填充、文本折行+对齐、图片、旋转仿射变换、
  真实表格网格与图表、SmartArt 虚线占位框+类型标签),并按 `step` 过滤未出现的形状;
  `PptPage` 提供缩略图导航、缩放、翻页(含动画/切换徽标)与**全屏演示模式**
  ——演示时点击/方向键**先逐步播完入场动画再翻页**,换页按 `transition` 类型做淡入/揭开/推入。
- **共享文本测量**(`packages/shared/src/textMeasure.ts`):参考 pretext,`font→segment` 两级缓存 +
  OffscreenCanvas + 字体加载失效 + 二分裁剪 + 折行,三个页面共用一个实例。

## 目录结构

```
office-R/
├── crates/            Rust cargo workspace
│   ├── core/          office-core:表格内核(sheet/csv/formula/numfmt/serial/limits/filter/format)
│   ├── ooxml/         office-ooxml:chart + OPC/XML 共享原语(quick-xml/zip)
│   ├── word/          office-word:docx.rs(→ docx-rs)
│   ├── excel/         office-excel:xlsx.rs(→ calamine + core + ooxml)
│   ├── ppt/           office-ppt:pptx.rs(→ ooxml)
│   ├── wasm-log/      office-wasm-log:三个 cdylib 共用的日志桥(rlib)
│   ├── word-wasm/ · excel-wasm/ · ppt-wasm/   三个独立 cdylib
│   └── xtask/         开发工具:e2e 夹具生成 / 漂移校验
├── packages/          pnpm workspace 前端包(exports 直指源码 .ts)
│   ├── shared/        @tengxiaohyx/office-shared:logger/textMeasure/chartDraw/FileUpload/fonts/page.css
│   ├── word/          @tengxiaohyx/office-word:model + wasm 加载器 + WordPage + 布局 + pkg/
│   ├── excel/         @tengxiaohyx/office-excel:SheetHandle + grid 渲染管线 + SheetCanvas + csvWorker + pkg/
│   └── ppt/           @tengxiaohyx/office-ppt:model + wasm 加载器 + PptPage + slideRender + pkg/
├── web/               演示站(壳):App.tsx Tab + main + 外壳 CSS + e2e/
├── docs/              RFC / Spec / Story / 报告 / 工作流 / 架构
└── .github/workflows/ CI 与 Pages 部署
```

各包的 `pkg/` 是 wasm-pack 产物(不入库);工具链 devDeps 集中在根 `package.json`。

## 构建与部署

- WASM(三份):`for a in word excel ppt; do wasm-pack build crates/$a-wasm --target web --out-dir packages/$a/pkg --out-name office_${a}_wasm; done`
- 前端:`pnpm -C web build`(通过 `VITE_BASE` 设置 Pages 子路径)
- 校验/测试全仓:`pnpm typecheck`(所有包)· `pnpm lint`(根 config)· `pnpm test`(vitest workspace)
- 部署:推送 `main` → GitHub Actions 构建三份 WASM + 前端 → 部署到 GitHub Pages

### CI 拓扑(`.github/workflows/ci.yml`)

```
wasm[word|excel|ppt] ──┬─→ web   (typecheck 全包 / eslint 根 / vitest workspace / vite build)
   (matrix 并行,3 artifact) └─→ e2e   (playwright install → e2e;夹具已入库,不需要 Rust)
rust      (fmt / clippy --locked / test --locked / 夹具漂移校验,与 wasm 并行)
```

`wasm` job **只构建一次**产物并 `upload-artifact`,`web` / `e2e` 下载复用 ——
release profile 是 `opt-level="z" + lto + codegen-units=1`(最慢一档),
而 `Swatinem/rust-cache` 的 key 含 job 名,三个 job 各编一次时缓存互不命中。
`web` job 因此完全不需要 Rust 工具链。

其余门禁:`concurrency` 取消同分支旧运行;`permissions: contents: read`;
每 job `timeout-minutes`;e2e 失败时上传 `playwright-report` + `test-results`
(`trace: on-first-retry` 的产物此前一直被丢弃,CI 上的 flaky 无法排查)。

前端另有 ESLint 门禁(`web/eslint.config.js`):`react-hooks/exhaustive-deps` 挡
canvas + hooks 的依赖数组错误,`no-console` 把「禁止裸 console.log」自动化。
`react-hooks` v7 的编译器系新规则(refs / set-state-in-effect / immutability)
暂列为 warn 并用 `--max-warnings` 冻结当前条数 —— 它们指向的 render 期写 ref、
effect 内同步 setState 是真实设计债,该由一次独立重构来还,而不是先把 CI 弄红。

> 改动 `crates/core` 的公共数据结构后**务必重新构建 WASM** —— 原因见 [AGENTS.md](../AGENTS.md)。
