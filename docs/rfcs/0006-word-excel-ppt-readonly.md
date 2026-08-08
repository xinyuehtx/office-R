# RFC-0006: Word / Excel / PPT 三应用只读渲染

- **状态**:已实现
- **作者**:office-R team
- **创建日期**:2026-08-08
- **关联**:Spec-0006、Story-0006、[RFC-0003](./0003-csv-canvas-grid.md)、[RFC-0005](./0005-view-filter-freeze.md)

## 动机

在 CSV 表格 + 公式 + 过滤的基础上,补齐三大应用的**只读渲染**:Excel 冻结行列与数字
格式化、Word 文档(文字/列表/标题/对齐/图片/表格/图文混排)、PPT(文本框/图片/形状/
演示模式/对齐)。原则:**重 CPU 全在 Rust/WASM**,渲染充分利用 canvas(虚拟化 + 多级缓存)。

## 开源调研与选型(优先纯净可用的开源项目)

| 领域 | 结论 | 依据 |
| --- | --- | --- |
| Word 解析 | 复用 `docx-rs`(已在依赖)的**读路径** `read_docx` | 它已处理 zip/document.xml/rels 与**图片字节↔embed 关联**,MIT、纯 Rust 可 wasm。字号/颜色私有字段经 serde 读出;图片字节取自 `images` 元组 |
| PPT 解析 | **直接用 `zip` + `quick-xml` 解析 OOXML** | crates.io 无干净的纯 Rust pptx 读库(`pptx` 0.1.0 依赖 fs 且拉第二份 quick-xml;`pptx-to-md`/`office_oxide` 丢弃几何)。直接解析仅数百行且依赖树干净 |
| 数字格式化 | **自研** `numfmt`(参考 ECMA-376 格式码语义) | 只需常用子集,自研无依赖负担 |
| 文本测量 | **参考 `chenglou/pretext` 的测量+缓存逻辑自研**,不引入依赖 | pretext 是多行段落库(~40KB bidi/kinsoku),我们只需单行 fit/折行;移植其「canvas measureText + font→segment 两级缓存 + 字体加载失效」精华 |

## 方案

### Excel(见 [Spec-0005](../specs/0005-view-filter-freeze.md) 续)

- **数字格式化**(`core/numfmt.rs`):解析格式码(占位符/小数/千分位/百分比/货币/
  日期时间/科学/分节)→ 渲染文本。重 CPU 在 Rust。
- **冻结行列**(`web/excel/grid`):`GridLayout` 增 `frozenRows/Cols` 与像素跨度;
  冻结时走**四象限全量重绘**隔离路径(不碰 50 万行滚动的瓦片热路径);
  `cellScreenRect`/`hitTest`/表头/覆盖层均冻结感知。

### Word(`core/docx.rs` + `web/word`)

- 解析:docx-rs 读路径 → 平面化只读模型(段落/run/标题/对齐/列表/内联图片/表格)。
- 渲染:`wordLayout` 流式布局(标题字号、对齐、列表缩进+符号、CJK 逐字折行、
  图文混排、表格等宽列)产出**带绝对 y 的绘制项**;`WordPage` 用
  **sticky canvas + spacer** 做纵向虚拟化,只画视口内的项。
- 图片:字节 → Blob object URL → canvas `drawImage`。

### PPT(`core/pptx.rs` + `web/ppt`)

- 解析:presentation.xml(EMU 尺寸 + sldIdLst 顺序)→ rels → 每张 slide 的 spTree
  (形状 xfrm/prstGeom/solidFill/txBody、图片 blip embed);slide rels 解析图片字节。
  用**元素名栈**区分 spPr 填充 vs rPr 颜色。EMU÷9525;字号百分之一磅。
- 渲染:`slideRender` 把一张幻灯按 `fitScale` 等比铺进画布(形状几何/填充、文本框
  折行+对齐、图片);`PptPage` 提供缩略图导航、前后翻页、**全屏演示模式**(方向键/Esc)。

### 共享文本测量(`web/shared/textMeasure.ts`)

`font → (segment → width)` 两级缓存 + OffscreenCanvas 优先 + 字体加载失效 + 二分裁剪
+ 折行。三个页面共用一个实例,缓存跨页面复用。

## 性能与优化

- 解析/格式化/过滤等重 CPU 全在 WASM;跨边界只传模型 + 图片字节(object URL,不走 base64)。
- Excel:瓦片 + GPU 平移(未冻结);冻结走隔离全量路径。Word:纵向虚拟化。
  PPT:按需绘制当前幻灯。文本测量多级缓存三页共用。

## 取舍与非目标

**后续增强(已在 polish 阶段补齐,见 [报告-0007](../reports/0007-office-readonly-polish.md)):**

- ✅ Word 列表**有序/无序**:查 `numbering.xml`(numId→abstractNum→level.format),`decimal`/`lowerRoman`/… 为有序并算序号,`bullet`/`none` 为无序。
- ✅ Word **两端对齐**(justify):非末行按词间空白拉伸,末行左对齐。
- ✅ PPT **母版/版式继承**:占位符无 xfrm 时向 slideLayout→slideMaster 借几何。
- ✅ PPT **主题配色** `schemeClr`:解析 `theme1.xml` 的 `clrScheme`(tx1/bg1→dk1/lt1)。
- ✅ **自动化 e2e**:引入 `@playwright/test`,覆盖 word/excel(公式/过滤/冻结)/ppt(渲染/翻页/演示)。
- ✅ **wasm 瘦身**:`docx-rs` 主依赖 `default-features=false`(image 特性移到 dev-dependency),WASM 1807KB → 1114KB(−38%)。

**仍非目标(本期不做):**

- Word:分栏、页眉页脚、修订/批注、精确行距缩进。
- PPT:动画/切换、SmartArt/图表、组合形状子坐标、旋转翻转、自定义几何、文本默认样式继承。
- Excel numfmt:颜色码 `[Red]`、条件 `[>=100]`、分数。

## 影响

- `crates/core`:新增 `numfmt`/`docx`/`pptx`/`filter`;`docx-rs` 主依赖 `default-features=false`,image 特性移至 dev-dependency(仅测试夹具用)。
- `crates/wasm`:新增 `word`/`ppt` 绑定。
- `web`:Word/PPT 页从占位改为真实渲染;Excel 增冻结 UI;新增共享测量缓存;新增 `e2e/`(Playwright)。
- 文档:architecture / AGENTS / README 同步。
