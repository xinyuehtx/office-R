# CLAUDE.md

本项目的开发规范、架构、命令与工作流以 **[AGENTS.md](./AGENTS.md)** 为**单一事实来源**,请先阅读它。

关键补充(便于快速上手):

- 架构详解见 [docs/architecture.md](./docs/architecture.md);开发流程见 [docs/workflow.md](./docs/workflow.md)。
- 修改功能后,**每次提交前必须更新项目网站(`web/`)与架构文档**,见 AGENTS.md 的「提交前检查清单」。
- 前端在仓库根用 pnpm 运行(如 `pnpm -C web test`);Rust 用 `cargo`;WASM 用 `wasm-pack`。
