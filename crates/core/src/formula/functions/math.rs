//! 数学与三角函数。

use std::cell::Cell;
use std::collections::HashMap;

use super::util::{arity, matches_criteria, numbers_for_agg, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

/// 注册本类别的所有函数。
pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("SUM", sum);
    m.insert("PRODUCT", product);
    m.insert("SUMSQ", sumsq);
    m.insert("ABS", abs);
    m.insert("SIGN", sign);
    m.insert("INT", int);
    m.insert("TRUNC", trunc);
    m.insert("MOD", mod_);
    m.insert("ROUND", round);
    m.insert("ROUNDUP", roundup);
    m.insert("ROUNDDOWN", rounddown);
    m.insert("MROUND", mround);
    m.insert("CEILING", ceiling);
    m.insert("FLOOR", floor);
    m.insert("POWER", power);
    m.insert("SQRT", sqrt);
    m.insert("EXP", exp);
    m.insert("LN", ln);
    m.insert("LOG", log);
    m.insert("LOG10", log10);
    m.insert("PI", pi);
    m.insert("SIN", sin);
    m.insert("COS", cos);
    m.insert("TAN", tan);
    m.insert("ASIN", asin);
    m.insert("ACOS", acos);
    m.insert("ATAN", atan);
    m.insert("ATAN2", atan2);
    m.insert("DEGREES", degrees);
    m.insert("RADIANS", radians);
    m.insert("GCD", gcd);
    m.insert("LCM", lcm);
    m.insert("QUOTIENT", quotient);
    m.insert("EVEN", even);
    m.insert("ODD", odd);
    m.insert("FACT", fact);
    m.insert("COMBIN", combin);
    m.insert("RAND", rand);
    m.insert("RANDBETWEEN", randbetween);
    m.insert("SUMIF", sumif);
    m.insert("SUMIFS", sumifs);
    m.insert("SUMPRODUCT", sumproduct);
    m.insert("SUBTOTAL", subtotal);
}

/// 一元数值函数的公共外壳:校验 1 个参数、强制为数值,再套用 `f`。
fn unary(ev: &mut Evaluator, args: &[Node], f: impl Fn(f64) -> Value) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    match ev.eval_number(&args[0]) {
        Ok(n) => f(n),
        Err(e) => Value::Error(e),
    }
}

/// 把浮点结果收敛成值:非有限(NaN/±inf)一律映射为 `#NUM!`。
///
/// 这是**整个函数库的收口点** —— 一旦 NaN 逃逸到 `Value::Number`,
/// 下游任何 `partial_cmp` 排序都会变成 panic 风险(见 `stats::median`)。
pub(super) fn num(n: f64) -> Value {
    if n.is_finite() {
        Value::Number(n)
    } else {
        Value::Error(ExcelError::Num)
    }
}

fn sum(ev: &mut Evaluator, args: &[Node]) -> Value {
    match numbers_for_agg(ev, args) {
        Ok(xs) => Value::Number(xs.iter().sum()),
        Err(e) => Value::Error(e),
    }
}

fn product(ev: &mut Evaluator, args: &[Node]) -> Value {
    match numbers_for_agg(ev, args) {
        Ok(xs) if xs.is_empty() => Value::Number(0.0),
        Ok(xs) => Value::Number(xs.iter().product()),
        Err(e) => Value::Error(e),
    }
}

fn sumsq(ev: &mut Evaluator, args: &[Node]) -> Value {
    match numbers_for_agg(ev, args) {
        Ok(xs) => Value::Number(xs.iter().map(|x| x * x).sum()),
        Err(e) => Value::Error(e),
    }
}

fn abs(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.abs()))
}

fn sign(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        Value::Number(if n > 0.0 {
            1.0
        } else if n < 0.0 {
            -1.0
        } else {
            0.0
        })
    })
}

fn int(ev: &mut Evaluator, args: &[Node]) -> Value {
    // Excel INT 向下取整(对负数也向下,如 INT(-1.5)=-2)
    unary(ev, args, |n| num(n.floor()))
}

fn trunc(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(2)) {
        return e;
    }
    let n = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let digits = match args.get(1) {
        Some(a) => match ev.eval_number(a) {
            Ok(d) => d.trunc() as i32,
            Err(e) => return Value::Error(e),
        },
        None => 0,
    };
    let f = 10f64.powi(digits);
    num((n * f).trunc() / f)
}

