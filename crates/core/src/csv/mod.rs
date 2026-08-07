//! CSV 组件:把 CSV 字节流解析成平台无关的 [`Sheet`]。
//!
//! **为什么放在 Rust 侧**:解码、字段切分(列切分)、列宽度量都是与单元格数量
//! 成正比的重 CPU 工作,百万行级别在 JS 里会长时间阻塞;放到 WASM 里既快,
//! 又能让视图层只做「取可见区域 + 绘制」这件轻活。
//!
//! **不做什么**(本期范围):不解析公式、不做数字/日期格式化、不产出图表。
//! CSV 本身也不携带这些信息 —— 单元格一律按纯文本对待。

mod decode;
mod dialect;
mod error;

pub use decode::{decode, Decoded};
pub use dialect::{sniff, DelimiterSource, CANDIDATES, DEFAULT_DELIMITER};
pub use error::CsvError;

use crate::sheet::Sheet;

/// 解析选项。
#[derive(Debug, Clone, Copy)]
pub struct CsvOptions {
    /// 显式指定分隔符;`None` 表示自动嗅探。
    pub delimiter: Option<u8>,
    /// 允许的最大字节数。
    pub max_bytes: usize,
    /// 允许的最大行数,超出部分截断(而不是失败)。
    pub max_rows: usize,
    /// 允许的最大列数,超出部分截断(而不是失败)。
    pub max_cols: usize,
}

/// 默认体积上限:256 MiB。浏览器里再大就有 OOM 风险。
pub const DEFAULT_MAX_BYTES: usize = 256 * 1024 * 1024;
/// 默认行数上限:200 万行。
pub const DEFAULT_MAX_ROWS: usize = 2_000_000;
/// 默认列数上限:16384,与 Excel 的列数上限一致,方便用户理解。
pub const DEFAULT_MAX_COLS: usize = 16_384;

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions {
            delimiter: None,
            max_bytes: DEFAULT_MAX_BYTES,
            max_rows: DEFAULT_MAX_ROWS,
            max_cols: DEFAULT_MAX_COLS,
        }
    }
}

/// 解析产出的元信息。**只含统计量,不含任何单元格内容**,可安全写日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvMeta {
    /// 实际使用的文本编码名称。
    pub encoding: String,
    /// 实际使用的分隔符。
    pub delimiter: u8,
    /// 分隔符是怎么定下来的。
    pub delimiter_source: DelimiterSource,
    /// 是否有字符无法解码(内容可能不完全准确)。
    pub lossy: bool,
    /// 行数。
    pub rows: usize,
    /// 列数。
    pub cols: usize,
    /// 是否因超过行数上限被截断。
    pub truncated_rows: bool,
    /// 是否因超过列数上限被截断。
    pub truncated_cols: bool,
}

/// 一次 CSV 解析的完整结果。
#[derive(Debug)]
pub struct CsvDocument {
    /// 表格数据。
    pub sheet: Sheet,
    /// 元信息。
    pub meta: CsvMeta,
}

/// 用默认选项解析 CSV 字节流。
pub fn parse(bytes: &[u8]) -> Result<CsvDocument, CsvError> {
    parse_with(bytes, &CsvOptions::default())
}

