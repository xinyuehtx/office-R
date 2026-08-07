//! 文本编码探测与解码。
//!
//! CSV 是纯文本格式,却**没有**任何地方声明自己的编码。中文环境里
//! GBK/GB18030 导出的 CSV 极为常见(Excel 默认另存即是),
//! 直接按 UTF-8 读会得到满屏乱码。因此这里的策略是:
//!
//! 1. 有 BOM(UTF-8 / UTF-16LE / UTF-16BE)→ 按 BOM 指定的编码解码;
//! 2. 无 BOM 且是合法 UTF-8 → **零拷贝**借用原始字节(绝大多数情况);
//! 3. 否则用 `chardetng`(Firefox 的编码探测器)猜测,再用 `encoding_rs` 解码。
//!
//! 解码失败不会中断:`encoding_rs` 会以替换字符兜底,并通过 `lossy` 标记
//! 告知上层「有字符解码不出来」,由视图层提示用户。

use std::borrow::Cow;

use chardetng::{Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_8};

/// 探测阶段最多读取的字节数。
///
/// 编码探测只需要样本,对 100MB 的文件全量扫描既慢又没有额外收益。
const DETECT_SAMPLE_BYTES: usize = 256 * 1024;

/// 解码结果。
#[derive(Debug)]
pub struct Decoded<'a> {
    /// 解码后的文本。UTF-8 输入时为借用,不产生拷贝。
    pub text: Cow<'a, str>,
    /// 实际使用的编码名称(如 `UTF-8`、`GBK`)。
    pub encoding: &'static str,
    /// 是否有字符无法解码而被替换(内容可能不完全准确)。
    pub lossy: bool,
}

/// 把字节流解码为文本。
///
/// 该函数**不会失败**:最坏情况下以替换字符兜底并置 `lossy = true`,
/// 由调用方决定是提示还是拒绝。
pub fn decode(bytes: &[u8]) -> Decoded<'_> {
    // 1. BOM 优先:BOM 是文件自述的编码,比任何统计探测都可靠
    if let Some((encoding, bom_len)) = Encoding::for_bom(bytes) {
        let (text, _, lossy) = encoding.decode(&bytes[bom_len..]);
        return Decoded {
            text,
            encoding: encoding.name(),
            lossy,
        };
    }

    // 2. 合法 UTF-8:直接借用,零拷贝
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Decoded {
            text: Cow::Borrowed(text),
            encoding: UTF_8.name(),
            lossy: false,
        };
    }

    // 3. 兜底:统计探测 + 解码
    //
    // 走到这里说明字节流已经不是合法 UTF-8,所以 UTF-8 也没必要作为候选;
    // ISO-2022-JP 是带转义序列的有状态编码,CSV 场景几乎不会遇到,一并排除,
    // 让探测器专注在 GBK / Big5 / Shift_JIS / 各种单字节编码上。
    let mut detector = chardetng::EncodingDetector::new(Iso2022JpDetection::Deny);
    let sample_len = bytes.len().min(DETECT_SAMPLE_BYTES);
    let exhaustive = sample_len == bytes.len();
    detector.feed(&bytes[..sample_len], exhaustive);
    let encoding = detector.guess(None, Utf8Detection::Deny);
    let (text, _, lossy) = encoding.decode(bytes);
    Decoded {
        text,
        encoding: encoding.name(),
        lossy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8_is_borrowed_without_copy() {
        let decoded = decode("姓名,年龄\n张三,18".as_bytes());
        assert!(
            matches!(decoded.text, Cow::Borrowed(_)),
            "合法 UTF-8 应零拷贝借用"
        );
        assert_eq!(decoded.encoding, "UTF-8");
        assert!(!decoded.lossy);
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("a,b".as_bytes());
        let decoded = decode(&bytes);
        assert_eq!(decoded.text, "a,b", "BOM 不应出现在首个单元格里");
        assert_eq!(decoded.encoding, "UTF-8");
    }

    #[test]
    fn utf16le_bom_is_decoded() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "a,中".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let decoded = decode(&bytes);
        assert_eq!(decoded.text, "a,中");
        assert_eq!(decoded.encoding, "UTF-16LE");
    }

    #[test]
    fn utf16be_bom_is_decoded() {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in "x,y".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        let decoded = decode(&bytes);
        assert_eq!(decoded.text, "x,y");
        assert_eq!(decoded.encoding, "UTF-16BE");
    }

    #[test]
    fn gbk_without_bom_is_detected() {
        // 「姓名,年龄」的 GBK 编码
        let (bytes, _, _) = encoding_rs::GBK.encode("姓名,年龄\n张三,18\n李四,20");
        assert!(std::str::from_utf8(&bytes).is_err(), "样本确实不是 UTF-8");

        let decoded = decode(&bytes);
        assert!(
            decoded.text.contains("姓名"),
            "应按 GBK 正确解码,实际:{}",
            decoded.text
        );
        assert!(matches!(decoded.text, Cow::Owned(_)), "非 UTF-8 需要转换");
    }

    #[test]
    fn undecodable_bytes_degrade_to_replacement_chars() {
        // 孤立的 0xFF 在任何编码下都无法构成有效文本,不应 panic
        let decoded = decode(&[0xFF, 0xFF, 0xFF, b',', 0xFE]);
        assert!(!decoded.text.is_empty(), "应有兜底文本而非崩溃");
    }

    #[test]
    fn empty_input_is_handled() {
        let decoded = decode(&[]);
        assert_eq!(decoded.text, "");
        assert!(!decoded.lossy);
    }
}
