//! A1 记法的**单元格引用**与**范围引用**。
//!
//! - 列用字母 `A、B、…、Z、AA、…、XFD`(0 基下标 0..=16383);行用 1 基数字。
//! - 支持绝对/相对标记 `$`(`$A$1`)。本期只保证**正确解析与格式化**,
//!   不做「复制公式时相对引用重写」的偏移逻辑。
//!
//! 内部一律用 **0 基**行列下标([`CellRef::row`]/[`CellRef::col`]),
//! 与 [`crate::sheet::Sheet`] 的下标口径一致,避免边界处 ±1 的错误。

use super::value::ExcelError;

/// Excel 列数上限(XFD),与 CSV 侧 `DEFAULT_MAX_COLS` 一致。
pub const MAX_COLS: u32 = 16_384;
/// Excel 行数上限。
pub const MAX_ROWS: u32 = 1_048_576;

/// 单个单元格引用,0 基下标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    /// 0 基行号。
    pub row: u32,
    /// 0 基列号。
    pub col: u32,
    /// 行是否为绝对引用(`$1`)。
    pub abs_row: bool,
    /// 列是否为绝对引用(`$A`)。
    pub abs_col: bool,
}

impl CellRef {
    /// 用 0 基行列构造一个相对引用。
    pub fn new(row: u32, col: u32) -> Self {
        CellRef {
            row,
            col,
            abs_row: false,
            abs_col: false,
        }
    }

    /// 解析 A1 记法(允许 `$`)。非法输入返回 `None`。
    pub fn parse(s: &str) -> Option<CellRef> {
        let bytes = s.as_bytes();
        let mut i = 0;
        let abs_col = bytes.get(i) == Some(&b'$');
        if abs_col {
            i += 1;
        }
        let col_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == col_start {
            return None; // 没有列字母
        }
        let col = col_to_index(&s[col_start..i])?;

        let abs_row = bytes.get(i) == Some(&b'$');
        if abs_row {
            i += 1;
        }
        let row_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == row_start || i != bytes.len() {
            return None; // 没有行号,或尾部有多余字符
        }
        let row1: u32 = s[row_start..i].parse().ok()?;
        if row1 == 0 {
            return None; // 行号从 1 开始
        }
        let row = row1 - 1;
        if row >= MAX_ROWS || col >= MAX_COLS {
            return None;
        }
        Some(CellRef {
            row,
            col,
            abs_row,
            abs_col,
        })
    }

    /// 格式化回 A1 记法(带 `$` 标记)。
    pub fn to_a1(&self) -> String {
        format!(
            "{}{}{}{}",
            if self.abs_col { "$" } else { "" },
            index_to_col(self.col),
            if self.abs_row { "$" } else { "" },
            self.row + 1
        )
    }
}

/// 矩形范围引用 `A1:B3`,端点已规范化为「左上 ≤ 右下」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeRef {
    /// 起始行(含,0 基)。
    pub row0: u32,
    /// 结束行(含,0 基)。
    pub row1: u32,
    /// 起始列(含,0 基)。
    pub col0: u32,
    /// 结束列(含,0 基)。
    pub col1: u32,
}

impl RangeRef {
    /// 由两个端点构造,自动规范化上下界(允许 `B3:A1` 这种反写)。
    pub fn from_corners(a: CellRef, b: CellRef) -> RangeRef {
        RangeRef {
            row0: a.row.min(b.row),
            row1: a.row.max(b.row),
            col0: a.col.min(b.col),
            col1: a.col.max(b.col),
        }
    }

    /// 行数。
    pub fn rows(&self) -> u32 {
        self.row1 - self.row0 + 1
    }

    /// 列数。
    pub fn cols(&self) -> u32 {
        self.col1 - self.col0 + 1
    }

    /// 单个单元格退化成的 1×1 范围。
    pub fn single(cell: CellRef) -> RangeRef {
        RangeRef {
            row0: cell.row,
            row1: cell.row,
            col0: cell.col,
            col1: cell.col,
        }
    }
}

