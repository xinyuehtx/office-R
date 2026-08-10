//! **Word (.docx) 的独立 wasm 后端**。
//!
//! 只把 `office-word` 的解析结果搬过跨语言边界。产物 `office_word_wasm` 里
//! 不含 calamine / pptx —— 只想渲染 Word 的消费方不会为另两种格式买单。

mod word;
pub use word::WasmWordDoc;

use wasm_bindgen::prelude::*;

// setLogLevel 从共用 rlib 转发(前端 ensureReady 会向本模块调一次)。
pub use office_wasm_log::set_log_level;

/// 模块初始化:安装 panic hook。
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// 本 wasm 后端的版本(工作区版本)。
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 字节是否像一个 .docx —— 供前端做文件路由,无需装另两个包。
#[wasm_bindgen(js_name = canOpen)]
pub fn can_open(bytes: &[u8]) -> bool {
    office_word::can_open(bytes)
}
