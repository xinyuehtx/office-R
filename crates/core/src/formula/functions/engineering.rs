//! 工程函数(子集):进制转换与位运算。
//!
//! 覆盖常用的 `DEC2BIN`/`BIN2DEC`/`DEC2HEX`/`HEX2DEC`/`DEC2OCT`/`OCT2DEC` 及
//! `BITAND`/`BITOR`/`BITXOR`/`BITLSHIFT`/`BITRSHIFT`。数值范围与 Excel 大体一致
//! (进制转换限 10 位补码;位运算限 48 位非负整数),越界返回 `#NUM!`。

use std::collections::HashMap;

use super::util::{arity, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("DEC2BIN", dec2bin);
    m.insert("DEC2OCT", dec2oct);
    m.insert("DEC2HEX", dec2hex);
    m.insert("BIN2DEC", bin2dec);
    m.insert("OCT2DEC", oct2dec);
    m.insert("HEX2DEC", hex2dec);
    m.insert("BITAND", bitand);
    m.insert("BITOR", bitor);
    m.insert("BITXOR", bitxor);
    m.insert("BITLSHIFT", bitlshift);
    m.insert("BITRSHIFT", bitrshift);
}

/// 取整数参数。
fn int_arg(ev: &mut Evaluator, node: &Node) -> Result<i64, ExcelError> {
    Ok(ev.eval_number(node)?.trunc() as i64)
}

/// 十进制 → 指定进制的补码字符串(bits 位,Excel 用 10 位)。
fn dec_to_base(n: i64, base: u32, bits: u32, args: &[Node], ev: &mut Evaluator) -> Value {
    // 范围:-2^(bits-1) .. 2^(bits-1)-1
    let max = 1i64 << (bits - 1);
    if n < -max || n >= max {
        return Value::Error(ExcelError::Num);
    }
    let s = if n < 0 {
        // 补码:2^bits + n
        let u = (1u64 << bits).wrapping_add(n as u64) & ((1u64 << bits) - 1);
        to_radix(u, base)
    } else {
        to_radix(n as u64, base)
    };
    // 可选 places 参数(第 2 个):左侧补 0
    if let Some(p) = args.get(1) {
        if let Ok(places) = ev.eval_number(p) {
            let places = places.trunc() as usize;
            if n >= 0 && places >= s.len() {
                return Value::Text(format!("{s:0>places$}"));
            }
        }
    }
    Value::Text(s)
}

fn to_radix(mut u: u64, base: u32) -> String {
    if u == 0 {
        return "0".to_string();
    }
    let digits = b"0123456789ABCDEF";
    let mut out = Vec::new();
    while u > 0 {
        out.push(digits[(u % base as u64) as usize]);
        u /= base as u64;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// 指定进制补码字符串 → 十进制(bits 位)。
fn base_to_dec(ev: &mut Evaluator, node: &Node, base: u32, bits: u32) -> Value {
    let s = match ev.eval_text(node) {
        Ok(s) => s.trim().to_uppercase(),
        Err(e) => return Value::Error(e),
    };
    if s.is_empty() || s.len() as u32 > bits.max(10) {
        return Value::Error(ExcelError::Num);
    }
    let u = match u64::from_str_radix(&s, base) {
        Ok(u) => u,
        Err(_) => return Value::Error(ExcelError::Num),
    };
    // 最高位为符号位(补码)
    let sign_bit = 1u64 << (bits - 1);
    let val = if s.len() as u32 == bits && (u & sign_bit) != 0 {
        (u as i64) - (1i64 << bits)
    } else {
        u as i64
    };
    Value::Number(val as f64)
}

fn dec2bin(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(2)) {
        return e;
    }
    match int_arg(ev, &args[0]) {
        Ok(n) => dec_to_base(n, 2, 10, args, ev),
        Err(e) => Value::Error(e),
    }
}
fn dec2oct(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(2)) {
        return e;
    }
    match int_arg(ev, &args[0]) {
        Ok(n) => dec_to_base(n, 8, 30, args, ev),
        Err(e) => Value::Error(e),
    }
}
fn dec2hex(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(2)) {
        return e;
    }
    match int_arg(ev, &args[0]) {
        Ok(n) => dec_to_base(n, 16, 40, args, ev),
        Err(e) => Value::Error(e),
    }
}
fn bin2dec(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    base_to_dec(ev, &args[0], 2, 10)
}
fn oct2dec(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    base_to_dec(ev, &args[0], 8, 30)
}
fn hex2dec(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    base_to_dec(ev, &args[0], 16, 40)
}

