//! CSV 表格的 WASM 绑定。
//!
//! # 跨边界数据流(为什么这么设计)
//!
//! 解析是重 CPU 的,必须放在 Web Worker 里,否则大文件会冻住 UI;
//! 而绘制每帧都要取「可见区域」的单元格,必须是**同步**的,否则会掉帧。
//! 两个诉求落在不同线程上,于是分成两段:
//!
//! ```text
//! Worker 线程                                  主线程
//! ─────────────────────────────────────────    ──────────────────────────────
//! parseCsvPacked(bytes)                        WasmSheet.fromPacked(buffers)
//!   → 解析 + 列切分 + 列宽度量(重 CPU)         → 重新装配为 Sheet
//!   → takeXxx() 移出紧凑缓冲(不克隆)          → window() 同步取可见区域
//!            └── postMessage(transfer) ───────┘   (ArrayBuffer 零拷贝转移)
//! ```
//!
//! 全程只有「移出 wasm 堆」和「移入 wasm 堆」两次必要拷贝,
//! 表格内容**不会**以 JS 字符串数组的形式整体存在。

use office_core::csv::{CsvMeta, CsvOptions};
use office_core::sheet::PackedSheet;
use office_core::Sheet;
use wasm_bindgen::prelude::*;

use crate::log::{self, Level};

/// 解析产出的紧凑缓冲,准备跨线程转移。
///
/// `take*` 系列方法是**一次性**的:它们把缓冲移出(而非克隆),
/// 再次调用会得到空数组。这样 200MB 的表格也不会因为取数据而翻倍占用内存。
#[wasm_bindgen]
pub struct PackedCsv {
    text: Vec<u8>,
    cell_ends: Vec<u32>,
    row_starts: Vec<u32>,
    col_width_units: Vec<u32>,
    cols: u32,
    meta: CsvMeta,
    parse_ms: f64,
}

#[wasm_bindgen]
impl PackedCsv {
    /// 移出文本缓冲(UTF-8)。
    #[wasm_bindgen(js_name = takeText)]
    pub fn take_text(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.text)
    }

    /// 移出单元格偏移数组。
    #[wasm_bindgen(js_name = takeCellEnds)]
    pub fn take_cell_ends(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.cell_ends)
    }

    /// 移出行索引数组。
    #[wasm_bindgen(js_name = takeRowStarts)]
    pub fn take_row_starts(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.row_starts)
    }

    /// 移出列宽数组(单位:半角字符数)。
    #[wasm_bindgen(js_name = takeColWidthUnits)]
    pub fn take_col_width_units(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.col_width_units)
    }

    /// 列数。
    #[wasm_bindgen(getter)]
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// 解析元信息(编码、分隔符、行列数、截断标记等)。不含单元格内容。
    #[wasm_bindgen(getter)]
    pub fn meta(&self) -> Result<JsValue, JsValue> {
        meta_to_js(&self.meta, self.parse_ms)
    }
}

/// 一块可见区域的单元格文本。
///
/// 视图层用 `ends` 在 `text` 上切片,得到每个单元格 —— 一次调用取回整屏数据。
#[wasm_bindgen]
pub struct CellWindow {
    text: String,
    ends: Vec<u32>,
    rows: u32,
    cols: u32,
}

#[wasm_bindgen]
impl CellWindow {
    /// 区域内所有单元格文本首尾相接。
    #[wasm_bindgen(js_name = takeText)]
    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text)
    }

    /// 每个单元格在文本中的结束字节偏移。
    #[wasm_bindgen(js_name = takeEnds)]
    pub fn take_ends(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.ends)
    }

    /// 区域行数。
    #[wasm_bindgen(getter)]
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// 区域列数。
    #[wasm_bindgen(getter)]
    pub fn cols(&self) -> u32 {
        self.cols
    }
}

/// 主线程持有的表格句柄。
///
/// 表格数据始终留在 WASM 线性内存里,JS 侧只拿到这个句柄;
/// 用完请调用 `free()`(wasm-bindgen 生成)释放。
#[wasm_bindgen]
pub struct WasmSheet {
    sheet: Sheet,
}

