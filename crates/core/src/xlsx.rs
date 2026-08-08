//! Excel (.xlsx) **工作簿解析**:calamine 读取 → 每张工作表一张只读 [`Sheet`] + 公式清单。
//!
//! 与 CSV 路径的关键差异:xlsx **自带缓存的计算值**(公式格里存着上次算出的结果),
//! 因此这里**不重算**——显示表直接取 calamine 给的单元格值(与 Excel 打开时所见一致),
//! 公式原文单独经 `worksheet_formula` 取出供公式栏回显。多工作表按 `sheet_names` 顺序保留。
//!
//! 单元格值 → 文本:数值按该格 **numfmt 格式码**渲染(自解析 `xl/styles.xml` 的
//! `numFmts` + `cellXfs`,内置 id 与自定义码经 `numfmt` 内核格式化;百分比/千分位/货币/小数),
//! 布尔归一为 `TRUE`/`FALSE`,日期序列数换算成 `YYYY-MM-DD[ HH:MM:SS]`(自实现,不引入 chrono)。
//! 合并区(`mergeCells`)解析进 `XlsxSheet::merges`。
//!
//! **非目标**:字体/填充/边框等**视觉样式的渲染**、合并区跨格视觉呈现(已解析,渲染待后续)。

use std::io::{Cursor, Read};

use calamine::{open_workbook_from_rs, Data, DataType, Reader, Xlsx};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader as XmlReader;
use zip::ZipArchive;

use crate::formula::CellFormula;
use crate::sheet::Sheet;

/// 一张工作表:名字 + 值显示表 + 公式清单 + 合并区。
#[derive(Debug)]
pub struct XlsxSheet {
    /// 工作表名(标签页显示)。
    pub name: String,
    /// 单元格**显示值**表(公式格为缓存计算值;数值按该格 numfmt 格式码渲染)。
    pub sheet: Sheet,
    /// 公式格的原始文本(含前导 `=`),供公式栏回显。
    pub formulas: Vec<CellFormula>,
    /// 合并单元格区域 `(row0, col0, row1, col1)`(0 基,含首尾)。
    pub merges: Vec<(u32, u32, u32, u32)>,
}

/// 一个工作簿:按原始顺序的工作表列表。
#[derive(Debug)]
pub struct XlsxWorkbook {
    pub sheets: Vec<XlsxSheet>,
}

/// 解析 xlsx 字节为工作簿。失败返回可读错误。
pub fn parse(bytes: &[u8]) -> Result<XlsxWorkbook, String> {
    let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e: calamine::XlsxError| e.to_string())?;
    let names = wb.sheet_names().to_vec();

    // 另开一份 zip 自解析样式(calamine 稳定 API 不给每格格式)与合并区。
    let mut zip = ZipArchive::new(Cursor::new(bytes.to_vec())).ok();
    let style_codes = zip.as_mut().map(read_styles).unwrap_or_default();
    let path_map = zip.as_mut().map(sheet_path_map).unwrap_or_default();

    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let values = wb.worksheet_range(&name).map_err(|e| e.to_string())?;
        let formulas_range = wb.worksheet_formula(&name).ok();

        // 每格样式索引 + 合并区(自解析 sheetN.xml)
        let (cell_styles, merges) = match (zip.as_mut(), path_map.get(&name)) {
            (Some(z), Some(path)) => read_cell_styles(z, path),
            _ => (std::collections::HashMap::new(), Vec::new()),
        };

        let (sheet, formulas) =
            build_sheet(&values, formulas_range.as_ref(), &cell_styles, &style_codes);
        sheets.push(XlsxSheet {
            name,
            sheet,
            formulas,
            merges,
        });
    }

    Ok(XlsxWorkbook { sheets })
}

