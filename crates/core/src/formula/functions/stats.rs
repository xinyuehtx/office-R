//! 统计函数。

use std::collections::HashMap;

use super::util::{
    arity, count_nonblank, count_numbers, matches_criteria, numbers_for_agg, FuncImpl,
};
use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("AVERAGE", average);
    m.insert("AVERAGEA", average); // 近似:与 AVERAGE 同处理数值
    m.insert("AVERAGEIF", averageif);
    m.insert("COUNT", count);
    m.insert("COUNTA", counta);
    m.insert("COUNTBLANK", countblank);
    m.insert("COUNTIF", countif);
    m.insert("COUNTIFS", countifs);
    m.insert("MAX", max);
    m.insert("MIN", min);
    m.insert("MEDIAN", median);
    m.insert("MODE", mode);
    m.insert("STDEV", stdev);
    m.insert("STDEVP", stdevp);
    m.insert("VAR", var);
    m.insert("VARP", varp);
    m.insert("LARGE", large);
    m.insert("SMALL", small);
    m.insert("RANK", rank);
}

fn average(ev: &mut Evaluator, args: &[Node]) -> Value {
    match numbers_for_agg(ev, args) {
        Ok(xs) if xs.is_empty() => Value::Error(ExcelError::Div0),
        Ok(xs) => Value::Number(xs.iter().sum::<f64>() / xs.len() as f64),
        Err(e) => Value::Error(e),
    }
}

fn count(ev: &mut Evaluator, args: &[Node]) -> Value {
    Value::Number(count_numbers(ev, args) as f64)
}
fn counta(ev: &mut Evaluator, args: &[Node]) -> Value {
    Value::Number(count_nonblank(ev, args) as f64)
}
fn countblank(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let n = ev
        .flatten_arg(&args[0])
        .iter()
        .filter(|v| matches!(v, Value::Blank) || matches!(v, Value::Text(s) if s.is_empty()))
        .count();
    Value::Number(n as f64)
}

fn countif(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let range = ev.flatten_arg(&args[0]);
    let crit = ev.eval(&args[1]);
    let n = range.iter().filter(|v| matches_criteria(v, &crit)).count();
    Value::Number(n as f64)
}

fn countifs(ev: &mut Evaluator, args: &[Node]) -> Value {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return Value::Error(ExcelError::Value);
    }
    let mut pairs = Vec::new();
    let mut i = 0;
    let mut len = None;
    while i + 1 < args.len() {
        let range = ev.flatten_arg(&args[i]);
        let crit = ev.eval(&args[i + 1]);
        match len {
            None => len = Some(range.len()),
            Some(l) if l != range.len() => return Value::Error(ExcelError::Value),
            _ => {}
        }
        pairs.push((range, crit));
        i += 2;
    }
    let len = len.unwrap_or(0);
    let mut n = 0;
    for idx in 0..len {
        if pairs.iter().all(|(r, c)| matches_criteria(&r[idx], c)) {
            n += 1;
        }
    }
    Value::Number(n as f64)
}

fn averageif(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let range = ev.flatten_arg(&args[0]);
    let crit = ev.eval(&args[1]);
    let avg_range = match args.get(2) {
        Some(a) => ev.flatten_arg(a),
        None => range.clone(),
    };
    let (mut sum, mut cnt) = (0.0, 0usize);
    for (i, v) in range.iter().enumerate() {
        if matches_criteria(v, &crit) {
            if let Some(Value::Number(n)) = avg_range.get(i) {
                sum += n;
                cnt += 1;
            }
        }
    }
    if cnt == 0 {
        return Value::Error(ExcelError::Div0);
    }
    Value::Number(sum / cnt as f64)
}

fn max(ev: &mut Evaluator, args: &[Node]) -> Value {
    match numbers_for_agg(ev, args) {
        // 空集合的 MAX 按 Excel 语义返回 0
        Ok(xs) if xs.is_empty() => Value::Number(0.0),
        Ok(xs) => Value::Number(xs.into_iter().fold(f64::NEG_INFINITY, f64::max)),
        Err(e) => Value::Error(e),
    }
}
fn min(ev: &mut Evaluator, args: &[Node]) -> Value {
    match numbers_for_agg(ev, args) {
        Ok(xs) if xs.is_empty() => Value::Number(0.0),
        Ok(xs) => Value::Number(xs.into_iter().fold(f64::INFINITY, f64::min)),
        Err(e) => Value::Error(e),
    }
}

fn median(ev: &mut Evaluator, args: &[Node]) -> Value {
    let mut xs = match numbers_for_agg(ev, args) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    if xs.is_empty() {
        return Value::Error(ExcelError::Num);
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = xs.len();
    let m = if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    };
    Value::Number(m)
}

fn mode(ev: &mut Evaluator, args: &[Node]) -> Value {
    let xs = match numbers_for_agg(ev, args) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    // 按首次出现顺序找频次最高且 >1 的值
    let mut best: Option<(f64, usize)> = None;
    for (i, &x) in xs.iter().enumerate() {
        let count = xs[i..].iter().filter(|&&y| y == x).count()
            + xs[..i].iter().filter(|&&y| y == x).count();
        if count > 1 {
            match best {
                Some((_, bc)) if bc >= count => {}
                _ => best = Some((x, count)),
            }
        }
    }
    match best {
        Some((x, _)) => Value::Number(x),
        None => Value::Error(ExcelError::Na),
    }
}

