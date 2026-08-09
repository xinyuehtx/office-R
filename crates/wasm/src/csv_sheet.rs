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
use office_core::filter::{ColumnFilter, NumOp, Predicate, TextOp};
use office_core::sheet::PackedSheet;
use office_core::Sheet;
use serde::Deserialize;
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
    /// 公式单元格清单(row, col, 原始文本),供公式栏回显。空表示本文件无公式。
    formulas: Vec<office_core::formula::CellFormula>,
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

    /// 公式单元格清单,序列化为 `[{row, col, formula}, ...]`(0 基下标)。
    ///
    /// 公式的原始文本会展示在公式栏,是**用户自己写的公式**、不涉及其它单元格内容,
    /// 因此可安全跨边界传递。
    #[wasm_bindgen(getter)]
    pub fn formulas(&self) -> Result<JsValue, JsValue> {
        formulas_to_js(&self.formulas)
    }
}

/// 公式清单 → JS 数组 `[{row, col, formula}, ...]`。CSV 与 xlsx 共用。
fn formulas_to_js(formulas: &[office_core::formula::CellFormula]) -> Result<JsValue, JsValue> {
    let array = js_sys::Array::new();
    for f in formulas {
        let obj = js_sys::Object::new();
        js_sys::Reflect::set(&obj, &"row".into(), &JsValue::from_f64(f.row as f64))?;
        js_sys::Reflect::set(&obj, &"col".into(), &JsValue::from_f64(f.col as f64))?;
        js_sys::Reflect::set(&obj, &"formula".into(), &JsValue::from_str(&f.source))?;
        array.push(&obj);
    }
    Ok(array.into())
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
    /// 过滤命中的底层行(`None` 表示未过滤)。
    filter_rows: Option<Vec<u32>>,
    /// 排序:`(列, 是否升序)`;`None` 表示未排序。
    sort_spec: Option<(u32, bool)>,
    /// 最近一次过滤/排序用的表头行数(重建复合映射时复用)。
    header_rows: u32,
    /// 过滤 + 排序复合后的「可视行 → 底层行」映射;`None` 表示恒等(可视行即底层行)。
    ///
    /// 关键点:可视行始终是连续的 `0..len`,渲染器的几何(等高行、列前缀和)因此
    /// **完全复用**,过滤/排序对渲染器透明 —— 只是行头标签要显示底层行号(见 `rowLabel`)。
    row_map: Option<Vec<u32>>,
}

impl WasmSheet {
    /// 从已构建的 [`Sheet`] 直接封装(xlsx 路径用;不跨 Worker,无需 packed）。
    pub(crate) fn from_sheet(sheet: Sheet) -> WasmSheet {
        WasmSheet {
            sheet,
            filter_rows: None,
            sort_spec: None,
            header_rows: 1,
            row_map: None,
        }
    }

