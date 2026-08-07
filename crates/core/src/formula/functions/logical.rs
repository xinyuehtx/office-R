//! 逻辑函数。多数需要**短路**,故直接拿未求值的 AST 参数。

use std::collections::HashMap;

use super::util::{arity, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::{cmp_values, Evaluator};
use crate::formula::value::{ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("IF", if_);
    m.insert("IFS", ifs);
    m.insert("IFERROR", iferror);
    m.insert("IFNA", ifna);
    m.insert("AND", and);
    m.insert("OR", or);
    m.insert("NOT", not);
    m.insert("XOR", xor);
    m.insert("TRUE", true_);
    m.insert("FALSE", false_);
    m.insert("SWITCH", switch);
}

fn if_(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    match ev.eval_bool(&args[0]) {
        Ok(true) => ev.eval(&args[1]),
        Ok(false) => match args.get(2) {
            Some(a) => ev.eval(a),
            None => Value::Bool(false), // 省略 else → FALSE
        },
        Err(e) => Value::Error(e),
    }
}

fn ifs(ev: &mut Evaluator, args: &[Node]) -> Value {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return Value::Error(ExcelError::Value);
    }
    let mut i = 0;
    while i + 1 < args.len() {
        match ev.eval_bool(&args[i]) {
            Ok(true) => return ev.eval(&args[i + 1]),
            Ok(false) => {}
            Err(e) => return Value::Error(e),
        }
        i += 2;
    }
    Value::Error(ExcelError::Na) // 无条件命中
}

fn iferror(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let v = ev.eval(&args[0]);
    if v.as_error().is_some() {
        ev.eval(&args[1])
    } else {
        v
    }
}

fn ifna(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let v = ev.eval(&args[0]);
    if v.as_error() == Some(ExcelError::Na) {
        ev.eval(&args[1])
    } else {
        v
    }
}

/// 从参数里收集逻辑值(遇错误传播);无任何可用逻辑值时返回 `None`(交由调用方判 `#VALUE!`)。
fn collect_bools(ev: &mut Evaluator, args: &[Node]) -> Result<Vec<bool>, ExcelError> {
    let mut out = Vec::new();
    for arg in args {
        for v in ev.flatten_arg(arg) {
            match v {
                Value::Bool(b) => out.push(b),
                Value::Number(n) => out.push(n != 0.0),
                Value::Text(s) => match s.trim().to_ascii_uppercase().as_str() {
                    "TRUE" => out.push(true),
                    "FALSE" => out.push(false),
                    _ => {} // 文本忽略
                },
                Value::Error(e) => return Err(e),
                _ => {}
            }
        }
    }
    Ok(out)
}

fn and(ev: &mut Evaluator, args: &[Node]) -> Value {
    match collect_bools(ev, args) {
        Ok(bs) if bs.is_empty() => Value::Error(ExcelError::Value),
        Ok(bs) => Value::Bool(bs.into_iter().all(|b| b)),
        Err(e) => Value::Error(e),
    }
}
fn or(ev: &mut Evaluator, args: &[Node]) -> Value {
    match collect_bools(ev, args) {
        Ok(bs) if bs.is_empty() => Value::Error(ExcelError::Value),
        Ok(bs) => Value::Bool(bs.into_iter().any(|b| b)),
        Err(e) => Value::Error(e),
    }
}
fn xor(ev: &mut Evaluator, args: &[Node]) -> Value {
    match collect_bools(ev, args) {
        Ok(bs) if bs.is_empty() => Value::Error(ExcelError::Value),
        Ok(bs) => Value::Bool(bs.into_iter().filter(|&b| b).count() % 2 == 1),
        Err(e) => Value::Error(e),
    }
}
fn not(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match ev.eval_bool(&args[0]) {
        Ok(b) => Value::Bool(!b),
        Err(e) => Value::Error(e),
    }
}

fn true_(_ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 0, Some(0)) {
        return e;
    }
    Value::Bool(true)
}
fn false_(_ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 0, Some(0)) {
        return e;
    }
    Value::Bool(false)
}

fn switch(ev: &mut Evaluator, args: &[Node]) -> Value {
    // SWITCH(expr, val1, res1, [val2, res2, ...], [default])
    if args.len() < 3 {
        return Value::Error(ExcelError::Value);
    }
    let target = ev.eval(&args[0]);
    if let Some(e) = target.as_error() {
        return Value::Error(e);
    }
    let mut i = 1;
    while i + 1 < args.len() {
        let candidate = ev.eval(&args[i]);
        if cmp_values(&target, &candidate) == std::cmp::Ordering::Equal {
            return ev.eval(&args[i + 1]);
        }
        i += 2;
    }
    // 剩下一个 → 默认值
    if i < args.len() {
        ev.eval(&args[i])
    } else {
        Value::Error(ExcelError::Na)
    }
}

#[cfg(test)]
mod tests {
    use crate::formula::eval::Workbook;
    use crate::formula::value::{ExcelError, Value};

    fn ev(f: &str) -> Value {
        Workbook::new().eval_formula(f)
    }

    #[test]
    fn if_branches_and_short_circuits() {
        assert_eq!(ev("IF(1>2,\"y\",\"n\")"), Value::Text("n".into()));
        assert_eq!(ev("IF(TRUE,10)"), Value::Number(10.0));
        assert_eq!(ev("IF(FALSE,10)"), Value::Bool(false));
        // 未命中的分支即使会报错也不该被求值
        assert_eq!(ev("IF(TRUE,1,1/0)"), Value::Number(1.0));
    }

    #[test]
    fn iferror_and_ifna() {
        assert_eq!(ev("IFERROR(1/0,\"err\")"), Value::Text("err".into()));
        assert_eq!(ev("IFERROR(5,\"err\")"), Value::Number(5.0));
        assert_eq!(ev("IFNA(#N/A,0)"), Value::Number(0.0));
        assert_eq!(ev("IFNA(1/0,0)"), Value::Error(ExcelError::Div0));
    }

    #[test]
    fn boolean_logic() {
        assert_eq!(ev("AND(TRUE,TRUE,1)"), Value::Bool(true));
        assert_eq!(ev("AND(TRUE,FALSE)"), Value::Bool(false));
        assert_eq!(ev("OR(FALSE,0,1)"), Value::Bool(true));
        assert_eq!(ev("NOT(FALSE)"), Value::Bool(true));
        assert_eq!(ev("XOR(TRUE,TRUE,TRUE)"), Value::Bool(true));
    }

    #[test]
    fn ifs_and_switch() {
        assert_eq!(ev("IFS(FALSE,1,TRUE,2)"), Value::Number(2.0));
        assert_eq!(ev("IFS(FALSE,1,FALSE,2)"), Value::Error(ExcelError::Na));
        assert_eq!(
            ev("SWITCH(3,1,\"a\",3,\"c\",\"def\")"),
            Value::Text("c".into())
        );
        assert_eq!(ev("SWITCH(9,1,\"a\",\"def\")"), Value::Text("def".into()));
    }
}
