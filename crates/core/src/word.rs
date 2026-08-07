//! Word (.docx) 组件 —— 基于 docx-rs 的最小真实解析。

use crate::format::Format;
use crate::render::RenderResult;

/// 读取 docx 字节,解析并返回摘要。
///
/// 目前提取顶层段落数作为最小真实解析验证;失败时优雅降级为错误结果。
pub fn render(bytes: &[u8]) -> RenderResult {
    match docx_rs::read_docx(bytes) {
        Ok(docx) => {
            let paragraphs = docx
                .document
                .children
                .iter()
                .filter(|c| matches!(c, docx_rs::DocumentChild::Paragraph(_)))
                .count();
            RenderResult::ok(
                Format::Docx,
                bytes.len(),
                format!("已解析 Word 文档:顶层含 {paragraphs} 个段落。"),
            )
        }
        Err(e) => RenderResult::err(Format::Docx, bytes.len(), format!("Word 解析失败:{e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_bytes_degrade_gracefully() {
        let result = render(b"PK\x03\x04word/");
        assert_eq!(result.format, Format::Docx);
        assert_eq!(result.byte_len, 9);
        assert!(!result.ok);
        assert!(!result.message.is_empty());
    }
}
