# RFC-0002: 核心依赖选型(异步与 OOXML 解析)

- **状态**:已实现
- **作者**:office-R 团队
- **创建日期**:2026-08-07
- **关联**:RFC-0001、Spec-0001

## 动机

骨架跑通后需引入核心依赖:异步运行时(如 tokio)与开源 OOXML 解析库,支撑后续真实解析。

## 方案

### 1. 异步:当前不引入 tokio

主目标是 `wasm32-unknown-unknown`(浏览器),**tokio 在此基本不可用**:

- 空闲时会阻塞 UI 线程;`tokio::time` 未接 `web-time` 时会 panic;
- 其同步原语在主线程可能触发 `Atomics.wait cannot be called in this context`;
- 仅在 Web Worker 中、且加特殊编译标志才勉强可用(需 `tokio_with_wasm` shim)。

而 office 解析是**同步 CPU 密集型**工作,浏览器侧的异步应在 JS 边界用 `wasm-bindgen-futures` 处理。
因此 **office-core 保持同步、平台无关**,暂不引入 tokio;待未来出现 native(CLI/server)目标时,
再在独立 native crate 或 feature gate 下引入,绝不编入 wasm。

### 2. OOXML 解析:单一用途成熟包

| 格式 | 选用 | 说明 |
| --- | --- | --- |
| xlsx | **calamine** | 纯 Rust、最成熟的电子表格读取库,已验证可在 wasm 运行 |
| docx | **docx-rs** | 下载量/星标最高的 docx 库,官方支持 WebAssembly |
| pptx | **zip + quick-xml** | pptx 无成熟专用库,自行用 zip 开容器、quick-xml 解析 |
| 容器/XML | **zip**、**quick-xml** | `zip` 关闭默认 features、仅留 `deflate` 以兼容 wasm(避免 bzip2/zstd 的 C 依赖) |
| 错误 | **thiserror** | 轻量错误定义 |

## 取舍与备选方案

- 备选统一多格式 crate(`ooxmlsdk` / `office_oxide`):API 统一但较新、稳定性待验证,暂不选。
- `zip` 若用默认 features 会引入 bzip2/zstd 等 C 库,**无法编译到 wasm**,故显式 `--no-default-features --features deflate`。

## 影响

- wasm 产物体积从 ~30KB 增至 ~1.4MB(`docx-rs` 传递引入 `image`/`png`/`tiff`)。
  后续可评估按需拆分 feature 或延迟加载以优化体积。
- `office-core` 各组件由纯占位升级为**最小真实解析**(见下)。

## 未决问题

- 是否为 docx 的图片解析做 feature 开关以裁剪 wasm 体积。
- pptx 后续是否引入/自研更完整的解析层。
