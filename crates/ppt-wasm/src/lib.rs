//! **PowerPoint (.pptx) 的独立 wasm 后端**。
//!
//! 只把 `office-ppt` 的解析结果搬过跨语言边界。产物里不含 calamine / docx-rs。

mod ppt;
pub use ppt::WasmPresentation;

use wasm_bindgen::prelude::*;

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

/// 字节是否像一个 .pptx。
#[wasm_bindgen(js_name = canOpen)]
pub fn can_open(bytes: &[u8]) -> bool {
    office_ppt::can_open(bytes)
}
