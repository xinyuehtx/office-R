//! 列过滤:给定 [`Sheet`] 与一组按列的条件,算出**命中的行下标**。
//!
//! **为什么在 Rust**:过滤是一次全表扫描(可达百万行),是与行数成正比的重 CPU 工作;
//! 数据本就在内核里,放这里既快又不必把整表搬到 JS。视图层只拿到一串命中行下标,
//! 用它把「可视行 → 底层行」重映射即可(见 `WasmSheet`),渲染器几何完全复用。
//!
//! **约定**:顶部 `header_rows` 行(通常是表头)**始终保留**、不参与条件;
//! 多列条件按 **AND** 组合(与 Excel 一致)。本模块还提供 [`sort_rows`]:在过滤结果
//! 之上按某列重排(数值感知、空值靠后、稳定),与过滤复合成同一套「可视行 → 底层行」映射。

use crate::sheet::Sheet;

/// 文本比较运算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOp {
    /// 包含子串。
    Contains,
    /// 不包含子串。
    NotContains,
    /// 完全相等。
    Equals,
    /// 以…开头。
    Begins,
    /// 以…结尾。
    Ends,
}

/// 数值比较运算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumOp {
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// 闭区间 `[a, b]`
    Between,
}

/// 单列的过滤条件。
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    /// 值集:单元格文本(忽略大小写)属于给定集合。
    Values(Vec<String>),
    /// 文本条件(忽略大小写)。
    Text { op: TextOp, needle: String },
    /// 数值条件;`Between` 用 `[a, b]`,其余只用 `a`。非数值单元格一律不匹配(除 `Ne`)。
    Number { op: NumOp, a: f64, b: f64 },
    /// 空白筛选:`true` 只留空白,`false` 只留非空白。
    Blank(bool),
}

/// 作用在某一列上的过滤条件。
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnFilter {
    /// 0 基列号。
    pub col: u32,
    /// 该列的条件。
    pub predicate: Predicate,
}

impl Predicate {
    /// 单元格文本是否满足本条件。
    fn matches(&self, cell: &str) -> bool {
        match self {
            Predicate::Blank(want_blank) => cell.trim().is_empty() == *want_blank,
            Predicate::Values(set) => {
                let c = cell.to_lowercase();
                set.iter().any(|v| v.to_lowercase() == c)
            }
            Predicate::Text { op, needle } => {
                let hay = cell.to_lowercase();
                let ndl = needle.to_lowercase();
                match op {
                    TextOp::Contains => hay.contains(&ndl),
                    TextOp::NotContains => !hay.contains(&ndl),
                    TextOp::Equals => hay == ndl,
                    TextOp::Begins => hay.starts_with(&ndl),
                    TextOp::Ends => hay.ends_with(&ndl),
                }
            }
            Predicate::Number { op, a, b } => match parse_num(cell) {
                Some(n) => match op {
                    NumOp::Eq => n == *a,
                    NumOp::Ne => n != *a,
                    NumOp::Gt => n > *a,
                    NumOp::Ge => n >= *a,
                    NumOp::Lt => n < *a,
                    NumOp::Le => n <= *a,
                    NumOp::Between => n >= a.min(*b) && n <= a.max(*b),
                },
                // 非数值单元格:除「不等于」外都不匹配
                None => matches!(op, NumOp::Ne),
            },
        }
    }
}

/// 解析单元格里的数字(去空白、支持末尾百分号),与公式引擎的强制口径一致。
fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(pct) = t.strip_suffix('%') {
        return pct.trim().parse::<f64>().ok().map(|n| n / 100.0);
    }
    t.parse::<f64>().ok()
}

/// 返回命中的行下标:顶部 `header_rows` 行始终保留,其后数据行需满足**全部**条件。
///
/// `filters` 为空时返回全部行下标(相当于不过滤)。
pub fn filter_rows(sheet: &Sheet, filters: &[ColumnFilter], header_rows: u32) -> Vec<u32> {
    let rows = sheet.rows() as u32;
    let header = header_rows.min(rows);
    let mut out: Vec<u32> = (0..header).collect();

    for r in header..rows {
        let keep = filters.iter().all(|f| {
            let cell = sheet.cell(r as usize, f.col as usize);
            f.predicate.matches(cell)
        });
        if keep {
            out.push(r);
        }
    }
    out
}

