# 架构文档

> 本文件描述 office-R 的整体架构。**每次提交前若架构或目录发生变化,必须同步更新本文件。**

## 定位

office-R 是一个**统一的 Office 三件套应用**(文档 / 表格 / 演示),特点:

- **视图层 = Web**:React + Vite + TypeScript。
- **计算内核 = Rust**:编译为 **WASM**,在浏览器内直接识别与(未来)解析 office 文件。
- **纯静态部署**:产物为静态资源,部署到 GitHub Pages,无需后端服务器。

## 分层

```
┌─────────────────────────────────────────────┐
│  Web 视图层 (web/, React + Vite + TS)          │
│  ├─ App.tsx        顶部 Tab:文档/表格/演示      │
│  ├─ apps/{word,excel,ppt}  三个页面,各自上传入口 │
│  ├─ apps/shared/   OfficePage / FileUpload /   │
│  │                 useOfficeFile(复用上传逻辑)  │
│  └─ wasm/index.ts  加载 & 封装 WASM 模块         │
└───────────────┬─────────────────────────────┘
                │ wasm-bindgen (Uint8Array → JS 对象)
┌───────────────▼─────────────────────────────┐
│  绑定层 office-wasm (crates/wasm/)             │
│  version() / detect() / render()              │
└───────────────┬─────────────────────────────┘
                │ 纯 Rust 调用
┌───────────────▼─────────────────────────────┐
│  计算内核 office-core (crates/core/)           │
│  ├─ format.rs   detect_format() 识别格式        │
│  ├─ render.rs   RenderResult 数据结构           │
│  ├─ word.rs     docx-rs 解析(段落数)          │
│  ├─ excel.rs    calamine 解析(工作表/尺寸)     │
│  ├─ ppt.rs      zip + quick-xml 解析(幻灯片)   │
│  └─ lib.rs      render() 统一分发入口           │
└─────────────────────────────────────────────┘
```

**关键设计**:`office-core` 平台无关、不依赖任何浏览器 API,因此可原生 `cargo test`;
`office-wasm` 仅做 wasm-bindgen 绑定与序列化,薄薄一层。

## 核心依赖(见 RFC-0002)

| 用途 | crate | 备注 |
| --- | --- | --- |
| xlsx 读取 | `calamine` | 纯 Rust,已验证 wasm 可用 |
| docx 读取 | `docx-rs` | 官方支持 WebAssembly |
| pptx / 容器 / XML | `zip`(仅 `deflate`)+ `quick-xml` | `zip` 关默认 features 以兼容 wasm |
| 错误 | `thiserror` | 轻量错误定义 |

> **异步**:主目标是浏览器 wasm,tokio 在此基本不可用(阻塞 UI、timer panic 等),
> 故当前**不引入 tokio**;office 解析为同步 CPU 密集型。详见 RFC-0002。

## 数据流:读取并渲染一个 office 文件

1. 用户在某个页面点击上传入口,选择 `.docx/.xlsx/.pptx`。
2. `useOfficeFile` 读取文件为 `Uint8Array`。
3. 调用 WASM `render(bytes)`。
4. `office-core::render` 先 `detect_format`(扫描 ZIP 魔数与特征目录 `word/`、`xl/`、`ppt/`),
   再分发到对应组件的 `render_placeholder`。
5. 返回 `RenderResult { format, format_name, byte_len, message, ok }`,页面渲染。

> 当前为**最小真实解析**阶段:xlsx 读工作表/尺寸、docx 读段落数、pptx 读幻灯片与文本块数;
> 解析失败会以 `ok=false` + 原因优雅降级,不 panic。后续按 RFC/Spec 逐步丰富解析深度。

## 目录结构

```
office-R/
├── crates/            Rust cargo workspace
│   ├── core/          office-core:平台无关计算内核
│   └── wasm/          office-wasm:wasm-bindgen 绑定
├── web/               React + Vite + TS 视图层(pnpm 管理)
│   └── src/wasm/pkg/  wasm-pack 生成产物(不入库)
├── docs/              RFC / Spec / Story / 工作流 / 架构
└── .github/workflows/ CI 与 Pages 部署
```

## 构建与部署

- WASM:`wasm-pack build crates/wasm --target web --out-dir web/src/wasm/pkg`
- 前端:`cd web && pnpm build`(通过 `VITE_BASE` 设置 Pages 子路径)
- 部署:推送 `main` → GitHub Actions 构建 WASM + 前端 → 部署到 GitHub Pages
