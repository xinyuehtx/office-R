//! office-core:平台无关的 office 计算内核。
//!
//! 负责 office 文件的**识别**与**解析**,不依赖任何浏览器 /
//! 操作系统 API,因此可原生单测,也可被 `office-wasm` 编译进 WASM。
//!
//! # 两条使用路径
//!
//! - [`render`]:识别格式 + 产出一段**摘要文本**,用于「这是什么文件」这类轻量场景。
//! - [`csv::parse`]:把 CSV 解析成 [`sheet::Sheet`],供视图层做表格渲染 ——
//!   这是本期 Excel 切片的主路径。
//!
//! - [`formula`]:**公式计算引擎**([`Workbook`])—— [`sheet::Sheet`] 之上独立的
//!   「值/公式层」,把 `=SUM(A1:A10)` 这类公式解析并求值,语义对齐 Excel。
//!
//! # 扩展边界
//!
//! [`sheet::Sheet`] 保持**只读纯文本**:公式求值在独立的 [`formula`] 模块,
//! 数字/日期格式化、图表仍未实现。新增能力一律在 `Sheet` 之上叠层,
//! 而不是把这些概念混进表格模型。详见 `docs/architecture.md`。

pub mod csv;
pub mod excel;
pub mod format;
pub mod formula;
pub mod ppt;
pub mod render;
pub mod sheet;
pub mod word;

pub use csv::{CsvDocument, CsvError, CsvMeta, CsvOptions};
pub use format::{detect_format, Format};
pub use formula::{Value as FormulaValue, Workbook};
pub use render::RenderResult;
pub use sheet::{CellWindow, PackedSheet, Sheet, SheetError};

/// 内核版本(取自 Cargo 包版本)。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 识别文件格式并分发到对应组件产出摘要。
///
/// 这是「读取 office 文件 → 识别 → 摘要」的统一入口。
/// CSV 的完整渲染走 [`csv::parse`],不经过这里。
pub fn render(bytes: &[u8]) -> RenderResult {
    match detect_format(bytes) {
        Format::Docx => word::render(bytes),
        Format::Xlsx => excel::render(bytes),
        Format::Pptx => ppt::render(bytes),
        // CSV 走表格渲染路径,这里只做引导,避免用户在 Word/演示 页面困惑
        Format::Csv => RenderResult::err(
            Format::Csv,
            bytes.len(),
            "这是 CSV 文本表格,请切换到「表格」页查看渲染结果。",
        ),
        Format::Unknown => RenderResult::err(
            Format::Unknown,
            bytes.len(),
            "无法识别的文件格式,请上传 .docx / .xlsx / .pptx / .csv 文件。",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn render_dispatches_by_format() {
        assert_eq!(render(b"PK\x03\x04word/document.xml").format, Format::Docx);
        assert_eq!(render(b"PK\x03\x04xl/workbook.xml").format, Format::Xlsx);
        assert_eq!(
            render(b"PK\x03\x04ppt/presentation.xml").format,
            Format::Pptx
        );
    }

    #[test]
    fn render_guides_csv_to_the_sheet_page() {
        let result = render(b"a,b\n1,2\n");
        assert_eq!(result.format, Format::Csv);
        assert!(!result.ok);
        assert!(result.message.contains("表格"));
    }

    #[test]
    fn render_unknown_for_garbage() {
        let result = render(b"not an office file");
        assert_eq!(result.format, Format::Unknown);
        assert_eq!(result.byte_len, 18);
    }
}
