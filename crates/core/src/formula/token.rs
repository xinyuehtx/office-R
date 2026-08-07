//! 词法分析:把公式文本切成 [`Token`] 序列。
//!
//! 关键设计:**引用 / 函数名 / 布尔常量在词法阶段不区分**,统一切成
//! [`Token::Ident`](name-ish 记号),留给语法阶段按上下文判定 ——
//! 因为 `A1`(引用)、`SUM`(函数,后面跟 `(`)、`TRUE`(常量)在字符层面
//! 长得一样,只有结合上下文才能确定身份。这样词法器无需回溯,简单可靠。

use super::value::ExcelError;

/// 词法记号。
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// 数字字面量。
    Num(f64),
    /// 字符串字面量(已去引号、已处理 `""` 转义)。
    Str(String),
    /// 名称记号:引用 / 函数名 / 布尔常量,身份由语法阶段判定。
    Ident(String),
    /// 错误字面量,如 `#DIV/0!`。
    Err(ExcelError),

    // 运算符
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `^`
    Caret,
    /// `&`
    Amp,
    /// `%`
    Percent,
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
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `,`
    Comma,
    /// `:`
    Colon,
}

/// 把公式**主体**(不含前导 `=`)切成记号序列。
///
/// 遇到无法识别的字符返回 [`ExcelError::Name`] —— 与「无法理解这个公式」的语义一致。
pub fn tokenize(input: &str) -> Result<Vec<Token>, ExcelError> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            b'+' => push(&mut tokens, Token::Plus, &mut i),
            b'-' => push(&mut tokens, Token::Minus, &mut i),
            b'*' => push(&mut tokens, Token::Star, &mut i),
            b'/' => push(&mut tokens, Token::Slash, &mut i),
            b'^' => push(&mut tokens, Token::Caret, &mut i),
            b'&' => push(&mut tokens, Token::Amp, &mut i),
            b'%' => push(&mut tokens, Token::Percent, &mut i),
            b'(' => push(&mut tokens, Token::LParen, &mut i),
            b')' => push(&mut tokens, Token::RParen, &mut i),
            b',' => push(&mut tokens, Token::Comma, &mut i),
            b':' => push(&mut tokens, Token::Colon, &mut i),
            b'=' => push(&mut tokens, Token::Eq, &mut i),
            b'<' => {
                if bytes.get(i + 1) == Some(&b'>') {
                    tokens.push(Token::Ne);
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'=') {
                    tokens.push(Token::Le);
                    i += 2;
                } else {
                    push(&mut tokens, Token::Lt, &mut i);
                }
            }
            b'>' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    tokens.push(Token::Ge);
                    i += 2;
                } else {
                    push(&mut tokens, Token::Gt, &mut i);
                }
            }
            b'"' => {
                let (s, next) = lex_string(input, i)?;
                tokens.push(Token::Str(s));
                i = next;
            }
            b'#' => {
                let (e, next) = lex_error(input, i)?;
                tokens.push(Token::Err(e));
                i = next;
            }
            _ if c.is_ascii_digit()
                || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) =>
            {
                let (n, next) = lex_number(input, i)?;
                tokens.push(Token::Num(n));
                i = next;
            }
            _ if is_ident_start(c) => {
                let (name, next) = lex_ident(input, i);
                tokens.push(Token::Ident(name));
                i = next;
            }
            _ => return Err(ExcelError::Name),
        }
    }
    Ok(tokens)
}

fn push(tokens: &mut Vec<Token>, tok: Token, i: &mut usize) {
    tokens.push(tok);
    *i += 1;
}

/// 名称首字符:字母、下划线,或引用里的 `$`。
fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}

/// 名称后续字符:再加数字、`.`(函数名如 `T.DIST`)。
fn is_ident_part(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'.'
}

fn lex_ident(input: &str, start: usize) -> (String, usize) {
    let bytes = input.as_bytes();
    let mut i = start;
    while i < bytes.len() && is_ident_part(bytes[i]) {
        i += 1;
    }
    (input[start..i].to_string(), i)
}

