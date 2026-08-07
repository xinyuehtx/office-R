//! 平台无关的**只读表格模型**。
//!
//! 视图层每帧只需要「可见区域」内的少量单元格,但表格整体可能有数百万个单元格。
//! 因此这里刻意**不用** `Vec<Vec<String>>`:那样每个单元格一次堆分配,
//! 百万级单元格会带来巨大的内存开销与糟糕的缓存局部性。
//!
//! 取而代之的紧凑布局:
//!
//! ```text
//! text:       "北京上海1234…"        所有单元格文本按行优先首尾相接
//! cell_ends:  [6, 12, 13, 14, …]     每个单元格在 text 中的结束字节偏移
//! row_starts: [0, 2, 4, …]           每行首个单元格在 cell_ends 中的下标(长度 rows+1)
//! ```
//!
//! 单元格 `(r, c)` 的文本 = `text[start..end]`,其中
//! `idx = row_starts[r] + c`、`end = cell_ends[idx]`、
//! `start = if idx == 0 { 0 } else { cell_ends[idx - 1] }`。
//!
//! 这样每个单元格的额外开销仅 4 字节,且 `text` 只有一次大块分配。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 单列显示宽度的上限(单位:半角字符数)。
///
/// 超长文本不应把列撑到屏幕之外,超出部分由视图层裁剪并显示省略号。
pub const MAX_COL_WIDTH_UNITS: u32 = 60;

/// 单列显示宽度的下限(单位:半角字符数),保证空列也能看清列头。
pub const MIN_COL_WIDTH_UNITS: u32 = 3;

/// 表格模型自身的错误(与具体文件格式无关)。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SheetError {
    /// 跨线程传输后的紧凑表示自校验失败,通常意味着数据在传输途中被破坏。
    #[error("表格数据结构损坏:{reason}")]
    CorruptPacked {
        /// 具体的不一致之处。
        reason: String,
    },
}

/// 只读表格:行 × 列的纯文本单元格。
///
/// 本期只承载文本(CSV 没有类型、公式与样式)。后续若要支持 xlsx 的
/// 值类型 / 数字格式,应在此之外新增「值层」,而不是把格式化塞进本结构 ——
/// 见 [`crate::sheet`] 模块文档与 `docs/architecture.md` 的「扩展边界」。
#[derive(Clone, PartialEq, Eq)]
pub struct Sheet {
    /// 所有单元格文本按行优先首尾相接。
    text: String,
    /// 每个单元格在 `text` 中的结束字节偏移(严格不减)。
    cell_ends: Vec<u32>,
    /// 每行首个单元格在 `cell_ends` 中的下标,长度为 `rows + 1`。
    row_starts: Vec<u32>,
    /// 最大列数(即视图的列数;短行右侧按空单元格处理)。
    cols: usize,
    /// 每列的建议显示宽度(单位:半角字符数),长度为 `cols`。
    col_width_units: Vec<u32>,
}

/// 手写 `Debug`:**只输出维度,绝不输出单元格内容**,避免日志泄露用户数据。
impl std::fmt::Debug for Sheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sheet")
            .field("rows", &self.rows())
            .field("cols", &self.cols)
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

/// 一块矩形区域内的单元格文本,用于**一次性**跨越 WASM 边界。
///
/// 逐个单元格调用 `cell()` 会产生 N 次跨边界调用与 N 次字符串分配;
/// 打包成「一个大字符串 + 一组偏移」后只需 2 次拷贝,视图层再自行切片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellWindow {
    /// 区域内单元格文本按行优先首尾相接。
    pub text: String,
    /// 每个单元格在 `text` 中的结束偏移,长度为 `rows * cols`。
    ///
    /// 单位是 **UTF-16 码元**而非 UTF-8 字节:`text` 跨到 JS 之后会变成
    /// JS 字符串,而 `String.prototype.slice` 按 UTF-16 码元计数。
    /// 用字节偏移会让含中文的单元格全部错位。
    pub ends: Vec<u32>,
    /// 区域行数。
    pub rows: usize,
    /// 区域列数。
    pub cols: usize,
}

/// [`Sheet`] 的**紧凑传输表示**。
///
/// 用于在 Web Worker(解析)与主线程(绘制)之间传递:各字段都是连续缓冲,
/// 可作为 `ArrayBuffer` 直接 transfer,避免结构化克隆带来的深拷贝。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackedSheet {
    /// UTF-8 文本缓冲。
    pub text: Vec<u8>,
    /// 单元格结束偏移。
    pub cell_ends: Vec<u32>,
    /// 每行起始下标。
    pub row_starts: Vec<u32>,
    /// 列数。
    pub cols: u32,
    /// 每列建议显示宽度。
    pub col_width_units: Vec<u32>,
}

