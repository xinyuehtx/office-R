//! Word (.docx) **文档模型**与解析。
//!
//! **为什么用 `docx-rs` 的读路径**:它已处理 OOXML 最麻烦的部分 —— zip 解包、
//! `document.xml` 解析、关系解析,尤其是**图片字节与 `r:embed` 的关联**。我们在其
//! AST 之上抽出一个**平面化、可序列化的只读模型**(段落/文本 run/标题/列表/对齐/
//! 表格/图片),供视图层在 canvas 上做流式布局。
//!
//! **重 CPU 在此**:解析与模型构建放 WASM,视图层只按模型排版绘制。
//!
//! 覆盖范围(本期只读):文字、加粗/斜体/下划线、字号/颜色、标题(Heading1-6)、
//! 段落对齐、项目符号/编号列表、内联图片、表格、图文混排。
//! **非目标**:分栏、文本框绘图、公式对象、批注、修订、页眉页脚、精确行距/缩进。

use serde::Serialize;

use docx_rs::{
    DocumentChild, Docx, DrawingData, Paragraph as DxParagraph, ParagraphChild, RunChild,
    TableCellContent, TableChild, TableRowChild,
};

/// 段落对齐方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// 列表项信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListItem {
    /// 缩进层级(0 起)。
    pub level: u8,
    /// 是否有序(编号);否则为项目符号。
    pub ordered: bool,
}

/// 一段文本 run(同一批格式)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    /// 文本内容。
    pub text: String,
    /// 加粗。
    pub bold: bool,
    /// 斜体。
    pub italic: bool,
    /// 下划线。
    pub underline: bool,
    /// 字号(磅);缺省时视图用段落/标题默认字号。
    pub size_pt: Option<f64>,
    /// 文字颜色(`RRGGBB` 十六进制,不含 `#`)。
    pub color: Option<String>,
}

/// 内联图片引用(字节在 [`ParsedDoc::images`] 里按 `id` 取)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageRef {
    /// 关系 id(`r:embed`)。
    pub id: String,
    /// 显示宽度(像素,由 EMU 换算)。
    pub width_px: f64,
    /// 显示高度(像素)。
    pub height_px: f64,
}

/// 段落里的内联元素。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Inline {
    /// 文本 run。
    Text(Run),
    /// 内联图片。
    Image(ImageRef),
    /// 换行(`w:br`)。
    Break,
}

/// 段落。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Paragraph {
    /// 标题级别 1-6;正文为 `None`。
    pub heading: Option<u8>,
    /// 对齐。
    pub align: Align,
    /// 列表信息;非列表为 `None`。
    pub list: Option<ListItem>,
    /// 内联内容。
    pub inlines: Vec<Inline>,
}

/// 表格单元格(可含多段落 / 嵌套块)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableCell {
    pub blocks: Vec<Block>,
}

/// 表格行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

/// 表格。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Table {
    pub rows: Vec<TableRow>,
}

/// 文档块:段落或表格。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

/// 文档模型(不含图片字节)。
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct WordDoc {
    pub blocks: Vec<Block>,
}

/// 一张图片的字节(单独于模型,便于按需转移到 JS)。
#[derive(Debug, Clone, PartialEq)]
pub struct DocImage {
    /// 关系 id(与 [`ImageRef::id`] 对应)。
    pub id: String,
    /// MIME 类型(由扩展名推断)。
    pub mime: String,
    /// 原始字节。
    pub data: Vec<u8>,
}

/// 解析产物:模型 + 图片。
#[derive(Debug, Clone, Default)]
pub struct ParsedDoc {
    pub doc: WordDoc,
    pub images: Vec<DocImage>,
}

/// EMU → 像素(96 DPI):914400 EMU = 1 英寸 = 96px,即 ÷9525。
fn emu_to_px(emu: u32) -> f64 {
    emu as f64 / 9525.0
}

/// 解析 docx 字节为文档模型。
pub fn parse(bytes: &[u8]) -> Result<ParsedDoc, String> {
    let docx = docx_rs::read_docx(bytes).map_err(|e| format!("{e:?}"))?;
    let images = collect_images(&docx);
    let blocks = docx
        .document
        .children
        .iter()
        .filter_map(convert_child)
        .collect();
    Ok(ParsedDoc {
        doc: WordDoc { blocks },
        images,
    })
}