/// 从 calamine 的值区域(+ 可选公式区域 + 每格样式)构建显示表与公式清单。
///
/// 用**绝对坐标**(从 (0,0) 到 `end`)建表,使网格 `(r,c)` 与 Excel A1 地址对齐。
/// 数值格若带 numfmt 格式码,按码渲染(百分比/千分位/货币/小数);日期由 calamine 归为
/// `DateTime`,走 ISO 换算,不再二次套码。
fn build_sheet(
    values: &calamine::Range<Data>,
    formulas: Option<&calamine::Range<String>>,
    cell_styles: &std::collections::HashMap<(u32, u32), usize>,
    style_codes: &[Option<String>],
) -> (Sheet, Vec<CellFormula>) {
    let mut builder = Sheet::builder();
    let mut formula_list = Vec::new();

    let Some((end_row, end_col)) = values.end() else {
        // 空表
        return (builder.finish(), formula_list);
    };

    for r in 0..=end_row {
        builder.start_row();
        for c in 0..=end_col {
            let data = values.get_value((r, c));
            // 数值 + 该格 numfmt 码 → 按码格式化;否则默认显示
            let cell = match data {
                Some(d @ (Data::Float(_) | Data::Int(_))) => {
                    let code = cell_styles
                        .get(&(r, c))
                        .and_then(|&s| style_codes.get(s))
                        .and_then(|o| o.clone());
                    match (code, d.as_f64()) {
                        (Some(code), Some(n)) => crate::numfmt::format_number(n, &code),
                        _ => cell_text(d),
                    }
                }
                Some(d) => cell_text(d),
                None => String::new(),
            };
            builder.push_field(&cell);

            if let Some(fr) = formulas {
                if let Some(src) = fr.get_value((r, c)) {
                    if !src.is_empty() {
                        formula_list.push(CellFormula {
                            row: r,
                            col: c,
                            source: format!("={src}"),
                        });
                    }
                }
            }
        }
    }
    builder.trim_trailing_empty_rows();
    (builder.finish(), formula_list)
}

/// 单元格数据 → 显示文本。
fn cell_text(data: &Data) -> String {
    match data {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        // calamine 的 DateTime Display 只打印裸序列数,这里换算成日期文本。
        Data::DateTime(dt) => excel_serial_to_string(dt.as_f64()),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

// ---- 自解析:样式(numfmt 码)+ 工作表路径映射 + 每格样式索引/合并区 ----

/// 读 zip 内某文本文件(缺失/错误返回 None)。
fn zip_text(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Option<String> {
    let mut s = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
    Some(s)
}

/// 元素本地名(去命名空间前缀)。
fn local(e: &BytesStart) -> String {
    let full = e.name();
    let bytes = full.as_ref();
    let name = bytes.rsplit(|&b| b == b':').next().unwrap_or(bytes);
    String::from_utf8_lossy(name).into_owned()
}

/// 取属性值。
fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let k = a.key;
        let kn = k
            .as_ref()
            .rsplit(|&b| b == b':')
            .next()
            .unwrap_or(k.as_ref());
        if kn == key.as_bytes() {
            Some(String::from_utf8_lossy(&a.value).into_owned())
        } else {
            None
        }
    })
}

