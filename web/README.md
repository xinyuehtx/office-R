# web/ — 三合一演示站

本目录**不是**某个应用,而是把 `@tengxiaohyx/office-{word,excel,ppt}` 三个独立包
组合起来的**演示壳**:顶部 Tab 切换,`App.tsx` 各 `import { WordPage / ExcelPage /
PptPage }` 自对应包。三个应用及其 wasm 后端都能脱离本站单独使用 —— 见
[RFC-0007](../docs/rfcs/0007-monorepo-split.md)。

- 页面组件、渲染管线、wasm 加载器都在 `packages/*`,本目录只有壳与 e2e。
- 本地开发:仓库根 `pnpm -C web dev`(先构建三份 wasm,见 AGENTS.md)。
- e2e(`web/e2e/`)驱动真实构建产物,覆盖三个应用;夹具由根 `xtask` 生成。