#[wasm_bindgen]
impl WasmSheet {
    /// 从 Worker 转移过来的紧凑缓冲重建表格。
    #[wasm_bindgen(js_name = fromPacked)]
    pub fn from_packed(
        text: Vec<u8>,
        cell_ends: Vec<u32>,
        row_starts: Vec<u32>,
        cols: u32,
        col_width_units: Vec<u32>,
    ) -> Result<WasmSheet, JsValue> {
        let packed = PackedSheet {
            text,
            cell_ends,
            row_starts,
            cols,
            col_width_units,
        };
        Sheet::from_packed(packed)
            .map(|sheet| WasmSheet { sheet })
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// 行数。
    #[wasm_bindgen(getter)]
    pub fn rows(&self) -> u32 {
        self.sheet.rows() as u32
    }

    /// 列数。
    #[wasm_bindgen(getter)]
    pub fn cols(&self) -> u32 {
        self.sheet.cols() as u32
    }

    /// 各列建议显示宽度(单位:半角字符数)。
    #[wasm_bindgen(js_name = colWidthUnits)]
    pub fn col_width_units(&self) -> Vec<u32> {
        self.sheet.col_width_units().to_vec()
    }

    /// 取 `[row0, row1) × [col0, col1)` 区域的单元格文本。越界入参会被夹紧。
    pub fn window(&self, row0: u32, row1: u32, col0: u32, col1: u32) -> CellWindow {
        let w = self
            .sheet
            .window(row0 as usize, row1 as usize, col0 as usize, col1 as usize);
        CellWindow {
            text: w.text,
            ends: w.ends,
            rows: w.rows as u32,
            cols: w.cols as u32,
        }
    }
}

/// 在 Worker 中解析 CSV,产出可转移的紧凑缓冲。
///
/// `trace_id` 由前端生成,用于把 WASM 与前端的日志串成一次会话。
/// `delimiter` 传 0 表示自动嗅探。
#[wasm_bindgen(js_name = parseCsvPacked)]
pub fn parse_csv_packed(bytes: &[u8], trace_id: &str, delimiter: u8) -> Result<PackedCsv, JsValue> {
    let started = log::now_ms();
    let options = CsvOptions {
        delimiter: (delimiter != 0).then_some(delimiter),
        ..Default::default()
    };

    log::log(
        Level::Debug,
        trace_id,
        "csv.parse.start",
        &format!("bytes={}", bytes.len()),
    );

    let document = office_core::csv::parse_with(bytes, &options).map_err(|err| {
        log::log(
            Level::Error,
            trace_id,
            "csv.parse.failed",
            &format!("bytes={} reason={err}", bytes.len()),
        );
        JsValue::from_str(&err.to_string())
    })?;

    let parse_ms = log::now_ms() - started;
    let meta = document.meta.clone();
    log::log(
        Level::Info,
        trace_id,
        "csv.parse.ok",
        &format!(
            "bytes={} rows={} cols={} encoding={} delimiter={:?} truncated={} ms={:.1}",
            bytes.len(),
            meta.rows,
            meta.cols,
            meta.encoding,
            meta.delimiter as char,
            meta.truncated_rows || meta.truncated_cols,
            parse_ms
        ),
    );

    let packed = document.sheet.into_packed();
    Ok(PackedCsv {
        text: packed.text,
        cell_ends: packed.cell_ends,
        row_starts: packed.row_starts,
        col_width_units: packed.col_width_units,
        cols: packed.cols,
        meta,
        parse_ms,
    })
}

/// 把元信息序列化成 JS 对象(字段名用 camelCase,贴合前端习惯)。
fn meta_to_js(meta: &CsvMeta, parse_ms: f64) -> Result<JsValue, JsValue> {
    let object = js_sys::Object::new();
    let set = |key: &str, value: JsValue| -> Result<(), JsValue> {
        js_sys::Reflect::set(&object, &JsValue::from_str(key), &value)?;
        Ok(())
    };
    set("encoding", JsValue::from_str(&meta.encoding))?;
    set(
        "delimiter",
        JsValue::from_str(&(meta.delimiter as char).to_string()),
    )?;
    set(
        "delimiterSource",
        JsValue::from_str(match meta.delimiter_source {
            office_core::csv::DelimiterSource::Explicit => "explicit",
            office_core::csv::DelimiterSource::Sniffed => "sniffed",
            office_core::csv::DelimiterSource::Fallback => "fallback",
        }),
    )?;
    set("lossy", JsValue::from_bool(meta.lossy))?;
    set("rows", JsValue::from_f64(meta.rows as f64))?;
    set("cols", JsValue::from_f64(meta.cols as f64))?;
    set("truncatedRows", JsValue::from_bool(meta.truncated_rows))?;
    set("truncatedCols", JsValue::from_bool(meta.truncated_cols))?;
    set("parseMs", JsValue::from_f64(parse_ms))?;
    Ok(object.into())
}
