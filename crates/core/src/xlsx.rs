//! Excel (.xlsx) **工作簿解析**:calamine 读取 → 每张工作表一张只读 [`Sheet`] + 公式清单。
//!
//! 与 CSV 路径的关键差异:xlsx **自带缓存的计算值**(公式格里存着上次算出的结果),
//! 因此这里**不重算**——显示表直接取 calamine 给的单元格值(与 Excel 打开时所见一致),
//! 公式原文单独经 `worksheet_formula` 取出供公式栏回显。多工作表按 `sheet_names` 顺序保留。
//!
//! 单元格值 → 文本:数值按该格 **numfmt 格式码**渲染(自解析 `xl/styles.xml` 的
//! `numFmts` + `cellXfs`,内置 id 与自定义码经 `numfmt` 内核格式化;百分比/千分位/货币/小数),
//! 布尔归一为 `TRUE`/`FALSE`,日期序列数换算成 `YYYY-MM-DD[ HH:MM:SS]`(自实现,不引入 chrono)。
//! 合并区(`mergeCells`)解析进 `XlsxSheet::merges`;单元格视觉样式(加粗/斜体/文字色/
//! 填充背景/水平对齐,自解析 `fonts`/`fills`/`cellXfs`)进 `XlsxSheet::formats`,网格按格渲染。
//! 内嵌图片(`xl/drawings` 锚点 + `media` 字节)进 `XlsxSheet::images` / `XlsxWorkbook::media`,
//! 网格覆盖层按锚点单元格近似定位绘制。
//!
//! 合并区在网格覆盖层**跨格合并绘制**(白底盖内部线 + 跨区左上角文本 + 外框)。
//! 单元格**边框线**(`borders`/`cellXfs@borderId`,四边线型→线宽 + 颜色)进 `CellFmt::border`,网格按格描边。
//! **条件格式**(`conditionalFormatting`:`cellIs` 比较 → dxf 填充、`colorScale` 2/3 色阶插值)
//! 求值后并入 `CellFmt::fill`,复用填充渲染。
//! **图表**(`xl/charts/chartN.xml`:柱/线/饼 的系列 `numCache` + 类别 `strCache` + 标题)进
//! `XlsxSheet::charts`,网格覆盖层绘制简单柱/线/饼图。
//! **列宽**(`col@width`)与**冻结窗格**(`sheetView/pane`)进 `XlsxSheet`,网格应用原始列宽 + 自动冻结。
//! **迷你图**(`extLst` 的 `x14:sparklineGroups`:类型 + `xm:f` 数据范围 → 取值、`xm:sqref` 宿主格)进
//! `XlsxSheet::sparklines`,网格在宿主单元格内画折线/柱。
//! **非目标**:数据条/图标集条件格式、公式型 cfRule、图表坐标轴/图例、变高行(网格假定等高行)。

use std::io::{Cursor, Read};

use calamine::{open_workbook_from_rs, Data, DataType, Reader, Xlsx};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader as XmlReader;
use zip::ZipArchive;

use crate::formula::CellFormula;
use crate::sheet::Sheet;

/// 一张工作表:名字 + 值显示表 + 公式清单 + 合并区。
#[derive(Debug)]
pub struct XlsxSheet {
    /// 工作表名(标签页显示)。
    pub name: String,
    /// 单元格**显示值**表(公式格为缓存计算值;数值按该格 numfmt 格式码渲染)。
    pub sheet: Sheet,
    /// 公式格的原始文本(含前导 `=`),供公式栏回显。
    pub formulas: Vec<CellFormula>,
    /// 合并单元格区域 `(row0, col0, row1, col1)`(0 基,含首尾)。
    pub merges: Vec<(u32, u32, u32, u32)>,
    /// 非默认单元格样式:`(row, col, 样式)`。仅收录有加粗/斜体/颜色/填充/对齐的格。
    pub formats: Vec<(u32, u32, CellFmt)>,
    /// 内嵌图片(锚定到单元格);字节在 [`XlsxWorkbook::media`] 里按 `media_key` 取。
    pub images: Vec<XlsxImage>,
    /// 内嵌图表(柱/线/饼),锚定到单元格区域,含系列数值与类别。
    pub charts: Vec<XlsxChart>,
    /// 列宽覆盖:`(col, Excel 字符宽度)`(来自 `col@width`)。
    pub col_widths: Vec<(u32, f64)>,
    /// 冻结窗格:顶部行数 / 左侧列数(来自 `sheetView/pane`)。
    pub freeze_rows: u32,
    pub freeze_cols: u32,
    /// 迷你图:宿主单元格 + 类型 + 数据值(已从数据范围取出)。
    pub sparklines: Vec<XlsxSparkline>,
}

/// 单元格内迷你图(只读渲染)。
#[derive(Debug, Clone)]
pub struct XlsxSparkline {
    /// 宿主单元格(0 基)。
    pub row: u32,
    pub col: u32,
    /// 类型:`"line"` / `"column"` / `"stacked"`。
    pub kind: String,
    /// 数据值(空单元格计 0)。
    pub values: Vec<f64>,
}

/// 内嵌图表(只读渲染)。
#[derive(Debug, Clone)]
pub struct XlsxChart {
    /// 左上锚单元格(0 基)。
    pub from_row: u32,
    pub from_col: u32,
    /// 右下锚单元格(twoCellAnchor);无则 `None`。
    pub to: Option<(u32, u32)>,
    /// 图表类型:`"bar"` / `"line"` / `"pie"`。
    pub kind: String,
    /// 各系列的数值(来自 numCache)。
    pub series: Vec<Vec<f64>>,
    /// 类别标签(来自首个 cat 的 strCache);可能为空。
    pub categories: Vec<String>,
    /// 图表标题(如有)。
    pub title: Option<String>,
}

/// 锚定到单元格的内嵌图片。位置以单元格下标 + 尺寸表达(网格按自身列宽近似定位)。
#[derive(Debug, Clone)]
pub struct XlsxImage {
    /// 媒体键(如 `xl/media/image1.png`),在 [`XlsxWorkbook::media`] 里唯一。
    pub media_key: String,
    /// 左上锚起始单元格(0 基)。
    pub from_row: u32,
    pub from_col: u32,
    /// 右下锚单元格(twoCellAnchor);oneCellAnchor 为 `None`(用 `ext_px`)。
    pub to: Option<(u32, u32)>,
    /// 尺寸(oneCellAnchor 的 `a:ext`,像素);twoCell 为 `None`。
    pub ext_px: Option<(f64, f64)>,
}

/// 一份媒体字节(图片)。
#[derive(Debug, Clone)]
pub struct XlsxMedia {
    pub key: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// 单元格视觉样式(只读渲染用)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellFmt {
    pub bold: bool,
    pub italic: bool,
    /// 文字色 `RRGGBB`。
    pub color: Option<String>,
    /// 填充背景 `RRGGBB`。
    pub fill: Option<String>,
    /// 水平对齐:`"left"`/`"center"`/`"right"`。
    pub align: Option<String>,
    /// 四边边框(上/右/下/左);无边框为 `None`。
    pub border: Option<Borders>,
}

/// 单元格四边边框(只读渲染)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Borders {
    pub top: Option<BorderSide>,
    pub right: Option<BorderSide>,
    pub bottom: Option<BorderSide>,
    pub left: Option<BorderSide>,
}

impl Borders {
    fn is_empty(&self) -> bool {
        self.top.is_none() && self.right.is_none() && self.bottom.is_none() && self.left.is_none()
    }
}

/// 一条边框线:粗细(px)+ 颜色 `RRGGBB`。
#[derive(Debug, Clone, PartialEq)]
pub struct BorderSide {
    pub width: f64,
    pub color: String,
}