    /// 按当前过滤与排序重建复合行映射。都不生效时 `row_map` 归位为 `None`。
    fn rebuild(&mut self) {
        let base: Option<Vec<u32>> = match (&self.filter_rows, self.sort_spec) {
            (None, None) => None,
            (Some(f), None) => Some(f.clone()),
            (filt, Some((col, asc))) => {
                let base: Vec<u32> = match filt {
                    Some(f) => f.clone(),
                    None => (0..self.sheet.rows() as u32).collect(),
                };
                Some(office_core::filter::sort_rows(
                    &self.sheet,
                    &base,
                    col,
                    asc,
                    self.header_rows,
                ))
            }
        };
        self.row_map = base;
    }
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
            .map(WasmSheet::from_sheet)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// 行数(过滤后为可视行数)。
    #[wasm_bindgen(getter)]
    pub fn rows(&self) -> u32 {
        match &self.row_map {
            Some(map) => map.len() as u32,
            None => self.sheet.rows() as u32,
        }
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
    ///
    /// 有过滤时,可视行区间会先经 `row_map` 映射到(可能不连续的)底层行再取数。
    pub fn window(&self, row0: u32, row1: u32, col0: u32, col1: u32) -> CellWindow {
        let w = match &self.row_map {
            Some(map) => {
                let r0 = (row0 as usize).min(map.len());
                let r1 = (row1 as usize).clamp(r0, map.len());
                let underlying: Vec<usize> = map[r0..r1].iter().map(|&r| r as usize).collect();
                self.sheet
                    .window_rows(&underlying, col0 as usize, col1 as usize)
            }
            None => self
                .sheet
                .window(row0 as usize, row1 as usize, col0 as usize, col1 as usize),
        };
        CellWindow {
            text: w.text,
            ends: w.ends,
            rows: w.rows as u32,
            cols: w.cols as u32,
        }
    }

    /// 可视行对应的**底层行号**(0 基);行头据此显示原始行号。未过滤时即入参。
    #[wasm_bindgen(js_name = rowLabel)]
    pub fn row_label(&self, visual: u32) -> u32 {
        match &self.row_map {
            Some(map) => map.get(visual as usize).copied().unwrap_or(visual),
            None => visual,
        }
    }

    /// 应用列过滤:在内核侧全表扫描算出命中行,存为行映射,返回可视行数。
    ///
    /// `specs` 是紧凑 JSON(见 RFC-0005),`header_rows` 是顶部始终保留的行数。
    pub fn filter(&mut self, specs: JsValue, header_rows: u32) -> Result<u32, JsValue> {
        let dtos: Vec<FilterSpec> =
            serde_wasm_bindgen::from_value(specs).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let mut filters = Vec::with_capacity(dtos.len());
        for d in dtos {
            filters.push(d.into_core()?);
        }
        let rows = office_core::filter::filter_rows(&self.sheet, &filters, header_rows);
        self.header_rows = header_rows;
        self.filter_rows = Some(rows);
        self.rebuild();
        Ok(self.rows())
    }

    /// 清除过滤,恢复全量(排序若在,继续保留)。
    #[wasm_bindgen(js_name = clearFilter)]
    pub fn clear_filter(&mut self) {
        self.filter_rows = None;
        self.rebuild();
    }

    /// 按第 `col` 列排序:`dir` 为 `"asc"` / `"desc"` 排序,其它值(如 `"none"`)取消排序。
    ///
    /// 与过滤复合:排序作用在当前过滤结果之上。返回可视行数。
    pub fn sort(&mut self, col: u32, dir: &str, header_rows: u32) -> u32 {
        self.header_rows = header_rows;
        self.sort_spec = match dir {
            "asc" => Some((col, true)),
            "desc" => Some((col, false)),
            _ => None,
        };
        self.rebuild();
        self.rows()
    }

    /// 全表查找:返回所有命中单元格的**可视坐标** `[{row, col}, ...]`(受过滤/排序影响,
    /// 搜索的是当前所见内容)。按可视行优先、列其次的顺序;最多 `limit` 个。
    ///
    /// `case_sensitive` 区分大小写;`whole_cell` 为真时整格精确相等,否则子串包含。
    pub fn find(
        &self,
        needle: &str,
        case_sensitive: bool,
        whole_cell: bool,
        limit: u32,
    ) -> Result<JsValue, JsValue> {
        let array = js_sys::Array::new();
        if needle.is_empty() {
            return Ok(array.into());
        }
        let cols = self.sheet.cols();
        let vis_rows = self.rows() as usize;
        let needle_cmp = if case_sensitive {
            needle.to_string()
        } else {
            needle.to_lowercase()
        };
        let mut count = 0u32;
        'outer: for v in 0..vis_rows {
            // 可视行 → 底层行
            let underlying = match &self.row_map {
                Some(map) => map[v] as usize,
                None => v,
            };
            for c in 0..cols {
                let cell = self.sheet.cell(underlying, c);
                let hay = if case_sensitive {
                    cell.to_string()
                } else {
                    cell.to_lowercase()
                };
                let hit = if whole_cell {
                    hay == needle_cmp
                } else {
                    hay.contains(&needle_cmp)
                };
                if hit {
                    let obj = js_sys::Object::new();
                    js_sys::Reflect::set(&obj, &"row".into(), &JsValue::from_f64(v as f64))?;
                    js_sys::Reflect::set(&obj, &"col".into(), &JsValue::from_f64(c as f64))?;
                    array.push(&obj);
                    count += 1;
                    if count >= limit {
                        break 'outer;
                    }
                }
            }
        }
        Ok(array.into())
    }