/// 位运算的非负整数参数(Excel 限 0 .. 2^48-1)。
fn bit_arg(ev: &mut Evaluator, node: &Node) -> Result<u64, ExcelError> {
    let n = ev.eval_number(node)?;
    if n < 0.0 || n.fract() != 0.0 || n >= (1u64 << 48) as f64 {
        return Err(ExcelError::Num);
    }
    Ok(n as u64)
}

fn bitand(ev: &mut Evaluator, args: &[Node]) -> Value {
    bit_binop(ev, args, |a, b| a & b)
}
fn bitor(ev: &mut Evaluator, args: &[Node]) -> Value {
    bit_binop(ev, args, |a, b| a | b)
}
fn bitxor(ev: &mut Evaluator, args: &[Node]) -> Value {
    bit_binop(ev, args, |a, b| a ^ b)
}

fn bit_binop(ev: &mut Evaluator, args: &[Node], op: fn(u64, u64) -> u64) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let a = match bit_arg(ev, &args[0]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let b = match bit_arg(ev, &args[1]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    Value::Number(op(a, b) as f64)
}

fn bitlshift(ev: &mut Evaluator, args: &[Node]) -> Value {
    bit_shift(ev, args, true)
}
fn bitrshift(ev: &mut Evaluator, args: &[Node]) -> Value {
    bit_shift(ev, args, false)
}

fn bit_shift(ev: &mut Evaluator, args: &[Node], left: bool) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let a = match bit_arg(ev, &args[0]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let shift = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    // 负位移反向移动(与 Excel 一致)
    let (left, amount) = if shift < 0 {
        (!left, (-shift) as u32)
    } else {
        (left, shift as u32)
    };
    if amount >= 64 {
        return Value::Number(0.0);
    }
    let r = if left { a << amount } else { a >> amount };
    Value::Number(r as f64)
}

#[cfg(test)]
mod tests {
    use crate::formula::{evaluate, Value};

    #[test]
    fn base_conversions() {
        assert_eq!(evaluate("DEC2BIN(9)"), Value::Text("1001".into()));
        assert_eq!(evaluate("DEC2BIN(9,8)"), Value::Text("00001001".into()));
        assert_eq!(evaluate("DEC2HEX(255)"), Value::Text("FF".into()));
        assert_eq!(evaluate("DEC2OCT(8)"), Value::Text("10".into()));
        assert_eq!(evaluate("BIN2DEC(\"1001\")"), Value::Number(9.0));
        assert_eq!(evaluate("HEX2DEC(\"FF\")"), Value::Number(255.0));
        // 补码负数:1111111111 (10位) = -1
        assert_eq!(evaluate("DEC2BIN(-1)"), Value::Text("1111111111".into()));
        assert_eq!(evaluate("BIN2DEC(\"1111111111\")"), Value::Number(-1.0));
    }

    #[test]
    fn bit_ops() {
        assert_eq!(evaluate("BITAND(6,3)"), Value::Number(2.0));
        assert_eq!(evaluate("BITOR(6,3)"), Value::Number(7.0));
        assert_eq!(evaluate("BITXOR(6,3)"), Value::Number(5.0));
        assert_eq!(evaluate("BITLSHIFT(1,4)"), Value::Number(16.0));
        assert_eq!(evaluate("BITRSHIFT(16,4)"), Value::Number(1.0));
    }
}
