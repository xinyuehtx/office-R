# 报告-0007:Office 只读渲染 完善(polish)

> ⚠️ **本文是带日期的快照**。下文的「仍未实现 / 当前边界」反映的是撰写当时的状态,
> 其中多数条目此后已经交付。**最新状态以 [AGENTS.md](../../AGENTS.md) 为准**。

- **关联**:RFC-0006
- **日期**:2026-08-08
- **状态**:已实现并通过验收(单测 + 自动化 e2e)

对上一轮遗留的 4 项非目标逐一补齐,并落地自动化浏览器 e2e。

## 1. WASM 瘦身(docx-rs 特性)

`docx-rs` 的 `image` 特性(位图解码 + 预览生成)仅测试夹具生成用到(`Pic::new`)。
把主依赖改为 `default-features = false`,并把带 `image` 特性的 `docx-rs` 放到
`[dev-dependencies]`——按 Cargo 特性统一规则:`cargo test` 启用、`cargo build`/`wasm-pack`
(不含 dev-deps)不启用。

- **效果**:`office_wasm_bg.wasm` **1807KB → 1114KB(−38%,约 693KB)**。
- `read_docx` 与图片原始字节不受影响(图片字节取自 `images` 元组,非 image 特性)。

## 2. Word:列表有序/无序 + 编号

`core/docx.rs` 读 `docx.numberings`:`Numbering{id, abstract_num_id}` →
`AbstractNumbering{levels}` → `Level{level, format.val}`。`format.val` 为
`bullet`/`none` → 无序;`decimal`/`lowerRoman`/`upperLetter`/… → 有序。
`ListItem` 增 `number`:有序列表按 `(numId, level)` 运行期递增算序号。
渲染时有序列表前缀用「序号.」,无序用「•」。单测覆盖有序(序号 1、2)与无序。

## 3. Word:两端对齐(justify)

`wordLayout` 对 `align=justify` 的**非末行**把富余宽度 `(avail - line.width)` 均摊到
词间空白;**末行左对齐**(与 Word 一致)。单测:两端对齐首行末词右缘比左对齐更靠右。

## 4. PPT:母版/版式继承 + 主题配色

- **占位符几何继承**:`p:sp` 若是占位符(`p:ph type/idx`)且无 `a:xfrm`,向
  slide→slideLayout→slideMaster 借几何。实现:slide rels 找 slideLayout,layout rels 找
  slideMaster,`collect_placeholder_geom` 从二者抽 `type|idx → 矩形`(母版打底、版式覆盖),
  幻灯占位符 sp 结束时若无 xfrm 则套用。
- **主题配色 `schemeClr`**:`load_theme` 解析 `ppt/theme/theme1.xml` 的 `a:clrScheme`
  (dk1/lt1/dk2/lt2/accent1..6/hlink/…,取 srgbClr@val 或 sysClr@lastClr);
  `resolve_scheme_color` 处理默认 clrMap(tx1→dk1、bg1→lt1、tx2→dk2、bg2→lt2)。
  幻灯里 `schemeClr` 填充/文字色据此解析为 RRGGBB。
- 单测:theme 解析、schemeClr 解析并作用于填充、占位符借版式几何。

## 5. 自动化浏览器 e2e(@playwright/test)

- `web/playwright.config.ts`:`vite preview`(生产构建)起 4173 端口,chromium 项目。
- 夹具 `web/e2e/fixtures/`:`sample.docx`/`sample.pptx`(Rust `write_browser_fixture`
  生成)+ `sample.csv`;`pnpm e2e:fixtures` 可重生成。
- 用例(`web/e2e/*.spec.ts`,共 7 个):
  - **word**:docx 渲染标题/正文/图片/表格(canvas 非空像素断言)、滚动虚拟化;
  - **excel**:公式 D2=B2*C2=14、列过滤单价>3、冻结首行;
  - **ppt**:渲染 + 翻页到 2/2、演示模式进入/方向键/Esc 退出。
- `vite.config` 排除 `e2e/**`,避免 vitest 误跑 Playwright 用例。
- 脚本:`pnpm e2e` / `pnpm e2e:ui` / `pnpm e2e:fixtures`。

## 质量门禁

- `cargo fmt --check` ✅ · `clippy -D warnings` ✅ · `cargo test` **228**(docx6/pptx9/numfmt13/…)
- `pnpm typecheck` ✅ · `pnpm test`(vitest)**189** ✅ · `pnpm build` ✅
- `pnpm e2e`(Playwright chromium)**7/7** ✅
- WASM 体积 −38%。

## 复现

```bash
wasm-pack build crates/wasm --target web --out-dir web/src/wasm/pkg --out-name office_wasm
pnpm -C web e2e:fixtures   # 生成 e2e 夹具(需 Rust)
pnpm -C web e2e            # 跑浏览器 e2e
```

## 仍非目标

Word 分栏/页眉页脚/修订;PPT 动画/图表/SmartArt/旋转/文本默认样式继承;numfmt 颜色/条件/分数。
