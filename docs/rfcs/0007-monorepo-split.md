# RFC-0007: Monorepo 拆分 —— 三个应用及其 WASM 后端独立可用

- **状态**:实施中
- **作者**:office-R team
- **创建日期**:2026-08-10
- **关联**:[RFC-0002](./0002-core-dependencies.md)、[RFC-0006](./0006-word-excel-ppt-readonly.md)、[架构](../architecture.md)

## 动机

`office-core` 已经长到 15306 行,同时装着三种格式的解析器;`office-wasm` 是一个单体
cdylib;前端 `web/` 是一个包,三个应用的代码全进同一个 chunk。

后果是**没有部分消费的可能**:只想要 Excel 的人会拿到 docx-rs、image 解码器和整个
pptx 解析器的字节。这与「三个可独立使用的只读查看器」这个产品形态相悖。

目标:Word / Excel / PPT 各自成为可独立依赖的单元 —— 一个 Rust crate + 一个 npm 包
+ 一份自己的 WASM 产物。

拆分前的测绘发现现状比预期干净:依赖图是无环 DAG、**反向依赖为零**;`docx.rs` 零 crate
内依赖;三个前端应用之间的实质耦合只有 **2 行 `FONT_FAMILY`**。所以这主要是**搬家 +
前缀替换**,只有一处(pptx 的 rels 路径归一)涉及真正的行为变更。

## 目标形态

```
crates/
  core/        office-core      sheet csv formula filter numfmt serial limits format
  ooxml/       office-ooxml     chart + local/attr/mime_of/emu_to_px/resolve_rel_path/…
  word/        office-word      docx.rs   → docx-rs serde_json office-ooxml
  excel/       office-excel     xlsx.rs   → calamine office-core office-ooxml
  ppt/         office-ppt       pptx.rs   → office-ooxml serde
  wasm-log/    office-wasm-log  三个 cdylib 共用的日志层(rlib)
  {word,excel,ppt}-wasm/        三个 cdylib
  xtask/       夹具生成(与拆分正交)

packages/
  shared/  @tengxiaohyx/office-shared   logger textMeasure chartDraw FileUpload fonts
  word/    @tengxiaohyx/office-word     src/ + pkg/
  excel/   @tengxiaohyx/office-excel    src/ + pkg/   ← CSV 与 xlsx 同包
  ppt/     @tengxiaohyx/office-ppt      src/ + pkg/

web/       三合一演示站(保留原路径)
```

**Word 与 PPT 对 `office-core` 的依赖为零** —— 只有 Excel 真正需要内核。

## 决策

| 决策 | 选择 | 理由 |
| --- | --- | --- |
| 范围 | monorepo 内独立依赖 + 保留三合一演示站 | 不做发布流程与各应用独立 dev 站点 —— 包名与 `exports` 结构已为它们留好余地 |
| CSV + 公式引擎(8700 行) | 留在 `office-core` | 它们是**表格**能力而非 **xlsx 格式**能力:CSV 路径在用求值器,而 `xlsx.rs` 只用了 `CellFormula` 这个三字段结构体(取 calamine 的缓存值,不重算)。塞进 Excel 会让 CSV 也被迫进 Excel |
| WASM 粒度 | 每应用一个 cdylib | 见下「代价」一节 |
| 包构建 | 不加 dist/.d.ts,`exports` 直指源码 `.ts` | 加构建意味着 CI 要拓扑构建 4 个包、demo 的 HMR 退化成「改一行要先 build」;而 `grid/` 里大量类型是渲染器内部形状,提前进 `.d.ts` 就成了半个公共契约。发布当天补 `tsc --emitDeclarationOnly` 是 20 行配置的事 |
| `detect` / `version` | 每个 cdylib 各留一份 | 消费方无需同时装三个包就能做文件路由 |
| 节奏 | 分 9 个阶段提交 | 每阶段 CI 绿、可独立回滚 |

### 三个非直觉的设计要点

**1. `office-word` 依赖 `office-ooxml` 不引入任何新传递依赖。**
`Cargo.lock` 显示 `docx-rs` 本身就依赖 `quick-xml` 和 `zip`。所以「Word 不该被 XML/zip
污染」在事实层面不成立 —— 读 .docx 本来就要解 zip 解 XML。于是 `mime_of` / `emu_to_px`
归 `office-ooxml`,不为 25 行新建第 9 个 crate。

**2. CSV 的 wasm 绑定必须与 xlsx 同 cdylib。**
`WasmWorkbook::sheet(i) -> WasmSheet` 要求两者在同一个 wasm 模块里 —— wasm-bindgen 的
类型不能跨模块实例传递。拆开就要复制一份 `WasmSheet`,代价是两次 `init()`、`Sheet` +
`filter` 各存两份、前端拿到两个同构异名的 class。副作用是好的:`WasmSheet::from_sheet`
的 `pub(crate)` 原封不动 —— 那是全案唯一会因拆分失效的可见性。

已知取舍:只想要 CSV 的消费方会带上 calamine。真要治,正解是给 `office-excel-wasm` 加
一个 `xlsx` feature(默认开),不是新建 crate。

**3. `canOpen` 不能走 `office_core::detect_format`**,因为 Word/PPT 不依赖 core。
各 crate 自己实现十行 `can_open(bytes) -> bool`(word 查 zip 里有无 `word/document.xml`,
ppt 查 `ppt/presentation.xml`,excel 查 `xl/workbook.xml` 或走 core 的 CSV 嗅探)。
`office-ooxml` 提供 `zip_has_entry` 原语。`office_core::detect_format` 与它的 11 个测试
原样保留。

## 代价

**三份 wasm 之和会大于现在的单一产物** —— 每份各带一套 wasm-bindgen glue、panic hook
和 allocator。这是「独立可用」的固有成本,不是实现缺陷。

