//! 渲染结果的通用数据结构。
//!
//! 各组件解析 office 文件后,以本结构返回摘要与状态。
//! 后续可在此扩展承载更丰富的解析内容(段落 / 单元格 / 幻灯片等)。

use serde::{Deserialize, Serialize};

use crate::format::Format;

/// 一次「读取并渲染」的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderResult {
    /// 识别出的格式。
    pub format: Format,
    /// 格式的中文名称(方便前端直接展示)。
    pub format_name: String,
    /// 文件字节数。
    pub byte_len: usize,
    /// 解析摘要 / 状态说明(成功摘要或失败原因)。
    pub message: String,
    /// 是否解析成功(false 表示 message 为失败原因)。
    pub ok: bool,
}

impl RenderResult {
    /// 构造一个成功结果。
    pub fn ok(format: Format, byte_len: usize, message: impl Into<String>) -> Self {
        RenderResult {
            format,
            format_name: format.display_name().to_string(),
            byte_len,
            message: message.into(),
            ok: true,
        }
    }

    /// 构造一个失败结果(携带原因)。
    pub fn err(format: Format, byte_len: usize, message: impl Into<String>) -> Self {
        RenderResult {
            format,
            format_name: format.display_name().to_string(),
            byte_len,
            message: message.into(),
            ok: false,
        }
    }
}
