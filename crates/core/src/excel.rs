//! Excel (.xlsx) 组件 —— 骨架占位。

use crate::format::Format;
use crate::render::RenderResult;

/// 读取 xlsx 字节并返回渲染结果。
///
/// 骨架阶段仅返回元信息与占位说明,尚未解析 `xl/workbook.xml` 与公式内核。
pub fn render_placeholder(bytes: &[u8]) -> RenderResult {
    RenderResult::placeholder(
        Format::Xlsx,
        bytes.len(),
        "Excel 表格解析尚未实现:后续将解析 xl/ 并接入公式计算内核。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_reports_format_and_len() {
        let result = render_placeholder(b"PK\x03\x04xl/");
        assert_eq!(result.format, Format::Xlsx);
        assert_eq!(result.byte_len, 7);
        assert!(!result.message.is_empty());
    }
}