impl Sheet {
    /// 从「逐行的字段迭代器」构建表格。
    ///
    /// 调用方(如 CSV 解析器)按行推入字段,构建器负责紧凑存储、
    /// 计算最大列数与各列显示宽度。
    pub fn builder() -> SheetBuilder {
        SheetBuilder::default()
    }

    /// 行数。
    pub fn rows(&self) -> usize {
        self.row_starts.len().saturating_sub(1)
    }

    /// 列数(所有行中的最大列数)。
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// 表格是否没有任何行。
    pub fn is_empty(&self) -> bool {
        self.rows() == 0
    }

    /// 第 `row` 行实际的字段数(可能小于 [`Self::cols`])。
    pub fn row_len(&self, row: usize) -> usize {
        if row + 1 >= self.row_starts.len() {
            return 0;
        }
        (self.row_starts[row + 1] - self.row_starts[row]) as usize
    }

    /// 取单元格文本;越界或短行右侧一律返回空串(而非 panic)。
    ///
    /// 「短行按空单元格补齐」是本项目对**各行列数不一致**的明确约定:
    /// 视图始终是规整矩形,不会错位。
    pub fn cell(&self, row: usize, col: usize) -> &str {
        if col >= self.row_len(row) {
            return "";
        }
        let idx = self.row_starts[row] as usize + col;
        let end = self.cell_ends[idx] as usize;
        let start = if idx == 0 {
            0
        } else {
            self.cell_ends[idx - 1] as usize
        };
        // 偏移由构建器维护,始终落在字符边界上;仍用 get 兜底以避免 panic。
        self.text.get(start..end).unwrap_or("")
    }

    /// 每列的建议显示宽度(单位:半角字符数)。
    pub fn col_width_units(&self) -> &[u32] {
        &self.col_width_units
    }

    /// 取出 `[row0, row1) × [col0, col1)` 矩形区域内的单元格文本。
    ///
    /// 入参会被夹到有效范围内,因此传入任意区间都安全。
    pub fn window(&self, row0: usize, row1: usize, col0: usize, col1: usize) -> CellWindow {
        let row0 = row0.min(self.rows());
        let row1 = row1.clamp(row0, self.rows());
        let col0 = col0.min(self.cols);
        let col1 = col1.clamp(col0, self.cols);

        let rows = row1 - row0;
        let cols = col1 - col0;
        let mut text = String::new();
        let mut ends = Vec::with_capacity(rows * cols);
        // 偏移按 UTF-16 码元累加(见 CellWindow::ends 的说明)
        let mut utf16_len: u32 = 0;
        for row in row0..row1 {
            for col in col0..col1 {
                let cell = self.cell(row, col);
                text.push_str(cell);
                utf16_len += cell.encode_utf16().count() as u32;
                ends.push(utf16_len);
            }
        }
        CellWindow {
            text,
            ends,
            rows,
            cols,
        }
    }

    /// 转成可跨线程传输的紧凑表示(移动语义,无深拷贝)。
    pub fn into_packed(self) -> PackedSheet {
        PackedSheet {
            text: self.text.into_bytes(),
            cell_ends: self.cell_ends,
            row_starts: self.row_starts,
            cols: self.cols as u32,
            col_width_units: self.col_width_units,
        }
    }

    /// 从紧凑表示还原,并做自校验。
    ///
    /// 校验的意义:数据来自另一个线程 / JS 侧,出错时应给出明确错误而非
    /// 在绘制热路径上 panic。
    pub fn from_packed(packed: PackedSheet) -> Result<Self, SheetError> {
        let corrupt = |reason: &str| SheetError::CorruptPacked {
            reason: reason.to_string(),
        };

        let text = String::from_utf8(packed.text).map_err(|_| corrupt("文本缓冲不是合法 UTF-8"))?;
        let cols = packed.cols as usize;

        if packed.row_starts.is_empty() {
            return Err(corrupt("缺少行索引"));
        }
        if packed.col_width_units.len() != cols {
            return Err(corrupt("列宽数量与列数不一致"));
        }
        if *packed.row_starts.last().expect("已校验非空") as usize != packed.cell_ends.len() {
            return Err(corrupt("行索引末位与单元格数量不一致"));
        }
        if packed.row_starts.windows(2).any(|w| w[0] > w[1]) {
            return Err(corrupt("行索引非递增"));
        }
        if packed.cell_ends.windows(2).any(|w| w[0] > w[1]) {
            return Err(corrupt("单元格偏移非递增"));
        }
        if packed.cell_ends.last().copied().unwrap_or(0) as usize != text.len() {
            return Err(corrupt("单元格偏移末位与文本长度不一致"));
        }

        Ok(Sheet {
            text,
            cell_ends: packed.cell_ends,
            row_starts: packed.row_starts,
            cols,
            col_width_units: packed.col_width_units,
        })
    }
}

