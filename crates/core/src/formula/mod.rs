//! **公式计算引擎**:平台无关,语义对齐 Excel。
//!
//! 是一条经典的解释器管线,叠加一个「值/公式层」[`Workbook`]:
//!
//! ```text
//! 公式文本 ──tokenize──▶ [Token] ──parse──▶ AST ──evaluate──▶ Value
//!                        token.rs           parser.rs         eval.rs
//! ```
//!
//! # 快速上手
//!
//! ```
//! use office_core::formula::{Workbook, Value};
//!
//! let mut wb = Workbook::new();
//! wb.set_input(0, 0, "1");        // A1
//! wb.set_input(1, 0, "2");        // A2
//! wb.set_input(2, 0, "=SUM(A1:A2)*2"); // A3
//! assert_eq!(wb.eval_cell(2, 0), Value::Number(6.0));
//! ```
//!
//! # 设计取舍
//!
//! - **错误是一等值**:`#DIV/0!` 等沿计算链传播,而非用 `Result` 逐层中断。
//! - **按需求值 + 记忆化 + 循环检测**:不预先拓扑排序,递归时用 `visiting` 集判环。
//! - **可扩展函数注册表**:补齐 Excel 更多函数是机械式新增,不触碰求值器。
//!
//! 参考的开源实现(仅作语义/函数目录参考,本引擎为 Rust 自研):
//! HyperFormula、Univer;Rust 生态的 IronCalc 佐证了路线可行性。见 `docs/rfcs/0004`。

pub mod reference;
pub mod value;

mod ast;
mod eval;
mod functions;
mod parser;
mod token;

pub use eval::Workbook;
pub use reference::{col_to_index, index_to_col, CellRef, RangeRef};
pub use value::{Array, ExcelError, Value};

/// 已注册的内置函数名(升序)。
pub fn function_names() -> Vec<&'static str> {
    functions::names()
}

/// 已注册的内置函数总数。
pub fn function_count() -> usize {
    functions::count()
}

/// 直接解析并求值一个公式主体(不含前导 `=`),不涉及任何单元格。
///
/// 便于「一次性算个表达式」的场景;需要引用单元格时请用 [`Workbook`]。
pub fn evaluate(formula: &str) -> Value {
    Workbook::new().eval_formula(formula.trim_start_matches('='))
}

/// 把一个 [`Value`] 渲染成单元格里显示的文本:错误显示为 `#DIV/0!` 这类标记,
/// 其余走 General 格式;数组取左上角元素。
pub fn value_to_display(v: &Value) -> String {
    match v {
        Value::Error(e) => e.as_str().to_string(),
        Value::Array(a) => value_to_display(&a.get(0, 0)),
        other => other.to_text().unwrap_or_else(|e| e.as_str().to_string()),
    }
}

/// 一份单元格公式的原始文本(供公式栏回显)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellFormula {
    /// 0 基行号。
    pub row: u32,
    /// 0 基列号。
    pub col: u32,
    /// 原始输入(含前导 `=`)。
    pub source: String,
}

/// 对文本表格求值的产物:计算值显示表 + 公式清单。
#[derive(Debug)]
pub struct EvaluatedGrid {
    /// 各单元格显示「计算值」(非公式单元格保持原文)的只读表。
    pub display: crate::sheet::Sheet,
    /// 所有公式单元格的原始文本。
    pub formulas: Vec<CellFormula>,
}

