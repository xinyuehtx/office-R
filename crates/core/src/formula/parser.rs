//! 语法分析:[`Token`] 序列 → [`Node`] AST。
//!
//! 采用 **Pratt(优先级爬升)解析器**,用绑定力(binding power)编码 Excel 的
//! 运算符优先级。相比手写一串递归下降函数,Pratt 用一张「优先级表」表达全部
//! 中缀/前缀/后缀规则,增删运算符只改表,不改控制流。
//!
//! Excel 优先级(高 → 低):
//! `:`(范围) > 一元 `-`/`+` > `%`(后缀) > `^` > `* /` > `+ -` > `&` > 比较。

use super::ast::{BinOp, Node, UnOp};
use super::reference::{CellRef, RangeRef};
use super::token::{tokenize, Token};
use super::value::ExcelError;

/// 把公式主体(不含前导 `=`)解析成 AST。
pub fn parse(input: &str) -> Result<Node, ExcelError> {
    let tokens = tokenize(input)?;
    let mut p = Parser { tokens, pos: 0 };
    let node = p.parse_expr(0)?;
    if p.pos != p.tokens.len() {
        // 还有没消费完的记号 → 语法错误
        return Err(ExcelError::Name);
    }
    Ok(node)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, tok: &Token) -> Result<(), ExcelError> {
        if self.peek() == Some(tok) {
            self.pos += 1;
            Ok(())
        } else {
            Err(ExcelError::Name)
        }
    }

    /// 优先级爬升主循环。`min_bp` 是当前允许的最小左绑定力。
    fn parse_expr(&mut self, min_bp: u8) -> Result<Node, ExcelError> {
        let mut lhs = self.parse_prefix()?;

        // 克隆当前记号以脱离对 self 的借用,便于循环体内推进 self.pos。
        while let Some(tok) = self.peek().cloned() {
            // 后缀 %(高优先级)
            if tok == Token::Percent {
                if PERCENT_BP < min_bp {
                    break;
                }
                self.pos += 1;
                lhs = Node::Unary(UnOp::Percent, Box::new(lhs));
                continue;
            }

            let Some((l_bp, r_bp, op)) = infix_binding_power(&tok) else {
                break;
            };
            if l_bp < min_bp {
                break;
            }
            self.pos += 1;
            let rhs = self.parse_expr(r_bp)?;
            lhs = combine(op, lhs, rhs)?;
        }
        Ok(lhs)
    }

    /// 前缀位置:字面量、引用、括号、一元运算、函数调用。
    fn parse_prefix(&mut self) -> Result<Node, ExcelError> {
        let tok = self.next().ok_or(ExcelError::Name)?;
        match tok {
            Token::Num(n) => Ok(Node::Number(n)),
            Token::Str(s) => Ok(Node::Text(s)),
            Token::Err(e) => Ok(Node::Error(e)),
            Token::Minus => {
                let operand = self.parse_expr(UNARY_BP)?;
                Ok(Node::Unary(UnOp::Neg, Box::new(operand)))
            }
            Token::Plus => {
                let operand = self.parse_expr(UNARY_BP)?;
                Ok(Node::Unary(UnOp::Plus, Box::new(operand)))
            }
            Token::LParen => {
                let inner = self.parse_expr(0)?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Token::Ident(name) => self.classify_ident(name),
            _ => Err(ExcelError::Name),
        }
    }

    /// 判定名称记号的身份:函数 / 布尔常量 / 单元格引用。
    fn classify_ident(&mut self, name: String) -> Result<Node, ExcelError> {
        // 后跟 `(` → 函数调用
        if self.peek() == Some(&Token::LParen) {
            self.pos += 1; // 吃掉 `(`
            let args = self.parse_args()?;
            self.expect(&Token::RParen)?;
            return Ok(Node::Func(name.to_ascii_uppercase(), args));
        }
        // 布尔常量
        match name.to_ascii_uppercase().as_str() {
            "TRUE" => return Ok(Node::Bool(true)),
            "FALSE" => return Ok(Node::Bool(false)),
            _ => {}
        }
        // 单元格引用
        if let Some(cell) = CellRef::parse(&name) {
            return Ok(Node::Ref(cell));
        }
        // 既不是函数也不是引用/常量 → 未知名称
        Err(ExcelError::Name)
    }

    /// 解析逗号分隔的实参列表(不含外层括号)。
    fn parse_args(&mut self) -> Result<Vec<Node>, ExcelError> {
        let mut args = Vec::new();
        if self.peek() == Some(&Token::RParen) {
            return Ok(args); // 零参数
        }
        loop {
            args.push(self.parse_expr(0)?);
            match self.peek() {
                Some(&Token::Comma) => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        Ok(args)
    }
}

/// 一元前缀绑定力:高于 `^`(50)与 `* /`(40),故 `-2^2 = 4`。
const UNARY_BP: u8 = 60;
/// 后缀 `%` 绑定力:高于一元,故 `2*3% = 0.06`。
const PERCENT_BP: u8 = 70;

/// 返回中缀运算符的 `(左绑定力, 右绑定力, 运算)`。右结合运算的右绑定力等于左绑定力。
///
/// `:`(范围)不返回 [`BinOp`],用一个哨兵值单独处理,故这里用 `Option`。
fn infix_binding_power(tok: &Token) -> Option<(u8, u8, InfixOp)> {
    let r = match tok {
        Token::Eq => (10, 11, InfixOp::Bin(BinOp::Eq)),
        Token::Ne => (10, 11, InfixOp::Bin(BinOp::Ne)),
        Token::Lt => (10, 11, InfixOp::Bin(BinOp::Lt)),
        Token::Gt => (10, 11, InfixOp::Bin(BinOp::Gt)),
        Token::Le => (10, 11, InfixOp::Bin(BinOp::Le)),
        Token::Ge => (10, 11, InfixOp::Bin(BinOp::Ge)),
        Token::Amp => (20, 21, InfixOp::Bin(BinOp::Concat)),
        Token::Plus => (30, 31, InfixOp::Bin(BinOp::Add)),
        Token::Minus => (30, 31, InfixOp::Bin(BinOp::Sub)),
        Token::Star => (40, 41, InfixOp::Bin(BinOp::Mul)),
        Token::Slash => (40, 41, InfixOp::Bin(BinOp::Div)),
        Token::Caret => (50, 50, InfixOp::Bin(BinOp::Pow)), // 右结合
        Token::Colon => (90, 91, InfixOp::Range),
        _ => return None,
    };
    Some(r)
}

#[derive(Clone, Copy)]
enum InfixOp {
    Bin(BinOp),
    Range,
}

/// 用中缀运算把左右子树合成一个节点。
fn combine(op: InfixOp, lhs: Node, rhs: Node) -> Result<Node, ExcelError> {
    match op {
        InfixOp::Bin(b) => Ok(Node::Binary(b, Box::new(lhs), Box::new(rhs))),
        InfixOp::Range => match (lhs, rhs) {
            // `A1:B2`:两端都必须是单元格引用,合成范围。
            (Node::Ref(a), Node::Ref(b)) => Ok(Node::Range(RangeRef::from_corners(a, b))),
            _ => Err(ExcelError::Ref),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> Node {
        Node::Number(n)
    }
    fn bin(op: BinOp, l: Node, r: Node) -> Node {
        Node::Binary(op, Box::new(l), Box::new(r))
    }

    #[test]
    fn precedence_mul_over_add() {
        // 1+2*3 → 1 + (2*3)
        assert_eq!(
            parse("1+2*3").unwrap(),
            bin(BinOp::Add, num(1.0), bin(BinOp::Mul, num(2.0), num(3.0)))
        );
    }

    #[test]
    fn power_is_right_associative() {
        // 2^3^2 → 2 ^ (3 ^ 2)
        assert_eq!(
            parse("2^3^2").unwrap(),
            bin(BinOp::Pow, num(2.0), bin(BinOp::Pow, num(3.0), num(2.0)))
        );
    }

    #[test]
    fn unary_minus_binds_tighter_than_power() {
        // -2^2 → (-2) ^ 2,与 Excel 一致
        assert_eq!(
            parse("-2^2").unwrap(),
            bin(
                BinOp::Pow,
                Node::Unary(UnOp::Neg, Box::new(num(2.0))),
                num(2.0)
            )
        );
    }

    #[test]
    fn percent_is_high_precedence_postfix() {
        // 2*3% → 2 * (3%)
        assert_eq!(
            parse("2*3%").unwrap(),
            bin(
                BinOp::Mul,
                num(2.0),
                Node::Unary(UnOp::Percent, Box::new(num(3.0)))
            )
        );
    }

    #[test]
    fn range_and_function() {
        let ast = parse("SUM(A1:A3)").unwrap();
        match ast {
            Node::Func(name, args) => {
                assert_eq!(name, "SUM");
                assert!(matches!(args[0], Node::Range(_)));
            }
            _ => panic!("应解析为函数调用"),
        }
    }

    #[test]
    fn bool_and_ref_classification() {
        assert_eq!(parse("TRUE").unwrap(), Node::Bool(true));
        assert!(matches!(parse("B2").unwrap(), Node::Ref(_)));
    }

    #[test]
    fn parens_override_precedence() {
        assert_eq!(
            parse("(1+2)*3").unwrap(),
            bin(BinOp::Mul, bin(BinOp::Add, num(1.0), num(2.0)), num(3.0))
        );
    }

    #[test]
    fn trailing_garbage_is_error() {
        assert_eq!(parse("1+2)").unwrap_err(), ExcelError::Name);
        assert_eq!(parse("1 2").unwrap_err(), ExcelError::Name);
    }

    #[test]
    fn nested_function_calls() {
        let ast = parse("IF(A1>0,SUM(B1:B2),0)").unwrap();
        match ast {
            Node::Func(name, args) => {
                assert_eq!(name, "IF");
                assert_eq!(args.len(), 3);
            }
            _ => panic!("应为 IF 调用"),
        }
    }
}
