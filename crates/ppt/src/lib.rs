//! **PowerPoint (.pptx) 只读解析**。
//!
//! 直接用 `zip` + `quick-xml`(经 `office-ooxml`)解析 PresentationML → 幻灯模型
//! (文本框 / 图片 / 自选图形 / 表格 / 图表 / 动画时间线)。对 `office-core` 零依赖 ——
//! 只用到 `office-ooxml` 的 chart 与容器原语。
//!
//! 见 [RFC-0006](../../../docs/rfcs/0006-word-excel-ppt-readonly.md)。

pub mod pptx;

/// 字节是否像一个 .pptx —— OPC 包内含 `ppt/presentation.xml`。
pub fn can_open(bytes: &[u8]) -> bool {
    office_ooxml::has_entry(bytes, "ppt/presentation.xml")
}

#[cfg(test)]
mod tests {
    #[test]
    fn can_open_rejects_non_pptx() {
        assert!(!super::can_open(b"not a zip"));
    }
}
