//! Word (.docx) 组件 —— 骨架占位。

use crate::format::Format;
use crate::render::RenderResult;

/// 读取 docx 字节并返回渲染结果。
///
/// 骨架阶段仅返回元信息与占位说明,尚未解析 `word/document.xml`。
pub fn render_placeholder(bytes: &[u8]) -> RenderResult {
    RenderResult::placeholder(
        Format::Docx,
        bytes.len(),
        "Word 文档解析尚未实现:后续将解析 word/document.xml 并渲染段落。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_reports_format_and_len() {
        let result = render_placeholder(b"PK\x03\x04word/");
        assert_eq!(result.format, Format::Docx);
        assert_eq!(result.byte_len, 9);
        assert!(!result.message.is_empty());
    }
}
