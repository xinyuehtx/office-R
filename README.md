# office-R

统一的 **Office 三件套**应用(文档 / 表格 / 演示):**Web 视图层 + Rust(WASM)计算内核**,纯静态部署到 GitHub Pages。

- 视图层:React + Vite + TypeScript(pnpm)
- 计算内核:Rust → WASM,在浏览器内识别与渲染 office 文件
- 当前为**骨架阶段**:三个页面各具上传入口,打通「上传 → 识别 → 占位渲染」全链路

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

浏览器打开后,在「文档 / 表格 / 演示」任一页面上传对应 office 文件即可看到识别结果。

## 测试

```bash
cargo test --all          # Rust 内核
pnpm -C web test          # 前端
```

## 文档

- 规范总览(单一事实来源):[AGENTS.md](./AGENTS.md)
- 架构:[docs/architecture.md](./docs/architecture.md)
- 开发工作流(SDD + TDD):[docs/workflow.md](./docs/workflow.md)
- RFC / Spec / Story:[docs/](./docs/)

## 部署

推送到 `main` 由 GitHub Actions 自动构建并部署到 GitHub Pages。
首次需在仓库 **Settings → Pages → Build and deployment → Source** 选择 **GitHub Actions**。

部署地址:`https://<用户名>.github.io/office-R/`(启用后填写)

## 许可

MIT