    /// 枚举某列的唯一值(供值集过滤 UI),跳过顶部 `header_rows` 行,最多 `limit` 个。
    /// 返回 `{ values: string[], truncated: boolean }`。
    #[wasm_bindgen(js_name = uniqueValues)]
    pub fn unique_values(
        &self,
        col: u32,
        header_rows: u32,
        limit: u32,
    ) -> Result<JsValue, JsValue> {
        let (values, truncated) = office_core::filter::column_unique_values(
            &self.sheet,
            col,
            header_rows,
            limit as usize,
        );
        let obj = js_sys::Object::new();
        let arr = js_sys::Array::new();
        for v in values {
            arr.push(&JsValue::from_str(&v));
        }
        js_sys::Reflect::set(&obj, &"values".into(), &arr)?;
        js_sys::Reflect::set(&obj, &"truncated".into(), &JsValue::from_bool(truncated))?;
        Ok(obj.into())
    }
}

/// 前端传入的单列过滤规格(紧凑 JSON,经 serde 反序列化)。
#[derive(Deserialize)]
struct FilterSpec {
    col: u32,
    /// `"values" | "text" | "number" | "blank"`
    kind: String,
    #[serde(default)]
    op: String,
    #[serde(default)]
    needle: String,
    #[serde(default)]
    a: f64,
    #[serde(default)]
    b: f64,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    blank: bool,
}

impl FilterSpec {
    fn into_core(self) -> Result<ColumnFilter, JsValue> {
        let predicate = match self.kind.as_str() {
            "values" => Predicate::Values(self.values),
            "blank" => Predicate::Blank(self.blank),
            "text" => Predicate::Text {
                op: match self.op.as_str() {
                    "contains" => TextOp::Contains,
                    "notContains" => TextOp::NotContains,
                    "equals" => TextOp::Equals,
                    "begins" => TextOp::Begins,
                    "ends" => TextOp::Ends,
                    other => return Err(JsValue::from_str(&format!("未知文本运算:{other}"))),
                },
                needle: self.needle,
            },
            "number" => Predicate::Number {
                op: match self.op.as_str() {
                    "eq" => NumOp::Eq,
                    "ne" => NumOp::Ne,
                    "gt" => NumOp::Gt,
                    "ge" => NumOp::Ge,
                    "lt" => NumOp::Lt,
                    "le" => NumOp::Le,
                    "between" => NumOp::Between,
                    other => return Err(JsValue::from_str(&format!("未知数值运算:{other}"))),
                },
                a: self.a,
                b: self.b,
            },
            other => return Err(JsValue::from_str(&format!("未知过滤类型:{other}"))),
        };
        Ok(ColumnFilter {
            col: self.col,
            predicate,
        })
    }
}

/// 在 Worker 中解析 CSV,产出可转移的紧凑缓冲。
///
/// `trace_id` 由前端生成,用于把 WASM 与前端的日志串成一次会话。
/// `delimiter` 传 0 表示自动嗅探。`now_serial` 是注入给公式 `TODAY`/`NOW` 的
/// 当前时间序列数(前端用 `Date` 换算;传 0 表示不需要)。
///
/// 若 CSV 里含有以 `=` 开头的**公式**单元格,这里会在内核侧求值,产出的紧凑缓冲
/// 里公式格是**计算结果**(与 Excel 「单元格显示值」一致),原始公式经 [`PackedCsv::formulas`]
/// 单独回传给公式栏。没有公式时零额外开销,行为与旧版完全一致。
#[wasm_bindgen(js_name = parseCsvPacked)]
pub fn parse_csv_packed(
    bytes: &[u8],
    trace_id: &str,
    delimiter: u8,
    now_serial: f64,
) -> Result<PackedCsv, JsValue> {
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

    // 求值公式(如有)。得到 Some 则用计算后的显示表,否则沿用原表。
    let (sheet, formulas) = match office_core::formula::evaluate_sheet(&document.sheet, now_serial)
    {
        Some(grid) => (grid.display, grid.formulas),
        None => (document.sheet, Vec::new()),
    };

    log::log(
        Level::Info,
        trace_id,
        "csv.parse.ok",
        &format!(
            "bytes={} rows={} cols={} encoding={} delimiter={:?} truncated={} formulas={} ms={:.1}",
            bytes.len(),
            meta.rows,
            meta.cols,
            meta.encoding,
            meta.delimiter as char,
            meta.truncated_rows || meta.truncated_cols,
            formulas.len(),
            parse_ms
        ),
    );

    let packed = sheet.into_packed();
    Ok(PackedCsv {
        text: packed.text,
        cell_ends: packed.cell_ends,
        row_starts: packed.row_starts,
        col_width_units: packed.col_width_units,
        cols: packed.cols,
        meta,
        parse_ms,
        formulas,
    })
}

