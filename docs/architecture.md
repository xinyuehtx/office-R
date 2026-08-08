# 架构文档

> 本文件描述 office-R 的整体架构。**每次提交前若架构或目录发生变化,必须同步更新本文件。**

## 定位

office-R 是一个**统一的 Office 三件套应用**(文档 / 表格 / 演示),特点:

- **视图层 = Web**:React + Vite + TypeScript,表格视图用 **canvas** 绘制。
- **计算内核 = Rust**:编译为 **WASM**,在浏览器内识别与解析 office 文件。
- **纯静态部署**:产物为静态资源,部署到 GitHub Pages,无需后端服务器。

## 分层

```
┌──────────────────────────────────────────────────────────────┐
│  Web 视图层 (web/, React + Vite + TS)                          │
│  ├─ App.tsx                顶部 Tab:文档 / 表格 / 演示          │
│  ├─ apps/word · apps/ppt   上传 → 识别 → 摘要                   │
│  ├─ apps/excel/            CSV 表格视图(本期重点)              │
│  │   ├─ ExcelPage.tsx      页面:上传 / 状态 / 元信息            │
│  │   ├─ SheetCanvas.tsx    交互:尺寸自适应 / 滚轮 / 键盘 / 拖拽  │
│  │   └─ grid/              渲染管线(见下)                      │
│  ├─ apps/shared/           OfficePage / FileUpload /            │
│  │                         useOfficeFile / useCsvFile /         │
│  │                         SheetHandle 接口 / logger             │
│  └─ wasm/                  WASM 封装 + csvWorker(解析线程)      │
└───────────────┬──────────────────────────────────────────────┘
                │ wasm-bindgen
┌───────────────▼──────────────────────────────────────────────┐
│  绑定层 office-wasm (crates/wasm/)                             │
│  version / detect / render / parseCsvPacked(含公式求值)/       │
│  WasmSheet(过滤/冻结)/ WasmWordDoc / WasmPresentation           │
│  log.rs:与前端同格式的分级日志                                  │
└───────────────┬──────────────────────────────────────────────┘
                │ 纯 Rust 调用
┌───────────────▼──────────────────────────────────────────────┐
│  计算内核 office-core (crates/core/)                           │
│  ├─ format.rs   detect_format() 识别格式(含 CSV 文本判定)      │
│  ├─ sheet.rs    Sheet:紧凑表格模型 / 窗口取数 / 列宽度量        │
│  ├─ csv/        CSV 解析:decode(编码)· dialect(分隔符)      │
│  │              · error(thiserror)· mod(解析主流程)         │
│  ├─ formula/    公式引擎:token→parser→ast→eval + functions/    │
│  │              值层 Workbook,对齐 Excel(140+ 函数)          │
│  ├─ filter.rs   列过滤:按列条件全表扫描 → 命中行下标            │
│  ├─ numfmt.rs   Excel 数字格式码 → 显示文本                      │
│  ├─ docx.rs     Word:docx-rs 读路径 → 平面化文档模型 + 图片      │
│  ├─ pptx.rs     PPT:zip+quick-xml 解析 → 幻灯模型 + 图片         │
│  ├─ render.rs   RenderResult 摘要结构                           │
│  ├─ word.rs · excel.rs · ppt.rs   docx / xlsx / pptx 摘要解析   │
│  └─ lib.rs      render() 统一分发入口                            │
└──────────────────────────────────────────────────────────────┘
```

**关键设计**:`office-core` 平台无关、不依赖任何浏览器 API,因此可原生 `cargo test`;
`office-wasm` 只做类型转换、跨边界搬运与日志,不含业务逻辑。

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

## 数据流一:摘要(Word / PPT / xlsx)

1. 用户在页面选择 `.docx/.xlsx/.pptx`;
2. `useOfficeFile` 读为 `Uint8Array`,调用 WASM `render(bytes)`;
3. `office-core::render` 先 `detect_format`,再分发到对应组件;
4. 返回 `RenderResult { format, format_name, byte_len, message, ok }`,页面展示。

CSV 走下面的表格渲染路径;若把 CSV 传到 Word/演示页,会得到「请切换到表格页」的引导。

## 数据流二:CSV 表格渲染(本期重点)

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

位于 `web/src/apps/excel/grid/`,分四段,每段可独立验证:

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

前端 `apps/shared/logger.ts` 与 WASM `crates/wasm/src/log.rs` **同格式**输出:

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
  functions/ 可扩展注册表:math/stats/logical/text/datetime/lookup/info/financial(140+)
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
**非目标**:动态数组溢出、engineering/database/cube 类别。

## 扩展边界(本期刻意不实现)

`Sheet` 只承载**纯文本单元格** —— CSV 本身也不携带更多信息。后续演进的边界:

