//! PowerPoint (.pptx) **幻灯模型**与解析(zip + quick-xml 直接解析 OOXML)。
//!
//! 无干净的纯 Rust pptx 读库(见 RFC-0006 调研),故直接解析 PresentationML:
//! - `ppt/presentation.xml`:幻灯尺寸 `p:sldSz`(EMU)+ 顺序 `p:sldIdLst`(r:id);
//! - `ppt/_rels/presentation.xml.rels`:r:id → `slides/slideN.xml`;
//! - 每张 `ppt/slides/slideN.xml`:遍历 `p:spTree`,取形状 `p:sp`(位置 `a:xfrm`、
//!   几何 `a:prstGeom`、填充 `a:solidFill`、文本 `p:txBody`)与图片 `p:pic`(`a:blip r:embed`);
//! - `ppt/slides/_rels/slideN.xml.rels`:embed id → `media/*` 字节。
//!
//! EMU→px:914400 EMU=1 英寸=96px,即 ÷9525;字号 `sz` 为百分之一磅,px = sz/100 × 4/3。
//!
//! **非目标**:母版/版式继承(占位符无显式 xfrm 时不渲染)、主题配色(schemeClr)、
//! 动画/切换、SmartArt/图表、组合形状子坐标、旋转翻转、自定义几何。

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader as XmlReader;
use serde::Serialize;
use zip::ZipArchive;

/// EMU → 像素。
fn emu(v: f64) -> f64 {
    v / 9525.0
}

/// 段落对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// 文本 run。
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// 字号(磅)。
    pub size_pt: Option<f64>,
    /// 颜色 RRGGBB。
    pub color: Option<String>,
}

/// 段落。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Para {
    pub align: Align,
    pub runs: Vec<Run>,
}

/// 一个形状(文本框 / 图片 / 自选图形;可同时带文本)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Shape {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// 预设几何(rect/ellipse/…);文本框/图片为 `None`。
    pub geom: Option<String>,
    /// 填充色 RRGGBB。
    pub fill: Option<String>,
    /// 图片 embed id(需经 slide rels 解析为字节)。
    pub image: Option<String>,
    /// 文本段落。
    pub paragraphs: Vec<Para>,
}

/// 一张幻灯片。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Slide {
    pub shapes: Vec<Shape>,
}

/// 演示文稿模型(不含图片字节)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Presentation {
    /// 幻灯宽/高(像素)。
    pub width_px: f64,
    pub height_px: f64,
    pub slides: Vec<Slide>,
}

/// 一张图片字节。
#[derive(Debug, Clone, PartialEq)]
pub struct SlideImage {
    pub id: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// 解析产物。
#[derive(Debug, Clone)]
pub struct ParsedPpt {
    pub presentation: Presentation,
    /// 幻灯序号 → (embed id → 该幻灯用到的图片)。用「幻灯序号 + embed」定位,
    /// 因为不同幻灯的 rels 里 embed id 会重复。
    pub images: Vec<SlideImage>,
    /// (slide_index, embed_id) → images 下标。
    pub image_index: HashMap<(usize, String), usize>,
}

/// 解析 pptx 字节。
pub fn parse(bytes: &[u8]) -> Result<ParsedPpt, String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes.to_vec())).map_err(|e| e.to_string())?;

    let (width_px, height_px) = read_slide_size(&mut zip).unwrap_or((960.0, 540.0));
    let slide_paths = ordered_slide_paths(&mut zip);

    let mut slides = Vec::new();
    let mut images = Vec::new();
    let mut image_index = HashMap::new();

    for (idx, path) in slide_paths.iter().enumerate() {
        let xml = match read_zip_text(&mut zip, path) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let rels = read_slide_rels(&mut zip, path);
        let shapes = parse_slide(&xml);
        slides.push(Slide { shapes });

        // 收集该幻灯用到的图片字节
        for (embed, target) in &rels {
            let media_path = normalize_media_path(target);
            if let Ok(data) = read_zip_bytes(&mut zip, &media_path) {
                let i = images.len();
                images.push(SlideImage {
                    id: embed.clone(),
                    mime: mime_of(&media_path),
                    data,
                });
                image_index.insert((idx, embed.clone()), i);
            }
        }
    }

    Ok(ParsedPpt {
        presentation: Presentation {
            width_px,
            height_px,
            slides,
        },
        images,
        image_index,
    })
}