/// 词法层面的数字:整数/小数/科学计数。不处理正负号(交给一元运算符)。
fn lex_number(input: &str, start: usize) -> Result<(f64, usize), ExcelError> {
    let bytes = input.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    // 指数部分 e / E,允许一个正负号
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    input[start..i]
        .parse::<f64>()
        .map(|n| (n, i))
        .map_err(|_| ExcelError::Name)
}

/// 字符串字面量:`"` 起止,内部 `""` 表示一个字面量双引号。
fn lex_string(input: &str, start: usize) -> Result<(String, usize), ExcelError> {
    let bytes = input.as_bytes();
    let mut i = start + 1; // 跳过起始引号
    let mut out = String::new();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if bytes.get(i + 1) == Some(&b'"') {
                out.push('"');
                i += 2;
            } else {
                return Ok((out, i + 1)); // 收尾引号
            }
        } else {
            // 按 UTF-8 逐字符拷贝(字符串里可能有中文)
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Err(ExcelError::Name) // 引号未闭合
}

/// 错误字面量:`#DIV/0!`、`#N/A`、`#REF!`、`#NAME?`、`#NULL!`、`#NUM!`、`#VALUE!`。
fn lex_error(input: &str, start: usize) -> Result<(ExcelError, usize), ExcelError> {
    let rest = &input[start..];
    // 逐个前缀匹配;`#N/A` 没有结尾标点,单独处理。
    const TABLE: &[(&str, ExcelError)] = &[
        ("#NULL!", ExcelError::Null),
        ("#DIV/0!", ExcelError::Div0),
        ("#VALUE!", ExcelError::Value),
        ("#REF!", ExcelError::Ref),
        ("#NAME?", ExcelError::Name),
        ("#NUM!", ExcelError::Num),
        ("#N/A", ExcelError::Na),
    ];
    for (lit, err) in TABLE {
        if rest.len() >= lit.len() && rest[..lit.len()].eq_ignore_ascii_case(lit) {
            return Ok((*err, start + lit.len()));
        }
    }
    Err(ExcelError::Name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Token> {
        tokenize(s).expect("应能词法分析")
    }

    #[test]
    fn numbers_including_scientific_and_decimal() {
        assert_eq!(toks("1"), vec![Token::Num(1.0)]);
        assert_eq!(toks("2.75"), vec![Token::Num(2.75)]);
        assert_eq!(toks(".5"), vec![Token::Num(0.5)]);
        assert_eq!(toks("1e3"), vec![Token::Num(1000.0)]);
        assert_eq!(toks("2.5E-2"), vec![Token::Num(0.025)]);
    }

    #[test]
    fn operators_and_multichar_comparisons() {
        assert_eq!(
            toks("<= >= <> < > ="),
            vec![
                Token::Le,
                Token::Ge,
                Token::Ne,
                Token::Lt,
                Token::Gt,
                Token::Eq
            ]
        );
    }

    #[test]
    fn strings_with_escaped_quotes() {
        assert_eq!(
            toks("\"he said \"\"hi\"\"\""),
            vec![Token::Str("he said \"hi\"".into())]
        );
        assert_eq!(toks("\"北京\""), vec![Token::Str("北京".into())]);
    }

    #[test]
    fn identifiers_refs_and_functions() {
        assert_eq!(
            toks("SUM(A1:B2)"),
            vec![
                Token::Ident("SUM".into()),
                Token::LParen,
                Token::Ident("A1".into()),
                Token::Colon,
                Token::Ident("B2".into()),
                Token::RParen,
            ]
        );
        assert_eq!(toks("$A$1"), vec![Token::Ident("$A$1".into())]);
        assert_eq!(toks("T.DIST"), vec![Token::Ident("T.DIST".into())]);
    }

    #[test]
    fn error_literals() {
        assert_eq!(toks("#DIV/0!"), vec![Token::Err(ExcelError::Div0)]);
        assert_eq!(toks("#n/a"), vec![Token::Err(ExcelError::Na)]);
    }

    #[test]
    fn unclosed_string_is_error() {
        assert_eq!(tokenize("\"abc"), Err(ExcelError::Name));
    }
}
