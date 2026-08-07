//! 文本函数。
//!
//! 一律按 **Unicode 字符**(而非字节)计位置与长度,含中文/表情也不会错位。

use std::collections::HashMap;

use super::util::{arity, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("CONCAT", concat);
    m.insert("CONCATENATE", concat);
    m.insert("LEFT", left);
    m.insert("RIGHT", right);
    m.insert("MID", mid);
    m.insert("LEN", len);
    m.insert("LOWER", lower);
    m.insert("UPPER", upper);
    m.insert("PROPER", proper);
    m.insert("TRIM", trim);
    m.insert("REPLACE", replace);
    m.insert("SUBSTITUTE", substitute);
    m.insert("FIND", find);
    m.insert("SEARCH", search);
    m.insert("REPT", rept);
    m.insert("EXACT", exact);
    m.insert("TEXTJOIN", textjoin);
    m.insert("VALUE", value);
    m.insert("CHAR", char_);
    m.insert("UNICHAR", char_);
    m.insert("CODE", code);
    m.insert("UNICODE", code);
    m.insert("T", t);
}

/// 求值单个文本参数。
fn text_arg(ev: &mut Evaluator, node: &Node) -> Result<String, ExcelError> {
    ev.eval_text(node)
}

fn concat(ev: &mut Evaluator, args: &[Node]) -> Value {
    let mut out = String::new();
    for arg in args {
        // CONCAT 会展开范围,逐格拼接
        for v in ev.flatten_arg(arg) {
            match v.to_text() {
                Ok(s) => out.push_str(&s),
                Err(e) => return Value::Error(e),
            }
        }
    }
    Value::Text(out)
}

fn left(ev: &mut Evaluator, args: &[Node]) -> Value {
    take_side(ev, args, true)
}
fn right(ev: &mut Evaluator, args: &[Node]) -> Value {
    take_side(ev, args, false)
}

fn take_side(ev: &mut Evaluator, args: &[Node], from_left: bool) -> Value {
    if let Err(e) = arity(args, 1, Some(2)) {
        return e;
    }
    let s = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let n = match args.get(1) {
        Some(a) => match ev.eval_number(a) {
            Ok(v) => v.trunc() as i64,
            Err(e) => return Value::Error(e),
        },
        None => 1,
    };
    if n < 0 {
        return Value::Error(ExcelError::Value);
    }
    let chars: Vec<char> = s.chars().collect();
    let n = (n as usize).min(chars.len());
    let slice: String = if from_left {
        chars[..n].iter().collect()
    } else {
        chars[chars.len() - n..].iter().collect()
    };
    Value::Text(slice)
}

fn mid(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 3, Some(3)) {
        return e;
    }
    let s = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let start = match ev.eval_number(&args[1]) {
        Ok(v) => v.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let count = match ev.eval_number(&args[2]) {
        Ok(v) => v.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    if start < 1 || count < 0 {
        return Value::Error(ExcelError::Value);
    }
    let chars: Vec<char> = s.chars().collect();
    let start = start as usize - 1;
    if start >= chars.len() {
        return Value::Text(String::new());
    }
    let end = (start + count as usize).min(chars.len());
    Value::Text(chars[start..end].iter().collect())
}

fn len(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match text_arg(ev, &args[0]) {
        Ok(s) => Value::Number(s.chars().count() as f64),
        Err(e) => Value::Error(e),
    }
}

fn lower(ev: &mut Evaluator, args: &[Node]) -> Value {
    map_text(ev, args, |s| s.to_lowercase())
}
fn upper(ev: &mut Evaluator, args: &[Node]) -> Value {
    map_text(ev, args, |s| s.to_uppercase())
}
fn trim(ev: &mut Evaluator, args: &[Node]) -> Value {
    // Excel TRIM:去首尾空格,并把词间多个空格压成一个
    map_text(ev, args, |s| {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    })
}
fn proper(ev: &mut Evaluator, args: &[Node]) -> Value {
    map_text(ev, args, |s| {
        let mut out = String::with_capacity(s.len());
        let mut prev_alpha = false;
        for ch in s.chars() {
            if ch.is_alphabetic() {
                if prev_alpha {
                    out.extend(ch.to_lowercase());
                } else {
                    out.extend(ch.to_uppercase());
                }
                prev_alpha = true;
            } else {
                out.push(ch);
                prev_alpha = false;
            }
        }
        out
    })
}

fn map_text(ev: &mut Evaluator, args: &[Node], f: impl Fn(&str) -> String) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match text_arg(ev, &args[0]) {
        Ok(s) => Value::Text(f(&s)),
        Err(e) => Value::Error(e),
    }
}