/// [`Sheet`] 的增量构建器:按行推入字段。
///
/// 构建期间 `row_starts` 每行一个元素(尚无末尾哨兵),
/// [`Self::finish`] 时补上哨兵使其长度变为 `rows + 1`。
#[derive(Debug, Default)]
pub struct SheetBuilder {
    text: String,
    cell_ends: Vec<u32>,
    row_starts: Vec<u32>,
    cols: usize,
    /// 逐列累计的最大显示宽度,随字段推入增量更新(避免二次遍历全部单元格)。
    col_width_units: Vec<u32>,
}

impl SheetBuilder {
    /// 开始新的一行。
    pub fn start_row(&mut self) {
        self.row_starts.push(self.cell_ends.len() as u32);
    }

    /// 向当前行追加一个字段。必须先调用 [`Self::start_row`]。
    pub fn push_field(&mut self, field: &str) {
        debug_assert!(!self.row_starts.is_empty(), "push_field 前必须先 start_row");
        let col = (self.cell_ends.len() as u32 - *self.row_starts.last().unwrap_or(&0)) as usize;
        self.text.push_str(field);
        self.cell_ends.push(self.text.len() as u32);

        if col + 1 > self.cols {
            self.cols = col + 1;
            self.col_width_units.resize(self.cols, MIN_COL_WIDTH_UNITS);
        }
        let width = display_width(field).clamp(MIN_COL_WIDTH_UNITS, MAX_COL_WIDTH_UNITS);
        if width > self.col_width_units[col] {
            self.col_width_units[col] = width;
        }
    }

    /// 当前已推入的行数。
    pub fn rows(&self) -> usize {
        self.row_starts.len()
    }

    /// 当前最大列数。
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// 丢弃末尾**整行为空**的若干行。
    ///
    /// 文本文件常以换行结尾,再多敲一个回车就会产生「末尾空行」;
    /// 这些行不是数据,显示出来只会让人误以为表格更长。
    /// 注意只裁剪末尾:表格中间的空行是有意义的留白,必须保留以对齐行号。
    pub fn trim_trailing_empty_rows(&mut self) {
        while let Some(&start) = self.row_starts.last() {
            let start = start as usize;
            // 整行为空 = 没有字段,或只有一个长度为 0 的字段
            let field_count = self.cell_ends.len() - start;
            let is_blank = field_count == 0
                || (field_count == 1 && {
                    let end = self.cell_ends[start];
                    let begin = if start == 0 {
                        0
                    } else {
                        self.cell_ends[start - 1]
                    };
                    end == begin
                });
            if !is_blank {
                break;
            }
            self.row_starts.pop();
            self.cell_ends.truncate(start);
            let text_len = if start == 0 {
                0
            } else {
                self.cell_ends[start - 1] as usize
            };
            self.text.truncate(text_len);
        }
    }

    /// 完成构建。
    pub fn finish(mut self) -> Sheet {
        // 补末尾哨兵,使 row_starts 长度为 rows + 1,便于 O(1) 求每行字段数
        self.row_starts.push(self.cell_ends.len() as u32);
        self.col_width_units.resize(self.cols, MIN_COL_WIDTH_UNITS);
        Sheet {
            text: self.text,
            cell_ends: self.cell_ends,
            row_starts: self.row_starts,
            cols: self.cols,
            col_width_units: self.col_width_units,
        }
    }
}

/// 估算字符串的**显示宽度**,单位为半角字符数。
///
/// 这里刻意不引入 `unicode-width` 依赖:列宽只需要一个足够好的估算值,
/// 精确的像素测量由视图层用 `ctx.measureText` 完成。
pub fn display_width(s: &str) -> u32 {
    s.chars().map(char_display_width).sum()
}