fn read_zip_text(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Result<String, String> {
    let mut s = String::new();
    zip.by_name(name)
        .map_err(|e| e.to_string())?
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    Ok(s)
}

fn read_zip_bytes(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    zip.by_name(name)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    Ok(b)
}

/// 读幻灯尺寸(EMU → px)。
fn read_slide_size(zip: &mut ZipArchive<Cursor<Vec<u8>>>) -> Option<(f64, f64)> {
    let xml = read_zip_text(zip, "ppt/presentation.xml").ok()?;
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local(&e) == "sldSz" => {
                let cx = attr(&e, "cx").and_then(|s| s.parse::<f64>().ok());
                let cy = attr(&e, "cy").and_then(|s| s.parse::<f64>().ok());
                if let (Some(cx), Some(cy)) = (cx, cy) {
                    return Some((emu(cx), emu(cy)));
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// 按 `p:sldIdLst` 的顺序解析幻灯路径;失败时回退到按文件名排序。
fn ordered_slide_paths(zip: &mut ZipArchive<Cursor<Vec<u8>>>) -> Vec<String> {
    let ordered = (|| {
        let pres = read_zip_text(zip, "ppt/presentation.xml").ok()?;
        let rels = read_zip_text(zip, "ppt/_rels/presentation.xml.rels").ok()?;
        let rid_to_target = parse_rels(&rels);

        // 按出现顺序收集 sldId 的 r:id
        let mut reader = XmlReader::from_str(&pres);
        let mut buf = Vec::new();
        let mut rids = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local(&e) == "sldId" => {
                    if let Some(rid) = attr_ns(&e, "id") {
                        rids.push(rid);
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        let paths: Vec<String> = rids
            .iter()
            .filter_map(|rid| rid_to_target.get(rid))
            .map(|t| normalize_ppt_path(t))
            .collect();
        if paths.is_empty() {
            None
        } else {
            Some(paths)
        }
    })();

    if let Some(p) = ordered {
        return p;
    }
    // 回退:枚举 slideN.xml 并按数字排序
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .collect();
    names.sort_by_key(|n| slide_number(n));
    names
}

fn slide_number(name: &str) -> usize {
    name.trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(usize::MAX)
}

/// `ppt/slides/slide1.xml` → 其 rels 里 embed id → target(相对路径已规整)。
fn read_slide_rels(
    zip: &mut ZipArchive<Cursor<Vec<u8>>>,
    slide_path: &str,
) -> Vec<(String, String)> {
    let rels_path = slide_rels_path(slide_path);
    let Ok(xml) = read_zip_text(zip, &rels_path) else {
        return Vec::new();
    };
    parse_rels(&xml).into_iter().collect()
}

/// `ppt/slides/slide1.xml` → `ppt/slides/_rels/slide1.xml.rels`。
fn slide_rels_path(slide_path: &str) -> String {
    match slide_path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{slide_path}.rels"),
    }
}

/// 解析 .rels:Id → Target。
fn parse_rels(xml: &str) -> HashMap<String, String> {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut map = HashMap::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if local(&e) == "Relationship" => {
                if let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    map.insert(id, target);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    map
}

/// presentation.xml.rels 的 target 形如 `slides/slide1.xml` → `ppt/slides/slide1.xml`。
fn normalize_ppt_path(target: &str) -> String {
    let t = target.trim_start_matches("/");
    if t.starts_with("ppt/") {
        t.to_string()
    } else {
        format!("ppt/{t}")
    }
}

/// slide rels 的图片 target 形如 `../media/image1.png` → `ppt/media/image1.png`。
fn normalize_media_path(target: &str) -> String {
    let t = target.trim_start_matches("/");
    if let Some(rest) = t.strip_prefix("../") {
        format!("ppt/{rest}")
    } else if t.starts_with("ppt/") {
        t.to_string()
    } else {
        format!("ppt/slides/{t}")
    }
}

fn mime_of(path: &str) -> String {
    let l = path.to_ascii_lowercase();
    if l.ends_with(".png") {
        "image/png"
    } else if l.ends_with(".jpg") || l.ends_with(".jpeg") {
        "image/jpeg"
    } else if l.ends_with(".gif") {
        "image/gif"
    } else if l.ends_with(".bmp") {
        "image/bmp"
    } else if l.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
    .to_string()
}

// ---------- 幻灯 spTree 解析 ----------

#[derive(Default)]
struct ShapeBuilder {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    geom: Option<String>,
    fill: Option<String>,
    image: Option<String>,
    paragraphs: Vec<Para>,
}

impl ShapeBuilder {
    fn finish(self) -> Shape {
        Shape {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            geom: self.geom,
            fill: self.fill,
            image: self.image,
            paragraphs: self.paragraphs,
        }
    }
    fn is_renderable(&self) -> bool {
        // 有显式几何(位置/尺寸)或有文本才渲染;占位符无 xfrm 则跳过
        self.width > 0.0 || self.height > 0.0 || !self.paragraphs.is_empty()
    }
}

/// 解析单张幻灯的形状树。用元素名栈跟踪上下文(区分 spPr 填充 vs rPr 颜色等)。
fn parse_slide(xml: &str) -> Vec<Shape> {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut shapes = Vec::new();
    let mut stack: Vec<String> = Vec::new();

    let mut cur: Option<ShapeBuilder> = None;
    let mut cur_para: Option<Para> = None;
    let mut cur_run: Option<Run> = None;

    while let Ok(ev) = reader.read_event_into(&mut buf) {
        match ev {
            // Start:处理后压栈(有对应 End)
            Event::Start(e) => {
                let name = local(&e);
                handle_start(&e, &name, &stack, &mut cur, &mut cur_para, &mut cur_run);
                stack.push(name);
            }
            // Empty:处理但**不压栈**(无对应 End),用当前栈作为其上下文
            Event::Empty(e) => {
                let name = local(&e);
                handle_start(&e, &name, &stack, &mut cur, &mut cur_para, &mut cur_run);
            }
            Event::End(_e) => {
                let name = stack.pop().unwrap_or_default();
                handle_end(&name, &mut shapes, &mut cur, &mut cur_para, &mut cur_run);
            }
            Event::Text(t) => {
                if stack.last().map(|s| s.as_str()) == Some("t") {
                    if let Some(run) = cur_run.as_mut() {
                        if let Ok(s) = t.decode() {
                            run.text.push_str(&s);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    shapes
}

fn handle_start(
    e: &BytesStart,
    name: &str,
    stack: &[String],
    cur: &mut Option<ShapeBuilder>,
    cur_para: &mut Option<Para>,
    cur_run: &mut Option<Run>,
) {
    match name {
        "sp" | "pic" => {
            *cur = Some(ShapeBuilder::default());
        }
        "off" => {
            if let Some(b) = cur.as_mut() {
                if let Some(x) = attr(e, "x").and_then(|s| s.parse::<f64>().ok()) {
                    b.x = emu(x);
                }
                if let Some(y) = attr(e, "y").and_then(|s| s.parse::<f64>().ok()) {
                    b.y = emu(y);
                }
            }
        }
        "ext" => {
            if let Some(b) = cur.as_mut() {
                if let Some(cx) = attr(e, "cx").and_then(|s| s.parse::<f64>().ok()) {
                    b.width = emu(cx);
                }
                if let Some(cy) = attr(e, "cy").and_then(|s| s.parse::<f64>().ok()) {
                    b.height = emu(cy);
                }
            }
        }
        "prstGeom" => {
            if let Some(b) = cur.as_mut() {
                b.geom = attr(e, "prst");
            }
        }
        "blip" => {
            if let Some(b) = cur.as_mut() {
                if let Some(id) = attr_ns(e, "embed") {
                    b.image = Some(id);
                }
            }
        }
        "srgbClr" => {
            let val = attr(e, "val");
            if let Some(v) = val {
                if stack.iter().any(|s| s == "rPr") {
                    if let Some(r) = cur_run.as_mut() {
                        r.color = Some(v);
                    }
                } else if stack.iter().any(|s| s == "spPr")
                    && stack.iter().any(|s| s == "solidFill")
                    && !stack.iter().any(|s| s == "ln")
                {
                    if let Some(b) = cur.as_mut() {
                        b.fill = Some(v);
                    }
                }
            }
        }
        "p" => {
            *cur_para = Some(Para {
                align: Align::Left,
                runs: Vec::new(),
            });
        }
        "pPr" => {
            if let Some(p) = cur_para.as_mut() {
                if let Some(a) = attr(e, "algn") {
                    p.align = match a.as_str() {
                        "ctr" => Align::Center,
                        "r" => Align::Right,
                        "just" | "dist" => Align::Justify,
                        _ => Align::Left,
                    };
                }
            }
        }
        "r" => {
            *cur_run = Some(Run::default());
        }
        "rPr" => {
            if let Some(r) = cur_run.as_mut() {
                if let Some(sz) = attr(e, "sz").and_then(|s| s.parse::<f64>().ok()) {
                    r.size_pt = Some(sz / 100.0);
                }
                if attr(e, "b").as_deref() == Some("1") {
                    r.bold = true;
                }
                if attr(e, "i").as_deref() == Some("1") {
                    r.italic = true;
                }
            }
        }
        _ => {}
    }
}

fn handle_end(
    name: &str,
    shapes: &mut Vec<Shape>,
    cur: &mut Option<ShapeBuilder>,
    cur_para: &mut Option<Para>,
    cur_run: &mut Option<Run>,
) {
    match name {
        "r" => {
            if let (Some(run), Some(para)) = (cur_run.take(), cur_para.as_mut()) {
                if !run.text.is_empty() {
                    para.runs.push(run);
                }
            }
        }
        "p" => {
            if let (Some(para), Some(b)) = (cur_para.take(), cur.as_mut()) {
                b.paragraphs.push(para);
            }
        }
        "sp" | "pic" => {
            if let Some(b) = cur.take() {
                if b.is_renderable() {
                    shapes.push(b.finish());
                }
            }
        }
        _ => {}
    }
}

/// 元素本地名(去命名空间前缀)。
fn local(e: &BytesStart) -> String {
    let full = e.name();
    let bytes = full.as_ref();
    let name = std::str::from_utf8(bytes).unwrap_or("");
    name.rsplit(':').next().unwrap_or(name).to_string()
}

/// 取属性(按本地名匹配,忽略命名空间前缀)。
fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let k = a.key;
        let kb = k.as_ref();
        let kname = std::str::from_utf8(kb).ok()?;
        let klocal = kname.rsplit(':').next().unwrap_or(kname);
        if klocal == key {
            Some(String::from_utf8_lossy(&a.value).to_string())
        } else {
            None
        }
    })
}

/// 取带命名空间的属性(如 `r:embed` / `r:id`),按本地名匹配。
fn attr_ns(e: &BytesStart, local_key: &str) -> Option<String> {
    attr(e, local_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, data) in files {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    const SLIDE: &[u8] = br#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="r">
      <p:cSld><p:spTree>
        <p:sp>
          <p:spPr>
            <a:xfrm><a:off x="914400" y="457200"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
            <a:prstGeom prst="rect"/>
            <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
          </p:spPr>
          <p:txBody>
            <a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="1800" b="1"/><a:t>Hello</a:t></a:r></a:p>
          </p:txBody>
        </p:sp>
        <p:pic>
          <p:blipFill><a:blip r:embed="rId2"/></p:blipFill>
          <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
        </p:pic>
      </p:spTree></p:cSld>
    </p:sld>"#;

    #[test]
    fn parses_shapes_text_geometry_fill() {
        let shapes = parse_slide(std::str::from_utf8(SLIDE).unwrap());
        assert_eq!(shapes.len(), 2);
        let sp = &shapes[0];
        assert_eq!(sp.x, 96.0); // 914400 EMU = 96px
        assert_eq!(sp.y, 48.0);
        assert_eq!(sp.width, 192.0);
        assert_eq!(sp.geom.as_deref(), Some("rect"));
        assert_eq!(sp.fill.as_deref(), Some("FF0000"));
        assert_eq!(sp.paragraphs.len(), 1);
        assert_eq!(sp.paragraphs[0].align, Align::Center);
        let run = &sp.paragraphs[0].runs[0];
        assert_eq!(run.text, "Hello");
        assert!(run.bold);
        assert_eq!(run.size_pt, Some(18.0));

        let pic = &shapes[1];
        assert_eq!(pic.image.as_deref(), Some("rId2"));
    }

    #[test]
    fn parses_presentation_size_and_order() {
        let pres = br#"<p:presentation xmlns:p="p" xmlns:r="r">
          <p:sldSz cx="9144000" cy="6858000"/>
          <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
        </p:presentation>"#;
        let rels = br#"<Relationships xmlns="x">
          <Relationship Id="rId1" Type="t" Target="slides/slide1.xml"/>
        </Relationships>"#;
        let pptx = zip_of(&[
            ("ppt/presentation.xml", pres),
            ("ppt/_rels/presentation.xml.rels", rels),
            ("ppt/slides/slide1.xml", SLIDE),
        ]);
        let parsed = parse(&pptx).expect("解析");
        assert_eq!(parsed.presentation.width_px, 960.0); // 9144000/9525
        assert_eq!(parsed.presentation.height_px, 720.0);
        assert_eq!(parsed.presentation.slides.len(), 1);
        assert_eq!(parsed.presentation.slides[0].shapes.len(), 2);
    }

    #[test]
    fn resolves_slide_images() {
        let pres = br#"<p:presentation xmlns:p="p" xmlns:r="r">
          <p:sldSz cx="9144000" cy="6858000"/>
          <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
        </p:presentation>"#;
        let pres_rels = br#"<Relationships xmlns="x"><Relationship Id="rId1" Type="t" Target="slides/slide1.xml"/></Relationships>"#;
        let slide_rels = br#"<Relationships xmlns="x"><Relationship Id="rId2" Type="image" Target="../media/image1.png"/></Relationships>"#;
        let png: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 1, 2, 3];
        let pptx = zip_of(&[
            ("ppt/presentation.xml", pres),
            ("ppt/_rels/presentation.xml.rels", pres_rels),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/slides/_rels/slide1.xml.rels", slide_rels),
            ("ppt/media/image1.png", png),
        ]);
        let parsed = parse(&pptx).expect("解析");
        assert_eq!(parsed.images.len(), 1);
        assert_eq!(parsed.images[0].mime, "image/png");
        assert_eq!(parsed.image_index.get(&(0, "rId2".to_string())), Some(&0));
    }

    #[test]
    fn invalid_bytes_error() {
        assert!(parse(b"not a pptx").is_err());
    }
}

#[cfg(test)]
mod fixture_gen {
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xFD,
        0xD4, 0x9A, 0x73, 0x00, 0x00, 0x00, 0x16, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0xF0, 0x1F, 0x8C, 0x0C, 0x0C, 0x0C, 0x00, 0x00, 0x0C, 0x0C, 0x02, 0xFC, 0x8B,
        0x8D, 0xB0, 0x8D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn slide_xml(title: &str, body: &str, with_pic: bool) -> String {
        let pic = if with_pic {
            r#"<p:pic><p:blipFill><a:blip r:embed="rId1"/></p:blipFill>
               <p:spPr><a:xfrm><a:off x="5000000" y="3000000"/><a:ext cx="2000000" cy="1500000"/></a:xfrm></p:spPr></p:pic>"#
        } else {
            r#"<p:sp><p:spPr><a:xfrm><a:off x="5000000" y="2500000"/><a:ext cx="2500000" cy="1500000"/></a:xfrm>
               <a:prstGeom prst="ellipse"/><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></p:spPr></p:sp>"#
        };
        format!(
            r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="r"><p:cSld><p:spTree>
             <p:sp><p:spPr><a:xfrm><a:off x="838200" y="365760"/><a:ext cx="7772400" cy="1000000"/></a:xfrm>
               <a:prstGeom prst="rect"/></p:spPr>
               <p:txBody><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="3200" b="1"><a:solidFill><a:srgbClr val="1F3864"/></a:solidFill></a:rPr><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
             <p:sp><p:spPr><a:xfrm><a:off x="838200" y="1800000"/><a:ext cx="7772400" cy="2000000"/></a:xfrm></p:spPr>
               <p:txBody><a:p><a:r><a:rPr sz="2000"/><a:t>{body}</a:t></a:r></a:p></p:txBody></p:sp>
             {pic}
           </p:spTree></p:cSld></p:sld>"#
        )
    }

    #[test]
    #[ignore]
    fn write_browser_fixture() {
        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let put = |w: &mut ZipWriter<Cursor<&mut Vec<u8>>>, name: &str, data: &[u8]| {
                w.start_file(name, opts).unwrap();
                w.write_all(data).unwrap();
            };
            put(&mut w, "[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/></Types>"#);
            put(&mut w, "_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="ppt/presentation.xml"/></Relationships>"#);
            put(&mut w, "ppt/presentation.xml", br#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldSz cx="9144000" cy="6858000"/><p:sldIdLst><p:sldId id="256" r:id="rId1"/><p:sldId id="257" r:id="rId2"/></p:sldIdLst></p:presentation>"#);
            put(&mut w, "ppt/_rels/presentation.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="slides/slide1.xml"/><Relationship Id="rId2" Type="t" Target="slides/slide2.xml"/></Relationships>"#);
            put(
                &mut w,
                "ppt/slides/slide1.xml",
                slide_xml(
                    "office-R 演示渲染",
                    "第一张:标题 + 蓝色图片(右下)+ 正文",
                    true,
                )
                .as_bytes(),
            );
            put(&mut w, "ppt/slides/_rels/slide1.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="image" Target="../media/image1.png"/></Relationships>"#);
            put(
                &mut w,
                "ppt/slides/slide2.xml",
                slide_xml("第二张幻灯", "居中标题 + 椭圆形状,演示模式可翻页", false).as_bytes(),
            );
            put(&mut w, "ppt/media/image1.png", PNG);
            w.finish().unwrap();
        }
        std::fs::write("/tmp/office-r-sample.pptx", &buf).unwrap();
        eprintln!("wrote /tmp/office-r-sample.pptx ({} bytes)", buf.len());
    }
}
