//! 查找与引用函数。

use std::cmp::Ordering;
use std::collections::HashMap;

use super::util::{arity, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::{cmp_values, Evaluator};
use crate::formula::value::{Array, ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("VLOOKUP", vlookup);
    m.insert("HLOOKUP", hlookup);
    m.insert("INDEX", index);
    m.insert("MATCH", match_);
    m.insert("LOOKUP", lookup);
    m.insert("CHOOSE", choose);
    m.insert("ROW", row);
    m.insert("COLUMN", column);
    m.insert("ROWS", rows);
    m.insert("COLUMNS", columns);
}

/// 把一个参数解析成二维数组:范围逐格取值,数组原样,标量视作 1×1。
fn to_array(ev: &mut Evaluator, node: &Node) -> Array {
    if let Some(r) = Evaluator::as_range(node) {
        return ev.array_from_range(r);
    }
    match ev.eval(node) {
        Value::Array(a) => a,
        v => Array::new(1, 1, vec![v]),
    }
}

fn vlookup(ev: &mut Evaluator, args: &[Node]) -> Value {
    lookup_table(ev, args, true)
}
fn hlookup(ev: &mut Evaluator, args: &[Node]) -> Value {
    lookup_table(ev, args, false)
}

/// VLOOKUP / HLOOKUP 共同实现。`vertical=true` 在首列查找、返回同行的第 index 列;
/// `false` 在首行查找、返回同列的第 index 行。
fn lookup_table(ev: &mut Evaluator, args: &[Node], vertical: bool) -> Value {
    if let Err(e) = arity(args, 3, Some(4)) {
        return e;
    }
    let needle = ev.eval(&args[0]);
    if let Some(e) = needle.as_error() {
        return Value::Error(e);
    }
    let table = to_array(ev, &args[1]);
    let idx = match ev.eval_number(&args[2]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let approximate = match args.get(3) {
        Some(a) => match ev.eval_bool(a) {
            Ok(b) => b,
            Err(e) => return Value::Error(e),
        },
        None => true, // 默认近似匹配
    };

    // 沿查找方向的长度
    let len = if vertical { table.rows } else { table.cols };
    let key_at = |t: &Array, i: usize| -> Value {
        if vertical {
            t.get(i, 0)
        } else {
            t.get(0, i)
        }
    };

    let hit = if approximate {
        // 近似:最后一个 <= needle 的位置(假定已升序)
        let mut found = None;
        for i in 0..len {
            match cmp_values(&key_at(&table, i), &needle) {
                Ordering::Less | Ordering::Equal => found = Some(i),
                Ordering::Greater => break,
            }
        }
        found
    } else {
        (0..len).find(|&i| cmp_values(&key_at(&table, i), &needle) == Ordering::Equal)
    };

    match hit {
        None => Value::Error(ExcelError::Na),
        Some(i) => {
            if idx < 1 {
                return Value::Error(ExcelError::Value);
            }
            let j = idx as usize - 1;
            if vertical {
                if j >= table.cols {
                    return Value::Error(ExcelError::Ref);
                }
                table.get(i, j)
            } else {
                if j >= table.rows {
                    return Value::Error(ExcelError::Ref);
                }
                table.get(j, i)
            }
        }
    }
}

fn index(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let arr = to_array(ev, &args[0]);
    let row_num = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let col_num = match args.get(2) {
        Some(a) => match ev.eval_number(a) {
            Ok(n) => Some(n.trunc() as i64),
            Err(e) => return Value::Error(e),
        },
        None => None,
    };

    // 省略列号时:单行数组用行号选列,否则用行号选行、列取 1
    let (r, c) = match col_num {
        Some(c) => (row_num, c),
        None => {
            if arr.rows == 1 {
                (1, row_num)
            } else {
                (row_num, 1)
            }
        }
    };
    if r < 1 || c < 1 {
        return Value::Error(ExcelError::Value);
    }
    let (r, c) = (r as usize - 1, c as usize - 1);
    if r >= arr.rows || c >= arr.cols {
        return Value::Error(ExcelError::Ref);
    }
    arr.get(r, c)
}

fn match_(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let needle = ev.eval(&args[0]);
    if let Some(e) = needle.as_error() {
        return Value::Error(e);
    }
    let arr = to_array(ev, &args[1]);
    let match_type = match args.get(2) {
        Some(a) => match ev.eval_number(a) {
            Ok(n) => n.trunc() as i64,
            Err(e) => return Value::Error(e),
        },
        None => 1,
    };
    // 展平成一维序列(按行优先)
    let seq = &arr.data;
    let pos = match match_type {
        0 => seq
            .iter()
            .position(|v| cmp_values(v, &needle) == Ordering::Equal),
        1 => {
            // 最大的 <= needle(升序)
            let mut found = None;
            for (i, v) in seq.iter().enumerate() {
                match cmp_values(v, &needle) {
                    Ordering::Less | Ordering::Equal => found = Some(i),
                    Ordering::Greater => break,
                }
            }
            found
        }
        -1 => {
            // 最小的 >= needle(降序)
            let mut found = None;
            for (i, v) in seq.iter().enumerate() {
                match cmp_values(v, &needle) {
                    Ordering::Greater | Ordering::Equal => found = Some(i),
                    Ordering::Less => break,
                }
            }
            found
        }
        _ => return Value::Error(ExcelError::Num),
    };
    match pos {
        Some(i) => Value::Number((i + 1) as f64),
        None => Value::Error(ExcelError::Na),
    }
}

fn lookup(ev: &mut Evaluator, args: &[Node]) -> Value {
    // 向量形式:LOOKUP(value, lookup_vector, [result_vector]),近似匹配
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let needle = ev.eval(&args[0]);
    let look = to_array(ev, &args[1]);
    let result = match args.get(2) {
        Some(a) => to_array(ev, a),
        None => look.clone(),
    };
    let mut found = None;
    for (i, v) in look.data.iter().enumerate() {
        match cmp_values(v, &needle) {
            Ordering::Less | Ordering::Equal => found = Some(i),
            Ordering::Greater => break,
        }
    }
    match found {
        Some(i) if i < result.data.len() => result.data[i].clone(),
        _ => Value::Error(ExcelError::Na),
    }
}

fn choose(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, None) {
        return e;
    }
    let idx = match ev.eval_number(&args[0]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    if idx < 1 || idx as usize >= args.len() {
        return Value::Error(ExcelError::Value);
    }
    ev.eval(&args[idx as usize])
}

fn row(ev: &mut Evaluator, args: &[Node]) -> Value {
    axis(ev, args, true)
}
fn column(ev: &mut Evaluator, args: &[Node]) -> Value {
    axis(ev, args, false)
}

fn axis(ev: &mut Evaluator, args: &[Node], want_row: bool) -> Value {
    if let Err(e) = arity(args, 0, Some(1)) {
        return e;
    }
    let cell = match args.first() {
        Some(a) => match Evaluator::as_range(a) {
            Some(r) => {
                let v = if want_row { r.row0 } else { r.col0 };
                return Value::Number((v + 1) as f64);
            }
            None => return Value::Error(ExcelError::Value),
        },
        None => ev.current_cell(),
    };
    match cell {
        Some(c) => Value::Number((if want_row { c.row } else { c.col } + 1) as f64),
        None => Value::Error(ExcelError::Value),
    }
}

fn rows(ev: &mut Evaluator, args: &[Node]) -> Value {
    dim(ev, args, true)
}
fn columns(ev: &mut Evaluator, args: &[Node]) -> Value {
    dim(ev, args, false)
}
fn dim(ev: &mut Evaluator, args: &[Node], want_rows: bool) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    if let Some(r) = Evaluator::as_range(&args[0]) {
        let v = if want_rows { r.rows() } else { r.cols() };
        return Value::Number(v as f64);
    }
    let arr = to_array(ev, &args[0]);
    Value::Number(if want_rows { arr.rows } else { arr.cols } as f64)
}

#[cfg(test)]
mod tests {
    use crate::formula::eval::Workbook;
    use crate::formula::value::{ExcelError, Value};

    fn table() -> Workbook {
        let mut wb = Workbook::new();
        // A1:B4 —— 编号 → 名称
        for (i, (id, name)) in [(1, "one"), (2, "two"), (3, "three"), (4, "four")]
            .iter()
            .enumerate()
        {
            wb.set_input(i as u32, 0, &id.to_string());
            wb.set_input(i as u32, 1, name);
        }
        wb
    }

    #[test]
    fn vlookup_exact_and_approx() {
        let wb = table();
        assert_eq!(
            wb.eval_formula("VLOOKUP(3,A1:B4,2,FALSE)"),
            Value::Text("three".into())
        );
        assert_eq!(
            wb.eval_formula("VLOOKUP(2,A1:B4,2,TRUE)"),
            Value::Text("two".into())
        );
        assert_eq!(
            wb.eval_formula("VLOOKUP(9,A1:B4,2,FALSE)"),
            Value::Error(ExcelError::Na)
        );
        assert_eq!(
            wb.eval_formula("VLOOKUP(3,A1:B4,3,FALSE)"),
            Value::Error(ExcelError::Ref)
        );
    }

    #[test]
    fn index_match() {
        let wb = table();
        assert_eq!(wb.eval_formula("INDEX(B1:B4,2)"), Value::Text("two".into()));
        assert_eq!(wb.eval_formula("MATCH(3,A1:A4,0)"), Value::Number(3.0));
        assert_eq!(
            wb.eval_formula("INDEX(A1:B4,MATCH(4,A1:A4,0),2)"),
            Value::Text("four".into())
        );
    }

    #[test]
    fn choose_and_lookup() {
        let wb = table();
        assert_eq!(
            wb.eval_formula("CHOOSE(2,\"a\",\"b\",\"c\")"),
            Value::Text("b".into())
        );
        assert_eq!(
            wb.eval_formula("LOOKUP(3,A1:A4,B1:B4)"),
            Value::Text("three".into())
        );
    }

    #[test]
    fn row_col_dims() {
        let wb = table();
        assert_eq!(wb.eval_formula("ROW(B3)"), Value::Number(3.0));
        assert_eq!(wb.eval_formula("COLUMN(B3)"), Value::Number(2.0));
        assert_eq!(wb.eval_formula("ROWS(A1:B4)"), Value::Number(4.0));
        assert_eq!(wb.eval_formula("COLUMNS(A1:B4)"), Value::Number(2.0));
    }

    #[test]
    fn row_without_arg_uses_current_cell() {
        let mut wb = Workbook::new();
        wb.set_input(4, 2, "=ROW()"); // C5 → 5
        assert_eq!(wb.eval_cell(4, 2), Value::Number(5.0));
    }
}