impl CellFmt {
    fn is_default(&self) -> bool {
        !self.bold
            && !self.italic
            && self.color.is_none()
            && self.fill.is_none()
            && self.align.is_none()
            && self.border.is_none()
    }
}

/// 一个工作簿:按原始顺序的工作表列表。
#[derive(Debug)]
pub struct XlsxWorkbook {
    pub sheets: Vec<XlsxSheet>,
    /// 全工作簿去重的媒体字节(图片),按 `media_key` 引用。
    pub media: Vec<XlsxMedia>,
}

/// 解析 xlsx 字节为工作簿。失败返回可读错误。
pub fn parse(bytes: &[u8]) -> Result<XlsxWorkbook, String> {
    let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e: calamine::XlsxError| e.to_string())?;
    let names = wb.sheet_names().to_vec();

    // 另开一份 zip 自解析样式(calamine 稳定 API 不给每格格式)与合并区。
    let mut zip = ZipArchive::new(Cursor::new(bytes.to_vec())).ok();
    let styles = zip.as_mut().map(read_style_table).unwrap_or_default();
    let path_map = zip.as_mut().map(sheet_path_map).unwrap_or_default();

    let mut sheets = Vec::with_capacity(names.len());
    let mut media: Vec<XlsxMedia> = Vec::new();
    for name in names {
        let values = wb.worksheet_range(&name).map_err(|e| e.to_string())?;
        let formulas_range = wb.worksheet_formula(&name).ok();

        // 每格样式索引 + 合并区 + 列宽 + 冻结(自解析 sheetN.xml)
        let geom = match (zip.as_mut(), path_map.get(&name)) {
            (Some(z), Some(path)) => read_cell_styles(z, path),
            _ => SheetGeom::default(),
        };
        let cell_styles = geom.styles;
        let merges = geom.merges;

        // 内嵌图片 + 图表(自解析 worksheet rels → drawing → media / chart)
        let (images, charts) = match (zip.as_mut(), path_map.get(&name)) {
            (Some(z), Some(path)) => read_sheet_drawings(z, path, &mut media),
            _ => (Vec::new(), Vec::new()),
        };

        let (sheet, formulas) = build_sheet(
            &values,
            formulas_range.as_ref(),
            &cell_styles,
            &styles.codes,
        );

        // 每格视觉样式(仅非默认)
        let mut fmt_map: std::collections::HashMap<(u32, u32), CellFmt> =
            std::collections::HashMap::new();
        for (&(r, c), &s) in &cell_styles {
            if let Some(fmt) = styles.fmts.get(s) {
                if !fmt.is_default() {
                    fmt_map.insert((r, c), fmt.clone());
                }
            }
        }

        // 条件格式:求值后把结果填充色并入(覆盖静态填充)
        let cf = match path_map.get(&name) {
            Some(path) => zip
                .as_mut()
                .map(|z| read_conditional_formatting(z, path))
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if !cf.is_empty() {
            let cf_fills = evaluate_cf(&cf, &sheet, &styles.dxf_fills);
            for ((r, c), fill) in cf_fills {
                fmt_map.entry((r, c)).or_default().fill = Some(fill);
            }
        }

        let mut formats: Vec<(u32, u32, CellFmt)> =
            fmt_map.into_iter().map(|((r, c), f)| (r, c, f)).collect();
        formats.sort_by_key(|&(r, c, _)| (r, c));

        let mut col_widths: Vec<(u32, f64)> = geom.col_widths.into_iter().collect();
        col_widths.sort_by_key(|&(c, _)| c);

        // 迷你图:把数据范围(去掉 `Sheet!` 前缀)解析成单元格,从显示表取数值
        let sparklines: Vec<XlsxSparkline> = geom
            .sparklines
            .into_iter()
            .filter_map(|(r, c, kind, f)| {
                let range = f.rsplit('!').next().unwrap_or(&f).replace('$', "");
                let (r0, c0, r1, c1) = parse_a1_range(&range)?;
                let mut values = Vec::new();
                for rr in r0..=r1 {
                    for cc in c0..=c1 {
                        let v = sheet
                            .cell(rr as usize, cc as usize)
                            .trim()
                            .parse::<f64>()
                            .ok();
                        values.push(v.unwrap_or(0.0));
                    }
                }
                Some(XlsxSparkline {
                    row: r,
                    col: c,
                    kind,
                    values,
                })
            })
            .collect();

        sheets.push(XlsxSheet {
            name,
            sheet,
            formulas,
            merges,
            formats,
            images,
            charts,
            col_widths,
            freeze_rows: geom.freeze_rows,
            freeze_cols: geom.freeze_cols,
            sparklines,
        });
    }

    Ok(XlsxWorkbook { sheets, media })
}

/// 解析某工作表的内嵌图片:worksheet rels → drawingN.xml → 锚点 + embed;
/// drawing rels → embed → media 路径;去重收集 media 字节到 `media`。
fn read_sheet_drawings(
    zip: &mut ZipArchive<Cursor<Vec<u8>>>,
    sheet_path: &str,
    media: &mut Vec<XlsxMedia>,
) -> (Vec<XlsxImage>, Vec<XlsxChart>) {
    // worksheet rels:.../worksheets/_rels/sheetN.xml.rels
    let (dir, file) = sheet_path
        .rsplit_once('/')
        .unwrap_or(("xl/worksheets", sheet_path));
    let ws_rels = format!("{dir}/_rels/{file}.rels");
    let drawing_target = match zip_text(zip, &ws_rels) {
        Some(xml) => find_rel_target(&xml, "drawing"),
        None => None,
    };
    let Some(drawing_path) = drawing_target.map(|t| resolve_rel_path(dir, &t)) else {
        return (Vec::new(), Vec::new());
    };
    // drawing rels:rId → 目标(图片 media 或 chartN.xml)
    let (ddir, dfile) = drawing_path
        .rsplit_once('/')
        .unwrap_or(("xl/drawings", &drawing_path));
    let drels = format!("{ddir}/_rels/{dfile}.rels");
    let rel_map = zip_text(zip, &drels)
        .map(|xml| all_rel_targets(&xml, ddir))
        .unwrap_or_default();

    let xml = match zip_text(zip, &drawing_path) {
        Some(x) => x,
        None => return (Vec::new(), Vec::new()),
    };
    let (raw_imgs, raw_charts) = parse_drawing(&xml);

    let mut out = Vec::new();
    for (embed, from, to, ext) in raw_imgs {
        let Some(key) = rel_map.get(&embed).cloned() else {
            continue;
        };
        // 收集 media 字节(去重)
        if !media.iter().any(|m| m.key == key) {
            if let Ok(data) = {
                let mut b = Vec::new();
                match zip.by_name(&key) {
                    Ok(mut f) => f.read_to_end(&mut b).map(|_| b),
                    Err(_) => Ok(Vec::new()),
                }
            } {
                if !data.is_empty() {
                    media.push(XlsxMedia {
                        mime: mime_of(&key),
                        key: key.clone(),
                        data,
                    });
                }
            }
        }
        out.push(XlsxImage {
            media_key: key,
            from_row: from.0,
            from_col: from.1,
            to,
            ext_px: ext,
        });
    }

    // 图表:rId → chartN.xml,解析系列
    let mut charts = Vec::new();
    for (rid, from, to) in raw_charts {
        let Some(chart_path) = rel_map.get(&rid).cloned() else {
            continue;
        };
        if let Some(cxml) = zip_text(zip, &chart_path) {
            if let Some(c) = crate::chart::parse_chart_xml(&cxml) {
                charts.push(XlsxChart {
                    from_row: from.0,
                    from_col: from.1,
                    to,
                    kind: c.kind,
                    series: c.series,
                    categories: c.categories,
                    title: c.title,
                });
            }
        }
    }
    (out, charts)
}

