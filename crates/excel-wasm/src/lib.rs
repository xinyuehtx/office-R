//! **Excel (.xlsx) 与 CSV 的独立 wasm 后端**。
//!
//! CSV 与 xlsx 同在一个 cdylib —— `WasmWorkbook::sheet(i) -> WasmSheet` 要求
//! 两者在同一个 wasm 模块里(wasm-bindgen 的类型不能跨模块实例传递)。
//! 见 RFC-0007。

mod csv_sheet;
mod xlsx_dto;

pub use csv_sheet::{parse_csv_packed, CellWindow, PackedCsv, WasmSheet, WasmWorkbook};

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

/// 字节是否像一个 .xlsx。CSV 无魔数,识别仍靠扩展名(前端处理)。
#[wasm_bindgen(js_name = canOpen)]
pub fn can_open(bytes: &[u8]) -> bool {
    office_excel::can_open(bytes)
}
