//! Office 文件格式识别。
//!
//! docx/xlsx/pptx (OOXML) 本质都是 ZIP 容器,魔数为 `PK\x03\x04`。
//! 三者通过容器内的特征目录区分:`word/`、`xl/`、`ppt/`。
//! 本模块不做完整 ZIP 解析,仅扫描原始字节中的特征名(ZIP 本地文件头
//! 以未压缩形式存储条目路径),足以支撑「识别」这一骨架能力。

use serde::{Deserialize, Serialize};

/// ZIP 容器魔数(OOXML 文件共有)。
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// 识别出的 office 文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Word 文档 (.docx)
    Docx,
    /// Excel 表格 (.xlsx)
    Xlsx,
    /// PowerPoint 演示 (.pptx)
    Pptx,
    /// 未知 / 不支持
    Unknown,
}

impl Format {
    /// 人类可读的中文名称。
    pub fn display_name(self) -> &'static str {
        match self {
            Format::Docx => "Word 文档",
            Format::Xlsx => "Excel 表格",
            Format::Pptx => "PowerPoint 演示",
            Format::Unknown => "未知格式",
        }
    }
}

/// 根据文件字节识别 office 格式。
///
/// 非 ZIP 容器或无法识别的内容返回 [`Format::Unknown`]。
pub fn detect_format(bytes: &[u8]) -> Format {
    if !bytes.starts_with(ZIP_MAGIC) {
        return Format::Unknown;
    }
    if contains(bytes, b"word/") {
        Format::Docx
    } else if contains(bytes, b"xl/") {
        Format::Xlsx
    } else if contains(bytes, b"ppt/") {
        Format::Pptx
    } else {
        Format::Unknown
    }
}

/// 朴素子串查找(避免引入额外依赖)。
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个带 ZIP 魔数、且包含指定特征目录名的最小样例。
    fn fake_ooxml(entry: &[u8]) -> Vec<u8> {
        let mut v = ZIP_MAGIC.to_vec();
        v.extend_from_slice(entry);
        v
    }

    #[test]
    fn empty_is_unknown() {
        assert_eq!(detect_format(&[]), Format::Unknown);
    }

    #[test]
    fn non_zip_is_unknown() {
        assert_eq!(detect_format(b"hello world"), Format::Unknown);
    }

    #[test]
    fn zip_without_known_entry_is_unknown() {
        assert_eq!(detect_format(&fake_ooxml(b"random/stuff")), Format::Unknown);
    }

    #[test]
    fn detects_docx() {
        assert_eq!(
            detect_format(&fake_ooxml(b"word/document.xml")),
            Format::Docx
        );
    }

    #[test]
    fn detects_xlsx() {
        assert_eq!(detect_format(&fake_ooxml(b"xl/workbook.xml")), Format::Xlsx);
    }

    #[test]
    fn detects_pptx() {
        assert_eq!(
            detect_format(&fake_ooxml(b"ppt/presentation.xml")),
            Format::Pptx
        );
    }

    #[test]
    fn display_names_are_chinese() {
        assert_eq!(Format::Docx.display_name(), "Word 文档");
        assert_eq!(Format::Unknown.display_name(), "未知格式");
    }
}
