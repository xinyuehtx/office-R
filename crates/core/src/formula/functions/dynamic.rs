//! 现代动态数组函数(子集):`XLOOKUP` / `UNIQUE` / `SORT` / `FILTER` / `SEQUENCE`。
//!
//! 本引擎已有 [`Value::Array`],这些函数按数组语义求值并返回数组值;
//! **注意**:视图层尚不支持「溢出」(spill)到相邻单元格,故在单元格里显示时取左上角
//! (`value_to_display`),但作为**中间值**(如 `SUM(UNIQUE(A1:A9))`)完全可用。

use std::collections::HashMap;

use super::util::{arity, FuncImpl};
use crate::formula::ast::{BinOp, Node};
use crate::formula::eval::Evaluator;
use crate::formula::value::{Array, ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("XLOOKUP", xlookup);
    m.insert("UNIQUE", unique);
    m.insert("SORT", sort);
    m.insert("FILTER", filter);
    m.insert("SEQUENCE", sequence);
}

/// 把参数求成一维值序列(范围/数组→逐元素;标量→单元素)。
fn as_vec(ev: &mut Evaluator, node: &Node) -> Vec<Value> {
    ev.flatten_arg(node)
}

/// `XLOOKUP(lookup, lookup_array, return_array, [if_not_found])`:精确匹配,返回对应值。
fn xlookup(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 3, Some(4)) {
        return e;
    }
    let key = ev.eval(&args[0]);
    if let Some(e) = key.as_error() {
        return Value::Error(e);
    }
    let haystack = as_vec(ev, &args[1]);
    let results = as_vec(ev, &args[2]);
    for (i, v) in haystack.iter().enumerate() {
        if values_equal(v, &key) {
            return results.get(i).cloned().unwrap_or(Value::Blank);
        }
    }
    match args.get(3) {
        Some(a) => ev.eval(a),
        None => Value::Error(ExcelError::Na),
    }
}

/// `UNIQUE(array)`:按首次出现顺序去重(一维)。
fn unique(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let xs = as_vec(ev, &args[0]);
    let mut out: Vec<Value> = Vec::new();
    for v in xs {
        if !out.iter().any(|o| values_equal(o, &v)) {
            out.push(v);
        }
    }
    if out.is_empty() {
        return Value::Error(ExcelError::Na);
    }
    let n = out.len();
    Value::Array(Array::new(n, 1, out))
}

/// `SORT(array, [sort_index], [sort_order])`:一维排序(数值/文本混合按 Excel 顺序)。
/// `sort_order` 1 升序(默认)、-1 降序。
fn sort(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(3)) {
        return e;
    }
    let mut xs = as_vec(ev, &args[0]);
    let order = match args.get(2) {
        Some(a) => ev.eval_number(a).unwrap_or(1.0),
        None => 1.0,
    };
    xs.sort_by(compare_values);
    if order < 0.0 {
        xs.reverse();
    }
    let n = xs.len();
    Value::Array(Array::new(n, 1, xs))
}

/// `FILTER(array, include, [if_empty])`:保留 include 为真的元素(一维)。
/// `include` 支持内联比较(如 `B1:B5>2`):按元素广播成掩码。
fn filter(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let xs = as_vec(ev, &args[0]);
    let mask = eval_mask(ev, &args[1], xs.len());
    let mut out = Vec::new();
    for (i, v) in xs.iter().enumerate() {
        if mask.get(i).copied().unwrap_or(false) {
            out.push(v.clone());
        }
    }
    if out.is_empty() {
        return match args.get(2) {
            Some(a) => ev.eval(a),
            None => Value::Error(ExcelError::Na),
        };
    }
    let n = out.len();
    Value::Array(Array::new(n, 1, out))
}

/// 求 `include` 掩码(长度 n)。内联比较 `范围 <op> 标量` 按元素广播;
/// 否则把参数展平后逐元素取真值。
fn eval_mask(ev: &mut Evaluator, node: &Node, n: usize) -> Vec<bool> {
    if let Node::Binary(op, l, r) = node {
        if is_cmp(*op) {
            let lv = ev.flatten_arg(l);
            let rv = ev.flatten_arg(r);
            let pick = |v: &[Value], i: usize| -> Value {
                if v.len() == 1 {
                    v[0].clone()
                } else {
                    v.get(i).cloned().unwrap_or(Value::Blank)
                }
            };
            return (0..n)
                .map(|i| cmp_bool(*op, &pick(&lv, i), &pick(&rv, i)))
                .collect();
        }
    }
    let m = ev.flatten_arg(node);
    (0..n)
        .map(|i| m.get(i).map(truthy).unwrap_or(false))
        .collect()
}