/// 从 rels XML 找首个类型以 `suffix` 结尾的关系 target。
fn find_rel_target(xml: &str, suffix: &str) -> Option<String> {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(&e) == "Relationship" => {
                let ty = attr(&e, "Type").unwrap_or_default();
                if ty.ends_with(suffix) {
                    return attr(&e, "Target");
                }
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// 从 rels XML 收集 id → 归一化后的目标路径(基于 `base_dir`)。
fn all_rel_targets(xml: &str, base_dir: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(&e) == "Relationship" => {
                if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    out.insert(id, resolve_rel_path(base_dir, &t));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// 相对 rels target(可能含 `../`)按 `base_dir` 归一到 zip 内绝对路径。
fn resolve_rel_path(base_dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base_dir.split('/').collect();
    for seg in target.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// 解析 drawingN.xml:返回 `(embed, (from_row,from_col), to?, ext_px?)` 列表。
#[allow(clippy::type_complexity)]
type DrawImage = (String, (u32, u32), Option<(u32, u32)>, Option<(f64, f64)>);
type DrawChart = (String, (u32, u32), Option<(u32, u32)>);

fn parse_drawing(xml: &str) -> (Vec<DrawImage>, Vec<DrawChart>) {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut charts: Vec<DrawChart> = Vec::new();
    // 当前锚状态
    let mut from: Option<(u32, u32)> = None;
    let mut to: Option<(u32, u32)> = None;
    let mut ext: Option<(f64, f64)> = None;
    let mut embed: Option<String> = None;
    let mut chart_rid: Option<String> = None;
    // from/to 内部当前读到的 col/row(文本在子元素里)
    let mut in_from = false;
    let mut in_to = false;
    let mut cur_col: Option<u32> = None;
    let mut cur_row: Option<u32> = None;
    let mut text_target: Option<char> = None; // 'c' col / 'r' row
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local(&e).as_str() {
                "twoCellAnchor" | "oneCellAnchor" => {
                    from = None;
                    to = None;
                    ext = None;
                    embed = None;
                    chart_rid = None;
                }
                // 图表引用 <c:chart r:id="..."/>(也可能作 Start)
                "chart" => {
                    if let Some(id) = attr(&e, "id") {
                        chart_rid = Some(id);
                    }
                }
                "from" => {
                    in_from = true;
                    cur_col = None;
                    cur_row = None;
                }
                "to" => {
                    in_to = true;
                    cur_col = None;
                    cur_row = None;
                }
                "col" if in_from || in_to => text_target = Some('c'),
                "row" if in_from || in_to => text_target = Some('r'),
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                let n = local(&e);
                if n == "ext" {
                    let cx = attr(&e, "cx").and_then(|s| s.parse::<f64>().ok());
                    let cy = attr(&e, "cy").and_then(|s| s.parse::<f64>().ok());
                    if let (Some(cx), Some(cy)) = (cx, cy) {
                        ext = Some((emu(cx), emu(cy)));
                    }
                } else if n == "blip" {
                    if let Some(id) = attr(&e, "embed") {
                        embed = Some(id);
                    }
                } else if n == "chart" {
                    if let Some(id) = attr(&e, "id") {
                        chart_rid = Some(id);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(kind) = text_target.take() {
                    if let Ok(s) = t.decode() {
                        if let Ok(v) = s.trim().parse::<u32>() {
                            if kind == 'c' {
                                cur_col = Some(v);
                            } else {
                                cur_row = Some(v);
                            }
                        }
                    }
                }
            }
            Ok(Event::End(e)) => match local_end(&e).as_str() {
                "from" => {
                    in_from = false;
                    if let (Some(r), Some(c)) = (cur_row, cur_col) {
                        from = Some((r, c));
                    }
                }
                "to" => {
                    in_to = false;
                    if let (Some(r), Some(c)) = (cur_row, cur_col) {
                        to = Some((r, c));
                    }
                }
                "twoCellAnchor" | "oneCellAnchor" => {
                    if let (Some(f), Some(em)) = (from, embed.take()) {
                        out.push((em, f, to, ext));
                    } else if let (Some(f), Some(rid)) = (from, chart_rid.take()) {
                        charts.push((rid, f, to));
                    }
                    from = None;
                    to = None;
                    ext = None;
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (out, charts)
}

/// 由扩展名推断图片 MIME。
fn mime_of(path: &str) -> String {
    let lower = path.to_lowercase();
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

/// EMU → 像素(96 DPI):÷9525。
fn emu(v: f64) -> f64 {
    v / 9525.0
}

/// 从 calamine 的值区域(+ 可选公式区域 + 每格样式)构建显示表与公式清单。
///
/// 用**绝对坐标**(从 (0,0) 到 `end`)建表,使网格 `(r,c)` 与 Excel A1 地址对齐。
/// 数值格若带 numfmt 格式码,按码渲染(百分比/千分位/货币/小数);日期由 calamine 归为
/// `DateTime`,走 ISO 换算,不再二次套码。
fn build_sheet(
    values: &calamine::Range<Data>,
    formulas: Option<&calamine::Range<String>>,
    cell_styles: &std::collections::HashMap<(u32, u32), usize>,
    style_codes: &[Option<String>],
) -> (Sheet, Vec<CellFormula>) {
    let mut builder = Sheet::builder();
    let mut formula_list = Vec::new();

    let Some((end_row, end_col)) = values.end() else {
        // 空表
        return (builder.finish(), formula_list);
    };

    for r in 0..=end_row {
        builder.start_row();
        for c in 0..=end_col {
            let data = values.get_value((r, c));
            // 数值 + 该格 numfmt 码 → 按码格式化;否则默认显示
            let cell = match data {
                Some(d @ (Data::Float(_) | Data::Int(_))) => {
                    let code = cell_styles
                        .get(&(r, c))
                        .and_then(|&s| style_codes.get(s))
                        .and_then(|o| o.clone());
                    match (code, d.as_f64()) {
                        (Some(code), Some(n)) => crate::numfmt::format_number(n, &code),
                        _ => cell_text(d),
                    }
                }
                Some(d) => cell_text(d),
                None => String::new(),
            };
            builder.push_field(&cell);

            if let Some(fr) = formulas {
                if let Some(src) = fr.get_value((r, c)) {
                    if !src.is_empty() {
                        formula_list.push(CellFormula {
                            row: r,
                            col: c,
                            source: format!("={src}"),
                        });
                    }
                }
            }
        }
    }
    builder.trim_trailing_empty_rows();
    (builder.finish(), formula_list)
}

/// 单元格数据 → 显示文本。
fn cell_text(data: &Data) -> String {
    match data {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        // calamine 的 DateTime Display 只打印裸序列数,这里换算成日期文本。
        Data::DateTime(dt) => excel_serial_to_string(dt.as_f64()),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

// ---- 自解析:样式(numfmt 码)+ 工作表路径映射 + 每格样式索引/合并区 ----

/// 读 zip 内某文本文件(缺失/错误返回 None)。
fn zip_text(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Option<String> {
    let mut s = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
    Some(s)
}

/// 元素本地名(去命名空间前缀)。
fn local(e: &BytesStart) -> String {
    let full = e.name();
    let bytes = full.as_ref();
    let name = bytes.rsplit(|&b| b == b':').next().unwrap_or(bytes);
    String::from_utf8_lossy(name).into_owned()
}

/// 取属性值。
fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let k = a.key;
        let kn = k
            .as_ref()
            .rsplit(|&b| b == b':')
            .next()
            .unwrap_or(k.as_ref());
        if kn == key.as_bytes() {
            Some(String::from_utf8_lossy(&a.value).into_owned())
        } else {
            None
        }
    })
}

/// 样式表:每个 cellXfs 索引 → (numfmt 格式码, 视觉样式)。
#[derive(Default)]
struct StyleTable {
    codes: Vec<Option<String>>,
    fmts: Vec<CellFmt>,
    /// 差异格式(`<dxfs>`)的填充色,供条件格式 `cellIs@dxfId` 引用。
    dxf_fills: Vec<Option<String>>,
}

/// 解析 `xl/styles.xml`:numFmts + fonts + fills + cellXfs → 每个样式索引的
/// numfmt 码与视觉样式(加粗/斜体/文字色/填充/对齐)。
fn read_style_table(zip: &mut ZipArchive<Cursor<Vec<u8>>>) -> StyleTable {
    let xml = match zip_text(zip, "xl/styles.xml") {
        Some(x) => x,
        None => return StyleTable::default(),
    };
    let mut custom: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut fonts: Vec<(bool, bool, Option<String>)> = Vec::new(); // (bold, italic, color)
    let mut fills: Vec<Option<String>> = Vec::new();
    let mut borders: Vec<Borders> = Vec::new();
    let mut table = StyleTable::default();

    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    // 区段与「当前正在构建」的状态
    let (mut in_fonts, mut in_fills, mut in_cellxfs, mut in_borders) = (false, false, false, false);
    // 差异格式 dxfs:每个 dxf 的填充色(bgColor);当前 dxf 累积
    let mut in_dxfs = false;
    let mut cur_dxf_fill: Option<String> = None;
    let mut cur_font: Option<(bool, bool, Option<String>)> = None;
    let mut cur_fill_solid = false;
    let mut cur_fill_color: Option<String> = None;
    let mut cur_border: Option<Borders> = None;
    // 当前正在读的边名(top/right/bottom/left)+ 其 style(颜色在其 <color> 子元素里)
    let mut cur_side: Option<(String, Option<String>)> = None;
    let mut cur_xf: Option<(u32, usize, usize, usize)> = None; // (numFmtId, fontId, fillId, borderId)
    let mut cur_align: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                let empty = matches!(ev, Event::Empty(_));
                let e = match &ev {
                    Event::Start(e) | Event::Empty(e) => e.clone(),
                    _ => unreachable!(),
                };
                let n = local(&e);
                match n.as_str() {
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (
                            attr(&e, "numFmtId").and_then(|s| s.parse::<u32>().ok()),
                            attr(&e, "formatCode"),
                        ) {
                            custom.insert(id, code);
                        }
                    }
                    "fonts" => in_fonts = true,
                    "fills" => in_fills = true,
                    "cellXfs" => in_cellxfs = true,
                    "font" if in_fonts => {
                        cur_font = Some((false, false, None));
                        if empty {
                            fonts.push((false, false, None));
                            cur_font = None;
                        }
                    }
                    "b" if cur_font.is_some() => {
                        if let Some(f) = cur_font.as_mut() {
                            f.0 = attr(&e, "val").as_deref() != Some("0");
                        }
                    }
                    "i" if cur_font.is_some() => {
                        if let Some(f) = cur_font.as_mut() {
                            f.1 = attr(&e, "val").as_deref() != Some("0");
                        }
                    }
                    "color" if cur_font.is_some() => {
                        if let Some(rgb) = attr(&e, "rgb").map(|s| normalize_argb(&s)) {
                            if let Some(f) = cur_font.as_mut() {
                                f.2 = Some(rgb);
                            }
                        }
                    }
                    "fill" if in_fills => {
                        cur_fill_solid = false;
                        cur_fill_color = None;
                        if empty {
                            fills.push(None);
                        }
                    }
                    "patternFill" if in_fills => {
                        cur_fill_solid = attr(&e, "patternType").as_deref() == Some("solid");
                    }
                    "fgColor" if in_fills && cur_fill_solid => {
                        if let Some(rgb) = attr(&e, "rgb").map(|s| normalize_argb(&s)) {
                            cur_fill_color = Some(rgb);
                        }
                    }
                    "borders" => in_borders = true,
                    "border" if in_borders => {
                        cur_border = Some(Borders::default());
                        if empty {
                            borders.push(Borders::default());
                            cur_border = None;
                        }
                    }
                    "left" | "right" | "top" | "bottom" if cur_border.is_some() => {
                        let style = attr(&e, "style");
                        // 无 style(或 none)= 无边框;有 style 才成边,颜色待 <color> 子元素
                        if style.as_deref().is_some_and(|s| s != "none") {
                            cur_side = Some((n.clone(), style));
                            if empty {
                                apply_side(
                                    cur_border.as_mut().unwrap(),
                                    &n,
                                    &cur_side.take().unwrap().1,
                                    None,
                                );
                            }
                        }
                    }
                    "color" if cur_side.is_some() => {
                        let rgb = attr(&e, "rgb").map(|s| normalize_argb(&s));
                        let (side_name, style) = cur_side.take().unwrap();
                        apply_side(cur_border.as_mut().unwrap(), &side_name, &style, rgb);
                    }
                    "dxfs" => in_dxfs = true,
                    "dxf" if in_dxfs => {
                        cur_dxf_fill = None;
                        if empty {
                            table.dxf_fills.push(None);
                        }
                    }
                    "bgColor" if in_dxfs => {
                        cur_dxf_fill = attr(&e, "rgb").map(|s| normalize_argb(&s));
                    }
                    "xf" if in_cellxfs => {
                        let numfmt = attr(&e, "numFmtId")
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(0);
                        let font_id = attr(&e, "fontId")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        let fill_id = attr(&e, "fillId")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        let border_id = attr(&e, "borderId")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        if empty {
                            push_xf(
                                &mut table, &custom, &fonts, &fills, &borders, numfmt, font_id,
                                fill_id, border_id, None,
                            );
                        } else {
                            cur_xf = Some((numfmt, font_id, fill_id, border_id));
                            cur_align = None;
                        }
                    }
                    "alignment" if cur_xf.is_some() => {
                        cur_align = attr(&e, "horizontal");
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => match local_end(&e).as_str() {
                "fonts" => in_fonts = false,
                "fills" => in_fills = false,
                "borders" => in_borders = false,
                "cellXfs" => in_cellxfs = false,
                "dxfs" => in_dxfs = false,
                "dxf" => {
                    if in_dxfs {
                        table.dxf_fills.push(cur_dxf_fill.take());
                    }
                }
                "font" => {
                    if let Some(f) = cur_font.take() {
                        fonts.push(f);
                    }
                }
                "fill" => {
                    if in_fills {
                        fills.push(cur_fill_color.take());
                    }
                }
                "border" => {
                    if let Some(b) = cur_border.take() {
                        borders.push(b);
                    }
                }
                "xf" => {
                    if let Some((numfmt, font_id, fill_id, border_id)) = cur_xf.take() {
                        push_xf(
                            &mut table,
                            &custom,
                            &fonts,
                            &fills,
                            &borders,
                            numfmt,
                            font_id,
                            fill_id,
                            border_id,
                            cur_align.take(),
                        );
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    table
}

/// 把一个 cellXfs 项归并成 (numfmt 码, CellFmt) 并压入表。
#[allow(clippy::too_many_arguments)]
fn push_xf(
    table: &mut StyleTable,
    custom: &std::collections::HashMap<u32, String>,
    fonts: &[(bool, bool, Option<String>)],
    fills: &[Option<String>],
    borders: &[Borders],
    numfmt_id: u32,
    font_id: usize,
    fill_id: usize,
    border_id: usize,
    align: Option<String>,
) {
    let code = custom
        .get(&numfmt_id)
        .cloned()
        .or_else(|| builtin_numfmt(numfmt_id));
    let (bold, italic, color) = fonts.get(font_id).cloned().unwrap_or((false, false, None));
    let fill = fills.get(fill_id).cloned().flatten();
    let align = align.and_then(|a| match a.as_str() {
        "left" | "center" | "right" => Some(a),
        _ => None,
    });
    let border = borders.get(border_id).filter(|b| !b.is_empty()).cloned();
    table.codes.push(code);
    table.fmts.push(CellFmt {
        bold,
        italic,
        color,
        fill,
        align,
        border,
    });
}

/// 把一条边(top/right/bottom/left)按 style + 颜色写入 Borders。
/// style → 线宽:thin/hair→1、medium/dashed/dotted→1.5、thick→2.5、double→2;缺省 1。
fn apply_side(b: &mut Borders, side: &str, style: &Option<String>, color: Option<String>) {
    let width = match style.as_deref() {
        Some("thick") => 2.5,
        Some("medium") | Some("mediumDashed") => 1.5,
        Some("double") => 2.0,
        _ => 1.0,
    };
    let s = BorderSide {
        width,
        color: color.unwrap_or_else(|| "8c959f".to_string()),
    };
    match side {
        "top" => b.top = Some(s),
        "right" => b.right = Some(s),
        "bottom" => b.bottom = Some(s),
        "left" => b.left = Some(s),
        _ => {}
    }
}

/// ARGB(`FFRRGGBB`)或 RGB(`RRGGBB`)→ 6 位 RRGGBB(大写)。非法返回原样上限 6 位。
fn normalize_argb(s: &str) -> String {
    let t = s.trim();
    let hex = if t.len() == 8 { &t[2..] } else { t };
    hex.to_uppercase()
}

/// End 事件本地名。
fn local_end(e: &quick_xml::events::BytesEnd) -> String {
    let full = e.name();
    let bytes = full.as_ref();
    let name = bytes.rsplit(|&b| b == b':').next().unwrap_or(bytes);
    String::from_utf8_lossy(name).into_owned()
}

/// 内置 numFmtId → 格式码(仅数值类;General/日期/文本返回 None,走默认/ISO 渲染)。
fn builtin_numfmt(id: u32) -> Option<String> {
    let code = match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        37 | 38 => "#,##0",
        39 | 40 => "#,##0.00",
        44 => "\"¥\"#,##0.00",
        // 0 General、11 科学计数、12/13 分数、14-22 日期时间、45-49 等 → 默认处理
        _ => return None,
    };
    Some(code.to_string())
}

/// 解析 `xl/workbook.xml` + rels,得到 工作表名 → `xl/worksheets/sheetN.xml` 路径。
fn sheet_path_map(
    zip: &mut ZipArchive<Cursor<Vec<u8>>>,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let wb_xml = match zip_text(zip, "xl/workbook.xml") {
        Some(x) => x,
        None => return out,
    };
    let rels_xml = zip_text(zip, "xl/_rels/workbook.xml.rels").unwrap_or_default();
    // rId → target
    let mut rid_target: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut reader = XmlReader::from_str(&rels_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(&e) == "Relationship" => {
                if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    rid_target.insert(id, t);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    // sheet name + r:id
    let mut reader = XmlReader::from_str(&wb_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(&e) == "sheet" => {
                if let (Some(name), Some(rid)) = (attr(&e, "name"), attr(&e, "id")) {
                    if let Some(t) = rid_target.get(&rid) {
                        let path = normalize_xl_path(t);
                        out.insert(name, path);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// workbook rels 的 target 多为 `worksheets/sheet1.xml`,归一到 `xl/worksheets/sheet1.xml`。
fn normalize_xl_path(target: &str) -> String {
    let t = target.trim_start_matches('/');
    if t.starts_with("xl/") {
        t.to_string()
    } else {
        format!("xl/{t}")
    }
}

/// 解析某 `sheetN.xml`:得到 每格 `(row,col)` → cellXfs 索引,以及合并区。
#[allow(clippy::type_complexity)]
/// 从 `sheetN.xml` 抽取的几何/样式信息。
#[derive(Default)]
struct SheetGeom {
    /// `(row, col)` → cellXfs 样式索引。
    styles: std::collections::HashMap<(u32, u32), usize>,
    /// 合并区。
    merges: Vec<(u32, u32, u32, u32)>,
    /// 列宽覆盖:列(0 基)→ Excel 字符宽度(`col@width`)。
    col_widths: std::collections::HashMap<u32, f64>,
    /// 冻结的顶部行数 / 左侧列数(来自 `sheetView/pane`)。
    freeze_rows: u32,
    freeze_cols: u32,
    /// 迷你图:`(host_row, host_col, 类型, 数据范围 A1)`(类型 line/column/stacked)。
    sparklines: Vec<(u32, u32, String, String)>,
}

fn read_cell_styles(zip: &mut ZipArchive<Cursor<Vec<u8>>>, path: &str) -> SheetGeom {
    let mut g = SheetGeom::default();
    let xml = match zip_text(zip, path) {
        Some(x) => x,
        None => return g,
    };
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    // 迷你图累积:当前组类型 + 当前 sparkline 的 f(数据范围)/sqref(宿主格)
    let mut spark_type = String::from("line");
    let mut cur_f: Option<String> = None;
    let mut cur_sqref: Option<String> = None;
    let mut in_f = false;
    let mut in_sqref = false;
    let mut txt = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let n = local(&e);
                match n.as_str() {
                    "sparklineGroup" => {
                        spark_type = attr(&e, "type").unwrap_or_else(|| "line".into());
                    }
                    "sparkline" => {
                        cur_f = None;
                        cur_sqref = None;
                    }
                    "f" => {
                        in_f = true;
                        txt.clear();
                    }
                    "sqref" => {
                        in_sqref = true;
                        txt.clear();
                    }
                    "c" => {
                        if let (Some(rc), Some(s)) = (
                            attr(&e, "r").and_then(|r| parse_a1(&r)),
                            attr(&e, "s").and_then(|s| s.parse::<usize>().ok()),
                        ) {
                            g.styles.insert(rc, s);
                        }
                    }
                    "mergeCell" => {
                        if let Some(m) = attr(&e, "ref").and_then(|r| parse_a1_range(&r)) {
                            g.merges.push(m);
                        }
                    }
                    "col" => {
                        // <col min= max= width= customWidth=/>(1 基列)
                        if let (Some(min), Some(max), Some(w)) = (
                            attr(&e, "min").and_then(|s| s.parse::<u32>().ok()),
                            attr(&e, "max").and_then(|s| s.parse::<u32>().ok()),
                            attr(&e, "width").and_then(|s| s.parse::<f64>().ok()),
                        ) {
                            for c in min..=max.min(min + 16_384) {
                                if c >= 1 {
                                    g.col_widths.insert(c - 1, w);
                                }
                            }
                        }
                    }
                    // 冻结窗格:xSplit=冻结列数、ySplit=冻结行数(state="frozen")
                    "pane" if attr(&e, "state").as_deref() == Some("frozen") => {
                        g.freeze_cols = attr(&e, "xSplit")
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.0) as u32;
                        g.freeze_rows = attr(&e, "ySplit")
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.0) as u32;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_f || in_sqref {
                    if let Ok(s) = t.decode() {
                        txt.push_str(&s);
                    }
                }
            }
            Ok(Event::End(e)) => match local_end(&e).as_str() {
                "f" => {
                    in_f = false;
                    cur_f = Some(txt.trim().to_string());
                }
                "sqref" => {
                    in_sqref = false;
                    cur_sqref = Some(txt.trim().to_string());
                }
                "sparkline" => {
                    if let (Some(f), Some(sq)) = (cur_f.take(), cur_sqref.take()) {
                        if let Some((r, c)) = parse_a1(&sq) {
                            g.sparklines.push((r, c, spark_type.clone(), f));
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    g
}

/// 条件格式规则。
enum CfKind {
    /// `cellIs`:按运算符与操作数比较,命中则用 dxf 填充色。
    CellIs {
        op: String,
        operands: Vec<f64>,
        dxf_id: usize,
    },
    /// 色阶:2 或 3 色,按区域内数值线性插值填充。
    ColorScale { colors: Vec<String> },
}

/// 一块条件格式区域:作用单元格 + 规则。
struct CfRegion {
    cells: Vec<(u32, u32)>,
    kind: CfKind,
}

/// 解析某 `sheetN.xml` 的 `conditionalFormatting` 区域(cellIs / colorScale)。
fn read_conditional_formatting(zip: &mut ZipArchive<Cursor<Vec<u8>>>, path: &str) -> Vec<CfRegion> {
    let xml = match zip_text(zip, path) {
        Some(x) => x,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    // 当前 conditionalFormatting 的作用格 + 当前 cfRule 累积
    let mut cur_cells: Vec<(u32, u32)> = Vec::new();
    let mut cur_type: Option<String> = None;
    let mut cur_op: Option<String> = None;
    let mut cur_dxf: usize = 0;
    let mut cur_formulas: Vec<f64> = Vec::new();
    let mut cur_colors: Vec<String> = Vec::new();
    let mut in_formula = false;
    let mut formula_buf = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(&e).as_str() {
                "conditionalFormatting" => {
                    cur_cells = attr(&e, "sqref")
                        .map(|s| expand_sqref(&s))
                        .unwrap_or_default();
                }
                "cfRule" => {
                    cur_type = attr(&e, "type");
                    cur_op = attr(&e, "operator");
                    cur_dxf = attr(&e, "dxfId").and_then(|s| s.parse().ok()).unwrap_or(0);
                    cur_formulas.clear();
                    cur_colors.clear();
                }
                "formula" => {
                    in_formula = true;
                    formula_buf.clear();
                }
                "color" if cur_type.as_deref() == Some("colorScale") => {
                    if let Some(rgb) = attr(&e, "rgb").map(|s| normalize_argb(&s)) {
                        cur_colors.push(rgb);
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if in_formula {
                    if let Ok(s) = t.decode() {
                        formula_buf.push_str(&s);
                    }
                }
            }
            Ok(Event::End(e)) => match local_end(&e).as_str() {
                "formula" => {
                    in_formula = false;
                    if let Ok(v) = formula_buf.trim().parse::<f64>() {
                        cur_formulas.push(v);
                    }
                }
                "cfRule" => {
                    let cells = cur_cells.clone();
                    match cur_type.as_deref() {
                        Some("cellIs") => out.push(CfRegion {
                            cells,
                            kind: CfKind::CellIs {
                                op: cur_op.clone().unwrap_or_default(),
                                operands: cur_formulas.clone(),
                                dxf_id: cur_dxf,
                            },
                        }),
                        Some("colorScale") if cur_colors.len() >= 2 => out.push(CfRegion {
                            cells,
                            kind: CfKind::ColorScale {
                                colors: cur_colors.clone(),
                            },
                        }),
                        _ => {}
                    }
                    cur_type = None;
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// 展开 `sqref`(空格分隔的多个 A1 范围或单元格)为单元格坐标列表。
fn expand_sqref(sqref: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for part in sqref.split_whitespace() {
        if let Some((r0, c0, r1, c1)) = parse_a1_range(part) {
            for r in r0..=r1 {
                for c in c0..=c1 {
                    out.push((r, c));
                }
            }
        } else if let Some(rc) = parse_a1(part) {
            out.push(rc);
        }
    }
    out
}

/// 对条件格式区域求值,产出 `(row, col) → 填充色 RRGGBB` 覆盖(后者优先)。
fn evaluate_cf(
    regions: &[CfRegion],
    sheet: &Sheet,
    dxf_fills: &[Option<String>],
) -> std::collections::HashMap<(u32, u32), String> {
    let mut out = std::collections::HashMap::new();
    let num_at = |r: u32, c: u32| -> Option<f64> {
        sheet
            .cell(r as usize, c as usize)
            .trim()
            .parse::<f64>()
            .ok()
    };
    for region in regions {
        match &region.kind {
            CfKind::CellIs {
                op,
                operands,
                dxf_id,
            } => {
                let Some(Some(fill)) = dxf_fills.get(*dxf_id) else {
                    continue;
                };
                for &(r, c) in &region.cells {
                    let Some(v) = num_at(r, c) else { continue };
                    if cell_is_match(op, v, operands) {
                        out.insert((r, c), fill.clone());
                    }
                }
            }
            CfKind::ColorScale { colors } => {
                let vals: Vec<((u32, u32), f64)> = region
                    .cells
                    .iter()
                    .filter_map(|&(r, c)| num_at(r, c).map(|v| ((r, c), v)))
                    .collect();
                if vals.is_empty() {
                    continue;
                }
                let min = vals.iter().map(|&(_, v)| v).fold(f64::INFINITY, f64::min);
                let max = vals
                    .iter()
                    .map(|&(_, v)| v)
                    .fold(f64::NEG_INFINITY, f64::max);
                for (rc, v) in vals {
                    let t = if max > min {
                        (v - min) / (max - min)
                    } else {
                        0.5
                    };
                    out.insert(rc, color_scale(colors, t));
                }
            }
        }
    }
    out
}

/// cellIs 运算符判定。
fn cell_is_match(op: &str, v: f64, operands: &[f64]) -> bool {
    let a = operands.first().copied().unwrap_or(0.0);
    let b = operands.get(1).copied().unwrap_or(0.0);
    match op {
        "greaterThan" => v > a,
        "greaterThanOrEqual" => v >= a,
        "lessThan" => v < a,
        "lessThanOrEqual" => v <= a,
        "equal" => v == a,
        "notEqual" => v != a,
        "between" => v >= a.min(b) && v <= a.max(b),
        "notBetween" => v < a.min(b) || v > a.max(b),
        _ => false,
    }
}

/// 色阶插值:t∈[0,1] 在 colors(2 或 3 色)间线性取色 → RRGGBB。
fn color_scale(colors: &[String], t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    let parse = |s: &str| -> (f64, f64, f64) {
        let n = u32::from_str_radix(s, 16).unwrap_or(0);
        (
            ((n >> 16) & 0xff) as f64,
            ((n >> 8) & 0xff) as f64,
            (n & 0xff) as f64,
        )
    };
    let lerp = |a: (f64, f64, f64), b: (f64, f64, f64), u: f64| -> String {
        let r = (a.0 + (b.0 - a.0) * u).round() as u32;
        let g = (a.1 + (b.1 - a.1) * u).round() as u32;
        let bl = (a.2 + (b.2 - a.2) * u).round() as u32;
        format!("{r:02X}{g:02X}{bl:02X}")
    };
    if colors.len() >= 3 {
        let (lo, mid, hi) = (parse(&colors[0]), parse(&colors[1]), parse(&colors[2]));
        if t <= 0.5 {
            lerp(lo, mid, t / 0.5)
        } else {
            lerp(mid, hi, (t - 0.5) / 0.5)
        }
    } else {
        lerp(parse(&colors[0]), parse(&colors[1]), t)
    }
}

/// A1 记法 → (row, col)(0 基)。
fn parse_a1(s: &str) -> Option<(u32, u32)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut col = 0u32;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        col = col * 26 + (bytes[i].to_ascii_uppercase() - b'A' + 1) as u32;
        i += 1;
    }
    if i == 0 || col == 0 {
        return None;
    }
    let row: u32 = s[i..].parse().ok()?;
    if row == 0 {
        return None;
    }
    Some((row - 1, col - 1))
}

/// `A1:B2` → (row0, col0, row1, col1)(0 基,归一)。
fn parse_a1_range(s: &str) -> Option<(u32, u32, u32, u32)> {
    let (a, b) = s.split_once(':')?;
    let (r0, c0) = parse_a1(a)?;
    let (r1, c1) = parse_a1(b)?;
    Some((r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1)))
}

/// Excel 日期**序列数** → `YYYY-MM-DD`(有时分秒则追加 ` HH:MM:SS`)。
///
/// Excel 纪元为 1899-12-30(1900 系统),整数部分为天、其余为当天时间比例。
/// 用 Howard Hinnant 的 civil-from-days 算法换算,不依赖 chrono。
fn excel_serial_to_string(serial: f64) -> String {
    let whole = serial.floor();
    let days_since_1970 = whole as i64 - 25569; // 1899-12-30 → 1970-01-01 相差 25569 天
    let (y, m, d) = civil_from_days(days_since_1970);

    // 当天时间(四舍五入到秒)
    let secs = ((serial - whole) * 86_400.0).round() as i64;
    let (secs, extra_day) = if secs >= 86_400 {
        (secs - 86_400, 1)
    } else {
        (secs, 0)
    };
    let (y, m, d) = if extra_day == 1 {
        civil_from_days(days_since_1970 + 1)
    } else {
        (y, m, d)
    };
    let (hh, mm, ss) = (secs / 3600, (secs % 3600) / 60, secs % 60);

    if hh == 0 && mm == 0 && ss == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
    }
}

/// 天数(自 1970-01-01)→ (年, 月, 日)。Hinnant civil_from_days。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_to_date_basic() {
        // 25569 = 1970-01-01;44197 = 2021-01-01
        assert_eq!(excel_serial_to_string(25569.0), "1970-01-01");
        assert_eq!(excel_serial_to_string(44197.0), "2021-01-01");
        // 1 = 1899-12-31(1900 系统,忽略闰年 bug 的边界)
        assert_eq!(excel_serial_to_string(2.0), "1900-01-01");
    }

    #[test]
    fn serial_to_date_with_time() {
        // 44197.5 = 2021-01-01 12:00:00
        assert_eq!(excel_serial_to_string(44197.5), "2021-01-01 12:00:00");
    }

    #[test]
    fn cell_text_variants() {
        assert_eq!(cell_text(&Data::Empty), "");
        assert_eq!(cell_text(&Data::Int(42)), "42");
        assert_eq!(cell_text(&Data::Float(14.0)), "14");
        assert_eq!(cell_text(&Data::Float(3.5)), "3.5");
        assert_eq!(cell_text(&Data::Bool(true)), "TRUE");
        assert_eq!(cell_text(&Data::String("hi".into())), "hi");
    }

    /// 手工构造一个最小 xlsx(2 张表、数值/字符串/公式+缓存值),验证解析。
    fn tiny_xlsx() -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};

        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let mut put = |name: &str, data: &str| {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            };
            put(
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            );
            put(
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            );
            put(
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="数据" sheetId="1" r:id="rId1"/><sheet name="第二表" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            );
            put(
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
            );
            // sheet1: A1=商品(inlineStr) B1=单价 ; A2=苹果 B2=3.5 C2=4 D2==B2*C2 缓存 14
            put(
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>商品</t></is></c><c r="B1" t="inlineStr"><is><t>单价</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>苹果</t></is></c><c r="B2"><v>3.5</v></c><c r="C2"><v>4</v></c><c r="D2"><f>B2*C2</f><v>14</v></c></row></sheetData></worksheet>"#,
            );
            put(
                "xl/worksheets/sheet2.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>仅此表</t></is></c></row></sheetData></worksheet>"#,
            );
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn parses_multi_sheet_values_and_formula() {
        let bytes = tiny_xlsx();
        let wb = parse(&bytes).expect("应能解析");
        assert_eq!(wb.sheets.len(), 2, "两张工作表");
        assert_eq!(wb.sheets[0].name, "数据");
        assert_eq!(wb.sheets[1].name, "第二表");

        let s0 = &wb.sheets[0].sheet;
        assert_eq!(s0.cell(0, 0), "商品");
        assert_eq!(s0.cell(1, 0), "苹果");
        assert_eq!(s0.cell(1, 1), "3.5");
        assert_eq!(s0.cell(1, 3), "14", "公式格显示缓存计算值");

        // 公式清单:D2(row1,col3)= =B2*C2
        let f = &wb.sheets[0].formulas;
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].row, f[0].col), (1, 3));
        assert_eq!(f[0].source, "=B2*C2");

        assert_eq!(wb.sheets[1].sheet.cell(0, 0), "仅此表");
        assert!(wb.sheets[1].formulas.is_empty());
    }

    /// 构造带样式(numfmt 百分比)+ 合并单元格的最小 xlsx。
    fn styled_xlsx() -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};
        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let mut put = |name: &str, data: &str| {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            };
            put(
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            );
            put(
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            );
            put(
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="表" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            );
            put(
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
            );
            // styles:cellXfs[0]=默认;cellXfs[1]=numFmtId 9(0%);cellXfs[2]=自定义 164
            put(
                "xl/styles.xml",
                r##"<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="#,##0.00"/></numFmts><fonts count="2"><font/><font><b/><color rgb="FFFF0000"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill></fills><borders count="2"><border><left/><right/><top/><bottom/></border><border><left style="thin"><color rgb="FF0000FF"/></left><right style="thin"><color rgb="FF0000FF"/></right><top style="thick"><color rgb="FF0000FF"/></top><bottom style="thin"><color rgb="FF0000FF"/></bottom></border></borders><cellXfs count="5"><xf numFmtId="0"/><xf numFmtId="9" applyNumberFormat="1"/><xf numFmtId="164" applyNumberFormat="1"/><xf numFmtId="0" fontId="1" fillId="2" applyFont="1" applyFill="1"><alignment horizontal="center"/></xf><xf numFmtId="164" borderId="1" applyNumberFormat="1" applyBorder="1"/></cellXfs></styleSheet>"##,
            );
            // A1=0.25 s=1(百分比);B1=1234.5 s=4(#,##0.00 + 蓝边框);A2 s=3(粗+红字+黄底+居中);合并 A2:B2
            put(
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView><pane xSplit="1" ySplit="1" topLeftCell="B2" state="frozen"/></sheetView></sheetViews><cols><col min="2" max="2" width="20.5" customWidth="1"/></cols><sheetData><row r="1"><c r="A1" s="1"><v>0.25</v></c><c r="B1" s="4"><v>1234.5</v></c></row><row r="2"><c r="A2" s="3" t="inlineStr"><is><t>合并</t></is></c></row></sheetData><mergeCells count="1"><mergeCell ref="A2:B2"/></mergeCells></worksheet>"#,
            );
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn applies_per_cell_numfmt_and_merges() {
        let wb = parse(&styled_xlsx()).expect("应能解析");
        let s = &wb.sheets[0];
        assert_eq!(s.sheet.cell(0, 0), "25%", "numFmtId 9 → 百分比");
        assert_eq!(s.sheet.cell(0, 1), "1,234.50", "自定义 #,##0.00");
        assert_eq!(s.merges, vec![(1, 0, 1, 1)], "合并区 A2:B2");
        // A2(row1,col0)= 粗 + 红字 + 黄底 + 居中
        let f = s
            .formats
            .iter()
            .find(|&&(r, c, _)| (r, c) == (1, 0))
            .map(|(_, _, f)| f);
        let f = f.expect("A2 应有样式");
        assert!(f.bold);
        assert_eq!(f.color.as_deref(), Some("FF0000"));
        assert_eq!(f.fill.as_deref(), Some("FFFF00"));
        assert_eq!(f.align.as_deref(), Some("center"));
        // B1(row0,col1)= 蓝色边框(上边 thick=2.5,其余 thin=1)
        let bf = s
            .formats
            .iter()
            .find(|&&(r, c, _)| (r, c) == (0, 1))
            .map(|(_, _, f)| f)
            .expect("B1 应有样式");
        let b = bf.border.as_ref().expect("B1 应有边框");
        assert_eq!(b.top.as_ref().unwrap().color, "0000FF");
        assert!(
            (b.top.as_ref().unwrap().width - 2.5).abs() < 0.01,
            "top thick"
        );
        assert!(
            (b.left.as_ref().unwrap().width - 1.0).abs() < 0.01,
            "left thin"
        );
        assert!(b.bottom.is_some() && b.right.is_some());
        // 列宽:B 列(col 1)= 20.5;冻结 1 行 1 列
        assert_eq!(s.col_widths, vec![(1, 20.5)]);
        assert_eq!((s.freeze_rows, s.freeze_cols), (1, 1));
    }

    /// 构造带内嵌图片(twoCellAnchor)的最小 xlsx。
    fn image_xlsx() -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};
        // 2x2 PNG(最小合法)
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00,
            0x00, 0xFD, 0xD4, 0x9A, 0x73, 0x00, 0x00, 0x00, 0x16, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x8C, 0x0C, 0x0C, 0x0C, 0x00, 0x00, 0x0C,
            0x0C, 0x02, 0xFC, 0x8B, 0x8D, 0xB0, 0x8D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
            0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let mut put = |name: &str, data: &[u8]| {
                w.start_file(name, opts).unwrap();
                w.write_all(data).unwrap();
            };
            put("[Content_Types].xml", br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#);
            put("_rels/.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#);
            put("xl/workbook.xml", br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#);
            put("xl/_rels/workbook.xml.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#);
            put("xl/worksheets/sheet1.xml", br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>x</t></is></c></row></sheetData><drawing r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></worksheet>"#);
            put("xl/worksheets/_rels/sheet1.xml.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#);
            put("xl/drawings/drawing1.xml", br#"<?xml version="1.0"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:pic><xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill></xdr:pic></xdr:twoCellAnchor></xdr:wsDr>"#);
            put("xl/drawings/_rels/drawing1.xml.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#);
            put("xl/media/image1.png", PNG);
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn parses_embedded_image() {
        let wb = parse(&image_xlsx()).expect("应能解析");
        let s = &wb.sheets[0];
        assert_eq!(s.images.len(), 1, "一张图片");
        let img = &s.images[0];
        assert_eq!((img.from_row, img.from_col), (2, 1), "from 锚 (row2,col1)");
        assert_eq!(img.to, Some((6, 4)), "to 锚 (row6,col4)");
        assert_eq!(img.media_key, "xl/media/image1.png");
        // 媒体字节已收集
        let m = wb
            .media
            .iter()
            .find(|m| m.key == img.media_key)
            .expect("media");
        assert_eq!(m.mime, "image/png");
        assert!(!m.data.is_empty());
    }

    #[test]
    fn parses_sparklines() {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};
        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let mut put = |name: &str, data: &str| {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            };
            put(
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            );
            put(
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            );
            put(
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            );
            put(
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            );
            // B1..E1 = 1,3,2,5;A1 宿主一条 line 迷你图,数据 S!B1:E1
            put(
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>趋势</t></is></c><c r="B1"><v>1</v></c><c r="C1"><v>3</v></c><c r="D1"><v>2</v></c><c r="E1"><v>5</v></c></row></sheetData><extLst><ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}"><x14:sparklineGroups xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroup type="column"><x14:sparklines><x14:sparkline><xm:f xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">S!B1:E1</xm:f><xm:sqref xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">A1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst></worksheet>"#,
            );
            w.finish().unwrap();
        }
        let wb = parse(&buf).expect("解析");
        let sp = &wb.sheets[0].sparklines;
        assert_eq!(sp.len(), 1);
        assert_eq!((sp[0].row, sp[0].col), (0, 0), "宿主 A1");
        assert_eq!(sp[0].kind, "column");
        assert_eq!(sp[0].values, vec![1.0, 3.0, 2.0, 5.0]);
    }

    #[test]
    fn conditional_formatting_cellis_and_colorscale() {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};
        let mut buf = Vec::new();
        {
            let mut w = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let mut put = |name: &str, data: &str| {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            };
            put(
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            );
            put(
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            );
            put(
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            );
            put(
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
            );
            // dxf[0] 填充红 FF0000
            put(
                "xl/styles.xml",
                r#"<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellXfs count="1"><xf numFmtId="0"/></cellXfs><dxfs count="1"><dxf><fill><patternFill><bgColor rgb="FFFF0000"/></patternFill></fill></dxf></dxfs></styleSheet>"#,
            );
            // A1..A3 = 1,5,9;cellIs >4 → A2/A3 红;B1..B3 = 0,50,100 colorScale 红→绿
            put(
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>0</v></c></row><row r="2"><c r="A2"><v>5</v></c><c r="B2"><v>50</v></c></row><row r="3"><c r="A3"><v>9</v></c><c r="B3"><v>100</v></c></row></sheetData><conditionalFormatting sqref="A1:A3"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>4</formula></cfRule></conditionalFormatting><conditionalFormatting sqref="B1:B3"><cfRule type="colorScale" priority="2"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FFFF0000"/><color rgb="FF00FF00"/></colorScale></cfRule></conditionalFormatting></worksheet>"#,
            );
            w.finish().unwrap();
        }
        let wb = parse(&buf).expect("解析");
        let s = &wb.sheets[0];
        let fill_at = |r: u32, c: u32| {
            s.formats
                .iter()
                .find(|&&(rr, cc, _)| (rr, cc) == (r, c))
                .and_then(|(_, _, f)| f.fill.clone())
        };
        // cellIs > 4:A1(1)无,A2(5)/A3(9)红
        assert_eq!(fill_at(0, 0), None);
        assert_eq!(fill_at(1, 0).as_deref(), Some("FF0000"));
        assert_eq!(fill_at(2, 0).as_deref(), Some("FF0000"));
        // colorScale:B1=min 红、B3=max 绿、B2 中间(非纯红/绿)
        assert_eq!(fill_at(0, 1).as_deref(), Some("FF0000"));
        assert_eq!(fill_at(2, 1).as_deref(), Some("00FF00"));
        let mid = fill_at(1, 1).unwrap();
        assert_ne!(mid, "FF0000");
        assert_ne!(mid, "00FF00");
    }

    #[test]
    #[ignore = "生成浏览器 e2e 夹具"]
    fn write_browser_fixture() {
        let bytes = tiny_xlsx();
        std::fs::write("/tmp/office-r-sample.xlsx", &bytes).unwrap();
        eprintln!("wrote /tmp/office-r-sample.xlsx ({} bytes)", bytes.len());
    }
}
