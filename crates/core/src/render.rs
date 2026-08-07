//! 渲染结果的通用数据结构。
//!
//! 骨架阶段各组件只返回文件元信息与占位说明;后续实现真实解析时,
//! 在此结构上扩展承载解析后的内容(段落 / 单元格 / 幻灯片等)。

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
    /// 占位 / 状态说明。
    pub message: String,
}

impl RenderResult {
    /// 构造一个占位结果。
    pub fn placeholder(format: Format, byte_len: usize, message: impl Into<String>) -> Self {
        RenderResult {
            format,
            format_name: format.display_name().to_string(),
            byte_len,
            message: message.into(),
        }
    }
}