fn is_cmp(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge
    )
}

fn cmp_bool(op: BinOp, a: &Value, b: &Value) -> bool {
    use std::cmp::Ordering;
    let ord = compare_values(a, b);
    match op {
        BinOp::Eq => ord == Ordering::Equal,
        BinOp::Ne => ord != Ordering::Equal,
        BinOp::Lt => ord == Ordering::Less,
        BinOp::Gt => ord == Ordering::Greater,
        BinOp::Le => ord != Ordering::Greater,
        BinOp::Ge => ord != Ordering::Less,
        _ => false,
    }
}

/// `SEQUENCE(rows, [cols], [start], [step])`:等差数列填充的二维数组。
fn sequence(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(4)) {
        return e;
    }
    let rows = match ev.eval_number(&args[0]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let cols = match args.get(1) {
        Some(a) => ev.eval_number(a).unwrap_or(1.0).trunc() as i64,
        None => 1,
    };
    if rows <= 0 || cols <= 0 {
        return Value::Error(ExcelError::Value);
    }
    let start = args
        .get(2)
        .and_then(|a| ev.eval_number(a).ok())
        .unwrap_or(1.0);
    let step = args
        .get(3)
        .and_then(|a| ev.eval_number(a).ok())
        .unwrap_or(1.0);
    let mut data = Vec::with_capacity((rows * cols) as usize);
    let mut k = 0.0;
    for _ in 0..(rows * cols) {
        data.push(Value::Number(start + k * step));
        k += 1.0;
    }
    Value::Array(Array::new(rows as usize, cols as usize, data))
}

/// 值相等判定(数值按数值、文本按文本、忽略空白差异)。
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Blank, Value::Blank) => true,
        _ => a.clone().to_text().ok() == b.clone().to_text().ok(),
    }
}

/// 排序比较:数值 < 文本(Excel 顺序简化),同类按大小/字典序。
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Number(_), _) => Ordering::Less,
        (_, Value::Number(_)) => Ordering::Greater,
        _ => {
            let sa = a.clone().to_text().unwrap_or_default();
            let sb = b.clone().to_text().unwrap_or_default();
            sa.to_lowercase().cmp(&sb.to_lowercase())
        }
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        _ => v.clone().to_bool().unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use crate::formula::value::Array;
    use crate::formula::{Value, Workbook};

    fn wb() -> Workbook {
        let mut wb = Workbook::new();
        for (i, v) in ["b", "a", "b", "c", "a"].iter().enumerate() {
            wb.set_input(i as u32, 0, v); // A1:A5
        }
        for (i, v) in [3, 1, 4, 1, 5].iter().enumerate() {
            wb.set_input(i as u32, 1, &v.to_string()); // B1:B5
        }
        wb
    }

    #[test]
    fn xlookup_returns_match() {
        let wb = wb();
        // A 列 "c" 在第 4 行,返回 B 列同行 = 1
        assert_eq!(
            wb.eval_formula("XLOOKUP(\"c\",A1:A5,B1:B5)"),
            Value::Number(1.0)
        );
        assert_eq!(
            wb.eval_formula("XLOOKUP(\"z\",A1:A5,B1:B5,\"无\")"),
            Value::Text("无".into())
        );
    }

    #[test]
    fn unique_and_sum() {
        let wb = wb();
        // UNIQUE(A) = b,a,c → 3 个;放进 COUNTA
        assert_eq!(wb.eval_formula("COUNTA(UNIQUE(A1:A5))"), Value::Number(3.0));
        // SUM(UNIQUE(B)) = 3+1+4+5 = 13
        assert_eq!(wb.eval_formula("SUM(UNIQUE(B1:B5))"), Value::Number(13.0));
    }

    #[test]
    fn sort_sequence_filter() {
        let wb = wb();
        // SORT(B) 升序 → [1,1,3,4,5]
        assert_eq!(
            wb.eval_formula("SORT(B1:B5)"),
            Value::Array(Array::new(
                5,
                1,
                vec![
                    Value::Number(1.0),
                    Value::Number(1.0),
                    Value::Number(3.0),
                    Value::Number(4.0),
                    Value::Number(5.0),
                ]
            ))
        );
        // SEQUENCE(3) → 1..3,SUM=6
        assert_eq!(wb.eval_formula("SUM(SEQUENCE(3))"), Value::Number(6.0));
        // FILTER(B, B>2) → 3,4,5;SUM=12
        assert_eq!(
            wb.eval_formula("SUM(FILTER(B1:B5,B1:B5>2))"),
            Value::Number(12.0)
        );
    }
}