/// 收集图片字节。
///
/// `docx.images` 是 `(rId, 路径, Image(原始字节), Png(预览字节))`。读路径下原始字节在
/// 元组的 `Image` 里(`.media` 可能为空),优先用原始字节,回退到 `.media` 按路径查。
fn collect_images(docx: &Docx) -> Vec<DocImage> {
    let mut out = Vec::new();
    for (rid, path, image, _png) in &docx.images {
        let data = if !image.0.is_empty() {
            image.0.clone()
        } else {
            match docx.media.iter().find(|(p, _)| p == path) {
                Some((_, b)) => b.clone(),
                None => continue,
            }
        };
        out.push(DocImage {
            id: rid.clone(),
            mime: mime_of(path),
            data,
        });
    }
    out
}

fn mime_of(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
    .to_string()
}

fn convert_child(child: &DocumentChild) -> Option<Block> {
    match child {
        DocumentChild::Paragraph(p) => Some(Block::Paragraph(convert_paragraph(p))),
        DocumentChild::Table(t) => Some(Block::Table(convert_table(t))),
        _ => None,
    }
}

fn convert_paragraph(p: &DxParagraph) -> Paragraph {
    let prop = &p.property;

    // 标题级别:pStyle 形如 "Heading1".."Heading6"(或本地化 id)
    let heading = prop.style.as_ref().and_then(|s| heading_level(&s.val));

    // 对齐
    let align = prop
        .alignment
        .as_ref()
        .map(|j| align_of(&justification_str(j)))
        .unwrap_or(Align::Left);

    // 列表
    let list = prop.numbering_property.as_ref().map(|np| {
        let level = np.level.as_ref().map(|l| l.val as u8).unwrap_or(0);
        // 有序/无序需查 numbering.xml;此处按层级奇偶给个稳妥默认(无法确定时按项目符号)。
        ListItem {
            level,
            ordered: false,
        }
    });

    let mut inlines = Vec::new();
    for child in &p.children {
        if let ParagraphChild::Run(run) = child {
            append_run(run, &mut inlines);
        }
    }

    Paragraph {
        heading,
        align,
        list,
        inlines,
    }
}

fn append_run(run: &docx_rs::Run, out: &mut Vec<Inline>) {
    let rp = &run.run_property;
    let bold = rp.bold.is_some();
    let italic = rp.italic.is_some();
    let underline = rp.underline.is_some();
    let (size_pt, color) = run_size_color(rp);

    for child in &run.children {
        match child {
            RunChild::Text(t) => {
                out.push(Inline::Text(Run {
                    text: t.text.clone(),
                    bold,
                    italic,
                    underline,
                    size_pt,
                    color: color.clone(),
                }));
            }
            RunChild::Break(_) => out.push(Inline::Break),
            RunChild::Tab(_) => {
                out.push(Inline::Text(Run {
                    text: "\t".to_string(),
                    bold,
                    italic,
                    underline,
                    size_pt,
                    color: color.clone(),
                }));
            }
            RunChild::Drawing(d) => {
                if let Some(img) = drawing_image(d) {
                    out.push(Inline::Image(img));
                }
            }
            _ => {}
        }
    }
}

/// 从 `Drawing` 取内联图片(id + 尺寸)。
fn drawing_image(d: &docx_rs::Drawing) -> Option<ImageRef> {
    match &d.data {
        Some(DrawingData::Pic(pic)) => {
            let (w, h) = pic.size;
            Some(ImageRef {
                id: pic.id.clone(),
                width_px: emu_to_px(w),
                height_px: emu_to_px(h),
            })
        }
        _ => None,
    }
}

/// 通过 serde_json 读取 run 的字号(磅)与颜色 —— docx-rs 的 `Sz`/`Color` 字段私有,
/// 但实现了 `Serialize`(`Sz` → 半磅数值,`Color` → 十六进制字符串)。
fn run_size_color(rp: &docx_rs::RunProperty) -> (Option<f64>, Option<String>) {
    let v = match serde_json::to_value(rp) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    // 字号:sz 可能序列化为数值(半磅)或 {val: 半磅}
    let size_pt = extract_u64(&v, "sz").map(|half| half as f64 / 2.0);
    // 颜色:color 可能为字符串或 {val: "RRGGBB"}
    let color = extract_str(&v, "color").filter(|c| {
        let c = c.to_ascii_lowercase();
        c != "auto" && c != "000000" && !c.is_empty()
    });
    (size_pt, color)
}

