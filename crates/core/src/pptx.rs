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
//! **已支持**:占位符几何**继承**(slide 无 xfrm → 借 slideLayout → slideMaster)、
//! 主题配色 `schemeClr`(解析 `ppt/theme/theme1.xml` 的 `clrScheme`,含 tx1/bg1→dk1/lt1 默认映射)、
//! 旋转/翻转(`a:xfrm@rot/@flipH/@flipV`)、文本默认样式继承(母版 `p:txStyles`)、
//! 动画/切换标记(`p:timing`/`p:transition` → `has_animation`/`has_transition`)、
//! 图表/SmartArt 占位(`p:graphicFrame` → `placeholder_kind`,仅占位框 + 类型标签)、
//! 组合形状 `p:grpSp`(按 `chOff`/`chExt`→`off`/`ext` 映射子坐标,支持嵌套)、
//! 渐变填充 `a:gradFill`(取首/末停靠色,视图上→下线性渐变)、
//! 内嵌表格 `a:tbl`(列宽/行/单元格文本 → 视图绘制真实网格)。
//! **非目标**:动画/切换的具体时间线回放、图表/SmartArt 的真实绘制、自定义几何、图片填充、阴影效果、
//! 表格单元格合并/样式。

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
    /// 旋转角度(度,顺时针);`a:xfrm@rot` 为 1/60000 度,已换算。
    #[serde(default)]
    pub rotation: f64,
    /// 水平翻转 / 垂直翻转(`a:xfrm@flipH/@flipV`)。
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
    /// 内容占位类型:`"chart"` / `"diagram"`(SmartArt) / `"table"`;普通形状为 `None`。
    /// 这些是 `p:graphicFrame` 里的内嵌对象,本期只渲染占位框 + 类型标签。
    #[serde(default)]
    pub placeholder_kind: Option<String>,
    /// 渐变填充的两端色(首/末停靠点 `RRGGBB`);无渐变为 `None`。视图按上→下线性渐变绘制。
    #[serde(default)]
    pub gradient: Option<(String, String)>,
    /// 内嵌表格(`a:tbl`);非表格为 `None`。有表格时视图绘制真实网格 + 单元格文本。
    #[serde(default)]
    pub table: Option<Table>,
}

/// 幻灯内表格(来自 `p:graphicFrame` 的 `a:tbl`)。
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Table {
    /// 各列宽(像素;`a:gridCol@w` EMU 换算)。
    pub col_widths: Vec<f64>,
    /// 行:每行是各单元格的纯文本(合并单元格的被跨单元格为空串)。
    pub rows: Vec<Vec<String>>,
}

/// 一张幻灯片。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Slide {
    pub shapes: Vec<Shape>,
    /// 该幻灯是否含动画(`p:timing`)。
    #[serde(default)]
    pub has_animation: bool,
    /// 该幻灯是否含切换效果(`p:transition`)。
    #[serde(default)]
    pub has_transition: bool,
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
    let theme = load_theme(&mut zip);

    let mut slides = Vec::new();
    let mut images = Vec::new();
    let mut image_index = HashMap::new();

    for (idx, path) in slide_paths.iter().enumerate() {
        let xml = match read_zip_text(&mut zip, path) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let rels = read_slide_rels(&mut zip, path);
        let (fallback, text_defaults) = layout_master_geom(&mut zip, &rels);
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &text_defaults,
        };
        let slide = parse_slide(&xml, &ctx);
        slides.push(slide);

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
    /// 占位符键 `type|idx`(用于向版式/母版借几何);非占位符为 `None`。
    ph: Option<String>,
    /// 本形状是否带显式 `a:xfrm`(有则不向版式借几何)。
    has_xfrm: bool,
    /// 旋转(度)与翻转。
    rotation: f64,
    flip_h: bool,
    flip_v: bool,
    /// graphicFrame 内容类型(chart/diagram/table)。
    placeholder_kind: Option<String>,
    /// 渐变填充停靠色(按出现顺序);finish 时取首/末为两端。
    grad_stops: Vec<String>,
    /// 内嵌表格(累积中);无表格为 `None`。
    table: Option<Table>,
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
            rotation: self.rotation,
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            placeholder_kind: self.placeholder_kind,
            gradient: match (self.grad_stops.first(), self.grad_stops.last()) {
                (Some(a), Some(b)) => Some((a.clone(), b.clone())),
                _ => None,
            },
            table: self.table,
        }
    }
    fn is_renderable(&self) -> bool {
        // 有显式几何(位置/尺寸)或有文本或是内容占位或有表格才渲染
        self.width > 0.0
            || self.height > 0.0
            || !self.paragraphs.is_empty()
            || self.placeholder_kind.is_some()
            || self.table.is_some()
    }
}

/// 主题配色:scheme 名(dk1/lt1/accent1/…)→ `RRGGBB`。
type Theme = std::collections::HashMap<String, String>;
/// 占位符几何:`type|idx` → (x, y, w, h)(像素)。
type PhGeom = std::collections::HashMap<String, (f64, f64, f64, f64)>;