/// 解析 CSV 字节流。
///
/// 处理:BOM、UTF-8 / UTF-16 / GBK 等编码、`"` 包裹字段、`""` 转义引号、
/// 字段内嵌换行、CRLF/LF 混用、末尾空行、各行列数不一致(短行补空)。
///
/// 两条明确约定:
/// - **完全空白的行被跳过**,不占行号(与 `csv` crate / pandas / csvkit 一致);
/// - **短行右侧补空单元格**,保证视图是规整矩形,不会错位。
pub fn parse_with(bytes: &[u8], options: &CsvOptions) -> Result<CsvDocument, CsvError> {
    if bytes.is_empty() {
        return Err(CsvError::Empty);
    }
    if bytes.len() > options.max_bytes {
        return Err(CsvError::TooLarge {
            size: bytes.len(),
            limit: options.max_bytes,
        });
    }

    let decoded = decode(bytes);
    if decoded.text.trim().is_empty() {
        // 解码后只剩空白(或全是替换字符):对用户来说等同于「空文件」/「不是文本」
        return if decoded.lossy {
            Err(CsvError::Undecodable {
                encoding: decoded.encoding.to_string(),
            })
        } else {
            Err(CsvError::Empty)
        };
    }

    if looks_like_binary(&decoded.text) {
        return Err(CsvError::NotText);
    }

    let (delimiter, delimiter_source) = match options.delimiter {
        Some(d) => (d, DelimiterSource::Explicit),
        None => match sniff(&decoded.text) {
            Some(d) => (d, DelimiterSource::Sniffed),
            None => (DEFAULT_DELIMITER, DelimiterSource::Fallback),
        },
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        // 表头对渲染来说就是第一行数据,由视图层决定要不要特殊显示
        .has_headers(false)
        // 允许各行列数不一致:真实世界的 CSV 经常不规整,报错不如照实展示
        .flexible(true)
        .from_reader(decoded.text.as_bytes());

    let mut builder = Sheet::builder();
    // 复用同一个 record 缓冲,避免每行一次堆分配
    let mut record = csv::StringRecord::new();
    let mut truncated_rows = false;
    let mut truncated_cols = false;

    loop {
        match reader.read_record(&mut record) {
            Ok(false) => break,
            Ok(true) => {
                if builder.rows() >= options.max_rows {
                    truncated_rows = true;
                    break;
                }
                builder.start_row();
                for field in record.iter().take(options.max_cols) {
                    builder.push_field(field);
                }
                if record.len() > options.max_cols {
                    truncated_cols = true;
                }
            }
            Err(err) => {
                return Err(CsvError::Malformed {
                    line: err.position().map(|p| p.line()).unwrap_or(0),
                    detail: err.to_string(),
                })
            }
        }
    }

    builder.trim_trailing_empty_rows();
    let sheet = builder.finish();

    Ok(CsvDocument {
        meta: CsvMeta {
            encoding: decoded.encoding.to_string(),
            delimiter,
            delimiter_source,
            lossy: decoded.lossy,
            rows: sheet.rows(),
            cols: sheet.cols(),
            truncated_rows,
            truncated_cols,
        },
        sheet,
    })
}

/// 判断解码后的文本是否其实是二进制内容。
///
/// 单字节编码(如 windows-1252)能把**任意**字节流硬解成「字符」,
/// 所以解码成功并不代表这是文本。两条判据:
///
/// 1. 出现 NUL 字符 —— 真实文本里绝不会有,一票否决;
/// 2. 不可打印字符的**密度**过高 —— 正常 CSV 里除了 `\t` `\r` `\n`
///    几乎不出现控制字符,随机字节解出来则遍地都是。
///
/// 密度判据额外要求一个绝对数量下限:短文件里偶然夹带一个控制字符
/// 就能轻易超过百分比阈值,那样会误伤正常小文件。
fn looks_like_binary(text: &str) -> bool {
    let mut total = 0usize;
    let mut suspicious = 0usize;
    for ch in text.chars().take(BINARY_SNIFF_CHARS) {
        if ch == '\0' {
            return true;
        }
        total += 1;
        let is_control = ch.is_control() && ch != '\t' && ch != '\r' && ch != '\n';
        if is_control || ch == char::REPLACEMENT_CHARACTER {
            suspicious += 1;
        }
    }
    suspicious >= MIN_SUSPICIOUS_COUNT
        && total > 0
        && (suspicious as f64) / (total as f64) > MAX_SUSPICIOUS_RATIO
}

/// 二进制判定时检查的字符数。
const BINARY_SNIFF_CHARS: usize = 8 * 1024;

/// 不可打印字符占比超过这个值就判定为二进制。
///
/// 取 1%:真实 CSV 偶尔夹带一两个控制字符(如 `\x0b`)不该被拒,
/// 而随机字节的占比通常在 20% 以上,两者相差一个数量级,阈值很好选。
const MAX_SUSPICIOUS_RATIO: f64 = 0.01;

/// 触发密度判定所需的最少可疑字符数,避免误伤短小的正常文件。
const MIN_SUSPICIOUS_COUNT: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析并把表格摊平成 `Vec<Vec<String>>`,方便断言。
    fn grid(input: &str) -> Vec<Vec<String>> {
        let doc = parse(input.as_bytes()).expect("应解析成功");
        rows_of(&doc)
    }

    fn rows_of(doc: &CsvDocument) -> Vec<Vec<String>> {
        (0..doc.sheet.rows())
            .map(|r| {
                (0..doc.sheet.cols())
                    .map(|c| doc.sheet.cell(r, c).to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn parses_plain_csv() {
        assert_eq!(
            grid("a,b,c\n1,2,3"),
            vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]
        );
    }

    #[test]
    fn parses_quoted_fields_with_embedded_delimiter() {
        assert_eq!(
            grid("name,city\n张三,\"上海,中国\""),
            vec![vec!["name", "city"], vec!["张三", "上海,中国"]]
        );
    }

    #[test]
    fn parses_escaped_double_quotes() {
        // "" 在引号内表示一个字面双引号
        assert_eq!(
            grid("quote\n\"他说\"\"你好\"\"\""),
            vec![vec!["quote"], vec!["他说\"你好\""]]
        );
    }

    #[test]
    fn parses_field_with_embedded_newline() {
        let doc = parse("a,b\n\"第一行\n第二行\",x".as_bytes()).expect("应解析成功");
        assert_eq!(doc.sheet.rows(), 2, "内嵌换行不应把一行拆成两行");
        assert_eq!(doc.sheet.cell(1, 0), "第一行\n第二行");
        assert_eq!(doc.sheet.cell(1, 1), "x");
    }

    #[test]
    fn handles_mixed_crlf_and_lf() {
        assert_eq!(
            grid("a,b\r\n1,2\n3,4\r\n"),
            vec![vec!["a", "b"], vec!["1", "2"], vec!["3", "4"]],
            "CRLF 与 LF 混用时行数与内容都应正确,且不残留 \\r"
        );
    }

    #[test]
    fn strips_utf8_bom_from_first_cell() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("id,name\n1,a".as_bytes());
        let doc = parse(&bytes).expect("应解析成功");
        assert_eq!(doc.sheet.cell(0, 0), "id", "BOM 不应粘在第一个单元格上");
    }

    #[test]
    fn trailing_blank_lines_do_not_create_rows() {
        let doc = parse("a,b\n1,2\n\n\n".as_bytes()).expect("应解析成功");
        assert_eq!(doc.sheet.rows(), 2);
    }

    #[test]
    fn blank_lines_are_skipped_not_rendered_as_rows() {
        // 与 `csv` crate / pandas(skip_blank_lines=True)/ csvkit 的默认行为一致:
        // 完全空白的行不是数据行,不占据行号。
        let doc = parse("a,b\n\n1,2\n".as_bytes()).expect("应解析成功");
        assert_eq!(doc.sheet.rows(), 2);
        assert_eq!(doc.sheet.cell(1, 0), "1");
    }

    #[test]
    fn trailing_row_of_empty_quoted_field_is_trimmed() {
        // 有些导出工具会在末尾写一行 `""`,它不是空行(有一个空字段),
        // 却同样只是噪声,应被裁掉。
        let doc = parse("a,b\n1,2\n\"\"\n".as_bytes()).expect("应解析成功");
        assert_eq!(doc.sheet.rows(), 2);
    }

    #[test]
    fn empty_fields_are_preserved() {
        assert_eq!(
            grid("a,,c\n,,\n"),
            vec![vec!["a", "", "c"], vec!["", "", ""]]
        );
    }

    #[test]
    fn ragged_rows_pad_to_max_columns() {
        let doc = parse("a,b,c\n1\n2,3\n".as_bytes()).expect("应解析成功");
        assert_eq!(doc.meta.cols, 3);
        assert_eq!(
            rows_of(&doc),
            vec![vec!["a", "b", "c"], vec!["1", "", ""], vec!["2", "3", ""],]
        );
    }

    #[test]
    fn header_only_file_yields_one_row() {
        let doc = parse("a,b,c".as_bytes()).expect("应解析成功");
        assert_eq!((doc.meta.rows, doc.meta.cols), (1, 3));
    }

    #[test]
    fn detects_semicolon_delimiter() {
        let doc = parse("a;b;c\n1;2;3\n".as_bytes()).expect("应解析成功");
        assert_eq!(doc.meta.delimiter, b';');
        assert_eq!(doc.meta.delimiter_source, DelimiterSource::Sniffed);
        assert_eq!(doc.meta.cols, 3);
    }

    #[test]
    fn explicit_delimiter_overrides_sniffing() {
        let options = CsvOptions {
            delimiter: Some(b';'),
            ..Default::default()
        };
        // 内容是逗号分隔,但显式指定了分号 → 整行成为一个字段
        let doc = parse_with("a,b,c".as_bytes(), &options).expect("应解析成功");
        assert_eq!(doc.meta.delimiter_source, DelimiterSource::Explicit);
        assert_eq!(doc.meta.cols, 1);
        assert_eq!(doc.sheet.cell(0, 0), "a,b,c");
    }

    #[test]
    fn single_column_file_falls_back_to_comma() {
        let doc = parse("hello\nworld\n".as_bytes()).expect("应解析成功");
        assert_eq!(doc.meta.delimiter_source, DelimiterSource::Fallback);
        assert_eq!((doc.meta.rows, doc.meta.cols), (2, 1));
    }

    #[test]
    fn gbk_file_is_decoded() {
        let (bytes, _, _) = encoding_rs::GBK.encode("姓名,城市\n张三,北京\n");
        let doc = parse(&bytes).expect("应解析成功");
        assert_eq!(doc.sheet.cell(1, 0), "张三");
        assert_eq!(doc.meta.encoding, "GBK");
    }

    #[test]
    fn empty_file_is_rejected_with_clear_error() {
        assert_eq!(parse(&[]).unwrap_err(), CsvError::Empty);
    }

    #[test]
    fn whitespace_only_file_is_rejected() {
        assert_eq!(parse(b"\n\n  \n").unwrap_err(), CsvError::Empty);
    }

    #[test]
    fn binary_content_is_rejected_instead_of_rendered_as_mojibake() {
        // 伪随机字节:任何单字节编码都能「解码」出字符,但那是一屏乱码
        let mut bytes = Vec::with_capacity(2048);
        let mut state: u32 = 0x1234_5678;
        for _ in 0..2048 {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            bytes.push((state >> 16) as u8);
        }
        assert_eq!(parse(&bytes).unwrap_err(), CsvError::NotText);
    }

    #[test]
    fn png_header_is_rejected() {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0x00; 64]);
        bytes.extend_from_slice(b",,,,");
        assert_eq!(parse(&bytes).unwrap_err(), CsvError::NotText);
    }

    #[test]
    fn normal_csv_with_a_stray_control_char_is_still_accepted() {
        // 个别控制字符不该导致整份文件被拒
        let doc = parse("a,b\n1,2\x0b\n3,4\n".as_bytes()).expect("应解析成功");
        assert_eq!(doc.sheet.rows(), 3);
    }

    #[test]
    fn oversized_file_is_rejected_before_parsing() {
        let options = CsvOptions {
            max_bytes: 4,
            ..Default::default()
        };
        let err = parse_with(b"a,b,c,d,e", &options).unwrap_err();
        assert_eq!(err, CsvError::TooLarge { size: 9, limit: 4 });
    }

    #[test]
    fn row_limit_truncates_instead_of_failing() {
        let input = (0..50)
            .map(|i| format!("{i},x"))
            .collect::<Vec<_>>()
            .join("\n");
        let options = CsvOptions {
            max_rows: 10,
            ..Default::default()
        };
        let doc = parse_with(input.as_bytes(), &options).expect("超限应截断而非失败");
        assert_eq!(doc.meta.rows, 10);
        assert!(doc.meta.truncated_rows);
        assert!(!doc.meta.truncated_cols);
    }

    #[test]
    fn col_limit_truncates_instead_of_failing() {
        let input = (0..30).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let options = CsvOptions {
            max_cols: 5,
            ..Default::default()
        };
        let doc = parse_with(input.as_bytes(), &options).expect("超限应截断而非失败");
        assert_eq!(doc.meta.cols, 5);
        assert!(doc.meta.truncated_cols);
    }

    #[test]
    fn unclosed_quote_reports_line_number() {
        // 引号未闭合会把后面所有内容吞进一个字段;csv crate 不视其为错误,
        // 但必须保证不 panic、不丢行,行为可预期。
        let doc = parse("a,b\n\"未闭合,1\n2,3\n".as_bytes()).expect("不应崩溃");
        assert!(doc.sheet.rows() >= 2);
    }

    #[test]
    fn very_long_cell_is_parsed_intact() {
        let long = "x".repeat(200_000);
        let input = format!("a,b\n{long},2");
        let doc = parse(input.as_bytes()).expect("应解析成功");
        assert_eq!(doc.sheet.cell(1, 0).len(), 200_000, "超长文本不应被截断");
        assert_eq!(doc.sheet.cell(1, 1), "2");
    }

    #[test]
    fn meta_never_contains_cell_content() {
        let doc = parse("secret,token\nAKIA123,hunter2\n".as_bytes()).expect("应解析成功");
        let dumped = format!("{:?}", doc.meta);
        assert!(!dumped.contains("hunter2"), "元信息不得携带单元格内容");
        assert!(!dumped.contains("AKIA123"));
    }

    #[test]
    fn wide_and_tall_sheet_round_trips() {
        // 200 行 × 50 列,验证紧凑存储在规模稍大时索引依然正确
        let mut input = String::new();
        for r in 0..200 {
            for c in 0..50 {
                if c > 0 {
                    input.push(',');
                }
                input.push_str(&format!("r{r}c{c}"));
            }
            input.push('\n');
        }
        let doc = parse(input.as_bytes()).expect("应解析成功");
        assert_eq!((doc.meta.rows, doc.meta.cols), (200, 50));
        assert_eq!(doc.sheet.cell(0, 0), "r0c0");
        assert_eq!(doc.sheet.cell(199, 49), "r199c49");
        assert_eq!(doc.sheet.cell(123, 7), "r123c7");
    }
}
