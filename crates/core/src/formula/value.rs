//! 公式引擎的**值模型**与**错误类型**,语义对齐 Excel。
//!
//! 设计要点:
//! - 错误是**一等值**([`Value::Error`]),会沿计算链自动传播 —— 与 Excel 一致,
//!   而不是用 `Result` 在每一步中断。这样 `=IFERROR(1/0, 0)` 这类「捕获错误」的
//!   函数才可能实现。
//! - **类型强制**(coercion)集中在这里:算术把 `TRUE→1`、文本数字 `"3"→3`、
//!   空单元格 `→0`;不可强制则得 [`ExcelError::Value`]。把规则收拢在一处,
//!   各函数与运算符复用,避免语义漂移。

use std::fmt;

/// Excel 标准错误值。
///
/// 文本表示(如 `#DIV/0!`)与 Excel 完全一致,直接展示给用户即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcelError {
    /// `#NULL!`:区域交集为空等。
    Null,
    /// `#DIV/0!`:除以零。
    Div0,
    /// `#VALUE!`:参数类型错误 / 无法强制。
    Value,
    /// `#REF!`:非法引用(越界、循环引用)。
    Ref,
    /// `#NAME?`:未知函数名 / 无法解析的名称。
    Name,
    /// `#NUM!`:数值超出可表示范围(如 `SQRT(-1)`)。
    Num,
    /// `#N/A`:值不可用(查找未命中)。
    Na,
}

impl ExcelError {
    /// Excel 里显示的文本(含前导 `#`)。
    pub fn as_str(&self) -> &'static str {
        match self {
            ExcelError::Null => "#NULL!",
            ExcelError::Div0 => "#DIV/0!",
            ExcelError::Value => "#VALUE!",
            ExcelError::Ref => "#REF!",
            ExcelError::Name => "#NAME?",
            ExcelError::Num => "#NUM!",
            ExcelError::Na => "#N/A",
        }
    }
}

impl fmt::Display for ExcelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一个矩形数组值:函数返回多值、或范围 `A1:B2` 展开后的载体。
#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    /// 行数。
    pub rows: usize,
    /// 列数。
    pub cols: usize,
    /// 按行优先存放的元素,长度为 `rows * cols`。
    pub data: Vec<Value>,
}

impl Array {
    /// 构造数组;`data.len()` 必须等于 `rows * cols`。
    pub fn new(rows: usize, cols: usize, data: Vec<Value>) -> Self {
        debug_assert_eq!(rows * cols, data.len(), "数组维度与数据长度不一致");
        Array { rows, cols, data }
    }

    /// 取 `(r, c)` 处的元素;越界返回 `#REF!`。
    pub fn get(&self, r: usize, c: usize) -> Value {
        if r >= self.rows || c >= self.cols {
            return Value::Error(ExcelError::Ref);
        }
        self.data[r * self.cols + c].clone()
    }
}

/// 单元格 / 表达式的求值结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 空单元格。算术里视作 `0`,文本里视作 `""`,但**不同于**空文本 `""`。
    Blank,
    /// 数值(日期/时间以 Excel 序列数表示)。
    Number(f64),
    /// 文本。
    Text(String),
    /// 逻辑值。
    Bool(bool),
    /// 错误值。
    Error(ExcelError),
    /// 矩形数组。
    Array(Array),
}

impl Value {
    /// 便捷构造:从错误。
    pub fn err(e: ExcelError) -> Value {
        Value::Error(e)
    }

    /// 若自身是错误值则返回 `Some(e)`,否则 `None`。用于错误传播。
    pub fn as_error(&self) -> Option<ExcelError> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }

    /// 是否是空白(空单元格)。
    pub fn is_blank(&self) -> bool {
        matches!(self, Value::Blank)
    }

    /// **强制为数值**(Excel 算术上下文规则):
    /// - 空单元格 → `0`
    /// - 数值 → 原值
    /// - 布尔 → `TRUE=1` / `FALSE=0`
    /// - 文本 → 解析为数字(去空白;支持末尾 `%`),失败得 `#VALUE!`
    /// - 错误 → 传播
    /// - 数组 → 取左上角元素再强制(标量上下文的隐式取值)
    pub fn to_number(&self) -> Result<f64, ExcelError> {
        match self {
            Value::Blank => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => parse_number(s).ok_or(ExcelError::Value),
            Value::Error(e) => Err(*e),
            Value::Array(a) => a.get(0, 0).to_number(),
        }
    }

    /// **强制为文本**(Excel 文本上下文规则):
    /// - 空单元格 → `""`
    /// - 数值 → General 格式(见 [`format_number`])
    /// - 布尔 → `TRUE` / `FALSE`
    /// - 文本 → 原值
    /// - 错误 → 传播
    pub fn to_text(&self) -> Result<String, ExcelError> {
        match self {
            Value::Blank => Ok(String::new()),
            Value::Number(n) => Ok(format_number(*n)),
            Value::Bool(b) => Ok(if *b { "TRUE".into() } else { "FALSE".into() }),
            Value::Text(s) => Ok(s.clone()),
            Value::Error(e) => Err(*e),
            Value::Array(a) => a.get(0, 0).to_text(),
        }
    }

    /// **强制为逻辑值**:
    /// - 布尔 → 原值
    /// - 数值 → `n != 0`
    /// - 空单元格 → `false`
    /// - 文本 → `"TRUE"`/`"FALSE"`(忽略大小写),否则 `#VALUE!`
    /// - 错误 → 传播
    pub fn to_bool(&self) -> Result<bool, ExcelError> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Blank => Ok(false),
            Value::Text(s) => match s.trim().to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" => Ok(false),
                _ => Err(ExcelError::Value),
            },
            Value::Error(e) => Err(*e),
            Value::Array(a) => a.get(0, 0).to_bool(),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}
