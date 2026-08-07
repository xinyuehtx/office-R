# office-R

统一的 **Office 三件套**应用(文档 / 表格 / 演示):**Web 视图层 + Rust(WASM)计算内核**,纯静态部署到 GitHub Pages。

- 视图层:React + Vite + TypeScript(pnpm),表格用 **canvas** 绘制
- 计算内核:Rust → WASM,在浏览器内识别与解析 office 文件
- 数据不出浏览器:全程本地解析,不上传任何服务器

## 当前能力

**表格 · Excel**:上传 **CSV** 即可查看表格视图 —— 这是当前功能最完整的一块。

![CSV 表格视图](./docs/assets/csv-grid-overview.png)

- 自动识别编码(UTF-8 / UTF-16 / GBK 等,带不带 BOM 都行)与分隔符(`,` `\t` `;` `|`)
- 正确处理引号包裹、`""` 转义、字段内嵌换行、CRLF/LF 混用、参差行
- 行列头固定、滚轮 / 触控板双向滚动、Ctrl(⌘)+ 滚轮以指针为锚点缩放、
  方向键移动选区、拖拽平移、自绘滚动条
- 视口虚拟化 + **三层 canvas 叠加、由 GPU 合成**:50 万行 × 12 列(40 MB)首屏 < 0.6 s,
  滚动零掉帧且主线程绘制仅 0.53 ms/帧 —— 多数滚动帧只改一个 CSS transform、**完全不绘制**
  ([实测数据](./docs/reports/0001-csv-grid-acceptance.md))
- 解析在 Web Worker 中完成,主线程不冻结;非整数 dpr(浏览器缩放 / 125% 显示缩放)下文字不发虚
- **公式计算引擎(Rust/WASM)**:以 `=` 开头的单元格按 **Excel 语义**求值,内置 **140+ 函数**
  (SUM/IF/VLOOKUP/DATE/PMT…),对齐运算符优先级、错误值(`#DIV/0!`)、类型强制与循环检测;
  网格显示计算值,选中后公式栏回显原始公式。点页面上的「加载公式示例」即可体验
  ([RFC-0004](./docs/rfcs/0004-formula-engine.md))

本期**不含**数字/日期格式化、图表、单元格编辑与动态数组溢出 —— 详见
[RFC-0003](./docs/rfcs/0003-csv-canvas-grid.md) 与 [RFC-0004](./docs/rfcs/0004-formula-engine.md) 的范围说明。

**文档 · Word / 演示 · PowerPoint**:上传 `.docx` / `.pptx`,识别格式并给出解析摘要。

## 快速开始

```bash
# 1. 安装 wasm-pack(若未安装)
curl -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh

# 2. 构建 WASM 内核(产物输出到 web/src/wasm/pkg)
wasm-pack build crates/wasm --target web --out-dir ../../web/src/wasm/pkg --out-name office_wasm

# 3. 安装前端依赖并启动
pnpm install
pnpm -C web dev
```

浏览器打开后:在「表格」页上传 CSV 查看表格视图,在「文档 / 演示」页上传
`.docx` / `.pptx` 查看解析摘要。

> 排查问题时可加 `?logLevel=debug` 打开详细日志(前端与 WASM 两侧格式一致、可串联)。

## 测试

```bash
cargo test --all          # Rust 内核
cargo clippy --all-targets -- -D warnings
pnpm -C web typecheck
pnpm -C web test          # 前端
```

## 文档

- 规范总览(单一事实来源):[AGENTS.md](./AGENTS.md)
- 架构:[docs/architecture.md](./docs/architecture.md)
- 开发工作流(SDD + TDD):[docs/workflow.md](./docs/workflow.md)
- RFC / Spec / Story / 验收报告:[docs/](./docs/)

## 部署

推送到 `main` 由 GitHub Actions 自动构建并部署到 GitHub Pages。
首次需在仓库 **Settings → Pages → Build and deployment → Source** 选择 **GitHub Actions**。

部署地址:`https://<用户名>.github.io/office-R/`(启用后填写)

## 许可

MIT
