//! 公式的抽象语法树(AST)。
//!
//! 函数库拿到的是**未求值的 [`Node`]** 而非已算好的值,这样:
//! - `IF`/`IFERROR`/`AND`/`OR` 可以**短路**,只求值需要的分支;
//! - `SUM`/`COUNT` 等可以**按需遍历范围**,不必先把百万单元格物化成数组。

use super::reference::{CellRef, RangeRef};
use super::value::ExcelError;

/// 二元运算符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `^`
    Pow,
    /// `&` 文本连接
    Concat,
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
}

/// 一元运算符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// 前缀负号 `-x`
    Neg,
    /// 前缀正号 `+x`(恒等,但会做数值强制)
    Plus,
    /// 后缀百分号 `x%`(等价 `x/100`)
    Percent,
}

/// AST 节点。
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// 数字字面量。
    Number(f64),
    /// 文本字面量。
    Text(String),
    /// 布尔字面量(`TRUE`/`FALSE`)。
    Bool(bool),
    /// 错误字面量(`#N/A` 等)。
    Error(ExcelError),
    /// 单元格引用。
    Ref(CellRef),
    /// 范围引用。
    Range(RangeRef),
    /// 一元运算。
    Unary(UnOp, Box<Node>),
    /// 二元运算。
    Binary(BinOp, Box<Node>, Box<Node>),
    /// 函数调用(名称已转大写)。
    Func(String, Vec<Node>),
}
