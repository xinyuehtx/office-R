//! 日期时间函数。
//!
//! 采用 **Excel 序列数**:整数部分是「1899-12-30 以来的天数」,小数部分是当天时间比例。
//! Excel 有著名的「1900 闰年 bug」(把 1900 年当作闰年,存在虚构的 1900-02-29 = 序列 60),
//! 这里为**兼容 Excel** 一并复刻:1900-03-01 及以后的序列数比真实天数多 1。
//!
//! `TODAY`/`NOW` 依赖「当前时间」,由外部注入([`Evaluator::now_serial`]),core 不读系统时钟。

use std::collections::HashMap;

use super::util::{arity, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("DATE", date);
    m.insert("TIME", time);
    m.insert("TODAY", today);
    m.insert("NOW", now);
    m.insert("YEAR", year);
    m.insert("MONTH", month);
    m.insert("DAY", day);
    m.insert("HOUR", hour);
    m.insert("MINUTE", minute);
    m.insert("SECOND", second);
    m.insert("WEEKDAY", weekday);
    m.insert("EDATE", edate);
    m.insert("EOMONTH", eomonth);
    m.insert("DATEVALUE", datevalue);
    m.insert("DAYS", days);
}

// ---- 历法内核(Howard Hinnant 的 days_from_civil 算法)----

/// 公历 (y, m, d) → 距 1970-01-01 的天数(可为负)。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 距 1970-01-01 的天数 → 公历 (y, m, d)。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 1899-12-31 距 1970 的天数,作为序列数换算基准。
fn epoch_offset() -> i64 {
    days_from_civil(1899, 12, 31)
}

/// 公历 → Excel 序列数(含 1900 闰年 bug 复刻)。
fn ymd_to_serial(y: i64, m: i64, d: i64) -> i64 {
    let naive = days_from_civil(y, m, d) - epoch_offset();
    // naive>=60 对应 1900-03-01 及以后,补上虚构的 1900-02-29
    if naive >= 60 {
        naive + 1
    } else {
        naive
    }
}

/// Excel 序列数(整数天)→ 公历。
fn serial_to_ymd(serial: i64) -> (i64, i64, i64) {
    if serial == 60 {
        return (1900, 2, 29); // 虚构的闰日
    }
    let naive = if serial >= 61 { serial - 1 } else { serial };
    civil_from_days(naive + epoch_offset())
}

/// 求值一个序列数参数(取整数天部分)。
fn serial_arg(ev: &mut Evaluator, node: &Node) -> Result<i64, ExcelError> {
    Ok(ev.eval_number(node)?.floor() as i64)
}

fn date(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 3, Some(3)) {
        return e;
    }
    let mut y = match ev.eval_number(&args[0]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let m = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let d = match ev.eval_number(&args[2]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    // Excel:0..1899 的年份加上 1900
    if (0..1900).contains(&y) {
        y += 1900;
    }
    // 月份溢出滚动到年;日溢出用「先算当月 1 号再加天数」处理
    let y = y + (m - 1).div_euclid(12);
    let m = (m - 1).rem_euclid(12) + 1;
    let serial = ymd_to_serial(y, m, 1) + (d - 1);
    if serial < 0 {
        return Value::Error(ExcelError::Num);
    }
    Value::Number(serial as f64)
}

fn time(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 3, Some(3)) {
        return e;
    }
    let h = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let mi = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let s = match ev.eval_number(&args[2]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let frac = (h * 3600.0 + mi * 60.0 + s) / 86400.0;
    Value::Number(frac.rem_euclid(1.0))
}

fn today(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 0, Some(0)) {
        return e;
    }
    Value::Number(ev.now_serial().floor())
}
fn now(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 0, Some(0)) {
        return e;
    }
    Value::Number(ev.now_serial())
}

fn date_part(ev: &mut Evaluator, args: &[Node], pick: fn((i64, i64, i64)) -> i64) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let serial = match serial_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    if serial < 0 {
        return Value::Error(ExcelError::Num);
    }
    Value::Number(pick(serial_to_ymd(serial)) as f64)
}

fn year(ev: &mut Evaluator, args: &[Node]) -> Value {
    date_part(ev, args, |(y, _, _)| y)
}
fn month(ev: &mut Evaluator, args: &[Node]) -> Value {
    date_part(ev, args, |(_, m, _)| m)
}
fn day(ev: &mut Evaluator, args: &[Node]) -> Value {
    date_part(ev, args, |(_, _, d)| d)
}

fn time_part(ev: &mut Evaluator, args: &[Node], unit: i64) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let serial = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let frac = serial - serial.floor();
    // 四舍五入到秒,避免浮点误差把 12:00:00 算成 11:59:59
    let total_secs = (frac * 86400.0).round() as i64;
    let v = match unit {
        3600 => (total_secs / 3600) % 24,
        60 => (total_secs / 60) % 60,
        _ => total_secs % 60,
    };
    Value::Number(v as f64)
}
fn hour(ev: &mut Evaluator, args: &[Node]) -> Value {
    time_part(ev, args, 3600)
}
fn minute(ev: &mut Evaluator, args: &[Node]) -> Value {
    time_part(ev, args, 60)
}
fn second(ev: &mut Evaluator, args: &[Node]) -> Value {
    time_part(ev, args, 1)
}

