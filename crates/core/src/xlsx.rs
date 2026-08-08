//! Excel (.xlsx) **工作簿解析**:calamine 读取 → 每张工作表一张只读 [`Sheet`] + 公式清单。
//!
//! 与 CSV 路径的关键差异:xlsx **自带缓存的计算值**(公式格里存着上次算出的结果),
//! 因此这里**不重算**——显示表直接取 calamine 给的单元格值(与 Excel 打开时所见一致),
//! 公式原文单独经 `worksheet_formula` 取出供公式栏回显。多工作表按 `sheet_names` 顺序保留。
//!
//! 单元格值 → 文本:数值用 Rust `f64` Display(`14.0`→`14`、`3.5`→`3.5`),
//! 布尔归一为 `TRUE`/`FALSE`,日期序列数换算成 `YYYY-MM-DD[ HH:MM:SS]`
//! (自实现,不引入 chrono,保持 wasm 体积)。
//!
//! **非目标**:单元格样式/合并、按 numfmt 格式码渲染(calamine 稳定 API 不直接给每格格式码)。

use std::io::Cursor;

use calamine::{open_workbook_from_rs, Data, Reader, Xlsx};

use crate::formula::CellFormula;
use crate::sheet::Sheet;

/// 一张工作表:名字 + 值显示表 + 公式清单。
#[derive(Debug)]
pub struct XlsxSheet {
    /// 工作表名(标签页显示)。
    pub name: String,
    /// 单元格**显示值**表(公式格为缓存计算值)。
    pub sheet: Sheet,
    /// 公式格的原始文本(含前导 `=`),供公式栏回显。
    pub formulas: Vec<CellFormula>,
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
    let mut sheets = Vec::with_capacity(names.len());

    for name in names {
        let values = wb.worksheet_range(&name).map_err(|e| e.to_string())?;
        // 公式表可能缺失(无公式的表);缺失时按空表处理。
        let formulas_range = wb.worksheet_formula(&name).ok();

        let (sheet, formulas) = build_sheet(&values, formulas_range.as_ref());
        sheets.push(XlsxSheet {
            name,
            sheet,
            formulas,
        });
    }

    Ok(XlsxWorkbook { sheets })
}

/// 从 calamine 的值区域(+ 可选公式区域)构建显示表与公式清单。
///
/// 用**绝对坐标**(从 (0,0) 到 `end`)建表,使网格 `(r,c)` 与 Excel A1 地址对齐 ——
/// 这样公式区域给出的绝对坐标能与显示表严丝合缝。
fn build_sheet(
    values: &calamine::Range<Data>,
    formulas: Option<&calamine::Range<String>>,
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
            let cell = values.get_value((r, c)).map(cell_text).unwrap_or_default();
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

    #[test]
    #[ignore = "生成浏览器 e2e 夹具"]
    fn write_browser_fixture() {
        let bytes = tiny_xlsx();
        std::fs::write("/tmp/office-r-sample.xlsx", &bytes).unwrap();
        eprintln!("wrote /tmp/office-r-sample.xlsx ({} bytes)", bytes.len());
    }
}