/// 一个 xlsx **工作簿**句柄:持有多张工作表,按需取出为 [`WasmSheet`]。
///
/// xlsx 自带缓存计算值,故不重算;显示表取 calamine 给的值,公式原文单独回传。
#[wasm_bindgen]
pub struct WasmWorkbook {
    sheets: Vec<office_core::xlsx::XlsxSheet>,
    media: Vec<office_core::xlsx::XlsxMedia>,
}

#[wasm_bindgen]
impl WasmWorkbook {
    /// 解析 xlsx 字节为工作簿句柄。
    pub fn parse(bytes: &[u8]) -> Result<WasmWorkbook, JsValue> {
        let wb = office_core::xlsx::parse(bytes).map_err(|e| JsValue::from_str(&e))?;
        Ok(WasmWorkbook {
            sheets: wb.sheets,
            media: wb.media,
        })
    }

    /// 工作表数量。
    #[wasm_bindgen(js_name = sheetCount)]
    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    /// 各工作表名(按原始顺序)。
    #[wasm_bindgen(js_name = sheetNames)]
    pub fn sheet_names(&self) -> Vec<String> {
        self.sheets.iter().map(|s| s.name.clone()).collect()
    }

    /// 取第 `i` 张工作表为可同步取数的表格句柄(克隆其只读表)。
    pub fn sheet(&self, i: usize) -> Result<WasmSheet, JsValue> {
        let s = self
            .sheets
            .get(i)
            .ok_or_else(|| JsValue::from_str("工作表下标越界"))?;
        Ok(WasmSheet::from_sheet(s.sheet.clone()))
    }

    /// 第 `i` 张工作表的公式清单 `[{row, col, formula}, ...]`。
    pub fn formulas(&self, i: usize) -> Result<JsValue, JsValue> {
        let s = self
            .sheets
            .get(i)
            .ok_or_else(|| JsValue::from_str("工作表下标越界"))?;
        formulas_to_js(&s.formulas)
    }

    /// 第 `i` 张工作表的非默认单元格样式 `[{row,col,bold,italic,color,fill,align}, ...]`。
    pub fn styles(&self, i: usize) -> Result<JsValue, JsValue> {
        let s = self
            .sheets
            .get(i)
            .ok_or_else(|| JsValue::from_str("工作表下标越界"))?;
        let arr = js_sys::Array::new();
        for (row, col, f) in &s.formats {
            let o = js_sys::Object::new();
            js_sys::Reflect::set(&o, &"row".into(), &JsValue::from_f64(*row as f64))?;
            js_sys::Reflect::set(&o, &"col".into(), &JsValue::from_f64(*col as f64))?;
            js_sys::Reflect::set(&o, &"bold".into(), &JsValue::from_bool(f.bold))?;
            js_sys::Reflect::set(&o, &"italic".into(), &JsValue::from_bool(f.italic))?;
            if let Some(c) = &f.color {
                js_sys::Reflect::set(&o, &"color".into(), &JsValue::from_str(c))?;
            }
            if let Some(c) = &f.fill {
                js_sys::Reflect::set(&o, &"fill".into(), &JsValue::from_str(c))?;
            }
            if let Some(a) = &f.align {
                js_sys::Reflect::set(&o, &"align".into(), &JsValue::from_str(a))?;
            }
            if let Some(b) = &f.border {
                // border: { top?:{w,color}, right?, bottom?, left? }
                let bo = js_sys::Object::new();
                let set_side = |name: &str,
                                side: &Option<office_core::xlsx::BorderSide>|
                 -> Result<(), JsValue> {
                    if let Some(sd) = side {
                        let so = js_sys::Object::new();
                        js_sys::Reflect::set(&so, &"w".into(), &JsValue::from_f64(sd.width))?;
                        js_sys::Reflect::set(&so, &"color".into(), &JsValue::from_str(&sd.color))?;
                        js_sys::Reflect::set(&bo, &name.into(), &so)?;
                    }
                    Ok(())
                };
                set_side("top", &b.top)?;
                set_side("right", &b.right)?;
                set_side("bottom", &b.bottom)?;
                set_side("left", &b.left)?;
                js_sys::Reflect::set(&o, &"border".into(), &bo)?;
            }
            arr.push(&o);
        }
        Ok(arr.into())
    }

