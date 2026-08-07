//! 信息函数(IS* 谓词等)。
//!
//! 注意:`IS*` 系列**不传播错误** —— `ISERROR(1/0)` 应返回 `TRUE` 而非 `#DIV/0!`。
//! 因此它们直接检视被求值出来的 [`Value`],包括错误值本身。

use std::collections::HashMap;

use super::util::{arity, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("ISBLANK", isblank);
    m.insert("ISNUMBER", isnumber);
    m.insert("ISTEXT", istext);
    m.insert("ISNONTEXT", isnontext);
    m.insert("ISLOGICAL", islogical);
    m.insert("ISERROR", iserror);
    m.insert("ISERR", iserr);
    m.insert("ISNA", isna);
    m.insert("ISREF", isref);
    m.insert("ISEVEN", iseven);
    m.insert("ISODD", isodd);
    m.insert("NA", na);
    m.insert("N", n_);
    m.insert("TYPE", type_);
    m.insert("ERROR.TYPE", error_type);
}

/// 「求值 1 个参数,按谓词判定」的公共外壳。
fn predicate(ev: &mut Evaluator, args: &[Node], f: impl Fn(&Value) -> bool) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let v = ev.eval(&args[0]);
    Value::Bool(f(&v))
}

fn isblank(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(ev, args, |v| matches!(v, Value::Blank))
}
fn isnumber(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(ev, args, |v| matches!(v, Value::Number(_)))
}
fn istext(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(ev, args, |v| matches!(v, Value::Text(_)))
}
fn isnontext(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(ev, args, |v| !matches!(v, Value::Text(_)))
}
fn islogical(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(ev, args, |v| matches!(v, Value::Bool(_)))
}
fn iserror(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(ev, args, |v| v.as_error().is_some())
}
fn iserr(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(
        ev,
        args,
        |v| matches!(v.as_error(), Some(e) if e != ExcelError::Na),
    )
}
fn isna(ev: &mut Evaluator, args: &[Node]) -> Value {
    predicate(ev, args, |v| v.as_error() == Some(ExcelError::Na))
}

fn isref(_ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    // 是否引用要看 AST 形态,而非求值结果
    Value::Bool(Evaluator::is_reference(&args[0]))
}

fn iseven(ev: &mut Evaluator, args: &[Node]) -> Value {
    parity(ev, args, true)
}
fn isodd(ev: &mut Evaluator, args: &[Node]) -> Value {
    parity(ev, args, false)
}
fn parity(ev: &mut Evaluator, args: &[Node], even: bool) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match ev.eval_number(&args[0]) {
        Ok(n) => {
            let is_even = (n.trunc() as i64) % 2 == 0;
            Value::Bool(is_even == even)
        }
        Err(e) => Value::Error(e),
    }
}

fn na(_ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 0, Some(0)) {
        return e;
    }
    Value::Error(ExcelError::Na)
}

fn n_(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match ev.eval(&args[0]) {
        Value::Number(n) => Value::Number(n),
        Value::Bool(b) => Value::Number(if b { 1.0 } else { 0.0 }),
        Value::Error(e) => Value::Error(e),
        _ => Value::Number(0.0), // 文本/空 → 0
    }
}

fn type_(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let t = match ev.eval(&args[0]) {
        Value::Number(_) | Value::Blank => 1,
        Value::Text(_) => 2,
        Value::Bool(_) => 4,
        Value::Error(_) => 16,
        Value::Array(_) => 64,
    };
    Value::Number(t as f64)
}

fn error_type(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match ev.eval(&args[0]).as_error() {
        Some(e) => {
            let code = match e {
                ExcelError::Null => 1,
                ExcelError::Div0 => 2,
                ExcelError::Value => 3,
                ExcelError::Ref => 4,
                ExcelError::Name => 5,
                ExcelError::Num => 6,
                ExcelError::Na => 7,
            };
            Value::Number(code as f64)
        }
        None => Value::Error(ExcelError::Na),
    }
}

#[cfg(test)]
mod tests {
    use crate::formula::eval::Workbook;
    use crate::formula::value::Value;

    fn ev(f: &str) -> Value {
        Workbook::new().eval_formula(f)
    }

    #[test]
    fn type_predicates() {
        assert_eq!(ev("ISNUMBER(3)"), Value::Bool(true));
        assert_eq!(ev("ISTEXT(\"a\")"), Value::Bool(true));
        assert_eq!(ev("ISLOGICAL(TRUE)"), Value::Bool(true));
        assert_eq!(ev("ISNONTEXT(3)"), Value::Bool(true));
    }

    #[test]
    fn blank_and_ref() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "=ISBLANK(Z9)");
        assert_eq!(wb.eval_cell(0, 0), Value::Bool(true));
        assert_eq!(ev("ISREF(A1)"), Value::Bool(true));
        assert_eq!(ev("ISREF(3)"), Value::Bool(false));
    }

    #[test]
    fn error_predicates_do_not_propagate() {
        assert_eq!(ev("ISERROR(1/0)"), Value::Bool(true));
        assert_eq!(ev("ISERR(#N/A)"), Value::Bool(false));
        assert_eq!(ev("ISNA(#N/A)"), Value::Bool(true));
        assert_eq!(ev("ERROR.TYPE(1/0)"), Value::Number(2.0));
    }

    #[test]
    fn parity_and_conversions() {
        assert_eq!(ev("ISEVEN(4)"), Value::Bool(true));
        assert_eq!(ev("ISODD(3)"), Value::Bool(true));
        assert_eq!(ev("N(TRUE)"), Value::Number(1.0));
        assert_eq!(ev("TYPE(\"x\")"), Value::Number(2.0));
    }
}