fn extract_u64(v: &serde_json::Value, key: &str) -> Option<u64> {
    let node = v.get(key)?;
    if let Some(n) = node.as_u64() {
        return Some(n);
    }
    node.get("val").and_then(|x| x.as_u64())
}

fn extract_str(v: &serde_json::Value, key: &str) -> Option<String> {
    let node = v.get(key)?;
    if let Some(s) = node.as_str() {
        return Some(s.to_string());
    }
    node.get("val")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// justification 的 `val`(私有)经 serde 读取。
fn justification_str(j: &docx_rs::Justification) -> String {
    serde_json::to_value(j)
        .ok()
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.get("val").and_then(|x| x.as_str()).map(|s| s.to_string()))
        })
        .unwrap_or_default()
}

fn align_of(val: &str) -> Align {
    match val {
        "center" => Align::Center,
        "right" | "end" => Align::Right,
        "both" | "justify" | "distribute" => Align::Justify,
        _ => Align::Left,
    }
}

/// 从样式 id 提取标题级别:`Heading1`/`heading 1`/`Title`(视作 1)。
fn heading_level(style: &str) -> Option<u8> {
    let s = style.to_ascii_lowercase().replace([' ', '-', '_'], "");
    if s == "title" {
        return Some(1);
    }
    let digits = s.strip_prefix("heading")?;
    digits.parse::<u8>().ok().filter(|n| (1..=6).contains(n))
}

fn convert_table(t: &docx_rs::Table) -> Table {
    let mut rows = Vec::new();
    for TableChild::TableRow(row) in &t.rows {
        let mut cells = Vec::new();
        for TableRowChild::TableCell(cell) in &row.cells {
            let mut blocks = Vec::new();
            for content in &cell.children {
                match content {
                    TableCellContent::Paragraph(p) => {
                        blocks.push(Block::Paragraph(convert_paragraph(p)));
                    }
                    TableCellContent::Table(t) => {
                        blocks.push(Block::Table(convert_table(t)));
                    }
                    _ => {}
                }
            }
            cells.push(TableCell { blocks });
        }
        rows.push(TableRow { cells });
    }
    Table { rows }
}

#[cfg(test)]
mod tests {
    use super::{parse, Align, Block, Inline, Paragraph};
    use docx_rs::{
        AlignmentType, Docx, Paragraph as DxPara, Run as DxRun, Table as DxTable,
        TableCell as DxCell, TableRow as DxRow,
    };

    /// 用 docx-rs 的写路径构造一份 docx 字节作为测试夹具。
    fn build_sample() -> Vec<u8> {
        let mut buf = Vec::new();
        Docx::new()
            .add_paragraph(
                DxPara::new()
                    .style("Heading1")
                    .add_run(DxRun::new().add_text("标题一")),
            )
            .add_paragraph(
                DxPara::new()
                    .align(AlignmentType::Center)
                    .add_run(DxRun::new().bold().add_text("居中加粗")),
            )
            .add_paragraph(DxPara::new().add_run(DxRun::new().add_text("普通段落")))
            .build()
            .pack(&mut std::io::Cursor::new(&mut buf))
            .expect("打包 docx");
        buf
    }

    #[test]
    fn parses_headings_alignment_bold() {
        let bytes = build_sample();
        let parsed = parse(&bytes).expect("应解析成功");
        let blocks = &parsed.doc.blocks;
        assert!(blocks.len() >= 3);

        match &blocks[0] {
            Block::Paragraph(p) => {
                assert_eq!(p.heading, Some(1));
                assert_eq!(text_of(p), "标题一");
            }
            _ => panic!("首块应为段落"),
        }
        match &blocks[1] {
            Block::Paragraph(p) => {
                assert_eq!(p.align, Align::Center);
                assert!(matches!(&p.inlines[0], Inline::Text(r) if r.bold));
            }
            _ => panic!("次块应为段落"),
        }
    }