/// 对给定的**底层行序列** `base` 按第 `col` 列排序,返回重排后的行序列。
///
/// - 顶部 `header_rows`(底层行号 < `header_rows`)**固定置顶**、不参与排序;
/// - `base` 通常是过滤结果(或全表 `0..rows`),因此**排序与过滤天然复合**;
/// - 比较是**数值感知**的:两侧都能解析为数才按数值比,否则按文本(忽略大小写);
/// - 稳定排序:键相等的行保持原有相对顺序。
pub fn sort_rows(
    sheet: &Sheet,
    base: &[u32],
    col: u32,
    ascending: bool,
    header_rows: u32,
) -> Vec<u32> {
    let mut headers: Vec<u32> = Vec::new();
    let mut data: Vec<u32> = Vec::new();
    for &r in base {
        if r < header_rows {
            headers.push(r);
        } else {
            data.push(r);
        }
    }

    data.sort_by(|&a, &b| {
        use std::cmp::Ordering;
        let ca = sheet.cell(a as usize, col as usize);
        let cb = sheet.cell(b as usize, col as usize);
        let (ta, tb) = (ca.trim(), cb.trim());
        // 空值恒排末尾,不随升/降序翻转(与 Excel 一致)
        match (ta.is_empty(), tb.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => {
                let ord = compare_nonblank(ta, tb);
                if ascending {
                    ord
                } else {
                    ord.reverse()
                }
            }
        }
    });

    headers.extend(data);
    headers
}