fn variance(xs: &[f64], population: bool) -> Result<f64, ExcelError> {
    let n = xs.len();
    if (population && n < 1) || (!population && n < 2) {
        return Err(ExcelError::Div0);
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let ss: f64 = xs.iter().map(|x| (x - mean).powi(2)).sum();
    let denom = if population { n as f64 } else { (n - 1) as f64 };
    Ok(ss / denom)
}

fn var(ev: &mut Evaluator, args: &[Node]) -> Value {
    agg_variance(ev, args, false, false)
}
fn varp(ev: &mut Evaluator, args: &[Node]) -> Value {
    agg_variance(ev, args, true, false)
}
fn stdev(ev: &mut Evaluator, args: &[Node]) -> Value {
    agg_variance(ev, args, false, true)
}
fn stdevp(ev: &mut Evaluator, args: &[Node]) -> Value {
    agg_variance(ev, args, true, true)
}

fn agg_variance(ev: &mut Evaluator, args: &[Node], population: bool, sqrt: bool) -> Value {
    let xs = match numbers_for_agg(ev, args) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    match variance(&xs, population) {
        Ok(v) => Value::Number(if sqrt { v.sqrt() } else { v }),
        Err(e) => Value::Error(e),
    }
}

fn large(ev: &mut Evaluator, args: &[Node]) -> Value {
    nth_ordered(ev, args, true)
}
fn small(ev: &mut Evaluator, args: &[Node]) -> Value {
    nth_ordered(ev, args, false)
}

fn nth_ordered(ev: &mut Evaluator, args: &[Node], largest: bool) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let mut xs = match numbers_for_agg(ev, &args[..1]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let k = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    if k < 1 || k as usize > xs.len() {
        return Value::Error(ExcelError::Num);
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = if largest {
        xs.len() - k as usize
    } else {
        k as usize - 1
    };
    Value::Number(xs[idx])
}

fn rank(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let x = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let xs = match numbers_for_agg(ev, &args[1..2]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let ascending = match args.get(2) {
        Some(a) => match ev.eval_number(a) {
            Ok(n) => n != 0.0,
            Err(e) => return Value::Error(e),
        },
        None => false,
    };
    if !xs.contains(&x) {
        return Value::Error(ExcelError::Na);
    }
    let rank = if ascending {
        xs.iter().filter(|&&y| y < x).count() + 1
    } else {
        xs.iter().filter(|&&y| y > x).count() + 1
    };
    Value::Number(rank as f64)
}

#[cfg(test)]
mod tests {
    use crate::formula::eval::Workbook;
    use crate::formula::value::{ExcelError, Value};

    fn wb() -> Workbook {
        let mut wb = Workbook::new();
        for (i, v) in [4, 8, 15, 16, 23].iter().enumerate() {
            wb.set_input(i as u32, 0, &v.to_string()); // A1:A5
        }
        wb
    }

    #[test]
    fn averages_and_counts() {
        let wb = wb();
        assert_eq!(wb.eval_formula("AVERAGE(A1:A5)"), Value::Number(13.2));
        assert_eq!(wb.eval_formula("COUNT(A1:A5)"), Value::Number(5.0));
        assert_eq!(wb.eval_formula("MAX(A1:A5)"), Value::Number(23.0));
        assert_eq!(wb.eval_formula("MIN(A1:A5)"), Value::Number(4.0));
        assert_eq!(wb.eval_formula("MEDIAN(A1:A5)"), Value::Number(15.0));
    }

    #[test]
    fn count_variants() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "1");
        wb.set_input(1, 0, "hi");
        wb.set_input(2, 0, ""); // 空
        wb.set_input(3, 0, "5");
        assert_eq!(wb.eval_formula("COUNT(A1:A4)"), Value::Number(2.0));
        assert_eq!(wb.eval_formula("COUNTA(A1:A4)"), Value::Number(3.0));
        assert_eq!(wb.eval_formula("COUNTBLANK(A1:A4)"), Value::Number(1.0));
    }

    #[test]
    fn conditional_counts() {
        let wb = wb();
        assert_eq!(
            wb.eval_formula("COUNTIF(A1:A5,\">10\")"),
            Value::Number(3.0)
        );
        assert_eq!(
            wb.eval_formula("AVERAGEIF(A1:A5,\">10\")"),
            Value::Number((15.0 + 16.0 + 23.0) / 3.0)
        );
    }

    #[test]
    fn dispersion() {
        let mut wb = Workbook::new();
        for (i, v) in [2, 4, 4, 4, 5, 5, 7, 9].iter().enumerate() {
            wb.set_input(i as u32, 0, &v.to_string());
        }
        assert_eq!(wb.eval_formula("VARP(A1:A8)"), Value::Number(4.0));
        assert_eq!(wb.eval_formula("STDEVP(A1:A8)"), Value::Number(2.0));
    }

    #[test]
    fn order_stats() {
        let wb = wb();
        assert_eq!(wb.eval_formula("LARGE(A1:A5,1)"), Value::Number(23.0));
        assert_eq!(wb.eval_formula("SMALL(A1:A5,2)"), Value::Number(8.0));
        assert_eq!(wb.eval_formula("RANK(16,A1:A5)"), Value::Number(2.0));
    }

    #[test]
    fn empty_average_is_div0() {
        assert_eq!(
            Workbook::new().eval_formula("AVERAGE(Z1:Z9)"),
            Value::Error(ExcelError::Div0)
        );
    }
}
