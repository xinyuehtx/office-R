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
//! 段落对齐、项目符号/编号列表、内联图片、表格、图文混排、分栏、页眉页脚、
//! 修订(插入/删除)、超链接(蓝色下划线)、脚注(文末汇总 + 引用标记 `[n]`)、
//! 左缩进 / 段前段后间距 / 行距倍数、批注(文末「作者:内容」汇总)。
//! **非目标**:文本框绘图、公式对象(OMML)、域/目录(TOC 的结果文本已随普通 run 渲染)。

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
    /// 有序列表的序号(从 1 起,按同一 numId+level 递增);无序为 `None`。
    pub number: Option<u32>,
}

/// 修订标记:普通 / 插入 / 删除(来自 `w:ins` / `w:del`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Revision {
    /// 非修订。
    #[default]
    None,
    /// 插入(修订):视图可加下划线/着色。
    Inserted,
    /// 删除(修订):视图可加删除线/着色。
    Deleted,
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
    /// 修订标记(插入/删除/无)。
    #[serde(default)]
    pub revision: Revision,
    /// 超链接目标(外部 URL 或 `#锚点`);非链接为 `None`。视图渲染为蓝色下划线。
    #[serde(default)]
    pub link: Option<String>,
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
    /// 左缩进(像素;`w:ind@start` twips ÷ 15)。
    #[serde(default)]
    pub indent_px: f64,
    /// 段前间距(像素;`w:spacing@before` twips ÷ 15)。
    #[serde(default)]
    pub space_before_px: f64,
    /// 段后间距(像素;`w:spacing@after` twips ÷ 15)。
    #[serde(default)]
    pub space_after_px: f64,
    /// 行距倍数(`w:spacing@line` 的 Auto 规则:line ÷ 240);无则视图用默认。
    #[serde(default)]
    pub line_pct: Option<f64>,
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
    /// 正文分栏数(来自 sectPr;默认 1)。
    #[serde(default)]
    pub columns: u32,
    /// 页眉段落(来自 header part);无则空。
    #[serde(default)]
    pub header: Vec<Block>,
    /// 页脚段落(来自 footer part);无则空。
    #[serde(default)]
    pub footer: Vec<Block>,
    /// 脚注(来自 footnotes part);每条一个块,渲染在正文末尾。
    #[serde(default)]
    pub footnotes: Vec<Block>,
    /// 批注(来自 comments part);每条「作者:内容」一个块,渲染在正文末尾。
    #[serde(default)]
    pub comments: Vec<Block>,
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

/// 解析上下文:携带编号定义与运行期序号计数。
struct Ctx {
    /// numId → (level → 是否有序)。
    ordered: std::collections::HashMap<usize, std::collections::HashMap<usize, bool>>,
    /// 有序列表运行期计数:(numId, level) → 已出现的序号。
    counters: std::collections::HashMap<(usize, usize), u32>,
    /// 超链接 rId → 目标 URL(来自 document.xml.rels)。
    hyperlinks: std::collections::HashMap<String, String>,
}

/// 解析 docx 字节为文档模型。
pub fn parse(bytes: &[u8]) -> Result<ParsedDoc, String> {
    let docx = docx_rs::read_docx(bytes).map_err(|e| format!("{e:?}"))?;
    let images = collect_images(&docx);
    // 超链接 rId → 目标 URL(document.xml.rels 里的 hyperlinks:(id, target, type))
    let hyperlinks = docx
        .document_rels
        .hyperlinks
        .iter()
        .map(|(id, target, _ty)| (id.clone(), target.clone()))
        .collect();
    let mut ctx = Ctx {
        ordered: build_numbering_map(&docx),
        counters: std::collections::HashMap::new(),
        hyperlinks,
    };
    let blocks = docx
        .document
        .children
        .iter()
        .filter_map(|c| convert_child(c, &mut ctx))
        .collect();

    // 分栏 / 页眉 / 页脚(来自 sectPr)
    let sect = &docx.document.section_property;
    let columns = (sect.columns as u32).max(1);
    let header = sect
        .header
        .as_ref()
        .map(|(_, h)| convert_header_children(&h.children, &mut ctx))
        .unwrap_or_default();
    let footer = sect
        .footer
        .as_ref()
        .map(|(_, f)| convert_footer_children(&f.children, &mut ctx))
        .unwrap_or_default();

    // 脚注:docx.footnotes 里每条 footnote 的文本转块,前缀「n.」编号
    let footnotes = collect_footnotes(&docx);
    // 批注:comments part,每条「作者:内容」
    let comments = collect_comments(&docx);

    Ok(ParsedDoc {
        doc: WordDoc {
            blocks,
            columns,
            header,
            footer,
            footnotes,
            comments,
        },
        images,
    })
}