fn mod_(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let a = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let b = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    if b == 0.0 {
        return Value::Error(ExcelError::Div0);
    }
    // Excel:结果符号跟随除数
    num(a - b * (a / b).floor())
}

/// 四舍五入到 `digits` 位小数(半值远离零),`op` 决定取整方向。
fn round_with(ev: &mut Evaluator, args: &[Node], op: fn(f64) -> f64) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let n = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let digits = match ev.eval_number(&args[1]) {
        Ok(d) => d.trunc() as i32,
        Err(e) => return Value::Error(e),
    };
    let f = 10f64.powi(digits);
    num(op(n * f) / f)
}

fn round(ev: &mut Evaluator, args: &[Node]) -> Value {
    round_with(ev, args, f64::round)
}
fn roundup(ev: &mut Evaluator, args: &[Node]) -> Value {
    // 远离零方向
    round_with(ev, args, |x| if x >= 0.0 { x.ceil() } else { x.floor() })
}
fn rounddown(ev: &mut Evaluator, args: &[Node]) -> Value {
    round_with(ev, args, |x| if x >= 0.0 { x.floor() } else { x.ceil() })
}

fn mround(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let n = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let mult = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    if mult == 0.0 {
        return Value::Number(0.0);
    }
    if n.signum() != mult.signum() && n != 0.0 {
        return Value::Error(ExcelError::Num); // 符号不一致
    }
    num((n / mult).round() * mult)
}

fn ceiling(ev: &mut Evaluator, args: &[Node]) -> Value {
    signed_step(ev, args, true)
}
fn floor(ev: &mut Evaluator, args: &[Node]) -> Value {
    signed_step(ev, args, false)
}

fn signed_step(ev: &mut Evaluator, args: &[Node], up: bool) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let n = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let s = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    if s == 0.0 {
        return Value::Number(0.0);
    }
    if n.signum() != s.signum() && n != 0.0 {
        return Value::Error(ExcelError::Num);
    }
    let q = n / s;
    num(if up { q.ceil() } else { q.floor() } * s)
}

fn power(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let base = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let exp = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    if base < 0.0 && exp.fract() != 0.0 {
        return Value::Error(ExcelError::Num);
    }
    num(base.powf(exp))
}

fn sqrt(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        if n < 0.0 {
            Value::Error(ExcelError::Num)
        } else {
            num(n.sqrt())
        }
    })
}
fn exp(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.exp()))
}
fn ln(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        if n <= 0.0 {
            Value::Error(ExcelError::Num)
        } else {
            num(n.ln())
        }
    })
}
fn log10(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        if n <= 0.0 {
            Value::Error(ExcelError::Num)
        } else {
            num(n.log10())
        }
    })
}
fn log(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(2)) {
        return e;
    }
    let n = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let base = match args.get(1) {
        Some(a) => match ev.eval_number(a) {
            Ok(b) => b,
            Err(e) => return Value::Error(e),
        },
        None => 10.0,
    };
    if n <= 0.0 || base <= 0.0 || base == 1.0 {
        return Value::Error(ExcelError::Num);
    }
    // 常见底数用专用实现,避免 ln/ln 的浮点误差(如 LOG(1000)=3 而非 2.9999…)
    let r = if base == 10.0 {
        n.log10()
    } else if base == 2.0 {
        n.log2()
    } else {
        n.log(base)
    };
    num(r)
}

fn pi(_ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 0, Some(0)) {
        return e;
    }
    Value::Number(std::f64::consts::PI)
}

fn sin(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.sin()))
}
fn cos(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.cos()))
}
fn tan(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.tan()))
}
fn asin(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        if !(-1.0..=1.0).contains(&n) {
            Value::Error(ExcelError::Num)
        } else {
            num(n.asin())
        }
    })
}
fn acos(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        if !(-1.0..=1.0).contains(&n) {
            Value::Error(ExcelError::Num)
        } else {
            num(n.acos())
        }
    })
}
fn atan(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.atan()))
}
fn atan2(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let x = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let y = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    // 注意 Excel 的参数顺序是 (x, y),标准库是 atan2(y, x)
    num(y.atan2(x))
}
fn degrees(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.to_degrees()))
}
fn radians(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| num(n.to_radians()))
}

fn int_args(ev: &mut Evaluator, args: &[Node]) -> Result<Vec<i64>, ExcelError> {
    let xs = numbers_for_agg(ev, args)?;
    Ok(xs.into_iter().map(|x| x.trunc() as i64).collect())
}