fn replace(ev: &mut Evaluator, args: &[Node]) -> Value {
    // REPLACE(old, start, num_chars, new)
    if let Err(e) = arity(args, 4, Some(4)) {
        return e;
    }
    let old = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let start = match ev.eval_number(&args[1]) {
        Ok(v) => v.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let count = match ev.eval_number(&args[2]) {
        Ok(v) => v.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let new = match text_arg(ev, &args[3]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    if start < 1 || count < 0 {
        return Value::Error(ExcelError::Value);
    }
    let chars: Vec<char> = old.chars().collect();
    let start = (start as usize - 1).min(chars.len());
    let end = (start + count as usize).min(chars.len());
    let mut out: String = chars[..start].iter().collect();
    out.push_str(&new);
    out.extend(&chars[end..]);
    Value::Text(out)
}

fn substitute(ev: &mut Evaluator, args: &[Node]) -> Value {
    // SUBSTITUTE(text, old, new, [instance])
    if let Err(e) = arity(args, 3, Some(4)) {
        return e;
    }
    let text = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let old = match text_arg(ev, &args[1]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let new = match text_arg(ev, &args[2]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    if old.is_empty() {
        return Value::Text(text);
    }
    match args.get(3) {
        None => Value::Text(text.replace(&old, &new)),
        Some(a) => {
            let inst = match ev.eval_number(a) {
                Ok(v) => v.trunc() as i64,
                Err(e) => return Value::Error(e),
            };
            if inst < 1 {
                return Value::Error(ExcelError::Value);
            }
            // 只替换第 inst 次出现
            let mut count = 0;
            let mut result = String::new();
            let mut rest = text.as_str();
            while let Some(pos) = rest.find(&old) {
                count += 1;
                if count == inst {
                    result.push_str(&rest[..pos]);
                    result.push_str(&new);
                    result.push_str(&rest[pos + old.len()..]);
                    return Value::Text(result);
                }
                result.push_str(&rest[..pos + old.len()]);
                rest = &rest[pos + old.len()..];
            }
            result.push_str(rest);
            Value::Text(result)
        }
    }
}

fn find(ev: &mut Evaluator, args: &[Node]) -> Value {
    locate(ev, args, true)
}
fn search(ev: &mut Evaluator, args: &[Node]) -> Value {
    locate(ev, args, false)
}

/// FIND(区分大小写)/ SEARCH(不区分)的共同实现;返回 1 基**字符**位置。
fn locate(ev: &mut Evaluator, args: &[Node], case_sensitive: bool) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let needle = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let haystack = match text_arg(ev, &args[1]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let start = match args.get(2) {
        Some(a) => match ev.eval_number(a) {
            Ok(v) => v.trunc() as i64,
            Err(e) => return Value::Error(e),
        },
        None => 1,
    };
    if start < 1 {
        return Value::Error(ExcelError::Value);
    }
    let start0 = start as usize - 1;
    // SEARCH 不区分大小写:两侧都折叠为小写后再朴素子串搜索。
    let hay_chars: Vec<char> = if case_sensitive {
        haystack.chars().collect()
    } else {
        haystack.to_lowercase().chars().collect()
    };
    let needle_chars: Vec<char> = if case_sensitive {
        needle.chars().collect()
    } else {
        needle.to_lowercase().chars().collect()
    };
    if start0 > hay_chars.len() {
        return Value::Error(ExcelError::Value);
    }
    if needle_chars.is_empty() {
        return Value::Number(start as f64);
    }
    if needle_chars.len() > hay_chars.len() {
        return Value::Error(ExcelError::Value);
    }
    for i in start0..=hay_chars.len() - needle_chars.len() {
        if hay_chars[i..i + needle_chars.len()] == needle_chars[..] {
            return Value::Number((i + 1) as f64);
        }
    }
    Value::Error(ExcelError::Value)
}

fn rept(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let s = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let n = match ev.eval_number(&args[1]) {
        Ok(v) => v.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    if n < 0 {
        return Value::Error(ExcelError::Value);
    }
    // 防止内存爆炸:上限 32767 字符(Excel 单元格长度上限)
    if s.len().saturating_mul(n as usize) > 32_767 {
        return Value::Error(ExcelError::Value);
    }
    Value::Text(s.repeat(n as usize))
}

fn exact(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let a = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let b = match text_arg(ev, &args[1]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    Value::Bool(a == b) // EXACT 区分大小写
}

fn textjoin(ev: &mut Evaluator, args: &[Node]) -> Value {
    // TEXTJOIN(delimiter, ignore_empty, text1, ...)
    if let Err(e) = arity(args, 3, None) {
        return e;
    }
    let delim = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let ignore_empty = match ev.eval_bool(&args[1]) {
        Ok(b) => b,
        Err(e) => return Value::Error(e),
    };
    let mut parts = Vec::new();
    for arg in &args[2..] {
        for v in ev.flatten_arg(arg) {
            let s = match v.to_text() {
                Ok(s) => s,
                Err(e) => return Value::Error(e),
            };
            if ignore_empty && s.is_empty() {
                continue;
            }
            parts.push(s);
        }
    }
    Value::Text(parts.join(&delim))
}

fn value(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let s = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    match s.trim().parse::<f64>() {
        Ok(n) => Value::Number(n),
        Err(_) => Value::Error(ExcelError::Value),
    }
}

fn char_(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let n = match ev.eval_number(&args[0]) {
        Ok(v) => v.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    if !(1..=0x10FFFF).contains(&n) {
        return Value::Error(ExcelError::Value);
    }
    match char::from_u32(n as u32) {
        Some(c) => Value::Text(c.to_string()),
        None => Value::Error(ExcelError::Value),
    }
}

fn code(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let s = match text_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    match s.chars().next() {
        Some(c) => Value::Number(c as u32 as f64),
        None => Value::Error(ExcelError::Value),
    }
}

fn t(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match ev.eval(&args[0]) {
        Value::Text(s) => Value::Text(s),
        Value::Error(e) => Value::Error(e),
        _ => Value::Text(String::new()),
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
    fn slicing() {
        assert_eq!(ev("LEFT(\"hello\",2)"), Value::Text("he".into()));
        assert_eq!(ev("RIGHT(\"hello\",3)"), Value::Text("llo".into()));
        assert_eq!(ev("MID(\"hello\",2,3)"), Value::Text("ell".into()));
        assert_eq!(ev("LEN(\"北京\")"), Value::Number(2.0));
        assert_eq!(ev("LEFT(\"北京市\",2)"), Value::Text("北京".into()));
    }

    #[test]
    fn casing_and_trim() {
        assert_eq!(ev("UPPER(\"abc\")"), Value::Text("ABC".into()));
        assert_eq!(ev("LOWER(\"ABC\")"), Value::Text("abc".into()));
        assert_eq!(
            ev("PROPER(\"hello world\")"),
            Value::Text("Hello World".into())
        );
        assert_eq!(ev("TRIM(\"  a   b  \")"), Value::Text("a b".into()));
    }

    #[test]
    fn concat_and_join() {
        assert_eq!(ev("CONCAT(\"a\",\"b\",\"c\")"), Value::Text("abc".into()));
        assert_eq!(ev("CONCATENATE(\"x\",1)"), Value::Text("x1".into()));
        assert_eq!(
            ev("TEXTJOIN(\"-\",TRUE,\"a\",\"\",\"b\")"),
            Value::Text("a-b".into())
        );
    }

    #[test]
    fn find_replace_substitute() {
        assert_eq!(ev("FIND(\"l\",\"hello\")"), Value::Number(3.0));
        assert_eq!(ev("SEARCH(\"L\",\"hello\")"), Value::Number(3.0));
        assert_eq!(
            ev("REPLACE(\"abcdef\",2,3,\"X\")"),
            Value::Text("aXef".into())
        );
        assert_eq!(
            ev("SUBSTITUTE(\"a-b-c\",\"-\",\"+\")"),
            Value::Text("a+b+c".into())
        );
        assert_eq!(
            ev("SUBSTITUTE(\"a-b-c\",\"-\",\"+\",2)"),
            Value::Text("a-b+c".into())
        );
    }

    #[test]
    fn misc_text() {
        assert_eq!(ev("REPT(\"ab\",3)"), Value::Text("ababab".into()));
        assert_eq!(ev("EXACT(\"a\",\"A\")"), Value::Bool(false));
        assert_eq!(ev("VALUE(\"3.5\")"), Value::Number(3.5));
        assert_eq!(ev("CHAR(65)"), Value::Text("A".into()));
        assert_eq!(ev("CODE(\"A\")"), Value::Number(65.0));
    }
}
