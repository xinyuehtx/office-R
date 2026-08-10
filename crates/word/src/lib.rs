//! **Word (.docx) 只读解析**。
//!
//! 基于 `docx-rs` 的读路径抽出平面化文档模型(段落 / run / 标题 / 对齐 / 列表 /
//! 内联图片 / 表格,以及分栏、页眉页脚、修订标记)。对 `office-core` 零依赖 ——
//! Word 与表格内核毫无关系。
//!
//! 见 [RFC-0006](../../../docs/rfcs/0006-word-excel-ppt-readonly.md)。

pub mod docx;

/// 字节是否像一个 .docx —— OPC 包内含 `word/document.xml`。
///
/// 供「一个文件该交给哪个查看器」这类路由用;不依赖 `office-core` 的 detect。
pub fn can_open(bytes: &[u8]) -> bool {
    office_ooxml::has_entry(bytes, "word/document.xml")
}

#[cfg(test)]
mod tests {
    #[test]
    fn can_open_rejects_non_docx() {
        assert!(!super::can_open(b"not a zip"));
        assert!(!super::can_open(b"PK\x03\x04 but no document.xml entry"));
    }
}