fn gcd(ev: &mut Evaluator, args: &[Node]) -> Value {
    let xs = match int_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let g = xs.into_iter().fold(0i64, |a, b| gcd2(a, b.abs()));
    Value::Number(g as f64)
}
fn lcm(ev: &mut Evaluator, args: &[Node]) -> Value {
    let xs = match int_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let mut l: i64 = 1;
    for x in xs {
        let x = x.abs();
        if x == 0 {
            return Value::Number(0.0);
        }
        l = l / gcd2(l, x) * x;
    }
    Value::Number(l as f64)
}
fn gcd2(a: i64, b: i64) -> i64 {
    if b == 0 {
        a
    } else {
        gcd2(b, a % b)
    }
}

fn quotient(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let a = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let b = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    if b == 0.0 {
        return Value::Error(ExcelError::Div0);
    }
    Value::Number((a / b).trunc())
}

fn even(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        let up = if n >= 0.0 { n.ceil() } else { n.floor() };
        let r = if (up as i64) % 2 == 0 {
            up
        } else {
            up + n.signum()
        };
        num(r)
    })
}
fn odd(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        let mut up = if n >= 0.0 { n.ceil() } else { n.floor() };
        if (up as i64) % 2 == 0 {
            up += if n >= 0.0 { 1.0 } else { -1.0 };
        }
        // n=0 时 Excel 返回 1
        num(if n == 0.0 { 1.0 } else { up })
    })
}

fn fact(ev: &mut Evaluator, args: &[Node]) -> Value {
    unary(ev, args, |n| {
        if n < 0.0 {
            return Value::Error(ExcelError::Num);
        }
        let k = n.trunc() as u64;
        if k > 170 {
            return Value::Error(ExcelError::Num); // 超出 f64 可表示
        }
        let mut acc = 1.0f64;
        for i in 2..=k {
            acc *= i as f64;
        }
        Value::Number(acc)
    })
}

fn combin(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let n = match ev.eval_number(&args[0]) {
        Ok(x) => x.trunc(),
        Err(e) => return Value::Error(e),
    };
    let k = match ev.eval_number(&args[1]) {
        Ok(x) => x.trunc(),
        Err(e) => return Value::Error(e),
    };
    if n < 0.0 || k < 0.0 || k > n {
        return Value::Error(ExcelError::Num);
    }
    // 用乘法迭代避免大阶乘溢出
    let (n, k) = (n as u64, k as u64);
    let k = k.min(n - k);
    let mut acc = 1.0f64;
    for i in 0..k {
        acc = acc * (n - i) as f64 / (i + 1) as f64;
    }
    num(acc.round())
}

thread_local! {
    /// 进程内的伪随机状态(LCG)。核不引入随机源依赖,用确定性 LCG 近似 RAND。
    static RNG: Cell<u64> = const { Cell::new(0x2545_F491_4F6C_DD1D) };
}

fn next_rand() -> f64 {
    RNG.with(|s| {
        let mut x = s.get();
        // xorshift64
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        // 取高 53 位映射到 [0,1)
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

fn rand(_ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 0, Some(0)) {
        return e;
    }
    Value::Number(next_rand())
}
fn randbetween(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let lo = match ev.eval_number(&args[0]) {
        Ok(n) => n.ceil() as i64,
        Err(e) => return Value::Error(e),
    };
    let hi = match ev.eval_number(&args[1]) {
        Ok(n) => n.floor() as i64,
        Err(e) => return Value::Error(e),
    };
    if lo > hi {
        return Value::Error(ExcelError::Num);
    }
    let span = (hi - lo + 1) as f64;
    Value::Number(lo as f64 + (next_rand() * span).floor())
}

fn sumif(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(3)) {
        return e;
    }
    let range = ev.flatten_arg(&args[0]);
    let criteria = ev.eval(&args[1]);
    let sum_range = match args.get(2) {
        Some(a) => ev.flatten_arg(a),
        None => range.clone(),
    };
    let mut acc = 0.0;
    for (i, v) in range.iter().enumerate() {
        if matches_criteria(v, &criteria) {
            if let Some(Value::Number(n)) = sum_range.get(i) {
                acc += n;
            }
        }
    }
    Value::Number(acc)
}

fn sumifs(ev: &mut Evaluator, args: &[Node]) -> Value {
    // SUMIFS(sum_range, crit_range1, crit1, [crit_range2, crit2, ...])
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return Value::Error(ExcelError::Value);
    }
    let sum_range = ev.flatten_arg(&args[0]);
    let n = sum_range.len();
    let mut pairs = Vec::new();
    let mut i = 1;
    while i + 1 < args.len() {
        let range = ev.flatten_arg(&args[i]);
        let crit = ev.eval(&args[i + 1]);
        if range.len() != n {
            return Value::Error(ExcelError::Value);
        }
        pairs.push((range, crit));
        i += 2;
    }
    let mut acc = 0.0;
    for idx in 0..n {
        if pairs.iter().all(|(r, c)| matches_criteria(&r[idx], c)) {
            if let Value::Number(x) = sum_range[idx] {
                acc += x;
            }
        }
    }
    Value::Number(acc)
}