fn weekday(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(2)) {
        return e;
    }
    let serial = match serial_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let kind = match args.get(1) {
        Some(a) => match ev.eval_number(a) {
            Ok(n) => n.trunc() as i64,
            Err(e) => return Value::Error(e),
        },
        None => 1,
    };
    // 序列 1(1900-01-01)在 Excel 里是星期日。sun0:0=周日,1=周一,…,6=周六。
    let sun0 = ((serial - 1) % 7 + 7) % 7;
    let result = match kind {
        1 => sun0 + 1,             // 1=周日..7=周六
        2 => ((sun0 + 6) % 7) + 1, // 1=周一..7=周日
        3 => (sun0 + 6) % 7,       // 0=周一..6=周日
        _ => return Value::Error(ExcelError::Num),
    };
    Value::Number(result as f64)
}

fn edate(ev: &mut Evaluator, args: &[Node]) -> Value {
    add_months(ev, args, false)
}
fn eomonth(ev: &mut Evaluator, args: &[Node]) -> Value {
    add_months(ev, args, true)
}

fn add_months(ev: &mut Evaluator, args: &[Node], end_of_month: bool) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let serial = match serial_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let months = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Value::Error(e),
    };
    let (y, m, d) = serial_to_ymd(serial.max(0));
    let total = (y * 12 + (m - 1)) + months;
    let ny = total.div_euclid(12);
    let nm = total.rem_euclid(12) + 1;
    let last = last_day_of_month(ny, nm);
    let nd = if end_of_month { last } else { d.min(last) };
    Value::Number(ymd_to_serial(ny, nm, nd) as f64)
}

fn last_day_of_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn datevalue(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 1, Some(1)) {
        return e;
    }
    let s = match ev.eval_text(&args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    // 支持 YYYY-MM-DD 与 YYYY/MM/DD
    let parts: Vec<&str> = s.trim().split(['-', '/']).collect();
    if parts.len() != 3 {
        return Value::Error(ExcelError::Value);
    }
    let nums: Result<Vec<i64>, _> = parts.iter().map(|p| p.trim().parse::<i64>()).collect();
    match nums {
        Ok(v) if v[1] >= 1 && v[1] <= 12 && v[2] >= 1 && v[2] <= 31 => {
            Value::Number(ymd_to_serial(v[0], v[1], v[2]) as f64)
        }
        _ => Value::Error(ExcelError::Value),
    }
}

fn days(ev: &mut Evaluator, args: &[Node]) -> Value {
    if let Err(e) = arity(args, 2, Some(2)) {
        return e;
    }
    let end = match serial_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    let start = match serial_arg(ev, &args[1]) {
        Ok(s) => s,
        Err(e) => return Value::Error(e),
    };
    Value::Number((end - start) as f64)
}

#[cfg(test)]
mod tests {
    use crate::formula::eval::Workbook;
    use crate::formula::value::Value;

    fn ev(f: &str) -> Value {
        Workbook::new().eval_formula(f)
    }

    #[test]
    fn date_serial_matches_excel() {
        assert_eq!(ev("DATE(2020,1,1)"), Value::Number(43831.0));
        assert_eq!(ev("DATE(1900,1,1)"), Value::Number(1.0));
        assert_eq!(ev("DATE(1900,3,1)"), Value::Number(61.0)); // 含 1900 闰年 bug
        assert_eq!(ev("DATE(2020,13,1)"), Value::Number(44197.0)); // 月溢出 → 2021-01-01
    }

    #[test]
    fn extract_parts() {
        assert_eq!(ev("YEAR(43831)"), Value::Number(2020.0));
        assert_eq!(ev("MONTH(43831)"), Value::Number(1.0));
        assert_eq!(ev("DAY(43831)"), Value::Number(1.0));
        assert_eq!(ev("YEAR(DATE(1999,12,31))"), Value::Number(1999.0));
    }

    #[test]
    fn time_and_parts() {
        assert_eq!(ev("TIME(12,0,0)"), Value::Number(0.5));
        assert_eq!(ev("HOUR(0.5)"), Value::Number(12.0));
        assert_eq!(ev("MINUTE(TIME(1,30,0))"), Value::Number(30.0));
    }

    #[test]
    fn weekday_and_arithmetic() {
        // 2020-01-01 是星期三 → 类型1 得 4
        assert_eq!(ev("WEEKDAY(DATE(2020,1,1))"), Value::Number(4.0));
        assert_eq!(ev("WEEKDAY(DATE(2020,1,1),2)"), Value::Number(3.0));
        assert_eq!(
            ev("DAYS(DATE(2020,1,31),DATE(2020,1,1))"),
            Value::Number(30.0)
        );
        assert_eq!(ev("EDATE(DATE(2020,1,31),1)"), Value::Number(43890.0)); // 2020-02-29
        assert_eq!(ev("EOMONTH(DATE(2020,2,10),0)"), Value::Number(43890.0)); // 2020-02-29
    }

    #[test]
    fn datevalue_parsing() {
        assert_eq!(ev("DATEVALUE(\"2020-01-01\")"), Value::Number(43831.0));
        assert_eq!(ev("DATEVALUE(\"2020/1/1\")"), Value::Number(43831.0));
    }

    #[test]
    fn today_uses_injected_clock() {
        let mut wb = Workbook::new();
        wb.set_now(43831.75);
        assert_eq!(wb.eval_formula("TODAY()"), Value::Number(43831.0));
        assert_eq!(wb.eval_formula("NOW()"), Value::Number(43831.75));
    }
}