| 能力 | 应该加在哪 | 不应该怎么做 |
| --- | --- | --- |
| 值类型 / 数字与日期格式化 | 在 `Sheet` **之外**新增「值层 / 格式层」,绘制时查表 | ❌ 把格式化塞进 `Sheet` 或在绘制代码里判断内容像不像日期 |
| 公式求值 | ✅ 已实现:独立的 `formula` 模块([值层 `Workbook`])求值,产出显示表喂给 `Sheet` | ❌ 让渲染管线感知公式 |
| 图表 | 独立组件,复用 `SheetHandle` 取数 | ❌ 混进表格渲染管线 |
| xlsx 表格视图 | 新增解析入口产出 `Sheet`,视图层复用 `SheetHandle` 接口,**一行不改** | ❌ 为 xlsx 再写一套渲染 |

## Word / PPT 只读渲染(`crates/core/src/{docx,pptx}.rs` + `web/apps/{word,ppt}`)

见 [RFC-0006](./rfcs/0006-word-excel-ppt-readonly.md)。两条与表格并列的渲染管线,同样「重 CPU 在
Rust、canvas 虚拟化 + 多级缓存」:

- **Word**:`docx.rs` 用 `docx-rs` 读路径抽出平面化模型(段落/run/标题/对齐/列表/内联图片/表格、
  **分栏 `sectPr`、页眉页脚、修订 ins/del 标记**);`web/word/wordLayout` 做**流式布局**产出带绝对 y
  的绘制项(分栏为贪心分配、页眉页脚各带分隔线、修订插入蓝色/删除红色+删除线),`WordPage` 用
  **sticky canvas + spacer** 纵向虚拟化(只画视口内的项)。字号/颜色经 serde 读 docx-rs 私有字段;图片字节 → Blob object URL。
- **PPT**:`pptx.rs` 直接用 `zip + quick-xml` 解析 PresentationML(尺寸/顺序 → rels → spTree 形状/图片,
  **旋转/翻转 `xfrm@rot/flipH/flipV`、母版 `txStyles` 文本默认样式继承、`graphicFrame` 图表/SmartArt 占位、
  `timing`/`transition` 动画/切换标记**),用元素名栈区分 spPr 填充与 rPr 颜色,EMU÷9525;
  `web/ppt/slideRender` 按 `fitScale` 等比铺进画布(形状几何/填充、文本折行+对齐、图片、旋转仿射变换、
  图表/SmartArt 虚线占位框+类型标签),`PptPage` 提供缩略图导航、翻页(含动画/切换徽标)与**全屏演示模式**。
- **共享文本测量**(`web/shared/textMeasure.ts`):参考 pretext,`font→segment` 两级缓存 +
  OffscreenCanvas + 字体加载失效 + 二分裁剪 + 折行,三个页面共用一个实例。

## 目录结构

```
office-R/
├── crates/            Rust cargo workspace
│   ├── core/          office-core:平台无关计算内核
│   │   ├── src/csv/   CSV 解析(decode / dialect / error)
│   │   ├── src/formula/  公式引擎(token/parser/ast/eval + functions/)
│   │   └── {filter,numfmt,docx,pptx}.rs  过滤/数字格式/Word/PPT 解析
│   └── wasm/          office-wasm:wasm-bindgen 绑定 + 日志
├── web/               React + Vite + TS 视图层(pnpm 管理)
│   ├── src/apps/      word / excel / ppt 三页 + shared 复用
│   │   ├── excel/grid/  canvas 表格渲染管线(瓦片/冻结/过滤)
│   │   ├── word/        docx 模型 + 流式布局 + 虚拟化渲染
│   │   ├── ppt/         幻灯模型 + slideRender + 演示模式
│   │   └── shared/      textMeasure(共享测量缓存)等
│   ├── src/wasm/      WASM 封装、解析 Worker(pkg/ 为构建产物,不入库)
│   └── src/test/      测试基建:setup、canvas 替身、表格替身
├── docs/              RFC / Spec / Story / 报告 / 工作流 / 架构
└── .github/workflows/ CI 与 Pages 部署
```

## 构建与部署

- WASM:`wasm-pack build crates/wasm --target web --out-dir web/src/wasm/pkg`
- 前端:`pnpm -C web build`(通过 `VITE_BASE` 设置 Pages 子路径)
- 部署:推送 `main` → GitHub Actions 构建 WASM + 前端 → 部署到 GitHub Pages

> 改动 `crates/core` 的公共数据结构后**务必重新构建 WASM**:
> 前端引用的是 `web/src/wasm/pkg` 里的产物,不重建就会拿着旧的二进制跑,
> 症状往往是「单元格内容错位」这类难以定位的问题。