/// 收集批注为块序列:每条 `作者:内容`(内容经 serde 递归取 run 文本)。
fn collect_comments(docx: &Docx) -> Vec<Block> {
    let mut out = Vec::new();
    for c in docx.comments.inner() {
        let mut text = String::new();
        if let Ok(v) = serde_json::to_value(&c.children) {
            gather_text(&v, &mut text);
        }
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let label = if c.author.is_empty() {
            text.to_string()
        } else {
            format!("{}:{}", c.author, text)
        };
        out.push(Block::Paragraph(Paragraph {
            heading: None,
            align: Align::Left,
            list: None,
            inlines: vec![Inline::Text(Run {
                text: label,
                bold: false,
                italic: true,
                underline: false,
                size_pt: Some(9.0),
                color: Some("8250df".to_string()),
                revision: Revision::None,
                link: None,
            })],
            indent_px: 0.0,
            space_before_px: 0.0,
            space_after_px: 0.0,
            line_pct: None,
        }));
    }
    out
}

/// 收集脚注为块序列。`Footnotes.footnotes` 字段是 `pub(crate)` 不可直接访问,
/// 故经 serde 取出 `{ footnotes: [{ id, content }] }`,递归收集各条可见文本
/// (run 的 `text`),渲染成「n. 文本」段落。跳过 id≤1 的分隔符脚注。
fn collect_footnotes(docx: &Docx) -> Vec<Block> {
    let mut out = Vec::new();
    let value = match serde_json::to_value(&docx.footnotes) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let Some(list) = value.get("footnotes").and_then(|v| v.as_array()) else {
        return out;
    };
    for fnt in list {
        let id = fnt.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        if id <= 1 {
            continue;
        }
        let mut text = String::new();
        if let Some(content) = fnt.get("content") {
            gather_text(content, &mut text);
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        out.push(Block::Paragraph(Paragraph {
            heading: None,
            align: Align::Left,
            list: None,
            inlines: vec![Inline::Text(Run {
                text: format!("{id}. {text}"),
                bold: false,
                italic: false,
                underline: false,
                size_pt: Some(9.0),
                color: Some("57606a".to_string()),
                revision: Revision::None,
                link: None,
            })],
            indent_px: 0.0,
            space_before_px: 0.0,
            space_after_px: 0.0,
            line_pct: None,
        }));
    }
    out
}

/// 递归收集 JSON 里所有 `"text"` 字符串字段(按出现顺序),用于抽取脚注可见文本。
fn gather_text(v: &serde_json::Value, out: &mut String) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                if k == "text" {
                    if let Some(s) = val.as_str() {
                        out.push_str(s);
                    }
                } else {
                    gather_text(val, out);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                gather_text(item, out);
            }
        }
        _ => {}
    }
}

/// 把页眉子元素转成块(段落 / 表格)。
fn convert_header_children(children: &[docx_rs::HeaderChild], ctx: &mut Ctx) -> Vec<Block> {
    children
        .iter()
        .filter_map(|c| match c {
            docx_rs::HeaderChild::Paragraph(p) => Some(Block::Paragraph(convert_paragraph(p, ctx))),
            docx_rs::HeaderChild::Table(t) => Some(Block::Table(convert_table(t, ctx))),
            _ => None,
        })
        .collect()
}

/// 把页脚子元素转成块(段落 / 表格)。
fn convert_footer_children(children: &[docx_rs::FooterChild], ctx: &mut Ctx) -> Vec<Block> {
    children
        .iter()
        .filter_map(|c| match c {
            docx_rs::FooterChild::Paragraph(p) => Some(Block::Paragraph(convert_paragraph(p, ctx))),
            docx_rs::FooterChild::Table(t) => Some(Block::Table(convert_table(t, ctx))),
            _ => None,
        })
        .collect()
}