/// `SUBTOTAL(function_num, ref1, ...)`:按功能号聚合。1-11 与 101-111 等价对待
/// (本引擎无「隐藏行」概念;视图层的过滤是独立的行映射,不改变公式看到的数据)。
fn subtotal(ev: &mut Evaluator, args: &[Node]) -> Value {
    if args.len() < 2 {
        return Value::Error(ExcelError::Value);
    }
    let fnum = match ev.eval_number(&args[0]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let code = if fnum >= 101 { fnum - 100 } else { fnum };
    let mut nums: Vec<f64> = Vec::new();
    let mut count_num = 0usize;
    let mut count_nonblank = 0usize;
    for arg in &args[1..] {
        for v in ev.flatten_arg(arg) {
            if !matches!(v, Value::Blank) {
                count_nonblank += 1;
            }
            if let Value::Number(n) = v {
                nums.push(n);
                count_num += 1;
            }
        }
    }
    match code {
        1 => {
            if nums.is_empty() {
                Value::Error(ExcelError::Div0)
            } else {
                Value::Number(nums.iter().sum::<f64>() / nums.len() as f64)
            }
        }
        2 => Value::Number(count_num as f64),
        3 => Value::Number(count_nonblank as f64),
        4 => Value::Number(
            nums.iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max)
                .max(0.0),
        ),
        5 => {
            if nums.is_empty() {
                Value::Number(0.0)
            } else {
                Value::Number(nums.iter().cloned().fold(f64::INFINITY, f64::min))
            }
        }
        6 => Value::Number(nums.iter().product()),
        9 => Value::Number(nums.iter().sum()),
        7 | 8 | 10 | 11 => {
            let n = nums.len();
            let ddof = if code == 7 || code == 10 { 1 } else { 0 };
            if n <= ddof {
                return Value::Error(ExcelError::Div0);
            }
            let mean = nums.iter().sum::<f64>() / n as f64;
            let ss: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
            let var = ss / (n - ddof) as f64;
            // 同样走 num() 收口,避免 NaN 逃逸(见 stats::agg_variance)
            if code == 7 || code == 8 {
                num(var.sqrt())
            } else {
                num(var)
            }
        }
        _ => Value::Error(ExcelError::Value),
    }
}

fn sumproduct(ev: &mut Evaluator, args: &[Node]) -> Value {
    if args.is_empty() {
        return Value::Error(ExcelError::Value);
    }
    let arrays: Vec<Vec<Value>> = args.iter().map(|a| ev.flatten_arg(a)).collect();
    let len = arrays[0].len();
    if arrays.iter().any(|a| a.len() != len) {
        return Value::Error(ExcelError::Value); // 维度不一致
    }
    let mut acc = 0.0;
    for i in 0..len {
        let mut prod = 1.0;
        for a in &arrays {
            // 非数值按 0 处理(Excel SUMPRODUCT 语义)
            prod *= match &a[i] {
                Value::Number(n) => *n,
                Value::Bool(true) => 1.0,
                Value::Bool(false) => 0.0,
                _ => 0.0,
            };
        }
        acc += prod;
    }
    Value::Number(acc)
}

#[cfg(test)]
mod tests {
    use crate::formula::eval::Workbook;
    use crate::formula::value::{ExcelError, Value};

    fn ev(f: &str) -> Value {
        Workbook::new().eval_formula(f)
    }

