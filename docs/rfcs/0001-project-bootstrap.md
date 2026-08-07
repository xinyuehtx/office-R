# RFC-0001: 项目初始化(Web 视图 + Rust WASM 内核)

- **状态**:已实现
- **作者**:office-R 团队
- **创建日期**:2026-08-07
- **关联**:Spec-0001、Story-0001

## 动机

需要一个**统一的 Office 三件套应用**(文档 / 表格 / 演示)。核心诉求:

- 视图层用 Web,便于跨平台访问与部署。
- 计算逻辑用 Rust,追求性能与正确性,且可复用。
- 能读取并渲染 office 文件,并可作为「页面」部署对外访问。

## 方案

- **计算内核**:Rust,拆为 `office-core`(平台无关)+ `office-wasm`(wasm-bindgen 绑定),
  编译为 WASM 在浏览器内运行。
- **视图层**:React + Vite + TypeScript,pnpm 管理;三个页面(Word/Excel/PPT)各自具备上传入口。
- **部署**:纯静态产物,GitHub Actions 构建后部署到 GitHub Pages,无需服务器。
- **首个切片**:仅搭骨架 —— 打通「上传 → 识别格式 → 占位渲染」端到端链路,暂不做真实 OOXML 解析。
- **工作流**:RFC → Spec/Story/Test → SDD+TDD → CI → 部署;每次提交前更新项目网站与架构文档。

## 取舍与备选方案

| 决策 | 选择 | 备选 | 理由 |
| --- | --- | --- | --- |
| 内核运行形态 | **WASM 浏览器内** | Rust 后端服务 | 契合纯静态 Pages 部署,无服务器成本 |
| 前端框架 | **React** | Vue / Svelte | 生态成熟,office 类复杂 UI 组件丰富 |
| 首切片范围 | **仅骨架占位** | 先做完整 Word | 优先打通全流程与 CI/部署,降低风险 |

## 影响

- 建立 monorepo:cargo workspace + pnpm 工作区。
- 引入 wasm-pack 作为构建工具(本地与 CI 均需安装)。
- 需在仓库 Settings 启用 GitHub Pages(Actions 部署源)。

## 未决问题

- 三个组件真实解析的优先级与拆分方式(后续各自开 RFC)。
- 是否需要 Web Worker 承载重计算以避免阻塞 UI。