/// 从 `numbering.xml`(docx-rs 的 `numberings`)构建 numId → level → 是否有序。
///
/// 路径:`Numbering{id, abstract_num_id}` → `AbstractNumbering{levels}` →
/// `Level{level, format.val}`。`format.val` 为 `"bullet"`/`"none"` → 无序;
/// `"decimal"`/`"lowerRoman"`/`"upperLetter"`/… → 有序。
fn build_numbering_map(
    docx: &Docx,
) -> std::collections::HashMap<usize, std::collections::HashMap<usize, bool>> {
    use std::collections::HashMap;
    // abstract_num_id → (level → ordered)
    let mut abstracts: HashMap<usize, HashMap<usize, bool>> = HashMap::new();
    for an in &docx.numberings.abstract_nums {
        let mut levels = HashMap::new();
        for lvl in &an.levels {
            let fmt = lvl.format.val.to_ascii_lowercase();
            let ordered = !matches!(fmt.as_str(), "bullet" | "none");
            levels.insert(lvl.level, ordered);
        }
        abstracts.insert(an.id, levels);
    }
    // numId → abstract → 展开
    let mut map: HashMap<usize, HashMap<usize, bool>> = HashMap::new();
    for num in &docx.numberings.numberings {
        if let Some(levels) = abstracts.get(&num.abstract_num_id) {
            map.insert(num.id, levels.clone());
        }
    }
    map
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

fn convert_child(child: &DocumentChild, ctx: &mut Ctx) -> Option<Block> {
    match child {
        DocumentChild::Paragraph(p) => Some(Block::Paragraph(convert_paragraph(p, ctx))),
        DocumentChild::Table(t) => Some(Block::Table(convert_table(t, ctx))),
        _ => None,
    }
}

fn convert_paragraph(p: &DxParagraph, ctx: &mut Ctx) -> Paragraph {
    let prop = &p.property;

    // 标题级别:pStyle 形如 "Heading1".."Heading6"(或本地化 id)
    let heading = prop.style.as_ref().and_then(|s| heading_level(&s.val));

    // 对齐
    let align = prop
        .alignment
        .as_ref()
        .map(|j| align_of(&justification_str(j)))
        .unwrap_or(Align::Left);

    // 左缩进(twips ÷ 15 = px);start 优先,负值夹到 0
    let indent_px = prop
        .indent
        .as_ref()
        .and_then(|i| i.start)
        .map(|t| (t as f64 / 15.0).max(0.0))
        .unwrap_or(0.0);

    // 行距 / 段间距:LineSpacing 字段私有,经 serde 取 {before, after, line, lineRule}
    let (space_before_px, space_after_px, line_pct) = prop
        .line_spacing
        .as_ref()
        .map(parse_line_spacing)
        .unwrap_or((0.0, 0.0, None));

    // 列表:查 numbering 定 有序/无序;有序则按 (numId, level) 递增算序号
    let list = prop.numbering_property.as_ref().map(|np| {
        let level = np.level.as_ref().map(|l| l.val).unwrap_or(0);
        let num_id = np.id.as_ref().map(|i| i.id);
        let ordered = num_id
            .and_then(|id| ctx.ordered.get(&id))
            .and_then(|lv| lv.get(&level))
            .copied()
            .unwrap_or(false);
        let number = if ordered {
            let id = num_id.unwrap_or(0);
            let c = ctx.counters.entry((id, level)).or_insert(0);
            *c += 1;
            Some(*c)
        } else {
            None
        };
        ListItem {
            level: level as u8,
            ordered,
            number,
        }
    });

    let mut inlines = Vec::new();
    for child in &p.children {
        match child {
            ParagraphChild::Run(run) => append_run(run, Revision::None, None, ctx, &mut inlines),
            // 修订:插入 / 删除 里的 run 打上标记
            ParagraphChild::Insert(ins) => {
                for ic in &ins.children {
                    if let docx_rs::InsertChild::Run(run) = ic {
                        append_run(run, Revision::Inserted, None, ctx, &mut inlines);
                    }
                }
            }
            ParagraphChild::Delete(del) => {
                for dc in &del.children {
                    if let docx_rs::DeleteChild::Run(run) = dc {
                        append_run(run, Revision::Deleted, None, ctx, &mut inlines);
                    }
                }
            }
            // 超链接:解析目标(外部 rId → URL;锚点 → #anchor),子 run 打上 link
            ParagraphChild::Hyperlink(h) => {
                let target = hyperlink_target(&h.link, ctx);
                for hc in &h.children {
                    if let ParagraphChild::Run(run) = hc {
                        append_run(run, Revision::None, target.as_deref(), ctx, &mut inlines);
                    }
                }
            }
            _ => {}
        }
    }

    Paragraph {
        heading,
        align,
        list,
        inlines,
        indent_px,
        space_before_px,
        space_after_px,
        line_pct,
    }
}

/// 从 docx-rs `LineSpacing`(字段私有)经 serde 抽出 (段前 px, 段后 px, 行距倍数)。
/// `before`/`after` 为 twips;`line` 在 Auto 规则下是 240ths(line÷240 = 倍数)。
fn parse_line_spacing(ls: &docx_rs::LineSpacing) -> (f64, f64, Option<f64>) {
    let v = match serde_json::to_value(ls) {
        Ok(v) => v,
        Err(_) => return (0.0, 0.0, None),
    };
    let twips_px = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_f64())
            .map(|t| t / 15.0)
            .unwrap_or(0.0)
    };
    let before = twips_px("before");
    let after = twips_px("after");
    // lineRule 缺省视为 auto;line/240 = 行距倍数(仅 auto 有意义)
    let rule = v.get("lineRule").and_then(|x| x.as_str()).unwrap_or("auto");
    let line_pct = if rule == "auto" || rule == "atLeast" {
        v.get("line").and_then(|x| x.as_f64()).map(|l| l / 240.0)
    } else {
        None
    };
    (before, after, line_pct)
}