    #[test]
    fn subtotal_by_function_num() {
        let mut wb = Workbook::new();
        for (i, v) in [10, 20, 30].iter().enumerate() {
            wb.set_input(i as u32, 0, &v.to_string()); // A1:A3
        }
        // 9=SUM, 1=AVERAGE, 4=MAX, 5=MIN, 2=COUNT;101 段等价
        assert_eq!(wb.eval_formula("SUBTOTAL(9,A1:A3)"), Value::Number(60.0));
        assert_eq!(wb.eval_formula("SUBTOTAL(1,A1:A3)"), Value::Number(20.0));
        assert_eq!(wb.eval_formula("SUBTOTAL(4,A1:A3)"), Value::Number(30.0));
        assert_eq!(wb.eval_formula("SUBTOTAL(5,A1:A3)"), Value::Number(10.0));
        assert_eq!(wb.eval_formula("SUBTOTAL(2,A1:A3)"), Value::Number(3.0));
        assert_eq!(wb.eval_formula("SUBTOTAL(109,A1:A3)"), Value::Number(60.0));
    }

    #[test]
    fn basic_math() {
        assert_eq!(ev("SUM(1,2,3)"), Value::Number(6.0));
        assert_eq!(ev("PRODUCT(2,3,4)"), Value::Number(24.0));
        assert_eq!(ev("ABS(-5)"), Value::Number(5.0));
        assert_eq!(ev("INT(-1.5)"), Value::Number(-2.0));
        assert_eq!(ev("MOD(7,3)"), Value::Number(1.0));
        assert_eq!(ev("MOD(-7,3)"), Value::Number(2.0));
        assert_eq!(ev("QUOTIENT(7,3)"), Value::Number(2.0));
    }

    #[test]
    fn rounding() {
        assert_eq!(ev("ROUND(2.345,2)"), Value::Number(2.35));
        assert_eq!(ev("ROUND(2.5,0)"), Value::Number(3.0));
        assert_eq!(ev("ROUNDUP(2.1,0)"), Value::Number(3.0));
        assert_eq!(ev("ROUNDDOWN(2.9,0)"), Value::Number(2.0));
        assert_eq!(ev("CEILING(4.2,1)"), Value::Number(5.0));
        assert_eq!(ev("FLOOR(4.9,1)"), Value::Number(4.0));
        assert_eq!(ev("MROUND(10,3)"), Value::Number(9.0));
    }

    #[test]
    fn powers_logs_roots() {
        assert_eq!(ev("POWER(2,10)"), Value::Number(1024.0));
        assert_eq!(ev("SQRT(16)"), Value::Number(4.0));
        assert_eq!(ev("SQRT(-1)"), Value::Error(ExcelError::Num));
        assert_eq!(ev("LOG(1000)"), Value::Number(3.0));
        assert_eq!(ev("LOG(8,2)"), Value::Number(3.0));
        assert_eq!(ev("LN(1)"), Value::Number(0.0));
    }

    #[test]
    fn trig() {
        assert_eq!(ev("SIN(0)"), Value::Number(0.0));
        assert_eq!(ev("COS(0)"), Value::Number(1.0));
        assert_eq!(ev("DEGREES(PI())"), Value::Number(180.0));
    }

    #[test]
    fn number_theory() {
        assert_eq!(ev("GCD(12,18)"), Value::Number(6.0));
        assert_eq!(ev("LCM(4,6)"), Value::Number(12.0));
        assert_eq!(ev("FACT(5)"), Value::Number(120.0));
        assert_eq!(ev("COMBIN(5,2)"), Value::Number(10.0));
        assert_eq!(ev("EVEN(3)"), Value::Number(4.0));
        assert_eq!(ev("ODD(2)"), Value::Number(3.0));
    }

    #[test]
    fn conditional_sums() {
        let mut wb = Workbook::new();
        for (i, v) in [1, 5, 8, 10].iter().enumerate() {
            wb.set_input(i as u32, 0, &v.to_string()); // A1:A4
        }
        assert_eq!(wb.eval_formula("SUMIF(A1:A4,\">4\")"), Value::Number(23.0));
        assert_eq!(
            wb.eval_formula("SUMPRODUCT(A1:A4,A1:A4)"),
            Value::Number(190.0)
        );
    }

    #[test]
    fn rand_in_range() {
        for _ in 0..20 {
            match ev("RAND()") {
                Value::Number(n) => assert!((0.0..1.0).contains(&n)),
                other => panic!("RAND 应返回数值,得到 {other:?}"),
            }
            match ev("RANDBETWEEN(1,6)") {
                Value::Number(n) => assert!((1.0..=6.0).contains(&n) && n.fract() == 0.0),
                other => panic!("RANDBETWEEN 应返回整数,得到 {other:?}"),
            }
        }
    }
}