/// 列字母 → 0 基下标:`A→0`、`Z→25`、`AA→26`、`XFD→16383`。
///
/// 忽略大小写。空串、含非字母、或超出 `XFD` 都返回 `None`。
pub fn col_to_index(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut acc: u32 = 0;
    for ch in s.chars() {
        let c = ch.to_ascii_uppercase();
        if !c.is_ascii_alphabetic() {
            return None;
        }
        // 26 进制,但字母无「0」,用「1..=26」表示,故先 +1 再累积。
        acc = acc
            .checked_mul(26)?
            .checked_add((c as u32) - ('A' as u32) + 1)?;
        if acc > MAX_COLS {
            return None;
        }
    }
    Some(acc - 1)
}

/// 0 基下标 → 列字母:`0→A`、`26→AA`。
pub fn index_to_col(mut idx: u32) -> String {
    let mut out = Vec::new();
    loop {
        let rem = (idx % 26) as u8;
        out.push(b'A' + rem);
        if idx < 26 {
            break;
        }
        idx = idx / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII")
}

/// 把「引用越界」映射到 `#REF!`,便于调用方 `?` 传播。
pub fn check_in_bounds(row: u32, col: u32) -> Result<(), ExcelError> {
    if row >= MAX_ROWS || col >= MAX_COLS {
        Err(ExcelError::Ref)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters_round_trip() {
        assert_eq!(col_to_index("A"), Some(0));
        assert_eq!(col_to_index("Z"), Some(25));
        assert_eq!(col_to_index("AA"), Some(26));
        assert_eq!(col_to_index("AZ"), Some(51));
        assert_eq!(col_to_index("BA"), Some(52));
        assert_eq!(col_to_index("XFD"), Some(16383));
        for &c in &[0u32, 25, 26, 27, 700, 16383] {
            assert_eq!(col_to_index(&index_to_col(c)), Some(c), "列 {c} 往返失败");
        }
    }

    #[test]
    fn column_out_of_range_rejected() {
        assert_eq!(col_to_index("XFE"), None); // XFD 之后越界
        assert_eq!(col_to_index(""), None);
        assert_eq!(col_to_index("A1"), None);
    }

    #[test]
    fn parse_plain_cell() {
        let r = CellRef::parse("B3").unwrap();
        assert_eq!((r.row, r.col), (2, 1));
        assert!(!r.abs_row && !r.abs_col);
    }

    #[test]
    fn parse_absolute_markers() {
        let r = CellRef::parse("$B$2").unwrap();
        assert_eq!((r.row, r.col, r.abs_row, r.abs_col), (1, 1, true, true));
        let r = CellRef::parse("$A1").unwrap();
        assert!(r.abs_col && !r.abs_row);
        let r = CellRef::parse("A$1").unwrap();
        assert!(!r.abs_col && r.abs_row);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(CellRef::parse("A"), None);
        assert_eq!(CellRef::parse("1"), None);
        assert_eq!(CellRef::parse("A0"), None); // 行从 1 起
        assert_eq!(CellRef::parse("A1B"), None); // 尾部多余
        assert_eq!(CellRef::parse("A1.5"), None);
    }

    #[test]
    fn a1_round_trip() {
        for s in ["A1", "$A$1", "B3", "AA10", "$XFD$1048576"] {
            assert_eq!(CellRef::parse(s).unwrap().to_a1(), s, "{s} 往返失败");
        }
    }

    #[test]
    fn range_normalizes_corners() {
        let a = CellRef::parse("B3").unwrap();
        let b = CellRef::parse("A1").unwrap();
        let rng = RangeRef::from_corners(a, b);
        assert_eq!((rng.row0, rng.row1, rng.col0, rng.col1), (0, 2, 0, 1));
        assert_eq!((rng.rows(), rng.cols()), (3, 2));
    }
}
