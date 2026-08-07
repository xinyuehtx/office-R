//! CSV 解析错误。
//!
//! 错误信息面向**最终用户**(中文、可操作),并尽量带上定位信息(行号),
//! 便于用户回到原文件排查。视图层据此优雅降级并允许重新选择文件。

use thiserror::Error;

/// CSV 解析过程中可能出现的错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CsvError {
    /// 文件没有任何内容(0 字节,或只有空白字符)。
    #[error("文件内容为空,没有可显示的数据")]
    Empty,

    /// 文件体积超过上限。浏览器内解析受内存限制,必须有硬上限。
    #[error("文件过大({size} 字节),超过 {limit} 字节上限,请拆分后再打开")]
    TooLarge {
        /// 实际字节数。
        size: usize,
        /// 允许的上限字节数。
        limit: usize,
    },

    /// 字节流无法解码为文本(既不是 UTF-8,也无法按探测到的编码解码)。
    #[error("无法把文件解码为文本(探测到的编码:{encoding}),请确认这是一个 CSV 文本文件")]
    Undecodable {
        /// 探测到的编码名称。
        encoding: String,
    },

    /// 内容能「解码」出字符,但明显是二进制文件(含大量不可打印字符)。
    ///
    /// 单独一种错误是有必要的:任何字节流都能被单字节编码硬解成字符,
    /// 不拦住的话用户会看到一屏乱码,还以为是渲染坏了。
    #[error("这看起来不是文本文件(含大量不可打印字符),请确认上传的是 CSV 文件")]
    NotText,

    /// CSV 结构损坏(如引号未闭合导致的解析失败),带出错行号。
    #[error("第 {line} 行解析失败:{detail}")]
    Malformed {
        /// 出错所在的物理行号(从 1 开始)。
        line: u64,
        /// 底层解析器给出的原因。
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_chinese_and_actionable() {
        assert_eq!(CsvError::Empty.to_string(), "文件内容为空,没有可显示的数据");

        let too_large = CsvError::TooLarge {
            size: 1024,
            limit: 512,
        };
        assert!(too_large.to_string().contains("1024"));
        assert!(too_large.to_string().contains("512"));
    }

    #[test]
    fn malformed_carries_line_number() {
        let err = CsvError::Malformed {
            line: 42,
            detail: "引号未闭合".into(),
        };
        assert!(err.to_string().contains("第 42 行"), "错误应带行号便于定位");
        assert!(err.to_string().contains("引号未闭合"));
    }
}