/// 文本默认样式(来自母版 `p:txStyles`),按 title / body / other 三类的 lvl1 defRPr。
#[derive(Default, Clone)]
struct TextDefaults {
    /// (字号磅, 颜色 RRGGBB) 三类默认。
    title: (Option<f64>, Option<String>),
    body: (Option<f64>, Option<String>),
    other: (Option<f64>, Option<String>),
}

impl TextDefaults {
    /// 按占位符类型选默认:title/ctrTitle→title;body/subTitle→body;其余→other。
    fn for_ph(&self, ph: &Option<String>) -> &(Option<f64>, Option<String>) {
        let ty = ph.as_deref().unwrap_or("").split('|').next().unwrap_or("");
        match ty {
            "title" | "ctrTitle" => &self.title,
            "body" | "subTitle" => &self.body,
            _ => &self.other,
        }
    }
}

/// 幻灯解析上下文:主题配色 + 占位符几何回退 + 文本默认样式(来自版式/母版)。
struct SlideCtx<'a> {
    theme: &'a Theme,
    fallback: &'a PhGeom,
    text_defaults: &'a TextDefaults,
}

/// 组合形状 `p:grpSp` 的坐标变换:把**子坐标系**里的形状映射到父(幻灯)坐标系。
///
/// 组的 `a:xfrm` 同时给出组在父系里的位置/尺寸(`off`/`ext`)与子坐标系的原点/范围
/// (`chOff`/`chExt`)。子形状 `(x,y,w,h)` 映射:
/// `sx = ext/chExt`,`X = off + (x - chOff) * sx`,`W = w * sx`(y 同理)。
/// 嵌套组由内到外依次套用。
#[derive(Default, Clone)]
struct GroupXform {
    off_x: f64,
    off_y: f64,
    ext_cx: f64,
    ext_cy: f64,
    ch_off_x: f64,
    ch_off_y: f64,
    ch_ext_cx: f64,
    ch_ext_cy: f64,
}

impl GroupXform {
    fn map(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let sx = if self.ch_ext_cx != 0.0 {
            self.ext_cx / self.ch_ext_cx
        } else {
            1.0
        };
        let sy = if self.ch_ext_cy != 0.0 {
            self.ext_cy / self.ch_ext_cy
        } else {
            1.0
        };
        (
            self.off_x + (x - self.ch_off_x) * sx,
            self.off_y + (y - self.ch_off_y) * sy,
            w * sx,
            h * sy,
        )
    }
}