/// 解析 `xl/styles.xml`:得到 cellXfs 索引 → numfmt 格式码(None=General/日期/文本)。
fn read_styles(zip: &mut ZipArchive<Cursor<Vec<u8>>>) -> Vec<Option<String>> {
    let xml = match zip_text(zip, "xl/styles.xml") {
        Some(x) => x,
        None => return Vec::new(),
    };
    let mut custom: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut codes: Vec<Option<String>> = Vec::new();
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    let mut in_cellxfs = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let n = local(&e);
                match n.as_str() {
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (
                            attr(&e, "numFmtId").and_then(|s| s.parse::<u32>().ok()),
                            attr(&e, "formatCode"),
                        ) {
                            custom.insert(id, code);
                        }
                    }
                    "cellXfs" => in_cellxfs = true,
                    "xf" if in_cellxfs => {
                        let id = attr(&e, "numFmtId")
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(0);
                        let code = custom.get(&id).cloned().or_else(|| builtin_numfmt(id));
                        codes.push(code);
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                if local_end(&e) == "cellXfs" {
                    in_cellxfs = false;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    codes
}

/// End 事件本地名。
fn local_end(e: &quick_xml::events::BytesEnd) -> String {
    let full = e.name();
    let bytes = full.as_ref();
    let name = bytes.rsplit(|&b| b == b':').next().unwrap_or(bytes);
    String::from_utf8_lossy(name).into_owned()
}

/// 内置 numFmtId → 格式码(仅数值类;General/日期/文本返回 None,走默认/ISO 渲染)。
fn builtin_numfmt(id: u32) -> Option<String> {
    let code = match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        37 | 38 => "#,##0",
        39 | 40 => "#,##0.00",
        44 => "\"¥\"#,##0.00",
        // 0 General、11 科学计数、12/13 分数、14-22 日期时间、45-49 等 → 默认处理
        _ => return None,
    };
    Some(code.to_string())
}

/// 解析 `xl/workbook.xml` + rels,得到 工作表名 → `xl/worksheets/sheetN.xml` 路径。
fn sheet_path_map(
    zip: &mut ZipArchive<Cursor<Vec<u8>>>,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let wb_xml = match zip_text(zip, "xl/workbook.xml") {
        Some(x) => x,
        None => return out,
    };
    let rels_xml = zip_text(zip, "xl/_rels/workbook.xml.rels").unwrap_or_default();
    // rId → target
    let mut rid_target: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut reader = XmlReader::from_str(&rels_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(&e) == "Relationship" => {
                if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    rid_target.insert(id, t);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    // sheet name + r:id
    let mut reader = XmlReader::from_str(&wb_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(&e) == "sheet" => {
                if let (Some(name), Some(rid)) = (attr(&e, "name"), attr(&e, "id")) {
                    if let Some(t) = rid_target.get(&rid) {
                        let path = normalize_xl_path(t);
                        out.insert(name, path);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// workbook rels 的 target 多为 `worksheets/sheet1.xml`,归一到 `xl/worksheets/sheet1.xml`。
fn normalize_xl_path(target: &str) -> String {
    let t = target.trim_start_matches('/');
    if t.starts_with("xl/") {
        t.to_string()
    } else {
        format!("xl/{t}")
    }
}

/// 解析某 `sheetN.xml`:得到 每格 `(row,col)` → cellXfs 索引,以及合并区。
#[allow(clippy::type_complexity)]
fn read_cell_styles(
    zip: &mut ZipArchive<Cursor<Vec<u8>>>,
    path: &str,
) -> (
    std::collections::HashMap<(u32, u32), usize>,
    Vec<(u32, u32, u32, u32)>,
) {
    let mut styles = std::collections::HashMap::new();
    let mut merges = Vec::new();
    let xml = match zip_text(zip, path) {
        Some(x) => x,
        None => return (styles, merges),
    };
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let n = local(&e);
                if n == "c" {
                    if let (Some(rc), Some(s)) = (
                        attr(&e, "r").and_then(|r| parse_a1(&r)),
                        attr(&e, "s").and_then(|s| s.parse::<usize>().ok()),
                    ) {
                        styles.insert(rc, s);
                    }
                } else if n == "mergeCell" {
                    if let Some(rf) = attr(&e, "ref") {
                        if let Some(m) = parse_a1_range(&rf) {
                            merges.push(m);
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (styles, merges)
}

/// A1 记法 → (row, col)(0 基)。
fn parse_a1(s: &str) -> Option<(u32, u32)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut col = 0u32;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        col = col * 26 + (bytes[i].to_ascii_uppercase() - b'A' + 1) as u32;
        i += 1;
    }
    if i == 0 || col == 0 {
        return None;
    }
    let row: u32 = s[i..].parse().ok()?;
    if row == 0 {
        return None;
    }
    Some((row - 1, col - 1))
}

/// `A1:B2` → (row0, col0, row1, col1)(0 基,归一)。
fn parse_a1_range(s: &str) -> Option<(u32, u32, u32, u32)> {
    let (a, b) = s.split_once(':')?;
    let (r0, c0) = parse_a1(a)?;
    let (r1, c1) = parse_a1(b)?;
    Some((r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1)))
}

/// Excel 日期**序列数** → `YYYY-MM-DD`(有时分秒则追加 ` HH:MM:SS`)。
///
/// Excel 纪元为 1899-12-30(1900 系统),整数部分为天、其余为当天时间比例。
/// 用 Howard Hinnant 的 civil-from-days 算法换算,不依赖 chrono。
fn excel_serial_to_string(serial: f64) -> String {
    let whole = serial.floor();
    let days_since_1970 = whole as i64 - 25569; // 1899-12-30 → 1970-01-01 相差 25569 天
    let (y, m, d) = civil_from_days(days_since_1970);

    // 当天时间(四舍五入到秒)
    let secs = ((serial - whole) * 86_400.0).round() as i64;
    let (secs, extra_day) = if secs >= 86_400 {
        (secs - 86_400, 1)
    } else {
        (secs, 0)
    };
    let (y, m, d) = if extra_day == 1 {
        civil_from_days(days_since_1970 + 1)
    } else {
        (y, m, d)
    };
    let (hh, mm, ss) = (secs / 3600, (secs % 3600) / 60, secs % 60);

    if hh == 0 && mm == 0 && ss == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
    }
}

/// 天数(自 1970-01-01)→ (年, 月, 日)。Hinnant civil_from_days。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_to_date_basic() {
        // 25569 = 1970-01-01;44197 = 2021-01-01
        assert_eq!(excel_serial_to_string(25569.0), "1970-01-01");
        assert_eq!(excel_serial_to_string(44197.0), "2021-01-01");
        // 1 = 1899-12-31(1900 系统,忽略闰年 bug 的边界)
        assert_eq!(excel_serial_to_string(2.0), "1900-01-01");
    }

    #[test]
    fn serial_to_date_with_time() {
        // 44197.5 = 2021-01-01 12:00:00
        assert_eq!(excel_serial_to_string(44197.5), "2021-01-01 12:00:00");
    }

    #[test]
    fn cell_text_variants() {
        assert_eq!(cell_text(&Data::Empty), "");
        assert_eq!(cell_text(&Data::Int(42)), "42");
        assert_eq!(cell_text(&Data::Float(14.0)), "14");
        assert_eq!(cell_text(&Data::Float(3.5)), "3.5");
        assert_eq!(cell_text(&Data::Bool(true)), "TRUE");
        assert_eq!(cell_text(&Data::String("hi".into())), "hi");
    }

    /// 手工构造一个最小 xlsx(2 张表、数值/字符串/公式+缓存值),验证解析。
    fn tiny_xlsx() -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};

        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let mut put = |name: &str, data: &str| {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            };
            put(
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            );
            put(
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            );
            put(
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="数据" sheetId="1" r:id="rId1"/><sheet name="第二表" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            );
            put(
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
            );
            // sheet1: A1=商品(inlineStr) B1=单价 ; A2=苹果 B2=3.5 C2=4 D2==B2*C2 缓存 14
            put(
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>商品</t></is></c><c r="B1" t="inlineStr"><is><t>单价</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>苹果</t></is></c><c r="B2"><v>3.5</v></c><c r="C2"><v>4</v></c><c r="D2"><f>B2*C2</f><v>14</v></c></row></sheetData></worksheet>"#,
            );
            put(
                "xl/worksheets/sheet2.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>仅此表</t></is></c></row></sheetData></worksheet>"#,
            );
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn parses_multi_sheet_values_and_formula() {
        let bytes = tiny_xlsx();
        let wb = parse(&bytes).expect("应能解析");
        assert_eq!(wb.sheets.len(), 2, "两张工作表");
        assert_eq!(wb.sheets[0].name, "数据");
        assert_eq!(wb.sheets[1].name, "第二表");

        let s0 = &wb.sheets[0].sheet;
        assert_eq!(s0.cell(0, 0), "商品");
        assert_eq!(s0.cell(1, 0), "苹果");
        assert_eq!(s0.cell(1, 1), "3.5");
        assert_eq!(s0.cell(1, 3), "14", "公式格显示缓存计算值");

        // 公式清单:D2(row1,col3)= =B2*C2
        let f = &wb.sheets[0].formulas;
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].row, f[0].col), (1, 3));
        assert_eq!(f[0].source, "=B2*C2");

        assert_eq!(wb.sheets[1].sheet.cell(0, 0), "仅此表");
        assert!(wb.sheets[1].formulas.is_empty());
    }

    /// 构造带样式(numfmt 百分比)+ 合并单元格的最小 xlsx。
    fn styled_xlsx() -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};
        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let mut put = |name: &str, data: &str| {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            };
            put(
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            );
            put(
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            );
            put(
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="表" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            );
            put(
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
            );
            // styles:cellXfs[0]=默认;cellXfs[1]=numFmtId 9(0%);cellXfs[2]=自定义 164
            put(
                "xl/styles.xml",
                r##"<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="#,##0.00"/></numFmts><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="9" applyNumberFormat="1"/><xf numFmtId="164" applyNumberFormat="1"/></cellXfs></styleSheet>"##,
            );
            // A1=0.25 用 s=1(百分比)→ 25%;B1=1234.5 用 s=2(#,##0.00)→ 1,234.50;合并 A2:B2
            put(
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="1"><v>0.25</v></c><c r="B1" s="2"><v>1234.5</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>合并</t></is></c></row></sheetData><mergeCells count="1"><mergeCell ref="A2:B2"/></mergeCells></worksheet>"#,
            );
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn applies_per_cell_numfmt_and_merges() {
        let wb = parse(&styled_xlsx()).expect("应能解析");
        let s = &wb.sheets[0];
        assert_eq!(s.sheet.cell(0, 0), "25%", "numFmtId 9 → 百分比");
        assert_eq!(s.sheet.cell(0, 1), "1,234.50", "自定义 #,##0.00");
        assert_eq!(s.merges, vec![(1, 0, 1, 1)], "合并区 A2:B2");
    }

    #[test]
    #[ignore = "生成浏览器 e2e 夹具"]
    fn write_browser_fixture() {
        let bytes = tiny_xlsx();
        std::fs::write("/tmp/office-r-sample.xlsx", &bytes).unwrap();
        eprintln!("wrote /tmp/office-r-sample.xlsx ({} bytes)", bytes.len());
    }
}