但**演示站的首屏字节反而下降**:现在任何一页都会拉同一个大 wasm;拆分后打开 Word 页
只 fetch word 的那一份。

### 产物尺寸基线

> 拆分前的单一产物已测得(Phase 0,`wasm-pack build --target web`,release profile
> `opt-level="z" + lto + codegen-units=1`);三份拆分后的尺寸待 Phase 4b 首次绿灯后回填。

| 产物 | 尺寸 |
| --- | --- |
| (拆分前)`office_wasm_bg.wasm` | 1,257,803 字节(1.20 MiB) |
| `office_word_wasm_bg.wasm` | **718,305 字节**(0.68 MiB) |
| `office_excel_wasm_bg.wasm` | **865,129 字节**(0.83 MiB) |
| `office_ppt_wasm_bg.wasm` | **260,294 字节**(0.25 MiB) |

判读:三份**各自**都显著小于 1.20 MiB —— 依赖真隔离了(ppt 只有 0.25 MiB,
因为它连 office-core 都不依赖)。三份之和 1.76 MiB > 1.20 MiB 是预期的固有代价
(各带一套 wasm-bindgen glue + panic hook + allocator);但演示站按页懒加载,
打开任一页只 fetch 那一份,**首屏字节反而低于拆分前**。

## 实施阶段

| # | 阶段 | 要点 |
| --- | --- | --- |
| 0 | RFC + 免费清理 | 不动结构。消灭三应用间仅存的 2 条耦合(`FONT_FAMILY` → `shared/fonts`);`format.rs` 的 `CANDIDATES` 方向倒转使其零 crate 内依赖;清死代码 |
| 1 | 前端 barrel 拆缝 | `web/src/wasm/index.ts` 是全仓唯一 barrel、也是全部包级循环的唯一源头。按应用切开但保留再导出层 |
| 2 | 抽 `office-ooxml` | **必须在三向拆分之前** —— 现在合并五组重复辅助是「写一份正确实现」,先拆再合是「跨三个 crate 调和三份已分叉的副本」 |
| 3 | 拆三个格式 crate | **只动 Rust,前端与 CI 零改动**。e2e 全绿本身就是行为保真的证明 |
| 4a | npm 骨架 + shared | 先在最简单的包上验证全套前端管线(pnpm link / Vite 解析源码 `.ts` / vitest project / 根 eslint) |
| 4b | 三个 cdylib + CI | **风险最高**。三个必验锐边见下 |
| 5 | 搬 word | 最小,且独占全部 4 处跨应用 CSS 借用 —— 在最小的面上先撞这个坑 |
| 6 | 搬 ppt | `imageKey` 的值导入在这一步自然消解 |
| 7 | 搬 excel | 最大,且失败模式对单测不可见 —— 等其余全部落定绿灯 |
| 8 | 文档同步 | 回填尺寸基线 |

### Phase 2 的行为变更风险

pptx 的 rels 归一从字符串前缀 hack(`normalize_ppt_path` / `normalize_media_path` /
`replace("../","")`)换成 xlsx 那份真正处理 `../` 的 `resolve_rel_path`,是**语义变化**。
按 TDD 分两步:先加测试钉住四种 target 形态(`../media/image1.png`、`/ppt/media/x.png`、
`media/x.png`、`slides/slide1.xml`)的当前行为,再换实现。16 个 pptx 测试 +
`e2e/ppt.spec.ts` 是它的守卫。

### Phase 4b 的三个必验锐边

1. **`csvWorker` 的 Worker URL 解析**:`new Worker(new URL("./csvWorker.ts", import.meta.url))`
   要求 client 与 worker 同包同相对位置。**单测抓不到这条边的回归** —— jsdom 里
   `typeof Worker === "undefined"` 会走主线程 fallback,测试全绿而 Worker 已废。
   唯一守卫是给 `e2e/excel.spec.ts` 加 `page.waitForEvent("worker")` 断言。
2. **rlib 里的 `#[wasm_bindgen]` 导出可能被 dead-strip**:build 后 grep 生成的 `.d.ts`
   确认 `setLogLevel` 在;不在就在各 cdylib 加三行转发。
3. **`ensureReady` 从一变三**:漏调任一 `init()` 表现为运行时 `TypeError`,typecheck
   抓不到(wasm glue 的 `.d.ts` 是生成的)。

### CSS 的静默失败

Word 页借用了 PPT 的 `ppt-zoom` 与 Excel 的 `sheet__find-*` 共 4 处。拆包后 word 单独
跑会**静默变丑而不报错** —— 没有任何测试选这些 class。修法是把 ~50 行 CSS 复制进
word 包并改名,并加两条 `toHaveCSS` 的 e2e 断言。

不把 find-bar / zoom 上提到 shared:shared 应该停在「页面外壳 + 上传」,把 widget 的
样式上提而不上提 widget 本身,正是现在这团乱的成因。

## 非目标

- npm / crates.io 发布流程(包名与 `exports` 结构已留余地)
- 各应用独立 dev 站点(它是 e2e 分包的唯一正当理由,一起推到将来)
- `SheetCanvas`(820 行)拆分 —— 与包边界正交,混进来会让 Phase 7 从机械移动变成移动+重构
- 23 条 react-hooks warning、ts-rs、`theme.ts COLORS` ↔ CSS 色值统一
  (后者的前置条件正是这次拆分 —— 两半第一次进同一个包)
- `web/` → `apps/demo` 改名:纯重命名税(5 处 `pnpm -C web`、artifact 路径、
  `deploy-pages.yml`、6 条 `.gitignore`、全部文档命令示例),而 `web/` 本来就**是**那个 demo
