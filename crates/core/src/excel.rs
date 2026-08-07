//! Excel (.xlsx) 组件 —— 基于 calamine 的最小真实解析。

use std::io::Cursor;

use calamine::{open_workbook_from_rs, Reader, Xlsx};

use crate::format::Format;
use crate::render::RenderResult;

/// 读取 xlsx 字节,解析并返回摘要。
///
/// 目前提取工作表数量与首表尺寸作为最小真实解析验证;失败时优雅降级。
pub fn render(bytes: &[u8]) -> RenderResult {
    match parse(bytes) {
        Ok(msg) => RenderResult::ok(Format::Xlsx, bytes.len(), msg),
        Err(e) => RenderResult::err(Format::Xlsx, bytes.len(), format!("Excel 解析失败:{e}")),
    }
}

fn parse(bytes: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(bytes.to_vec());
    let mut workbook: Xlsx<_> =
        open_workbook_from_rs(cursor).map_err(|e: calamine::XlsxError| e.to_string())?;
    let names = workbook.sheet_names().to_owned();
    let sheet_count = names.len();
    let (first, rows, cols) = match names.first() {
        Some(name) => {
            let (rows, cols) = workbook
                .worksheet_range(name)
                .map(|r| r.get_size())
                .unwrap_or((0, 0));
            (name.clone(), rows, cols)
        }
        None => (String::new(), 0, 0),
    };
    Ok(format!(
        "已解析 Excel:{sheet_count} 个工作表,首表「{first}」{rows} 行 × {cols} 列。"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_bytes_degrade_gracefully() {
        let result = render(b"PK\x03\x04xl/");
        assert_eq!(result.format, Format::Xlsx);
        assert_eq!(result.byte_len, 7);
        assert!(!result.ok);
        assert!(!result.message.is_empty());
    }
}
