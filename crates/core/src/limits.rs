//! 解析与求值的**资源预算**。
//!
//! 这是个只读查看器,面对的是任意来源的文件 —— 其中一部分是畸形的,
//! 少数是刻意构造的。在 WASM 里,一次 OOM 或栈溢出会让整个模块 trap,
//! 而前端缓存了 init promise:**用户必须刷新页面才能再打开任何文件**。
//! 所以宁可对超限输入给出可读错误,也不能让它把内核带走。
//!
//! CSV 路径一开始就有一套上限([`crate::csv::DEFAULT_MAX_BYTES`] 等);
//! 这里补齐 OOXML 与公式引擎侧的对应预算。
//!
//! 取值原则:**宽到不影响真实文件,窄到不至于耗尽内存**。
//! 参考量级:一张 100 万格的表在紧凑存储下约几十 MB,而 wasm32 的
//! 线性内存上限是 4 GiB、实际可用往往远低于此。

/// 行号上限(0 基,含),与 Excel 的 1048576 行一致。
pub const MAX_ROW: u32 = 1_048_575;
/// 列号上限(0 基,含),与 Excel 的 XFD(16384 列)一致。
pub const MAX_COL: u32 = 16_383;

/// 单张工作表最多渲染的单元格数。
///
/// 超出时按行截断(与 CSV 的 `truncatedRows` 同思路),而不是拒绝整个文件 ——
/// 用户通常只想看看前面几屏。
pub const MAX_SHEET_CELLS: u64 = 8_000_000;

/// 一次范围展开 / 动态数组产出的最大元素数。
///
/// `=SUM(A1:XFD1048576)` 这类整表引用是合法写法,展开却是 172 亿格:
/// `rows() * cols()` 先在 u32 上溢出回绕成 0,`with_capacity(0)` 之后
/// 再 push 到内存耗尽。这里在展开前先判定,直接返回 `#NUM!`。
pub const MAX_ARRAY_ELEMS: u64 = 1_000_000;

/// 公式表达式的最大嵌套深度。
///
/// 解析是递归下降的,`=((((…1…))))` 会按嵌套层数吃原生栈。
/// 求值侧另有 `MAX_DEPTH`,但那是在解析**之后** —— 栈溢出发生得更早。
pub const MAX_PARSE_DEPTH: u32 = 128;

/// 条件格式 `sqref` 单次展开的最大单元格数。
///
/// 「全选 + 条件格式」在 Excel 里会生成 `sqref="A1:XFD1048576"`,
/// 这是**真实文件形态**而非畸形构造;逐格展开是 172 亿个 `(u32, u32)`。
pub const MAX_CF_CELLS: usize = 200_000;

/// 单条迷你图的最大取值数。
///
/// 前端把它画进一个单元格那么大的框里,几千个点已经远超可辨识密度。
pub const MAX_SPARKLINE_VALUES: usize = 4_096;

/// 把 `(rows, cols)` 的乘积算成 u64,避免 u32 溢出回绕。
pub fn area(rows: u32, cols: u32) -> u64 {
    u64::from(rows) * u64::from(cols)
}

#[cfg(test)]
mod tests {
    use crate::formula::value::{ExcelError, Value};
    use crate::formula::Workbook;

    /// 整表引用是合法写法,但展开是 172 亿格;早先 `rows() * cols()` 先在 u32 上
    /// 溢出回绕成 0,`with_capacity(0)` 之后一路 push 到内存耗尽。
    #[test]
    fn whole_sheet_range_is_num_error_not_oom() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "1");
        assert_eq!(
            wb.eval_formula("SUM(A1:XFD1048576)"),
            Value::Error(ExcelError::Num)
        );
        // 正常规模的范围不受影响
        assert_eq!(wb.eval_formula("SUM(A1:A10)"), Value::Number(1.0));
    }

    /// `=SEQUENCE(100000,100000)` 是 100 亿元素,且 `as usize` 在 wasm32 上会截断。
    #[test]
    fn oversized_sequence_is_num_error() {
        let wb = Workbook::new();
        assert_eq!(
            wb.eval_formula("SEQUENCE(100000,100000)"),
            Value::Error(ExcelError::Num)
        );
        assert!(matches!(wb.eval_formula("SEQUENCE(2,2)"), Value::Array(_)));
    }

    /// 深嵌套表达式必须在**解析**阶段就被拦下 —— 求值侧的 MAX_DEPTH 来得太晚,
    /// 递归下降解析器早已把原生栈吃光(在 WASM 里就是整个模块 trap)。
    #[test]
    fn deeply_nested_formula_does_not_blow_the_stack() {
        let wb = Workbook::new();
        let deep = format!("{}1{}", "(".repeat(50_000), ")".repeat(50_000));
        assert_eq!(wb.eval_formula(&deep), Value::Error(ExcelError::Num));
        // 一元前缀同样是递归点
        let neg = format!("{}1", "-".repeat(50_000));
        assert_eq!(wb.eval_formula(&neg), Value::Error(ExcelError::Num));
        // 正常深度仍可解析
        assert_eq!(wb.eval_formula("((((1+2))))"), Value::Number(3.0));
    }
}