/// 把一张**文本表格**里以 `=` 开头的单元格当作公式求值,产出:
/// - `display`:公式格显示计算结果、其余格保持原文的新表(供 canvas 渲染);
/// - `formulas`:公式格的原始文本(供公式栏)。
///
/// **没有任何公式时返回 [`None`]**,调用方直接沿用原表 —— 普通 CSV 零额外开销。
///
/// `now_serial` 是注入给 `TODAY`/`NOW` 的当前时间序列数。
pub fn evaluate_sheet(sheet: &crate::sheet::Sheet, now_serial: f64) -> Option<EvaluatedGrid> {
    let rows = sheet.rows();

    // 先快速扫描:没有任何 `=` 开头的格就直接放弃,避免为普通 CSV 建工作簿。
    let has_formula =
        (0..rows).any(|r| (0..sheet.row_len(r)).any(|c| sheet.cell(r, c).starts_with('=')));
    if !has_formula {
        return None;
    }

    // 建工作簿:逐格按「用户输入」语义写入(= 开头成为公式,其余成字面量)。
    let mut wb = Workbook::new();
    wb.set_now(now_serial);
    for r in 0..rows {
        for c in 0..sheet.row_len(r) {
            let text = sheet.cell(r, c);
            if !text.is_empty() {
                wb.set_input(r as u32, c as u32, text);
            }
        }
    }
    let values = wb.evaluate_all();

    // 生成显示表:公式格取计算值,其余格保留原文;行列形状与原表一致。
    let mut builder = crate::sheet::Sheet::builder();
    let mut formulas = Vec::new();
    for r in 0..rows {
        builder.start_row();
        for c in 0..sheet.row_len(r) {
            let raw = sheet.cell(r, c);
            if raw.starts_with('=') {
                formulas.push(CellFormula {
                    row: r as u32,
                    col: c as u32,
                    source: raw.to_string(),
                });
                let display = match values.get(&(r as u32, c as u32)) {
                    Some(v) => value_to_display(v),
                    None => String::new(), // 计算成空(如 =""),显示空
                };
                builder.push_field(&display);
            } else {
                builder.push_field(raw);
            }
        }
    }
    Some(EvaluatedGrid {
        display: builder.finish(),
        formulas,
    })
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn end_to_end_spreadsheet() {
        let mut wb = Workbook::new();
        // 成绩表:A 列分数,B 列判定
        let scores = [88, 55, 72, 91, 40];
        for (i, s) in scores.iter().enumerate() {
            wb.set_input(i as u32, 0, &s.to_string());
            wb.set_input(
                i as u32,
                1,
                &format!("=IF(A{}>=60,\"及格\",\"不及格\")", i + 1),
            );
        }
        // 汇总行
        wb.set_input(5, 0, "=AVERAGE(A1:A5)");
        wb.set_input(5, 1, "=COUNTIF(B1:B5,\"及格\")");

        assert_eq!(wb.eval_cell(1, 1), Value::Text("不及格".into()));
        assert_eq!(wb.eval_cell(3, 1), Value::Text("及格".into()));
        assert_eq!(wb.eval_cell(5, 0), Value::Number(69.2));
        assert_eq!(wb.eval_cell(5, 1), Value::Number(3.0));
    }

    #[test]
    fn evaluate_helper_works() {
        assert_eq!(evaluate("=1+2*3"), Value::Number(7.0));
        assert_eq!(evaluate("SUM(1,2,3)"), Value::Number(6.0));
    }

    /// 用二维文本构造一张只读表。
    fn text_sheet(rows: &[&[&str]]) -> crate::sheet::Sheet {
        let mut b = crate::sheet::Sheet::builder();
        for row in rows {
            b.start_row();
            for f in *row {
                b.push_field(f);
            }
        }
        b.finish()
    }

    #[test]
    fn evaluate_sheet_replaces_formula_cells_only() {
        let sheet = text_sheet(&[&["1", "2", "=A1+B1"], &["x", "=A1*10", "note"]]);
        let out = evaluate_sheet(&sheet, 0.0).expect("含公式应返回 Some");
        // 公式格显示计算值
        assert_eq!(out.display.cell(0, 2), "3");
        assert_eq!(out.display.cell(1, 1), "10");
        // 非公式格保持原文
        assert_eq!(out.display.cell(0, 0), "1");
        assert_eq!(out.display.cell(1, 0), "x");
        assert_eq!(out.display.cell(1, 2), "note");
        // 公式清单
        assert_eq!(out.formulas.len(), 2);
        assert_eq!(out.formulas[0].source, "=A1+B1");
    }

    #[test]
    fn evaluate_sheet_returns_none_without_formulas() {
        let sheet = text_sheet(&[&["a", "b"], &["1", "2"]]);
        assert!(evaluate_sheet(&sheet, 0.0).is_none(), "无公式应返回 None");
    }

    #[test]
    fn evaluate_sheet_shows_errors_as_text() {
        let sheet = text_sheet(&[&["=1/0", "=FOO()"]]);
        let out = evaluate_sheet(&sheet, 0.0).unwrap();
        assert_eq!(out.display.cell(0, 0), "#DIV/0!");
        assert_eq!(out.display.cell(0, 1), "#NAME?");
    }
}
