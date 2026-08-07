//! office-wasm:将 [`office_core`] 暴露给 Web 视图层的 wasm-bindgen 绑定。

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

/// 识别文件格式,返回小写字符串:`docx` / `xlsx` / `pptx` / `unknown`。
#[wasm_bindgen]
pub fn detect(bytes: &[u8]) -> String {
    match office_core::detect_format(bytes) {
        office_core::Format::Docx => "docx",
        office_core::Format::Xlsx => "xlsx",
        office_core::Format::Pptx => "pptx",
        office_core::Format::Unknown => "unknown",
    }
    .to_string()
}

/// 读取 office 文件字节,识别并(占位)渲染,返回结构化结果。
///
/// 结果结构见 [`office_core::RenderResult`],通过 serde 序列化为 JS 对象。
#[wasm_bindgen]
pub fn render(bytes: &[u8]) -> Result<JsValue, JsValue> {
    let result = office_core::render(bytes);
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}