/// 解析超链接目标:外部链接经 `document_rels` 把 rId 映射为 URL;锚点返回 `#anchor`。
fn hyperlink_target(link: &docx_rs::HyperlinkData, ctx: &Ctx) -> Option<String> {
    match link {
        docx_rs::HyperlinkData::External { rid, path } => {
            if !path.is_empty() {
                Some(path.clone())
            } else {
                ctx.hyperlinks.get(rid).cloned()
            }
        }
        docx_rs::HyperlinkData::Anchor { anchor } => Some(format!("#{anchor}")),
    }
}

fn append_run(
    run: &docx_rs::Run,
    revision: Revision,
    link: Option<&str>,
    _ctx: &Ctx,
    out: &mut Vec<Inline>,
) {
    let rp = &run.run_property;
    let bold = rp.bold.is_some();
    let italic = rp.italic.is_some();
    let underline = rp.underline.is_some();
    let (size_pt, color) = run_size_color(rp);
    let link = link.map(|s| s.to_string());
    let mk = |text: String| {
        Inline::Text(Run {
            text,
            bold,
            italic,
            underline,
            size_pt,
            color: color.clone(),
            revision,
            link: link.clone(),
        })
    };

    for child in &run.children {
        match child {
            RunChild::Text(t) => out.push(mk(t.text.clone())),
            RunChild::DeleteText(t) => {
                // 删除修订里的文本(w:delText);DeleteText.text 私有,经 serde 取
                let text = serde_json::to_value(t)
                    .ok()
                    .and_then(|v| v.get("text").and_then(|x| x.as_str()).map(String::from))
                    .unwrap_or_default();
                out.push(Inline::Text(Run {
                    text,
                    bold,
                    italic,
                    underline,
                    size_pt,
                    color: color.clone(),
                    revision: Revision::Deleted,
                    link: link.clone(),
                }));
            }
            RunChild::Break(_) => out.push(Inline::Break),
            RunChild::Tab(_) => out.push(mk("\t".to_string())),
            // 脚注引用:插入一个上标式标记 [n](内容渲染在文末脚注区)
            RunChild::FootnoteReference(f) => {
                out.push(Inline::Text(Run {
                    text: format!("[{}]", f.id),
                    bold: false,
                    italic: false,
                    underline: false,
                    size_pt: None,
                    color: Some("0969da".to_string()),
                    revision,
                    link: None,
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

fn convert_table(t: &docx_rs::Table, ctx: &mut Ctx) -> Table {
    let mut rows = Vec::new();
    for TableChild::TableRow(row) in &t.rows {
        let mut cells = Vec::new();
        for TableRowChild::TableCell(cell) in &row.cells {
            let mut blocks = Vec::new();
            for content in &cell.children {
                match content {
                    TableCellContent::Paragraph(p) => {
                        blocks.push(Block::Paragraph(convert_paragraph(p, ctx)));
                    }
                    TableCellContent::Table(t) => {
                        blocks.push(Block::Table(convert_table(t, ctx)));
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
    use super::{parse, Align, Block, Inline, Paragraph, Revision};
    use docx_rs::{
        AbstractNumbering, AlignmentType, Comment, Delete, Docx, Footer, Header, IndentLevel,
        Insert, Level, LevelJc, LevelText, LineSpacing, NumberFormat, Numbering, NumberingId,
        Paragraph as DxPara, Run as DxRun, Start, Table as DxTable, TableCell as DxCell,
        TableRow as DxRow,
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
    fn parses_ordered_and_bullet_lists() {
        // abstract 0 = 有序(decimal),abstract 1 = 无序(bullet)
        let ordered_abs = AbstractNumbering::new(0).add_level(Level::new(
            0,
            Start::new(1),
            NumberFormat::new("decimal"),
            LevelText::new("%1."),
            LevelJc::new("left"),
        ));
        let bullet_abs = AbstractNumbering::new(1).add_level(Level::new(
            0,
            Start::new(1),
            NumberFormat::new("bullet"),
            LevelText::new("•"),
            LevelJc::new("left"),
        ));
        let mut buf = Vec::new();
        Docx::new()
            .add_abstract_numbering(ordered_abs)
            .add_abstract_numbering(bullet_abs)
            .add_numbering(Numbering::new(1, 0)) // numId 1 → 有序
            .add_numbering(Numbering::new(2, 1)) // numId 2 → 无序
            .add_paragraph(
                DxPara::new()
                    .numbering(NumberingId::new(1), IndentLevel::new(0))
                    .add_run(DxRun::new().add_text("第一项")),
            )
            .add_paragraph(
                DxPara::new()
                    .numbering(NumberingId::new(1), IndentLevel::new(0))
                    .add_run(DxRun::new().add_text("第二项")),
            )
            .add_paragraph(
                DxPara::new()
                    .numbering(NumberingId::new(2), IndentLevel::new(0))
                    .add_run(DxRun::new().add_text("要点")),
            )
            .build()
            .pack(&mut std::io::Cursor::new(&mut buf))
            .expect("打包");
        let parsed = parse(&buf).expect("解析");
        let paras: Vec<&Paragraph> = parsed
            .doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(p),
                _ => None,
            })
            .collect();
        // 前两段有序,序号 1、2
        let l0 = paras[0].list.as_ref().expect("列表");
        assert!(l0.ordered);
        assert_eq!(l0.number, Some(1));
        let l1 = paras[1].list.as_ref().expect("列表");
        assert!(l1.ordered);
        assert_eq!(l1.number, Some(2));
        // 第三段无序
        let l2 = paras[2].list.as_ref().expect("列表");
        assert!(!l2.ordered);
        assert_eq!(l2.number, None);
    }

    #[test]
    fn resolves_hyperlink_target() {
        use super::{hyperlink_target, Ctx};
        use docx_rs::HyperlinkData;
        let mut hl = std::collections::HashMap::new();
        hl.insert("rId7".to_string(), "https://example.com/".to_string());
        let ctx = Ctx {
            ordered: Default::default(),
            counters: Default::default(),
            hyperlinks: hl,
        };
        // 外部链接:rId 经 rels 解析为 URL
        assert_eq!(
            hyperlink_target(
                &HyperlinkData::External {
                    rid: "rId7".into(),
                    path: String::new()
                },
                &ctx
            ),
            Some("https://example.com/".to_string())
        );
        // 外部链接:path 直给时优先
        assert_eq!(
            hyperlink_target(
                &HyperlinkData::External {
                    rid: "x".into(),
                    path: "http://direct/".into()
                },
                &ctx
            ),
            Some("http://direct/".to_string())
        );
        // 锚点
        assert_eq!(
            hyperlink_target(
                &HyperlinkData::Anchor {
                    anchor: "sec1".into()
                },
                &ctx
            ),
            Some("#sec1".to_string())
        );
    }

    #[test]
    fn parses_comments_to_blocks() {
        let mut buf = Vec::new();
        Docx::new()
            .add_paragraph(
                DxPara::new()
                    .add_comment_start(
                        Comment::new(1).author("张三").add_paragraph(
                            DxPara::new().add_run(DxRun::new().add_text("改一下这里")),
                        ),
                    )
                    .add_run(DxRun::new().add_text("正文"))
                    .add_comment_end(1),
            )
            .build()
            .pack(&mut std::io::Cursor::new(&mut buf))
            .expect("打包");
        let parsed = parse(&buf).expect("解析");
        let joined: String = parsed
            .doc
            .comments
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(p) => p.inlines.iter().find_map(|i| match i {
                    Inline::Text(r) => Some(r.text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(joined.contains("改一下这里"), "批注内容:{joined}");
        assert!(joined.contains("张三"), "批注作者:{joined}");
    }

    #[test]
    fn parses_indent_and_spacing() {
        let mut buf = Vec::new();
        Docx::new()
            .add_paragraph(
                DxPara::new()
                    .indent(Some(720), None, None, None) // 720 twips = 48px
                    .line_spacing(LineSpacing::new().before(240).after(120).line(360))
                    .add_run(DxRun::new().add_text("缩进段")),
            )
            .build()
            .pack(&mut std::io::Cursor::new(&mut buf))
            .expect("打包");
        let parsed = parse(&buf).expect("解析");
        let Block::Paragraph(p) = &parsed.doc.blocks[0] else {
            panic!("应为段落");
        };
        assert!((p.indent_px - 48.0).abs() < 0.01, "indent={}", p.indent_px);
        assert!(
            (p.space_before_px - 16.0).abs() < 0.01,
            "before={}",
            p.space_before_px
        );
        assert!(
            (p.space_after_px - 8.0).abs() < 0.01,
            "after={}",
            p.space_after_px
        );
        assert_eq!(p.line_pct, Some(1.5), "line 360/240 = 1.5x");
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

    #[test]
    fn parses_revisions_ins_del() {
        // 一段:普通 run + 插入 run + 删除 run
        let mut buf = Vec::new();
        Docx::new()
            .add_paragraph(
                DxPara::new()
                    .add_run(DxRun::new().add_text("原文"))
                    .add_insert(Insert::new(DxRun::new().add_text("新增")))
                    .add_delete(Delete::new().add_run(DxRun::new().add_delete_text("删掉"))),
            )
            .build()
            .pack(&mut std::io::Cursor::new(&mut buf))
            .expect("打包");
        let parsed = parse(&buf).expect("解析");
        let Block::Paragraph(p) = &parsed.doc.blocks[0] else {
            panic!("应为段落");
        };
        let revs: Vec<(String, Revision)> = p
            .inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text(r) => Some((r.text.clone(), r.revision)),
                _ => None,
            })
            .collect();
        assert!(revs.contains(&("原文".into(), Revision::None)));
        assert!(revs.contains(&("新增".into(), Revision::Inserted)));
        assert!(revs.contains(&("删掉".into(), Revision::Deleted)));
    }

    #[test]
    fn parses_header_and_footer() {
        let mut buf = Vec::new();
        Docx::new()
            .header(
                Header::new().add_paragraph(DxPara::new().add_run(DxRun::new().add_text("页眉X"))),
            )
            .footer(
                Footer::new().add_paragraph(DxPara::new().add_run(DxRun::new().add_text("页脚Y"))),
            )
            .add_paragraph(DxPara::new().add_run(DxRun::new().add_text("正文")))
            .build()
            .pack(&mut std::io::Cursor::new(&mut buf))
            .expect("打包");
        let parsed = parse(&buf).expect("解析");
        assert_eq!(header_text(&parsed.doc.header), "页眉X");
        assert_eq!(header_text(&parsed.doc.footer), "页脚Y");
        // 未设分栏时默认 1
        assert_eq!(parsed.doc.columns, 1);
    }

    fn header_text(blocks: &[Block]) -> String {
        blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(text_of(p)),
                _ => None,
            })
            .collect()
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