/// 单个字符的显示宽度:东亚宽字符算 2,其余算 1。
///
/// 覆盖常用区间(CJK 统一汉字、假名、韩文、全角形式、CJK 标点等),
/// 足以让中文表格的列宽估算不至于偏窄。
fn char_display_width(c: char) -> u32 {
    match c as u32 {
        0x1100..=0x115F // 韩文字母 Jamo
        | 0x2E80..=0x303E // CJK 部首补充 / 康熙部首 / CJK 符号与标点
        | 0x3041..=0x33FF // 假名、注音、韩文兼容字母、CJK 兼容
        | 0x3400..=0x4DBF // CJK 扩展 A
        | 0x4E00..=0x9FFF // CJK 统一汉字
        | 0xA000..=0xA4CF // 彝文
        | 0xAC00..=0xD7A3 // 韩文音节
        | 0xF900..=0xFAFF // CJK 兼容汉字
        | 0xFE30..=0xFE6F // CJK 兼容形式
        | 0xFF00..=0xFF60 // 全角 ASCII
        | 0xFFE0..=0xFFE6 // 全角符号
        | 0x1F300..=0x1F9FF // 绘文字
        | 0x20000..=0x3FFFD => 2, // CJK 扩展 B 及以上
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用二维数组快速构造 Sheet,便于测试。
    fn sheet_of(rows: &[&[&str]]) -> Sheet {
        let mut b = Sheet::builder();
        for row in rows {
            b.start_row();
            for f in *row {
                b.push_field(f);
            }
        }
        b.finish()
    }

    #[test]
    fn empty_sheet_has_no_rows() {
        let sheet = sheet_of(&[]);
        assert_eq!(sheet.rows(), 0);
        assert_eq!(sheet.cols(), 0);
        assert!(sheet.is_empty());
        // 空表取任意单元格都安全
        assert_eq!(sheet.cell(0, 0), "");
        assert_eq!(sheet.cell(999, 999), "");
    }

    #[test]
    fn cells_round_trip() {
        let sheet = sheet_of(&[&["a", "b", "c"], &["1", "", "3"]]);
        assert_eq!(sheet.rows(), 2);
        assert_eq!(sheet.cols(), 3);
        assert_eq!(sheet.cell(0, 0), "a");
        assert_eq!(sheet.cell(0, 2), "c");
        assert_eq!(sheet.cell(1, 1), "");
        assert_eq!(sheet.cell(1, 2), "3");
    }

    #[test]
    fn ragged_rows_are_padded_with_empty_cells() {
        let sheet = sheet_of(&[&["a"], &["1", "2", "3"], &["x", "y"]]);
        assert_eq!(sheet.cols(), 3, "列数取各行最大值");
        assert_eq!(sheet.row_len(0), 1);
        assert_eq!(sheet.cell(0, 1), "", "短行右侧补空");
        assert_eq!(sheet.cell(0, 2), "");
        assert_eq!(sheet.cell(2, 1), "y");
        assert_eq!(sheet.cell(2, 2), "");
    }

    #[test]
    fn multibyte_cells_slice_on_char_boundary() {
        let sheet = sheet_of(&[&["北京", "上海"], &["🎉", "ok"]]);
        assert_eq!(sheet.cell(0, 0), "北京");
        assert_eq!(sheet.cell(0, 1), "上海");
        assert_eq!(sheet.cell(1, 0), "🎉");
    }

    #[test]
    fn window_extracts_rectangle() {
        let sheet = sheet_of(&[
            &["a1", "b1", "c1"],
            &["a2", "b2", "c2"],
            &["a3", "b3", "c3"],
        ]);
        let w = sheet.window(1, 3, 0, 2);
        assert_eq!(w.rows, 2);
        assert_eq!(w.cols, 2);
        assert_eq!(w.text, "a2b2a3b3");
        assert_eq!(w.ends, vec![2, 4, 6, 8]);
    }

    #[test]
    fn window_offsets_are_utf16_units_for_js_slicing() {
        // 「北京」在 UTF-8 里是 6 字节,但在 JS 字符串里只有 2 个码元;
        // 表情符号 🎉 是 1 个 char / 4 字节 / 2 个 UTF-16 码元(代理对)。
        let sheet = sheet_of(&[&["北京", "🎉", "ab"]]);
        let w = sheet.window(0, 1, 0, 3);
        assert_eq!(w.ends, vec![2, 4, 6], "偏移应按 UTF-16 码元累加");

        // 用偏移在「JS 字符串」上切片应还原出原文
        let utf16: Vec<u16> = w.text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&utf16[0..2]).unwrap(), "北京");
        assert_eq!(String::from_utf16(&utf16[2..4]).unwrap(), "🎉");
        assert_eq!(String::from_utf16(&utf16[4..6]).unwrap(), "ab");
    }

    #[test]
    fn window_clamps_out_of_range_input() {
        let sheet = sheet_of(&[&["a", "b"]]);
        let w = sheet.window(0, 100, 0, 100);
        assert_eq!((w.rows, w.cols), (1, 2));

        // 反向区间不会 panic,退化为空区域
        let empty = sheet.window(5, 1, 5, 1);
        assert_eq!((empty.rows, empty.cols), (0, 0));
        assert!(empty.text.is_empty());
    }

    #[test]
    fn packed_round_trip_preserves_content() {
        let sheet = sheet_of(&[&["北京", "1"], &["", "2"]]);
        let packed = sheet.clone().into_packed();
        let restored = Sheet::from_packed(packed).expect("应能还原");
        assert_eq!(restored, sheet);
        assert_eq!(restored.cell(0, 0), "北京");
        assert_eq!(restored.cell(1, 0), "");
    }

    #[test]
    fn packed_round_trip_of_empty_sheet() {
        let sheet = sheet_of(&[]);
        let restored = Sheet::from_packed(sheet.clone().into_packed()).expect("空表也应能还原");
        assert_eq!(restored.rows(), 0);
    }

    #[test]
    fn corrupt_packed_is_rejected_with_reason() {
        let mut packed = sheet_of(&[&["a", "b"]]).into_packed();
        packed.cell_ends.pop(); // 破坏偏移数组
        let err = Sheet::from_packed(packed).expect_err("损坏数据应报错");
        assert!(
            matches!(err, SheetError::CorruptPacked { .. }),
            "实际错误:{err}"
        );
        assert!(err.to_string().contains("损坏"));
    }

    #[test]
    fn corrupt_packed_rejects_bad_utf8() {
        let mut packed = sheet_of(&[&["ab"]]).into_packed();
        packed.text = vec![0xFF, 0xFE];
        assert!(Sheet::from_packed(packed).is_err());
    }

    #[test]
    fn col_widths_track_widest_cell_and_are_clamped() {
        let sheet = sheet_of(&[&["a", "中文标题"], &["long-value", "x"]]);
        let widths = sheet.col_width_units();
        assert_eq!(widths.len(), 2);
        assert_eq!(widths[0], 10, "取该列最宽单元格 long-value");
        assert_eq!(widths[1], 8, "4 个汉字 = 8 个半角宽");
    }

    #[test]
    fn col_width_is_capped_for_very_long_text() {
        let long = "x".repeat(5_000);
        let sheet = sheet_of(&[&[long.as_str()]]);
        assert_eq!(sheet.col_width_units()[0], MAX_COL_WIDTH_UNITS);
    }

    #[test]
    fn col_width_has_lower_bound_for_empty_column() {
        let sheet = sheet_of(&[&["", ""]]);
        assert_eq!(sheet.col_width_units(), &[MIN_COL_WIDTH_UNITS; 2]);
    }

    #[test]
    fn trailing_empty_rows_are_trimmed_but_inner_ones_kept() {
        let mut b = Sheet::builder();
        for row in [vec!["a", "b"], vec![""], vec!["c", "d"], vec![""], vec![""]] {
            b.start_row();
            for f in row {
                b.push_field(f);
            }
        }
        b.trim_trailing_empty_rows();
        let sheet = b.finish();
        assert_eq!(sheet.rows(), 3, "末尾两个空行被裁掉,中间空行保留");
        assert_eq!(sheet.cell(1, 0), "");
        assert_eq!(sheet.cell(2, 0), "c");
    }

    #[test]
    fn all_empty_rows_trim_to_empty_sheet() {
        let mut b = Sheet::builder();
        for _ in 0..3 {
            b.start_row();
            b.push_field("");
        }
        b.trim_trailing_empty_rows();
        assert_eq!(b.finish().rows(), 0);
    }

    #[test]
    fn debug_does_not_leak_cell_content() {
        let sheet = sheet_of(&[&["机密数据"]]);
        let dumped = format!("{sheet:?}");
        assert!(!dumped.contains("机密数据"), "Debug 不得输出单元格内容");
        assert!(dumped.contains("rows"));
    }

    #[test]
    fn display_width_counts_wide_chars_as_two() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("a中"), 3);
        assert_eq!(display_width(""), 0);
    }
}
