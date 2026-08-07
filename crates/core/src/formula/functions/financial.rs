//! 财务函数(货币时间价值族)。
//!
//! 统一的现金流模型:`pv·(1+r)ⁿ + pmt·(1+r·type)·((1+r)ⁿ−1)/r + fv = 0`。
//! `type` 为 0(期末付,默认)或 1(期初付)。

use std::collections::HashMap;

use super::util::{arity, numbers_for_agg, FuncImpl};
use crate::formula::ast::Node;
use crate::formula::eval::Evaluator;
use crate::formula::value::{ExcelError, Value};

pub fn register(m: &mut HashMap<&'static str, FuncImpl>) {
    m.insert("PMT", pmt);
    m.insert("PV", pv);
    m.insert("FV", fv);
    m.insert("NPER", nper);
    m.insert("NPV", npv);
}

/// 读取第 `i` 个参数为数值;不存在则用 `default`。
fn opt(ev: &mut Evaluator, args: &[Node], i: usize, default: f64) -> Result<f64, ExcelError> {
    match args.get(i) {
        Some(a) => ev.eval_number(a),
        None => Ok(default),
    }
}

fn req(ev: &mut Evaluator, args: &[Node], i: usize) -> Result<f64, ExcelError> {
    ev.eval_number(&args[i])
}

fn fv(ev: &mut Evaluator, args: &[Node]) -> Value {
    // FV(rate, nper, pmt, [pv], [type])
    if let Err(e) = arity(args, 3, Some(5)) {
        return e;
    }
    let r = req(ev, args, 0);
    let n = req(ev, args, 1);
    let pmt = req(ev, args, 2);
    let pv = opt(ev, args, 3, 0.0);
    let ty = opt(ev, args, 4, 0.0);
    match (r, n, pmt, pv, ty) {
        (Ok(r), Ok(n), Ok(pmt), Ok(pv), Ok(ty)) => {
            let result = if r == 0.0 {
                -(pv + pmt * n)
            } else {
                let pow = (1.0 + r).powf(n);
                -(pv * pow + pmt * (1.0 + r * ty) * (pow - 1.0) / r)
            };
            num(result)
        }
        _ => first_err(&[r, n, pmt, pv, ty]),
    }
}

fn pv(ev: &mut Evaluator, args: &[Node]) -> Value {
    // PV(rate, nper, pmt, [fv], [type])
    if let Err(e) = arity(args, 3, Some(5)) {
        return e;
    }
    let r = req(ev, args, 0);
    let n = req(ev, args, 1);
    let pmt = req(ev, args, 2);
    let fv = opt(ev, args, 3, 0.0);
    let ty = opt(ev, args, 4, 0.0);
    match (r, n, pmt, fv, ty) {
        (Ok(r), Ok(n), Ok(pmt), Ok(fv), Ok(ty)) => {
            let result = if r == 0.0 {
                -(fv + pmt * n)
            } else {
                let pow = (1.0 + r).powf(n);
                -(fv + pmt * (1.0 + r * ty) * (pow - 1.0) / r) / pow
            };
            num(result)
        }
        _ => first_err(&[r, n, pmt, fv, ty]),
    }
}

fn pmt(ev: &mut Evaluator, args: &[Node]) -> Value {
    // PMT(rate, nper, pv, [fv], [type])
    if let Err(e) = arity(args, 3, Some(5)) {
        return e;
    }
    let r = req(ev, args, 0);
    let n = req(ev, args, 1);
    let pv = req(ev, args, 2);
    let fv = opt(ev, args, 3, 0.0);
    let ty = opt(ev, args, 4, 0.0);
    match (r, n, pv, fv, ty) {
        (Ok(r), Ok(n), Ok(pv), Ok(fv), Ok(ty)) => {
            if n == 0.0 {
                return Value::Error(ExcelError::Num);
            }
            let result = if r == 0.0 {
                -(pv + fv) / n
            } else {
                let pow = (1.0 + r).powf(n);
                -(pv * pow + fv) * r / ((1.0 + r * ty) * (pow - 1.0))
            };
            num(result)
        }
        _ => first_err(&[r, n, pv, fv, ty]),
    }
}

fn nper(ev: &mut Evaluator, args: &[Node]) -> Value {
    // NPER(rate, pmt, pv, [fv], [type])
    if let Err(e) = arity(args, 3, Some(5)) {
        return e;
    }
    let r = req(ev, args, 0);
    let pmt = req(ev, args, 1);
    let pv = req(ev, args, 2);
    let fv = opt(ev, args, 3, 0.0);
    let ty = opt(ev, args, 4, 0.0);
    match (r, pmt, pv, fv, ty) {
        (Ok(r), Ok(pmt), Ok(pv), Ok(fv), Ok(ty)) => {
            let result = if r == 0.0 {
                if pmt == 0.0 {
                    return Value::Error(ExcelError::Num);
                }
                -(pv + fv) / pmt
            } else {
                let adj = pmt * (1.0 + r * ty);
                let numer = adj - fv * r;
                let denom = adj + pv * r;
                if numer <= 0.0 || denom <= 0.0 {
                    return Value::Error(ExcelError::Num);
                }
                (numer / denom).ln() / (1.0 + r).ln()
            };
            num(result)
        }
        _ => first_err(&[r, pmt, pv, fv, ty]),
    }
}

fn npv(ev: &mut Evaluator, args: &[Node]) -> Value {
    // NPV(rate, value1, [value2], ...)
    if let Err(e) = arity(args, 2, None) {
        return e;
    }
    let r = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    if r == -1.0 {
        return Value::Error(ExcelError::Div0);
    }
    let flows = match numbers_for_agg(ev, &args[1..]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let mut acc = 0.0;
    for (i, cf) in flows.iter().enumerate() {
        acc += cf / (1.0 + r).powi(i as i32 + 1);
    }
    num(acc)
}

fn first_err(rs: &[Result<f64, ExcelError>]) -> Value {
    for r in rs {
        if let Err(e) = r {
            return Value::Error(*e);
        }
    }
    Value::Error(ExcelError::Value)
}

fn num(n: f64) -> Value {
    if n.is_finite() {
        Value::Number(n)
    } else {
        Value::Error(ExcelError::Num)
    }
}

#[cfg(test)]
mod tests {
    use crate::formula::eval::Workbook;
    use crate::formula::value::Value;

    fn ev(f: &str) -> f64 {
        match Workbook::new().eval_formula(f) {
            Value::Number(n) => n,
            other => panic!("期望数值,得到 {other:?}"),
        }
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "期望 {b},得到 {a}");
    }

    #[test]
    fn pmt_matches_excel() {
        // 年利率 6%/12,贷款 20 期,现值 1000 → 每期还款约 -52.67(与 Excel 一致)
        approx(ev("PMT(0.06/12,20,1000)"), -52.66645204782392);
    }

    #[test]
    fn fv_and_pv() {
        // 每期存 -100,年 5% 月息,10 期
        approx(ev("FV(0.05/12,10,-100,0)"), 1018.9598600524725);
        approx(ev("PV(0.05/12,10,-100,0)"), 977.4601653327106);
    }

    #[test]
    fn npv_discounts_flows() {
        approx(ev("NPV(0.1,100,100,100)"), 248.685199);
    }

    #[test]
    fn nper_counts_periods() {
        approx(ev("NPER(0,-100,0,1000)"), 10.0);
    }

    #[test]
    fn zero_rate_pmt() {
        approx(ev("PMT(0,10,1000)"), -100.0);
    }
}