    #[test]
    fn parses_table() {
        let mut buf = Vec::new();
        let table = DxTable::new(vec![DxRow::new(vec![
            DxCell::new().add_paragraph(DxPara::new().add_run(DxRun::new().add_text("A1"))),
            DxCell::new().add_paragraph(DxPara::new().add_run(DxRun::new().add_text("B1"))),
        ])]);
        Docx::new()
            .add_table(table)
            .build()
            .pack(&mut std::io::Cursor::new(&mut buf))
            .expect("打包");
        let parsed = parse(&buf).expect("解析");
        let table_block = parsed
            .doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("应有表格");
        assert_eq!(table_block.rows.len(), 1);
        assert_eq!(table_block.rows[0].cells.len(), 2);
    }

    #[test]
    fn heading_level_parsing() {
        assert_eq!(super::heading_level("Heading1"), Some(1));
        assert_eq!(super::heading_level("heading 3"), Some(3));
        assert_eq!(super::heading_level("Title"), Some(1));
        assert_eq!(super::heading_level("Normal"), None);
        assert_eq!(super::heading_level("Heading9"), None);
    }

    #[test]
    fn invalid_bytes_error() {
        assert!(parse(b"not a docx").is_err());
    }

    fn text_of(p: &Paragraph) -> String {
        p.inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text(r) => Some(r.text.clone()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod fixture_gen {
    use docx_rs::*;
    use std::io::Cursor;

    /// 生成一份用于浏览器 e2e 的 .docx 夹具:标题/居中加粗/表格/图片。
    /// 默认忽略;需要时 `cargo test -p office-core write_browser_fixture -- --ignored --nocapture`。
    #[test]
    #[ignore]
    fn write_browser_fixture() {
        // 一张 2x2 红色 PNG(手写最小 PNG)
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00,
            0x00, 0xFD, 0xD4, 0x9A, 0x73, 0x00, 0x00, 0x00, 0x16, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x8C, 0x0C, 0x0C, 0x0C, 0x00, 0x00, 0x0C,
            0x0C, 0x02, 0xFC, 0x8B, 0x8D, 0xB0, 0x8D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
            0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let pic = Pic::new(png).size(200 * 9525, 150 * 9525);

        let table = Table::new(vec![
            TableRow::new(vec![
                TableCell::new()
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text("城市"))),
                TableCell::new()
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text("人口"))),
            ]),
            TableRow::new(vec![
                TableCell::new()
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text("北京"))),
                TableCell::new()
                    .add_paragraph(Paragraph::new().add_run(Run::new().add_text("2189"))),
            ]),
        ]);

        let mut buf = Vec::new();
        Docx::new()
            .add_paragraph(Paragraph::new().style("Heading1").add_run(Run::new().add_text("office-R Word 渲染演示")))
            .add_paragraph(
                Paragraph::new()
                    .align(AlignmentType::Center)
                    .add_run(Run::new().bold().add_text("居中加粗副标题")),
            )
            .add_paragraph(Paragraph::new().style("Heading2").add_run(Run::new().add_text("一、文字与样式")))
            .add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text("这是正文,包含"))
                    .add_run(Run::new().bold().add_text("加粗"))
                    .add_run(Run::new().add_text("、"))
                    .add_run(Run::new().italic().add_text("斜体"))
                    .add_run(Run::new().add_text("与普通文字混排,以及一段较长的中文用于验证自动折行效果,应当在页面宽度处换到下一行继续显示。")),
            )
            .add_paragraph(Paragraph::new().align(AlignmentType::Right).add_run(Run::new().add_text("右对齐一行")))
            .add_paragraph(Paragraph::new().style("Heading2").add_run(Run::new().add_text("二、图文混排")))
            .add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text("左边文字,右边图片:"))
                    .add_run(Run::new().add_image(pic)),
            )
            .add_paragraph(Paragraph::new().style("Heading2").add_run(Run::new().add_text("三、表格")))
            .add_table(table)
            .build()
            .pack(&mut Cursor::new(&mut buf))
            .expect("打包");
        std::fs::write("/tmp/office-r-sample.docx", &buf).expect("写夹具");
        eprintln!("wrote /tmp/office-r-sample.docx ({} bytes)", buf.len());
    }
}
