//! PowerPoint (.pptx) 组件 —— 骨架占位。

use crate::format::Format;
use crate::render::RenderResult;

/// 读取 pptx 字节并返回渲染结果。
///
/// 骨架阶段仅返回元信息与占位说明,尚未解析 `ppt/presentation.xml` 与幻灯片布局。
pub fn render_placeholder(bytes: &[u8]) -> RenderResult {
    RenderResult::placeholder(
        Format::Pptx,
        bytes.len(),
        "PowerPoint 演示解析尚未实现:后续将解析 ppt/ 并渲染幻灯片。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_reports_format_and_len() {
        let result = render_placeholder(b"PK\x03\x04ppt/");
        assert_eq!(result.format, Format::Pptx);
        assert_eq!(result.byte_len, 8);
        assert!(!result.message.is_empty());
    }
}
