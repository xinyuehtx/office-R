//! PowerPoint (.pptx) 组件 —— 基于 zip + quick-xml 的最小真实解析。
//!
//! pptx 无成熟的专用解析 crate,这里用 zip 打开容器、统计幻灯片数量,
//! 再用 quick-xml 解析首张幻灯片、统计文本块(`<a:t>`)数量。

use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use zip::ZipArchive;

use crate::format::Format;
use crate::render::RenderResult;

/// 读取 pptx 字节,解析并返回摘要。失败时优雅降级。
pub fn render(bytes: &[u8]) -> RenderResult {
    match parse(bytes) {
        Ok(msg) => RenderResult::ok(Format::Pptx, bytes.len(), msg),
        Err(e) => RenderResult::err(
            Format::Pptx,
            bytes.len(),
            format!("PowerPoint 解析失败:{e}"),
        ),
    }
}

fn parse(bytes: &[u8]) -> Result<String, String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes.to_vec())).map_err(|e| e.to_string())?;

    // 收集幻灯片文件名(ppt/slides/slideN.xml)
    let mut slides: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .collect();
    slides.sort();
    let slide_count = slides.len();

    // 统计首张幻灯片的文本块数量
    let mut text_runs = 0usize;
    if let Some(first) = slides.first() {
        let mut xml = String::new();
        zip.by_name(first)
            .map_err(|e| e.to_string())?
            .read_to_string(&mut xml)
            .map_err(|e| e.to_string())?;
        text_runs = count_text_runs(&xml)?;
    }

    Ok(format!(
        "已解析 PowerPoint:{slide_count} 张幻灯片,首张含 {text_runs} 个文本块。"
    ))
}

/// 统计 XML 中 `<a:t>` 文本元素数量。
fn count_text_runs(xml: &str) -> Result<usize, String> {
    let mut reader = XmlReader::from_str(xml);
    let mut count = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.name().as_ref() == b"a:t" => count += 1,
            Ok(Event::Eof) => break,
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
        buf.clear();
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    #[test]
    fn invalid_bytes_degrade_gracefully() {
        let result = render(b"PK\x03\x04ppt/");
        assert_eq!(result.format, Format::Pptx);
        assert_eq!(result.byte_len, 8);
        assert!(!result.ok);
        assert!(!result.message.is_empty());
    }

    #[test]
    fn parses_real_pptx_slide() {
        // 构造一个含单张幻灯片、两个文本块的最小 pptx(未压缩)
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            writer.start_file("ppt/slides/slide1.xml", opts).unwrap();
            writer
                .write_all(br#"<p:sld xmlns:a="a"><a:t>Hello</a:t><a:t>World</a:t></p:sld>"#)
                .unwrap();
            writer.finish().unwrap();
        }

        let result = render(&buf);
        assert_eq!(result.format, Format::Pptx);
        assert!(result.ok, "{}", result.message);
        assert!(result.message.contains("1 张幻灯片"), "{}", result.message);
        assert!(result.message.contains("2 个文本块"), "{}", result.message);
    }

    #[test]
    fn counts_text_runs() {
        let xml = r#"<root xmlns:a="a"><a:t>a</a:t><a:t>b</a:t><a:t>c</a:t></root>"#;
        assert_eq!(count_text_runs(xml).unwrap(), 3);
    }
}
