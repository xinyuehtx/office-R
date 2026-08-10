//! **Excel (.xlsx) 只读解析**。
//!
//! 用 `calamine` 取工作表的缓存计算值(公式不重算),自解析 `xl/styles.xml` 等
//! 侧部件补齐 numfmt 格式码、单元格样式、合并区、内嵌图片/图表、列宽/冻结、迷你图。
//! 产出 [`office_core::Sheet`],与 CSV 走同一渲染管线。
//!
//! 这是三个格式 crate 里**唯一**依赖 `office-core` 的 —— 因为它复用表格内核
//! (`Sheet` / numfmt / serial / limits,以及公式的 `CellFormula` 结构体)。
//!
//! 见 [RFC-0006](../../../docs/rfcs/0006-word-excel-ppt-readonly.md)。

pub mod xlsx;

/// 字节是否像一个 .xlsx —— OPC 包内含 `xl/workbook.xml`。
///
/// 只答 xlsx;CSV 的识别仍在 [`office_core::detect_format`]。
pub fn can_open(bytes: &[u8]) -> bool {
    office_ooxml::has_entry(bytes, "xl/workbook.xml")
}

#[cfg(test)]
mod tests {
    #[test]
    fn can_open_rejects_non_xlsx() {
        assert!(!super::can_open(b"not a zip"));
    }
}