/// 非空单元格比较:两侧都能解析为数则按数值,否则按文本(忽略大小写)。
fn compare_nonblank(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if let (Ok(na), Ok(nb)) = (a.parse::<f64>(), b.parse::<f64>()) {
        return na.partial_cmp(&nb).unwrap_or(Ordering::Equal);
    }
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// 枚举某列的**唯一值**(供值集过滤的 UI),跳过顶部 `header_rows` 行。
///
/// 按首次出现顺序返回,最多 `limit` 个;返回 `(values, truncated)`,`truncated` 表示还有更多。
/// 忽略纯空白单元格(空值单独在 UI 里作「空白」项处理)。
pub fn column_unique_values(
    sheet: &Sheet,
    col: u32,
    header_rows: u32,
    limit: usize,
) -> (Vec<String>, bool) {
    use std::collections::HashSet;
    let rows = sheet.rows() as u32;
    let header = header_rows.min(rows);
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();

    for r in header..rows {
        let cell = sheet.cell(r as usize, col as usize);
        if cell.trim().is_empty() {
            continue;
        }
        // 去重键用小写,展示保留原样
        let key = cell.to_lowercase();
        if seen.insert(key) {
            if out.len() >= limit {
                return (out, true);
            }
            out.push(cell.to_string());
        }
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(rows: &[&[&str]]) -> Sheet {
        let mut b = Sheet::builder();
        for row in rows {
            b.start_row();
            for f in *row {
                b.push_field(f);
            }
        }
        b.finish()
    }

    /// 一张:表头 + 城市/金额。
    fn demo() -> Sheet {
        sheet(&[
            &["城市", "金额"],
            &["北京", "1200"],
            &["上海", "800"],
            &["北京", "1500"],
            &["广州", ""],
            &["深圳", "2000"],
        ])
    }

    fn num(col: u32, op: NumOp, a: f64) -> ColumnFilter {
        ColumnFilter {
            col,
            predicate: Predicate::Number { op, a, b: 0.0 },
        }
    }

    #[test]
    fn no_filter_keeps_all_rows() {
        let s = demo();
        assert_eq!(filter_rows(&s, &[], 1), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn header_rows_always_kept() {
        let s = demo();
        // 金额 > 999999:没有数据行命中,但表头行仍在
        let out = filter_rows(&s, &[num(1, NumOp::Gt, 999_999.0)], 1);
        assert_eq!(out, vec![0]);
    }

    #[test]
    fn number_filter() {
        let s = demo();
        let out = filter_rows(&s, &[num(1, NumOp::Gt, 1000.0)], 1);
        // 表头 + 1200、1500、2000
        assert_eq!(out, vec![0, 1, 3, 5]);
    }

    #[test]
    fn between_filter() {
        let s = demo();
        let f = ColumnFilter {
            col: 1,
            predicate: Predicate::Number {
                op: NumOp::Between,
                a: 800.0,
                b: 1500.0,
            },
        };
        assert_eq!(filter_rows(&s, &[f], 1), vec![0, 1, 2, 3]);
    }

    #[test]
    fn text_and_values_filter() {
        let s = demo();
        let contains = ColumnFilter {
            col: 0,
            predicate: Predicate::Text {
                op: TextOp::Equals,
                needle: "北京".into(),
            },
        };
        assert_eq!(filter_rows(&s, &[contains], 1), vec![0, 1, 3]);

        let values = ColumnFilter {
            col: 0,
            predicate: Predicate::Values(vec!["上海".into(), "深圳".into()]),
        };
        assert_eq!(filter_rows(&s, &[values], 1), vec![0, 2, 5]);
    }

    #[test]
    fn blank_filter() {
        let s = demo();
        let only_blank = ColumnFilter {
            col: 1,
            predicate: Predicate::Blank(true),
        };
        assert_eq!(filter_rows(&s, &[only_blank], 1), vec![0, 4]);
        let only_nonblank = ColumnFilter {
            col: 1,
            predicate: Predicate::Blank(false),
        };
        assert_eq!(filter_rows(&s, &[only_nonblank], 1), vec![0, 1, 2, 3, 5]);
    }

    #[test]
    fn multiple_filters_are_anded() {
        let s = demo();
        let city = ColumnFilter {
            col: 0,
            predicate: Predicate::Text {
                op: TextOp::Equals,
                needle: "北京".into(),
            },
        };
        let amount = num(1, NumOp::Ge, 1300.0);
        // 北京 且 金额>=1300 → 只有 1500(行 3)
        assert_eq!(filter_rows(&s, &[city, amount], 1), vec![0, 3]);
    }

    #[test]
    fn unique_values_dedup_and_truncate() {
        let s = demo();
        let (vals, truncated) = column_unique_values(&s, 0, 1, 10);
        assert_eq!(vals, vec!["北京", "上海", "广州", "深圳"]);
        assert!(!truncated);

        let (vals, truncated) = column_unique_values(&s, 0, 1, 2);
        assert_eq!(vals.len(), 2);
        assert!(truncated);
    }

    #[test]
    fn case_insensitive_text() {
        let s = sheet(&[&["h"], &["Apple"], &["apple"], &["BANANA"]]);
        let f = ColumnFilter {
            col: 0,
            predicate: Predicate::Text {
                op: TextOp::Contains,
                needle: "APP".into(),
            },
        };
        assert_eq!(filter_rows(&s, &[f], 1), vec![0, 1, 2]);
    }

    #[test]
    fn sort_numeric_ascending_keeps_header() {
        let s = demo();
        let base: Vec<u32> = (0..s.rows() as u32).collect();
        // 按金额升序;空值(广州)排末尾;表头固定第 0 行
        let out = sort_rows(&s, &base, 1, true, 1);
        assert_eq!(out[0], 0, "表头固定置顶");
        let vals: Vec<&str> = out[1..].iter().map(|&r| s.cell(r as usize, 1)).collect();
        assert_eq!(vals, vec!["800", "1200", "1500", "2000", ""]);
    }

    #[test]
    fn sort_descending() {
        let s = demo();
        let base: Vec<u32> = (0..s.rows() as u32).collect();
        let out = sort_rows(&s, &base, 1, false, 1);
        let vals: Vec<&str> = out[1..].iter().map(|&r| s.cell(r as usize, 1)).collect();
        // 降序:数值从大到小,空值仍靠后
        assert_eq!(vals, vec!["2000", "1500", "1200", "800", ""]);
    }

    #[test]
    fn sort_text_column() {
        let s = demo();
        let base: Vec<u32> = (0..s.rows() as u32).collect();
        let out = sort_rows(&s, &base, 0, true, 1);
        let vals: Vec<&str> = out[1..].iter().map(|&r| s.cell(r as usize, 0)).collect();
        // 文本按 Unicode 码点排序:上(4E0A)<北(5317)<广(5E7F)<深(6DF1)
        assert_eq!(vals, vec!["上海", "北京", "北京", "广州", "深圳"]);
    }

    #[test]
    fn sort_composes_with_filter() {
        let s = demo();
        // 先过滤金额 >= 1000,再按金额降序
        let base = filter_rows(&s, &[num(1, NumOp::Ge, 1000.0)], 1);
        let out = sort_rows(&s, &base, 1, false, 1);
        let vals: Vec<&str> = out[1..].iter().map(|&r| s.cell(r as usize, 1)).collect();
        assert_eq!(vals, vec!["2000", "1500", "1200"]);
    }
}