/// 解析单张幻灯的形状树。用元素名栈跟踪上下文(区分 spPr 填充 vs rPr 颜色等)。
///
/// `ctx` 提供主题配色(解析 `schemeClr`)与占位符几何回退(占位符 sp 无 xfrm 时借用版式/母版)。
fn parse_slide(xml: &str, ctx: &SlideCtx) -> Slide {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut shapes = Vec::new();
    let mut stack: Vec<String> = Vec::new();

    let mut cur: Option<ShapeBuilder> = None;
    let mut cur_para: Option<Para> = None;
    let mut cur_run: Option<Run> = None;
    // graphicFrame:内嵌 chart/diagram/table,当作一个「内容占位」形状
    let mut has_animation = false;
    let mut has_transition = false;
    // 组合形状变换栈(外→内);子形状落盘时由内到外套用映射到幻灯坐标
    let mut groups: Vec<GroupXform> = Vec::new();
    // 表格累积:是否在表格内 / 当前单元格文本缓冲(在单元格内则文本走此处而非形状段落)
    let mut in_cell = false;
    let mut cell_text = String::new();

    while let Ok(ev) = reader.read_event_into(&mut buf) {
        match ev {
            // Start:处理后压栈(有对应 End)
            Event::Start(e) => {
                let name = local(&e);
                match name.as_str() {
                    // 动画 / 切换:只标记存在
                    "timing" => has_animation = true,
                    "transition" => has_transition = true,
                    // 组合形状:压一层变换(其 xfrm 的 off/ext/chOff/chExt 随后填入)
                    "grpSp" => groups.push(GroupXform::default()),
                    // graphicFrame 起始:作为一个内容占位形状
                    "graphicFrame" => cur = Some(ShapeBuilder::default()),
                    // 表格:在当前形状上开始累积
                    "tbl" => {
                        if let Some(b) = cur.as_mut() {
                            b.table = Some(Table::default());
                            b.placeholder_kind = Some("table".to_string());
                        }
                    }
                    "gridCol" => {
                        if let Some(w) = attr(&e, "w").and_then(|s| s.parse::<f64>().ok()) {
                            if let Some(t) = cur.as_mut().and_then(|b| b.table.as_mut()) {
                                t.col_widths.push(emu(w));
                            }
                        }
                    }
                    "tr" => {
                        if let Some(t) = cur.as_mut().and_then(|b| b.table.as_mut()) {
                            t.rows.push(Vec::new());
                        }
                    }
                    "tc" if cur.as_ref().and_then(|b| b.table.as_ref()).is_some() => {
                        // 新单元格:开始文本累积
                        in_cell = true;
                        cell_text.clear();
                    }
                    _ => {}
                }
                // 表格内的 gridCol/tr/tc(及其 Empty 形式)不走形状文本路径
                if in_cell || cur.as_ref().and_then(|b| b.table.as_ref()).is_some() {
                    stack.push(name);
                    buf.clear();
                    continue;
                }
                handle_start(
                    &e,
                    &name,
                    &stack,
                    ctx.theme,
                    &mut cur,
                    &mut cur_para,
                    &mut cur_run,
                );
                stack.push(name);
            }
            // Empty:处理但**不压栈**(无对应 End),用当前栈作为其上下文
            Event::Empty(e) => {
                let name = local(&e);
                if name == "transition" {
                    has_transition = true;
                }
                // graphicData@uri 指明内容类型(chart/diagram/table)
                if name == "graphicData" {
                    if let Some(b) = cur.as_mut() {
                        b.placeholder_kind = graphic_kind(&attr(&e, "uri").unwrap_or_default());
                    }
                }
                // 表格列宽(gridCol 常为空元素)
                if name == "gridCol" {
                    if let Some(w) = attr(&e, "w").and_then(|s| s.parse::<f64>().ok()) {
                        if let Some(t) = cur.as_mut().and_then(|b| b.table.as_mut()) {
                            t.col_widths.push(emu(w));
                        }
                    }
                }
                // 表格内不走形状文本/填充路径
                if in_cell || cur.as_ref().and_then(|b| b.table.as_ref()).is_some() {
                    buf.clear();
                    continue;
                }
                // 组 xfrm 的 off/ext/chOff/chExt:无活动形状(cur=None)且在组内时,填入当前组
                let group_ctx = cur.is_none() && !groups.is_empty();
                if group_ctx && matches!(name.as_str(), "off" | "ext" | "chOff" | "chExt") {
                    if let Some(g) = groups.last_mut() {
                        fill_group_xform(g, &name, &e);
                    }
                } else {
                    handle_start(
                        &e,
                        &name,
                        &stack,
                        ctx.theme,
                        &mut cur,
                        &mut cur_para,
                        &mut cur_run,
                    );
                }
            }
            Event::End(_e) => {
                let name = stack.pop().unwrap_or_default();
                // 表格内元素:tc 收尾落一格文本;其它表格内元素跳过(不触发形状文本收尾)。
                // 唯 graphicFrame 例外——它是外层容器,End 时才把带表格的形状落盘。
                let in_table = cur.as_ref().and_then(|b| b.table.as_ref()).is_some();
                if name == "tc" {
                    if let Some(t) = cur.as_mut().and_then(|b| b.table.as_mut()) {
                        if let Some(row) = t.rows.last_mut() {
                            row.push(cell_text.trim().to_string());
                        }
                    }
                    in_cell = false;
                    cell_text.clear();
                    buf.clear();
                    continue;
                }
                if in_table && name != "graphicFrame" {
                    buf.clear();
                    continue;
                }
                if name == "grpSp" {
                    groups.pop();
                } else {
                    let end_name = if name == "graphicFrame" {
                        "sp"
                    } else {
                        name.as_str()
                    };
                    let before = shapes.len();
                    handle_end(
                        end_name,
                        ctx.fallback,
                        ctx.text_defaults,
                        &mut shapes,
                        &mut cur,
                        &mut cur_para,
                        &mut cur_run,
                    );
                    // 新落盘的形状若在组内,按组变换映射到幻灯坐标(由内到外)
                    if shapes.len() > before && !groups.is_empty() {
                        if let Some(s) = shapes.last_mut() {
                            let (mut x, mut y, mut w, mut h) = (s.x, s.y, s.width, s.height);
                            for g in groups.iter().rev() {
                                let m = g.map(x, y, w, h);
                                x = m.0;
                                y = m.1;
                                w = m.2;
                                h = m.3;
                            }
                            s.x = x;
                            s.y = y;
                            s.width = w;
                            s.height = h;
                        }
                    }
                }
            }
            Event::Text(t) => {
                if stack.last().map(|s| s.as_str()) == Some("t") {
                    if let Ok(s) = t.decode() {
                        if in_cell {
                            cell_text.push_str(&s);
                        } else if let Some(run) = cur_run.as_mut() {
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

    Slide {
        shapes,
        has_animation,
        has_transition,
    }
}

/// 把组 `a:xfrm` 的 `off`/`ext`/`chOff`/`chExt`(EMU→px)填入组变换。
fn fill_group_xform(g: &mut GroupXform, name: &str, e: &BytesStart) {
    let x = attr(e, "x").and_then(|s| s.parse::<f64>().ok());
    let y = attr(e, "y").and_then(|s| s.parse::<f64>().ok());
    let cx = attr(e, "cx").and_then(|s| s.parse::<f64>().ok());
    let cy = attr(e, "cy").and_then(|s| s.parse::<f64>().ok());
    match name {
        "off" => {
            if let Some(x) = x {
                g.off_x = emu(x);
            }
            if let Some(y) = y {
                g.off_y = emu(y);
            }
        }
        "ext" => {
            if let Some(cx) = cx {
                g.ext_cx = emu(cx);
            }
            if let Some(cy) = cy {
                g.ext_cy = emu(cy);
            }
        }
        "chOff" => {
            if let Some(x) = x {
                g.ch_off_x = emu(x);
            }
            if let Some(y) = y {
                g.ch_off_y = emu(y);
            }
        }
        "chExt" => {
            if let Some(cx) = cx {
                g.ch_ext_cx = emu(cx);
            }
            if let Some(cy) = cy {
                g.ch_ext_cy = emu(cy);
            }
        }
        _ => {}
    }
}

/// graphicData@uri → 内容类型标签。
fn graphic_kind(uri: &str) -> Option<String> {
    if uri.contains("/chart") {
        Some("chart".to_string())
    } else if uri.contains("diagram") || uri.contains("smartart") {
        Some("diagram".to_string())
    } else if uri.contains("/table") {
        Some("table".to_string())
    } else {
        None
    }
}

/// 加载主题配色:从 `ppt/theme/theme1.xml` 的 `a:clrScheme` 取各 scheme 名 → RRGGBB。
/// 每个 scheme 子元素(dk1/lt1/dk2/lt2/accent1..6/hlink/folHlink)含 `srgbClr@val` 或 `sysClr@lastClr`。
fn load_theme(zip: &mut ZipArchive<Cursor<Vec<u8>>>) -> Theme {
    let mut theme = Theme::new();
    let xml = match read_zip_text(zip, "ppt/theme/theme1.xml") {
        Ok(x) => x,
        Err(_) => return theme,
    };
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    let mut in_scheme = false;
    let mut cur_name: Option<String> = None; // 当前 scheme 槽名(dk1/accent1/…)
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let n = local(&e);
                if n == "clrScheme" {
                    in_scheme = true;
                } else if in_scheme && cur_name.is_none() && n != "srgbClr" && n != "sysClr" {
                    cur_name = Some(n);
                }
            }
            Ok(Event::Empty(e)) => {
                let n = local(&e);
                if in_scheme {
                    if let Some(slot) = cur_name.clone() {
                        if n == "srgbClr" {
                            if let Some(v) = attr(&e, "val") {
                                theme.insert(slot, v);
                                cur_name = None;
                            }
                        } else if n == "sysClr" {
                            if let Some(v) = attr(&e, "lastClr") {
                                theme.insert(slot, v);
                                cur_name = None;
                            }
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let n = local_end(&e);
                if n == "clrScheme" {
                    break;
                }
                // scheme 槽结束(如 </a:dk1>):清空当前槽名
                if in_scheme && Some(&n) == cur_name.as_ref() {
                    cur_name = None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    theme
}

/// 把 `schemeClr@val`(tx1/bg1/accent1/…)解析成 RRGGBB。
/// 标准默认 clrMap:tx1→dk1、bg1→lt1、tx2→dk2、bg2→lt2;其余同名直查。
fn resolve_scheme_color(theme: &Theme, val: &str) -> Option<String> {
    let key = match val {
        "tx1" => "dk1",
        "bg1" => "lt1",
        "tx2" => "dk2",
        "bg2" => "lt2",
        other => other,
    };
    theme.get(key).or_else(|| theme.get(val)).cloned()
}

/// 从 slide rels 找到版式与母版,提取占位符几何(`type|idx` → 像素矩形)。母版打底、版式覆盖。
fn layout_master_geom(
    zip: &mut ZipArchive<Cursor<Vec<u8>>>,
    slide_rels: &[(String, String)],
) -> (PhGeom, TextDefaults) {
    let mut geom = PhGeom::new();
    let mut defaults = TextDefaults::default();
    // slide → 版式
    let layout_path = slide_rels
        .iter()
        .find(|(_, t)| t.contains("slideLayout"))
        .map(|(_, t)| normalize_ppt_path(&t.replace("../", "")));
    let Some(layout_path) = layout_path else {
        return (geom, defaults);
    };
    // 版式 → 母版
    let layout_rels = read_rels_for(zip, &layout_path);
    if let Some(master) = layout_rels
        .iter()
        .find(|(_, t)| t.contains("slideMaster"))
        .map(|(_, t)| normalize_ppt_path(&t.replace("../", "")))
    {
        if let Ok(xml) = read_zip_text(zip, &master) {
            for (k, v) in collect_placeholder_geom(&xml) {
                geom.insert(k, v); // 母版打底
            }
            defaults = collect_text_defaults(&xml); // 文本默认样式来自母版 txStyles
        }
    }
    if let Ok(xml) = read_zip_text(zip, &layout_path) {
        for (k, v) in collect_placeholder_geom(&xml) {
            geom.insert(k, v); // 版式覆盖母版
        }
    }
    (geom, defaults)
}

/// 从母版 `p:txStyles` 提取 title/body/other 的 lvl1 `a:defRPr`(字号磅 + 颜色)。
fn collect_text_defaults(xml: &str) -> TextDefaults {
    let mut out = TextDefaults::default();
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    // 当前处于哪个样式块;进入 lvl1pPr 后遇到的 defRPr 记录其 sz/color
    let mut style: Option<&'static str> = None;
    let mut in_lvl1 = false;
    let mut in_defrpr = false;
    let mut cur_sz: Option<f64> = None;
    let mut cur_color: Option<String> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let n = local(&e);
                match n.as_str() {
                    "titleStyle" => style = Some("title"),
                    "bodyStyle" => style = Some("body"),
                    "otherStyle" => style = Some("other"),
                    "lvl1pPr" => in_lvl1 = true,
                    "defRPr" if in_lvl1 => {
                        in_defrpr = true;
                        cur_sz = attr(&e, "sz")
                            .and_then(|s| s.parse::<f64>().ok())
                            .map(|v| v / 100.0);
                        cur_color = None;
                    }
                    "srgbClr" if in_defrpr => {
                        cur_color = attr(&e, "val");
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let n = local_end(&e);
                match n.as_str() {
                    "defRPr" if in_lvl1 => {
                        if let Some(s) = style {
                            let slot = match s {
                                "title" => &mut out.title,
                                "body" => &mut out.body,
                                _ => &mut out.other,
                            };
                            if cur_sz.is_some() {
                                slot.0 = cur_sz;
                            }
                            if cur_color.is_some() {
                                slot.1 = cur_color.clone();
                            }
                        }
                        in_defrpr = false;
                    }
                    "lvl1pPr" => in_lvl1 = false,
                    "titleStyle" | "bodyStyle" | "otherStyle" => style = None,
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// 读取任意部件的 `_rels/<file>.rels`。
fn read_rels_for(zip: &mut ZipArchive<Cursor<Vec<u8>>>, part_path: &str) -> Vec<(String, String)> {
    let rels_path = match part_path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part_path}.rels"),
    };
    match read_zip_text(zip, &rels_path) {
        Ok(xml) => parse_rels(&xml).into_iter().collect(),
        Err(_) => Vec::new(),
    }
}

/// 从版式/母版 XML 提取占位符几何:遍历 sp,记 ph 键与 xfrm,有 xfrm 者入表。
fn collect_placeholder_geom(xml: &str) -> PhGeom {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut out = PhGeom::new();
    let mut ph: Option<String> = None;
    let mut xfrm: Option<(f64, f64, f64, f64)> = None;
    let mut in_sp = false;
    let (mut x, mut y, mut w, mut h) = (0.0, 0.0, 0.0, 0.0);
    let mut has_off = false;
    let mut has_ext = false;

    macro_rules! handle_geom {
        ($e:expr, $n:expr) => {
            match $n {
                "ph" => {
                    let ty = attr($e, "type").unwrap_or_default();
                    let idx = attr($e, "idx").unwrap_or_default();
                    ph = Some(format!("{ty}|{idx}"));
                }
                "off" => {
                    x = attr($e, "x")
                        .and_then(|s| s.parse().ok())
                        .map(emu)
                        .unwrap_or(0.0);
                    y = attr($e, "y")
                        .and_then(|s| s.parse().ok())
                        .map(emu)
                        .unwrap_or(0.0);
                    has_off = true;
                }
                "ext" => {
                    w = attr($e, "cx")
                        .and_then(|s| s.parse().ok())
                        .map(emu)
                        .unwrap_or(0.0);
                    h = attr($e, "cy")
                        .and_then(|s| s.parse().ok())
                        .map(emu)
                        .unwrap_or(0.0);
                    has_ext = true;
                }
                _ => {}
            }
        };
    }

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let n = local(&e);
                if n == "sp" {
                    in_sp = true;
                    ph = None;
                    has_off = false;
                    has_ext = false;
                }
                if in_sp {
                    handle_geom!(&e, n.as_str());
                }
            }
            Ok(Event::Empty(e)) => {
                if in_sp {
                    let n = local(&e);
                    handle_geom!(&e, n.as_str());
                }
            }
            Ok(Event::End(e)) => {
                if local_end(&e) == "sp" {
                    if has_off && has_ext {
                        xfrm = Some((x, y, w, h));
                    }
                    if let (Some(k), Some(g)) = (ph.take(), xfrm.take()) {
                        out.entry(k).or_insert(g);
                    }
                    in_sp = false;
                    xfrm = None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn handle_start(
    e: &BytesStart,
    name: &str,
    stack: &[String],
    theme: &Theme,
    cur: &mut Option<ShapeBuilder>,
    cur_para: &mut Option<Para>,
    cur_run: &mut Option<Run>,
) {
    match name {
        "sp" | "pic" => {
            *cur = Some(ShapeBuilder::default());
        }
        "ph" => {
            // 占位符标识:type|idx(缺省空),用于向版式/母版借几何
            if let Some(b) = cur.as_mut() {
                let ty = attr(e, "type").unwrap_or_default();
                let idx = attr(e, "idx").unwrap_or_default();
                b.ph = Some(format!("{ty}|{idx}"));
            }
        }
        "xfrm" => {
            // 旋转(1/60000 度)与翻转在 xfrm 自身;off/ext 是其子元素
            if let Some(b) = cur.as_mut() {
                if let Some(rot) = attr(e, "rot").and_then(|s| s.parse::<f64>().ok()) {
                    b.rotation = rot / 60000.0;
                }
                if attr(e, "flipH").as_deref() == Some("1") {
                    b.flip_h = true;
                }
                if attr(e, "flipV").as_deref() == Some("1") {
                    b.flip_v = true;
                }
            }
        }
        "off" => {
            if let Some(b) = cur.as_mut() {
                b.has_xfrm = true;
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
        // 直接色(srgbClr)与主题色(schemeClr,经 theme 解析)统一处理
        "srgbClr" | "schemeClr" => {
            let raw = attr(e, "val");
            let resolved = raw.and_then(|v| {
                if name == "schemeClr" {
                    resolve_scheme_color(theme, &v)
                } else {
                    Some(v)
                }
            });
            if let Some(v) = resolved {
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
                } else if stack.iter().any(|s| s == "spPr")
                    && stack.iter().any(|s| s == "gradFill")
                    && !stack.iter().any(|s| s == "ln")
                {
                    // 渐变停靠色:按出现顺序收集(位置忽略,视图按首/末两端线性渐变)
                    if let Some(b) = cur.as_mut() {
                        b.grad_stops.push(v);
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
    fallback: &PhGeom,
    text_defaults: &TextDefaults,
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
            if let Some(mut b) = cur.take() {
                // 占位符无显式 xfrm:向版式/母版借几何
                if !b.has_xfrm {
                    if let Some(key) = &b.ph {
                        if let Some(&(x, y, w, h)) = fallback.get(key) {
                            b.x = x;
                            b.y = y;
                            b.width = w;
                            b.height = h;
                        }
                    }
                }
                // 文本默认样式继承:run 缺字号/颜色时,按占位符类型从母版 txStyles 补
                let (def_sz, def_color) = text_defaults.for_ph(&b.ph).clone();
                if def_sz.is_some() || def_color.is_some() {
                    for para in &mut b.paragraphs {
                        for run in &mut para.runs {
                            if run.size_pt.is_none() {
                                run.size_pt = def_sz;
                            }
                            if run.color.is_none() {
                                run.color = def_color.clone();
                            }
                        }
                    }
                }
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
    local_from(e.name().as_ref())
}

/// 结束标签的本地名。
fn local_end(e: &quick_xml::events::BytesEnd) -> String {
    local_from(e.name().as_ref())
}

fn local_from(bytes: &[u8]) -> String {
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

    /// 无主题、无回退、无默认样式的空上下文。
    fn empty_ctx() -> (Theme, PhGeom, TextDefaults) {
        (Theme::new(), PhGeom::new(), TextDefaults::default())
    }

    #[test]
    fn parses_shapes_text_geometry_fill() {
        let (theme, fallback, text_defaults) = empty_ctx();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &text_defaults,
        };
        let shapes = parse_slide(std::str::from_utf8(SLIDE).unwrap(), &ctx).shapes;
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

    #[test]
    fn theme_scheme_color_resolved() {
        // 主题:accent1=4472C4、dk1=000000;schemeClr 用 accent1 与 tx1(→dk1)
        let mut theme = Theme::new();
        theme.insert("accent1".into(), "4472C4".into());
        theme.insert("dk1".into(), "1A1A1A".into());
        assert_eq!(
            resolve_scheme_color(&theme, "accent1").as_deref(),
            Some("4472C4")
        );
        assert_eq!(
            resolve_scheme_color(&theme, "tx1").as_deref(),
            Some("1A1A1A")
        ); // tx1→dk1
        assert_eq!(resolve_scheme_color(&theme, "unknown"), None);

        // 在幻灯里 schemeClr 填充应被解析
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:sp><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm>
            <a:prstGeom prst="rect"/><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></p:spPr></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let empty = PhGeom::new();
        let defaults = TextDefaults::default();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &empty,
            text_defaults: &defaults,
        };
        let shapes = parse_slide(slide, &ctx).shapes;
        assert_eq!(shapes[0].fill.as_deref(), Some("4472C4"));
    }

    #[test]
    fn load_theme_parses_clrscheme() {
        let theme_xml = br#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office">
          <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
          <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
          <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
        </a:clrScheme></a:themeElements></a:theme>"#;
        let pptx = zip_of(&[("ppt/theme/theme1.xml", theme_xml)]);
        let mut zip = zip::ZipArchive::new(Cursor::new(pptx)).unwrap();
        let theme = super::load_theme(&mut zip);
        assert_eq!(theme.get("accent1").map(String::as_str), Some("4472C4"));
        assert_eq!(theme.get("dk1").map(String::as_str), Some("000000"));
        assert_eq!(theme.get("lt1").map(String::as_str), Some("FFFFFF"));
    }

    #[test]
    fn placeholder_inherits_layout_geometry() {
        // 版式里 title 占位符带 xfrm;幻灯里同类型占位符**无** xfrm → 应借用版式几何
        let layout = br#"<p:sldLayout xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="title" idx=""/></p:nvPr></p:nvSpPr>
            <p:spPr><a:xfrm><a:off x="838200" y="365760"/><a:ext cx="7772400" cy="1000000"/></a:xfrm></p:spPr></p:sp>
        </p:spTree></p:cSld></p:sldLayout>"#;
        let slide_str = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="title" idx=""/></p:nvPr></p:nvSpPr>
            <p:spPr/><p:txBody><a:p><a:r><a:t>继承标题</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let slide = slide_str.as_bytes();
        let pres = br#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldSz cx="9144000" cy="6858000"/>
          <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#;
        let pres_rels = br#"<Relationships xmlns="x"><Relationship Id="rId1" Type="t" Target="slides/slide1.xml"/></Relationships>"#;
        let slide_rels = br#"<Relationships xmlns="x"><Relationship Id="rId9" Type="slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#;
        let pptx = zip_of(&[
            ("ppt/presentation.xml", pres),
            ("ppt/_rels/presentation.xml.rels", pres_rels),
            ("ppt/slides/slide1.xml", slide),
            ("ppt/slides/_rels/slide1.xml.rels", slide_rels),
            ("ppt/slideLayouts/slideLayout1.xml", layout),
        ]);
        let parsed = parse(&pptx).expect("解析");
        let sp = &parsed.presentation.slides[0].shapes[0];
        // 借到版式的位置/尺寸(838200 EMU = 88px)
        assert!((sp.x - 88.0).abs() < 1.0, "x={}", sp.x);
        assert!((sp.width - 816.0).abs() < 1.0, "w={}", sp.width);
    }

    #[test]
    fn parses_rotation_and_flip() {
        // rot=5400000(1/60000 度)= 90 度;flipH=1
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:sp><p:spPr><a:xfrm rot="5400000" flipH="1"><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm>
            <a:prstGeom prst="rect"/></p:spPr></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let (theme, fallback, defaults) = empty_ctx();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &defaults,
        };
        let shapes = parse_slide(slide, &ctx).shapes;
        assert_eq!(shapes[0].rotation, 90.0);
        assert!(shapes[0].flip_h);
        assert!(!shapes[0].flip_v);
    }

    #[test]
    fn text_default_style_inherited_from_master() {
        // 母版 txStyles:titleStyle lvl1 defRPr sz=4400 color=1F3864
        let master = br#"<p:sldMaster xmlns:p="p" xmlns:a="a"><p:txStyles>
          <p:titleStyle><a:lvl1pPr><a:defRPr sz="4400"><a:solidFill><a:srgbClr val="1F3864"/></a:solidFill></a:defRPr></a:lvl1pPr></p:titleStyle>
          <p:bodyStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:bodyStyle>
        </p:txStyles></p:sldMaster>"#;
        let layout = br#"<p:sldLayout xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree></p:spTree></p:cSld></p:sldLayout>"#;
        // 幻灯 title 占位符文本无 sz/color → 应继承母版 44pt / 1F3864
        let slide_str = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
            <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm></p:spPr>
            <p:txBody><a:p><a:r><a:t>标题文本</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let pres = br#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldSz cx="9144000" cy="6858000"/>
          <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#;
        let pres_rels = br#"<Relationships xmlns="x"><Relationship Id="rId1" Type="t" Target="slides/slide1.xml"/></Relationships>"#;
        let slide_rels = br#"<Relationships xmlns="x"><Relationship Id="rId9" Type="slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#;
        let layout_rels = br#"<Relationships xmlns="x"><Relationship Id="rIdM" Type="slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#;
        let pptx = zip_of(&[
            ("ppt/presentation.xml", pres),
            ("ppt/_rels/presentation.xml.rels", pres_rels),
            ("ppt/slides/slide1.xml", slide_str.as_bytes()),
            ("ppt/slides/_rels/slide1.xml.rels", slide_rels),
            ("ppt/slideLayouts/slideLayout1.xml", layout),
            ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", layout_rels),
            ("ppt/slideMasters/slideMaster1.xml", master),
        ]);
        let parsed = parse(&pptx).expect("解析");
        let run = &parsed.presentation.slides[0].shapes[0].paragraphs[0].runs[0];
        assert_eq!(run.size_pt, Some(44.0), "应继承母版 title 字号");
        assert_eq!(
            run.color.as_deref(),
            Some("1F3864"),
            "应继承母版 title 颜色"
        );
    }

    #[test]
    fn parses_slide_table() {
        // graphicFrame 内嵌 2 列 × 2 行表格
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="3657600" cy="1828800"/></p:xfrm>
            <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
              <a:tbl><a:tblGrid><a:gridCol w="1828800"/><a:gridCol w="1828800"/></a:tblGrid>
                <a:tr h="457200">
                  <a:tc><a:txBody><a:p><a:r><a:t>姓名</a:t></a:r></a:p></a:txBody></a:tc>
                  <a:tc><a:txBody><a:p><a:r><a:t>分数</a:t></a:r></a:p></a:txBody></a:tc>
                </a:tr>
                <a:tr h="457200">
                  <a:tc><a:txBody><a:p><a:r><a:t>张三</a:t></a:r></a:p></a:txBody></a:tc>
                  <a:tc><a:txBody><a:p><a:r><a:t>88</a:t></a:r></a:p></a:txBody></a:tc>
                </a:tr>
              </a:tbl>
            </a:graphicData></a:graphic></p:graphicFrame>
        </p:spTree></p:cSld></p:sld>"#;
        let (theme, fallback, defaults) = empty_ctx();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &defaults,
        };
        let shapes = parse_slide(slide, &ctx).shapes;
        assert_eq!(shapes.len(), 1);
        let t = shapes[0].table.as_ref().expect("应有表格");
        assert_eq!(t.col_widths.len(), 2, "两列");
        assert_eq!(t.rows.len(), 2, "两行");
        assert_eq!(t.rows[0], vec!["姓名".to_string(), "分数".to_string()]);
        assert_eq!(t.rows[1], vec!["张三".to_string(), "88".to_string()]);
    }

    #[test]
    fn parses_gradient_fill() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:sp><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm>
            <a:prstGeom prst="rect"/>
            <a:gradFill><a:gsLst>
              <a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs>
              <a:gs pos="100000"><a:srgbClr val="0000FF"/></a:gs>
            </a:gsLst></a:gradFill></p:spPr></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let (theme, fallback, defaults) = empty_ctx();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &defaults,
        };
        let shapes = parse_slide(slide, &ctx).shapes;
        assert_eq!(
            shapes[0].gradient,
            Some(("FF0000".to_string(), "0000FF".to_string()))
        );
    }

    #[test]
    fn detects_graphic_frame_kind() {
        // graphicFrame 内嵌图表 / SmartArt → 占位框 + placeholder_kind
        let chart = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></p:xfrm>
            <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic>
          </p:graphicFrame></p:spTree></p:cSld></p:sld>"#;
        let (theme, fallback, defaults) = empty_ctx();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &defaults,
        };
        let shapes = parse_slide(chart, &ctx).shapes;
        assert_eq!(shapes[0].placeholder_kind.as_deref(), Some("chart"));

        let smart = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></p:xfrm>
            <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic>
          </p:graphicFrame></p:spTree></p:cSld></p:sld>"#;
        let shapes = parse_slide(smart, &ctx).shapes;
        assert_eq!(shapes[0].placeholder_kind.as_deref(), Some("diagram"));
    }

    #[test]
    fn detects_animation_and_transition() {
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree></p:spTree></p:cSld>
          <p:transition><p:fade/></p:transition>
          <p:timing><p:tnLst/></p:timing></p:sld>"#;
        let (theme, fallback, defaults) = empty_ctx();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &defaults,
        };
        let parsed = parse_slide(slide, &ctx);
        assert!(parsed.has_animation);
        assert!(parsed.has_transition);
    }

    #[test]
    fn group_shape_maps_child_coordinates() {
        // 组:父系 off=(0,0) ext=(9144000,4572000);子系 chOff=(0,0) chExt=(4572000,2286000)
        // → 缩放 sx=sy=2。子矩形 off=(1000000,500000) ext=(1000000,500000)
        // → 映射后 x=2000000→px, w=2000000→px。
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
          <p:grpSp><p:grpSpPr><a:xfrm>
            <a:off x="0" y="0"/><a:ext cx="9144000" cy="4572000"/>
            <a:chOff x="0" y="0"/><a:chExt cx="4572000" cy="2286000"/>
          </a:xfrm></p:grpSpPr>
          <p:sp><p:spPr><a:xfrm><a:off x="1000000" y="500000"/><a:ext cx="1000000" cy="500000"/></a:xfrm>
            <a:prstGeom prst="rect"/></p:spPr></p:sp>
          </p:grpSp>
        </p:spTree></p:cSld></p:sld>"#;
        let (theme, fallback, defaults) = empty_ctx();
        let ctx = SlideCtx {
            theme: &theme,
            fallback: &fallback,
            text_defaults: &defaults,
        };
        let shapes = parse_slide(slide, &ctx).shapes;
        assert_eq!(shapes.len(), 1);
        let s = &shapes[0];
        // 子 x=1000000 EMU;映射 X = 0 + (1000000-0)*2 = 2000000 EMU → px
        assert!((s.x - emu(2_000_000.0)).abs() < 0.5, "x={}", s.x);
        assert!((s.width - emu(2_000_000.0)).abs() < 0.5, "w={}", s.width);
        assert!((s.y - emu(1_000_000.0)).abs() < 0.5, "y={}", s.y);
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
            // 第二张:标题 + 旋转矩形 + 图表占位(graphicFrame),并带切换/动画标记。
            let slide2 = r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="r"><p:cSld><p:spTree>
             <p:sp><p:spPr><a:xfrm><a:off x="838200" y="365760"/><a:ext cx="7772400" cy="1000000"/></a:xfrm>
               <a:prstGeom prst="rect"/></p:spPr>
               <p:txBody><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="3200" b="1"><a:solidFill><a:srgbClr val="1F3864"/></a:solidFill></a:rPr><a:t>第二张幻灯</a:t></a:r></a:p></p:txBody></p:sp>
             <p:sp><p:spPr><a:xfrm rot="2700000"><a:off x="838200" y="1800000"/><a:ext cx="2200000" cy="1200000"/></a:xfrm>
               <a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></p:spPr></p:sp>
             <p:graphicFrame><p:xfrm><a:off x="4200000" y="1800000"/><a:ext cx="3500000" cy="2600000"/></p:xfrm>
               <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic></p:graphicFrame>
           </p:spTree></p:cSld>
           <p:transition><p:fade/></p:transition>
           <p:timing><p:tnLst/></p:timing></p:sld>"#;
            put(&mut w, "ppt/slides/slide2.xml", slide2.as_bytes());
            put(&mut w, "ppt/media/image1.png", PNG);
            w.finish().unwrap();
        }
        std::fs::write("/tmp/office-r-sample.pptx", &buf).unwrap();
        eprintln!("wrote /tmp/office-r-sample.pptx ({} bytes)", buf.len());
    }
}