impl From<ExcelError> for Value {
    fn from(e: ExcelError) -> Self {
        Value::Error(e)
    }
}
/// 让 `Result<f64, ExcelError>` 之类能直接落成 `Value`(Ok→值,Err→错误)。
impl From<Result<f64, ExcelError>> for Value {
    fn from(r: Result<f64, ExcelError>) -> Self {
        match r {
            Ok(n) => Value::Number(n),
            Err(e) => Value::Error(e),
        }
    }
}

/// 把文本解析成数值(用于强制)。
///
/// 规则贴近 Excel:去除首尾空白;支持末尾百分号 `"50%" → 0.5`;
/// 支持科学计数 `"1e3"`;空串或非数字返回 `None`(交由调用方转成 `#VALUE!`)。
fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(pct) = t.strip_suffix('%') {
        return pct.trim().parse::<f64>().ok().map(|n| n / 100.0);
    }
    t.parse::<f64>().ok()
}

/// 把 f64 按 Excel「常规(General)」格式渲染成字符串。
///
/// Excel 的常规格式:整数不带小数点;小数去掉末尾多余的 0;为消除
/// 浮点噪声(如 `0.1+0.2=0.30000000000000004`),先四舍五入到 **15 位有效数字**
/// 再用最短往返表示输出。极大/极小值退化到科学计数(交给标准库默认)。
pub fn format_number(n: f64) -> String {
    if n == 0.0 {
        // 处理 -0.0
        return "0".to_string();
    }
    if n.is_nan() {
        return ExcelError::Num.to_string();
    }
    if n.is_infinite() {
        return ExcelError::Num.to_string();
    }
    let rounded = round_to_significant(n, 15);
    // `{}` 给出能往返的最短十进制表示,自动去掉末尾 0。
    format!("{rounded}")
}

/// 四舍五入到 `digits` 位有效数字。
fn round_to_significant(n: f64, digits: i32) -> f64 {
    if n == 0.0 || !n.is_finite() {
        return n;
    }
    let d = n.abs().log10().floor() as i32 + 1; // 整数部分位数
    let power = digits - d;
    // 用 10 的幂缩放后四舍五入;power 过大/过小时退化为原值,避免溢出。
    if !(-300..=300).contains(&power) {
        return n;
    }
    let factor = 10f64.powi(power);
    (n * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_matches_excel() {
        assert_eq!(ExcelError::Div0.to_string(), "#DIV/0!");
        assert_eq!(ExcelError::Na.to_string(), "#N/A");
        assert_eq!(ExcelError::Name.to_string(), "#NAME?");
    }

    #[test]
    fn to_number_coerces_like_excel() {
        assert_eq!(Value::Blank.to_number(), Ok(0.0));
        assert_eq!(Value::Bool(true).to_number(), Ok(1.0));
        assert_eq!(Value::Bool(false).to_number(), Ok(0.0));
        assert_eq!(Value::Text(" 3 ".into()).to_number(), Ok(3.0));
        assert_eq!(Value::Text("50%".into()).to_number(), Ok(0.5));
        assert_eq!(Value::Text("1e3".into()).to_number(), Ok(1000.0));
        assert_eq!(
            Value::Text("abc".into()).to_number(),
            Err(ExcelError::Value)
        );
        assert_eq!(Value::Text("".into()).to_number(), Err(ExcelError::Value));
    }

    #[test]
    fn errors_propagate_through_coercion() {
        assert_eq!(
            Value::Error(ExcelError::Div0).to_number(),
            Err(ExcelError::Div0)
        );
        assert_eq!(Value::Error(ExcelError::Na).to_text(), Err(ExcelError::Na));
    }

    #[test]
    fn to_text_uses_general_format() {
        assert_eq!(Value::Number(3.0).to_text().unwrap(), "3");
        assert_eq!(Value::Number(3.5).to_text().unwrap(), "3.5");
        assert_eq!(Value::Bool(true).to_text().unwrap(), "TRUE");
        assert_eq!(Value::Blank.to_text().unwrap(), "");
    }

    #[test]
    fn to_bool_coerces() {
        assert_eq!(Value::Number(5.0).to_bool(), Ok(true));
        assert_eq!(Value::Number(0.0).to_bool(), Ok(false));
        assert_eq!(Value::Text("true".into()).to_bool(), Ok(true));
        assert_eq!(Value::Text("nope".into()).to_bool(), Err(ExcelError::Value));
    }

    #[test]
    fn general_format_removes_float_noise() {
        // 0.1 + 0.2 的浮点噪声应被 15 位有效数字四舍五入吸收
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_number(2.0), "2");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(1234567.0), "1234567");
    }

    #[test]
    fn array_get_out_of_bounds_is_ref_error() {
        let a = Array::new(1, 2, vec![Value::Number(1.0), Value::Number(2.0)]);
        assert_eq!(a.get(0, 0), Value::Number(1.0));
        assert_eq!(a.get(5, 5), Value::Error(ExcelError::Ref));
    }
}