    /// 第 `i` 张工作表的合并区 `[[r0,c0,r1,c1], ...]`。
    pub fn merges(&self, i: usize) -> Result<JsValue, JsValue> {
        let s = self
            .sheets
            .get(i)
            .ok_or_else(|| JsValue::from_str("工作表下标越界"))?;
        let arr = js_sys::Array::new();
        for &(r0, c0, r1, c1) in &s.merges {
            let m = js_sys::Array::new();
            for v in [r0, c0, r1, c1] {
                m.push(&JsValue::from_f64(v as f64));
            }
            arr.push(&m);
        }
        Ok(arr.into())
    }

    /// 第 `i` 张工作表的内嵌图片锚点
    /// `[{mediaKey, fromRow, fromCol, toRow?, toCol?, extW?, extH?}, ...]`。
    pub fn images(&self, i: usize) -> Result<JsValue, JsValue> {
        let s = self
            .sheets
            .get(i)
            .ok_or_else(|| JsValue::from_str("工作表下标越界"))?;
        let arr = js_sys::Array::new();
        for img in &s.images {
            let o = js_sys::Object::new();
            js_sys::Reflect::set(&o, &"mediaKey".into(), &JsValue::from_str(&img.media_key))?;
            js_sys::Reflect::set(
                &o,
                &"fromRow".into(),
                &JsValue::from_f64(img.from_row as f64),
            )?;
            js_sys::Reflect::set(
                &o,
                &"fromCol".into(),
                &JsValue::from_f64(img.from_col as f64),
            )?;
            if let Some((tr, tc)) = img.to {
                js_sys::Reflect::set(&o, &"toRow".into(), &JsValue::from_f64(tr as f64))?;
                js_sys::Reflect::set(&o, &"toCol".into(), &JsValue::from_f64(tc as f64))?;
            }
            if let Some((w, h)) = img.ext_px {
                js_sys::Reflect::set(&o, &"extW".into(), &JsValue::from_f64(w))?;
                js_sys::Reflect::set(&o, &"extH".into(), &JsValue::from_f64(h))?;
            }
            arr.push(&o);
        }
        Ok(arr.into())
    }

    /// 媒体(图片)数量。
    #[wasm_bindgen(js_name = mediaCount)]
    pub fn media_count(&self) -> usize {
        self.media.len()
    }

    /// 第 `i` 份媒体的键(如 `xl/media/image1.png`)。
    #[wasm_bindgen(js_name = mediaKey)]
    pub fn media_key(&self, i: usize) -> Option<String> {
        self.media.get(i).map(|m| m.key.clone())
    }

    /// 第 `i` 份媒体的 MIME。
    #[wasm_bindgen(js_name = mediaMime)]
    pub fn media_mime(&self, i: usize) -> Option<String> {
        self.media.get(i).map(|m| m.mime.clone())
    }

    /// 第 `i` 份媒体的字节。
    #[wasm_bindgen(js_name = mediaBytes)]
    pub fn media_bytes(&self, i: usize) -> Vec<u8> {
        self.media
            .get(i)
            .map(|m| m.data.clone())
            .unwrap_or_default()
    }
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
