//! office-core:平台无关的 office 计算内核。
//!
//! 负责 office 文件的**识别**与(骨架阶段的)**渲染**,不依赖任何浏览器 /
//! 操作系统 API,因此可原生单测,也可被 `office-wasm` 编译进 WASM。

pub mod excel;
pub mod format;
pub mod ppt;
pub mod render;
pub mod word;

pub use format::{detect_format, Format};
pub use render::RenderResult;

/// 内核版本(取自 Cargo 包版本)。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 识别文件格式并分发到对应组件进行(占位)渲染。
///
/// 这是「读取 office 文件 → 识别 → 渲染」的统一入口。
pub fn render(bytes: &[u8]) -> RenderResult {
    match detect_format(bytes) {
        Format::Docx => word::render_placeholder(bytes),
        Format::Xlsx => excel::render_placeholder(bytes),
        Format::Pptx => ppt::render_placeholder(bytes),
        Format::Unknown => RenderResult::placeholder(
            Format::Unknown,
            bytes.len(),
            "无法识别的文件格式,请上传 .docx / .xlsx / .pptx 文件。",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn render_dispatches_by_format() {
        assert_eq!(render(b"PK\x03\x04word/document.xml").format, Format::Docx);
        assert_eq!(render(b"PK\x03\x04xl/workbook.xml").format, Format::Xlsx);
        assert_eq!(
            render(b"PK\x03\x04ppt/presentation.xml").format,
            Format::Pptx
        );
    }

    #[test]
    fn render_unknown_for_garbage() {
        let result = render(b"not an office file");
        assert_eq!(result.format, Format::Unknown);
        assert_eq!(result.byte_len, 18);
    }
}
