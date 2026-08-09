//! Office 文件格式识别。
//!
//! docx/xlsx/pptx (OOXML) 本质都是 ZIP 容器,魔数为 `PK\x03\x04`。
//! 三者通过容器内的特征目录区分:`word/`、`xl/`、`ppt/`。
//! 本模块不做完整 ZIP 解析,仅扫描原始字节中的特征名(ZIP 本地文件头
//! 以未压缩形式存储条目路径),足以支撑「识别」这一骨架能力。
//!
//! CSV 没有魔数,只能靠内容判断:排除二进制(含 NUL 字节)后,
//! 样本里出现候选分隔符即认定为 CSV。宁可漏判也不误判 ——
//! 误判会让用户看到一堆乱码单元格。

use serde::{Deserialize, Serialize};

/// CSV 识别与分隔符嗅探共用的候选分隔符,按优先级排列(同分时的兜底顺序)。
///
/// 定义放在这里而不是 [`crate::csv::dialect`]:识别是**所有格式的共同入口**,
/// 让它零 crate 内依赖,将来把 detect 提成独立单元时就是零成本。
/// 嗅探侧反向引用它。
pub const CANDIDATES: [u8; 4] = *b",\t;|";

/// ZIP 容器魔数(OOXML 文件共有)。
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// CSV 识别时扫描的样本字节数。
const CSV_SNIFF_BYTES: usize = 8 * 1024;

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
    /// 字符分隔文本表格 (.csv / .tsv)
    Csv,
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
            Format::Csv => "CSV 表格",
            Format::Unknown => "未知格式",
        }
    }
}

/// 根据文件字节识别 office 格式。
///
/// 非 ZIP 容器时退回 CSV 文本判定;都不像则返回 [`Format::Unknown`]。
pub fn detect_format(bytes: &[u8]) -> Format {
    if !bytes.starts_with(ZIP_MAGIC) {
        return if looks_like_csv(bytes) {
            Format::Csv
        } else {
            Format::Unknown
        };
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

/// 判断字节流是否像 CSV:是文本(无 NUL)且样本中出现候选分隔符。
fn looks_like_csv(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let sample = &bytes[..bytes.len().min(CSV_SNIFF_BYTES)];

    // NUL 字节是二进制文件的强信号。UTF-16 文本同样含大量 NUL,
    // 但那种情况一定带 BOM,所以带 BOM 时跳过这条判定。
    let has_bom = encoding_rs::Encoding::for_bom(bytes).is_some();
    if !has_bom && sample.contains(&0) {
        return false;
    }
    CANDIDATES.iter().any(|d| sample.contains(d))
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
        assert_eq!(Format::Csv.display_name(), "CSV 表格");
        assert_eq!(Format::Unknown.display_name(), "未知格式");
    }

    #[test]
    fn detects_csv_by_delimiter() {
        assert_eq!(detect_format(b"a,b,c\n1,2,3\n"), Format::Csv);
        assert_eq!(detect_format(b"a;b\n1;2\n"), Format::Csv);
        assert_eq!(detect_format(b"a\tb\n1\t2\n"), Format::Csv);
    }

    #[test]
    fn text_without_delimiter_is_not_csv() {
        // 宁可漏判也不误判:没有分隔符的纯文本不当成表格
        assert_eq!(detect_format(b"just some prose\n"), Format::Unknown);
    }

    #[test]
    fn binary_with_commas_is_not_csv() {
        // 含 NUL 的二进制即便有逗号也不是 CSV
        assert_eq!(
            detect_format(&[0x00, b',', 0x01, b',', 0x02]),
            Format::Unknown
        );
    }

    #[test]
    fn utf16_csv_with_bom_is_detected() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "a,b".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(detect_format(&bytes), Format::Csv, "带 BOM 的 UTF-16 CSV");
    }
}
