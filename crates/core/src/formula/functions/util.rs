//! 函数库共用的辅助:参数检查、数值收集、条件(criteria)匹配。
//!
//! 这些规则(范围里的文本被忽略、`SUMIF` 的 `">5"` 条件语法、通配符匹配)在多个
//! 函数间复用,集中在此避免各函数各写一套、语义漂移。

use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

/// 函数实现签名:拿 `&mut Evaluator`(可短路求值参数)+ 未求值的实参 AST。
pub type FuncImpl = fn(&mut Evaluator, &[Node]) -> Value;

/// 校验参数个数落在 `[min, max]`(`max = None` 表示不设上限)。
/// 不满足则返回 `Err(#VALUE!)`,便于 `?` 风格提前返回。
pub fn arity(args: &[Node], min: usize, max: Option<usize>) -> Result<(), Value> {
    let n = args.len();
    if n < min || max.is_some_and(|m| n > m) {
        return Err(Value::Error(ExcelError::Value));
    }
    Ok(())
}

/// 收集用于**聚合**(SUM/AVERAGE/MAX/…)的数值,遵循 Excel 规则:
/// - 引用/数组里的元素:只计入数值,遇错误则传播,其余(文本/布尔/空)忽略;
/// - 标量参数:数值/布尔计入(`TRUE=1`),文本按数字解析(失败得 `#VALUE!`),空忽略。
pub fn numbers_for_agg(ev: &mut Evaluator, args: &[Node]) -> Result<Vec<f64>, ExcelError> {
    let mut out = Vec::new();
    for arg in args {
        if Evaluator::is_reference(arg) {
            for v in ev.flatten_arg(arg) {
                match v {
                    Value::Number(n) => out.push(n),
                    Value::Error(e) => return Err(e),
                    _ => {} // 引用里的文本/布尔/空:忽略
                }
            }
        } else {
            match ev.eval(arg) {
                Value::Number(n) => out.push(n),
                Value::Bool(b) => out.push(if b { 1.0 } else { 0.0 }),
                Value::Blank => {}
                Value::Text(s) => match s.trim().parse::<f64>() {
                    Ok(n) => out.push(n),
                    Err(_) => return Err(ExcelError::Value),
                },
                Value::Error(e) => return Err(e),
                Value::Array(a) => {
                    for v in a.data {
                        match v {
                            Value::Number(n) => out.push(n),
                            Value::Error(e) => return Err(e),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// 统计「数值」个数(COUNT 语义):引用/数组里只数数值;标量里数值/布尔/可解析文本计入。
/// 错误值被忽略(COUNT 不传播错误)。
pub fn count_numbers(ev: &mut Evaluator, args: &[Node]) -> usize {
    let mut n = 0;
    for arg in args {
        if Evaluator::is_reference(arg) {
            for v in ev.flatten_arg(arg) {
                if matches!(v, Value::Number(_)) {
                    n += 1;
                }
            }
        } else {
            match ev.eval(arg) {
                Value::Number(_) | Value::Bool(_) => n += 1,
                Value::Text(s) if s.trim().parse::<f64>().is_ok() => n += 1,
                Value::Array(a) => {
                    n += a
                        .data
                        .iter()
                        .filter(|v| matches!(v, Value::Number(_)))
                        .count()
                }
                _ => {}
            }
        }
    }
    n
}

/// 统计「非空」个数(COUNTA 语义):任何非 [`Value::Blank`] 都计入(含错误、文本)。
pub fn count_nonblank(ev: &mut Evaluator, args: &[Node]) -> usize {
    let mut n = 0;
    for arg in args {
        for v in ev.flatten_arg(arg) {
            if !v.is_blank() {
                n += 1;
            }
        }
    }
    n
}

/// 判断一个值是否满足条件(用于 COUNTIF / SUMIF / AVERAGEIF)。
///
/// 条件可以是:
/// - 数值 `5`:等于比较;
/// - 文本 `">5"` / `"<=3"` / `"<>x"`:前缀运算符 + 操作数;
/// - 文本 `"apple"` / `"a*"`:等值比较,支持 `*`(任意串)`?`(单字符)通配。
pub fn matches_criteria(value: &Value, criteria: &Value) -> bool {
    match criteria {
        Value::Number(n) => value.to_number().map(|v| v == *n).unwrap_or(false),
        Value::Bool(b) => matches!(value, Value::Bool(x) if x == b),
        Value::Text(s) => matches_text_criteria(value, s),
        _ => false,
    }
}

fn matches_text_criteria(value: &Value, criteria: &str) -> bool {
    let (op, rest) = split_operator(criteria.trim());

    // 操作数是数字?则做数值比较(仅数值单元格参与,`<>` 例外)。
    if let Ok(target) = rest.trim().parse::<f64>() {
        return match value.to_number() {
            Ok(v) => compare_num(op, v, target),
            Err(_) => op == Op::Ne, // 非数值单元格只满足 "<>数字"
        };
    }

    // 操作数是文本:等值/不等用通配符;大小/等号用文本序。
    match op {
        Op::Eq => text_equals_wild(value, rest),
        Op::Ne => !text_equals_wild(value, rest),
        _ => {
            let vs = match value {
                Value::Text(s) => s.clone(),
                Value::Blank => String::new(),
                other => other.to_text().unwrap_or_default(),
            };
            let a = vs.to_lowercase();
            let b = rest.to_lowercase();
            match op {
                Op::Lt => a < b,
                Op::Le => a <= b,
                Op::Gt => a > b,
                Op::Ge => a >= b,
                _ => false,
            }
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

fn split_operator(s: &str) -> (Op, &str) {
    if let Some(r) = s.strip_prefix(">=") {
        (Op::Ge, r)
    } else if let Some(r) = s.strip_prefix("<=") {
        (Op::Le, r)
    } else if let Some(r) = s.strip_prefix("<>") {
        (Op::Ne, r)
    } else if let Some(r) = s.strip_prefix('>') {
        (Op::Gt, r)
    } else if let Some(r) = s.strip_prefix('<') {
        (Op::Lt, r)
    } else if let Some(r) = s.strip_prefix('=') {
        (Op::Eq, r)
    } else {
        (Op::Eq, s)
    }
}

fn compare_num(op: Op, v: f64, target: f64) -> bool {
    match op {
        Op::Eq => v == target,
        Op::Ne => v != target,
        Op::Lt => v < target,
        Op::Le => v <= target,
        Op::Gt => v > target,
        Op::Ge => v >= target,
    }
}

/// 文本等值比较(忽略大小写,支持 `*` `?` 通配)。
fn text_equals_wild(value: &Value, pattern: &str) -> bool {
    let text = match value {
        Value::Text(s) => s.clone(),
        Value::Blank => String::new(),
        Value::Number(n) => crate::formula::value::format_number(*n),
        Value::Bool(b) => {
            if *b {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }
        _ => return false,
    };
    wildcard_match(&text.to_lowercase(), &pattern.to_lowercase())
}

/// 通配符匹配:`*` 匹配任意长度(含空),`?` 匹配单个字符。
///
/// 用经典的双指针 + 回溯,遇到 `*` 记录回溯点,不匹配时回退并让 `*` 多吞一个字符。
fn wildcard_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    let (mut ti, mut pi) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_criteria() {
        assert!(matches_criteria(
            &Value::Number(6.0),
            &Value::Text(">5".into())
        ));
        assert!(!matches_criteria(
            &Value::Number(4.0),
            &Value::Text(">5".into())
        ));
        assert!(matches_criteria(&Value::Number(5.0), &Value::Number(5.0)));
        assert!(matches_criteria(
            &Value::Number(3.0),
            &Value::Text("<>5".into())
        ));
    }

    #[test]
    fn text_criteria_with_wildcards() {
        assert!(matches_criteria(
            &Value::Text("apple".into()),
            &Value::Text("a*".into())
        ));
        assert!(matches_criteria(
            &Value::Text("Apple".into()),
            &Value::Text("apple".into())
        ));
        assert!(matches_criteria(
            &Value::Text("cat".into()),
            &Value::Text("?at".into())
        ));
        assert!(!matches_criteria(
            &Value::Text("dog".into()),
            &Value::Text("a*".into())
        ));
        assert!(matches_criteria(
            &Value::Text("dog".into()),
            &Value::Text("<>cat".into())
        ));
    }

    #[test]
    fn wildcard_edge_cases() {
        assert!(wildcard_match("", "*"));
        assert!(wildcard_match("abc", "*"));
        assert!(wildcard_match("abc", "a*c"));
        assert!(wildcard_match("abbbc", "a*c"));
        assert!(!wildcard_match("abd", "a*c"));
        assert!(wildcard_match("abc", "a?c"));
    }
}
