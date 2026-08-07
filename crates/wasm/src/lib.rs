//! office-wasm:将 [`office_core`] 暴露给 Web 视图层的 wasm-bindgen 绑定。
//!
//! 这一层刻意保持**轻薄**:只做类型转换、跨边界数据搬运与日志,
//! 不含业务逻辑 —— 所有解析与计算都在平台无关的 `office-core` 里,
//! 以便原生 `cargo test`。

mod csv_sheet;
mod log;
mod word;

pub use csv_sheet::{parse_csv_packed, CellWindow, PackedCsv, WasmSheet};
pub use log::set_log_level;
pub use word::WasmWordDoc;

use wasm_bindgen::prelude::*;

/// 模块初始化:安装 panic hook,便于在浏览器控制台看到 Rust panic 信息。
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// 返回计算内核版本。
#[wasm_bindgen]
pub fn version() -> String {
    office_core::version().to_string()
}

/// 识别文件格式,返回小写字符串:`docx` / `xlsx` / `pptx` / `csv` / `unknown`。
#[wasm_bindgen]
pub fn detect(bytes: &[u8]) -> String {
    match office_core::detect_format(bytes) {
        office_core::Format::Docx => "docx",
        office_core::Format::Xlsx => "xlsx",
        office_core::Format::Pptx => "pptx",
        office_core::Format::Csv => "csv",
        office_core::Format::Unknown => "unknown",
    }
    .to_string()
}

/// 读取 office 文件字节,识别并产出摘要,返回结构化结果。
///
/// 结果结构见 [`office_core::RenderResult`],通过 serde 序列化为 JS 对象。
/// CSV 的表格渲染走 [`parse_csv_packed`],不经过这里。
#[wasm_bindgen]
pub fn render(bytes: &[u8]) -> Result<JsValue, JsValue> {
    let result = office_core::render(bytes);
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}
