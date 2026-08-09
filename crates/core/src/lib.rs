//! office-core:平台无关的 office 计算内核。
//!
//! 负责 office 文件的**识别**与**解析**,不依赖任何浏览器 /
//! 操作系统 API,因此可原生单测,也可被 `office-wasm` 编译进 WASM。
//!
//! # 使用路径
//!
//! 每种格式一个解析模块,各自产出可直接渲染的模型:
//!
//! - [`format::detect_format`]:先识别格式(魔术字节 + zip 内条目名)。
//! - [`csv::parse`] / [`xlsx::parse`]:→ [`sheet::Sheet`](+ 样式/合并/图片/图表/迷你图)。
//! - [`docx::parse`]:→ Word 平面化模型;[`pptx::parse`]:→ 幻灯模型。
//! - [`formula`]:**公式计算引擎**([`Workbook`])—— [`sheet::Sheet`] 之上独立的
//!   「值/公式层」,把 `=SUM(A1:A10)` 这类公式解析并求值,语义对齐 Excel。
//! - [`numfmt`]:numfmt 格式码渲染;[`chart`]:xlsx/pptx 共用的图表数据解析。
//!
//! # 扩展边界
//!
//! [`sheet::Sheet`] 保持**只读纯文本** —— 没有任何写回或导出路径。
//! 公式求值、数字/日期格式化、图表都是 `Sheet` 之上的独立叠层,
//! 而不是把这些概念混进表格模型。详见 `docs/architecture.md`。

pub mod chart;
pub mod csv;
pub mod docx;
pub mod filter;
pub mod format;
pub mod formula;
pub mod limits;
pub mod numfmt;
pub mod pptx;
pub mod serial;
pub mod sheet;
pub mod xlsx;

pub use csv::{CsvDocument, CsvError, CsvMeta, CsvOptions};
pub use filter::{filter_rows, ColumnFilter, NumOp, Predicate, TextOp};
pub use format::{detect_format, Format};
pub use formula::{Value as FormulaValue, Workbook};
pub use sheet::{CellWindow, PackedSheet, Sheet, SheetError};

/// 内核版本(取自 Cargo 包版本)。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }
}
