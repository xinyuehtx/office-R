//! **OOXML / OPC 容器级共享原语** —— docx / xlsx / pptx 三种格式共用的那一层。
//!
//! 这个 crate 的边界是「无论读三种格式的哪一种都成立的事实」:
//!
//! - OPC 包是 zip,part 之间用 `.rels` 关系文件互指,target 可能带 `../`;
//! - XML 元素与属性名带命名空间前缀,解析时一律取本地名;
//! - 度量单位是 EMU(914400 EMU = 1 英寸 = 96px);
//! - part 的 content-type 由扩展名映射。
//!
//! 这些原语此前在 `docx.rs` / `xlsx.rs` / `pptx.rs` 里**各有一份**(`mime_of` 三份
//! 逻辑逐字节相同,EMU 换算三份,`local`/`attr` 三种写法,rels 解析两种形状)。
//! 按格式拆 crate 时如果不先合并,重复就会被固化成三个 crate 的永久债 ——
//! 所以合并发生在拆分**之前**。
//!
//! [`chart`] 也在这里:它是 xlsx 与 pptx 之间唯一真正的横向依赖。

pub mod chart;

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::events::{BytesEnd, BytesStart};
use quick_xml::Reader as XmlReader;
use zip::ZipArchive;

/// 打开的 OPC 包(zip)。
pub type Package = ZipArchive<Cursor<Vec<u8>>>;

// ---------- 度量与 content-type ----------

/// EMU → CSS 像素。914400 EMU = 1 英寸 = 96px,即 ÷9525。
pub fn emu_to_px(emu: f64) -> f64 {
    emu / 9525.0
}

/// 按扩展名给出 part 的 content-type。
///
/// 用 `to_ascii_lowercase` 而非 `to_lowercase`:扩展名本来就是 ASCII 语义,
/// 且避免土耳其无点 ı 一类 locale 相关的意外(三份旧实现里有一份用的是后者)。
pub fn mime_of(path: &str) -> String {
    let l = path.to_ascii_lowercase();
    let mime = if l.ends_with(".png") {
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
    };
    mime.to_string()
}

// ---------- XML 名与属性 ----------

/// 元素本地名(去命名空间前缀),零分配版 —— 热路径用它。
pub fn local_name(raw: &[u8]) -> &[u8] {
    raw.rsplit(|&b| b == b':').next().unwrap_or(raw)
}

/// 起始/自闭合元素的本地名。
pub fn local(e: &BytesStart) -> String {
    String::from_utf8_lossy(local_name(e.name().as_ref())).into_owned()
}

/// 结束元素的本地名。
pub fn local_end(e: &BytesEnd) -> String {
    String::from_utf8_lossy(local_name(e.name().as_ref())).into_owned()
}

/// 取属性值,按**本地名**匹配 —— 于是 `r:embed` 用 `"embed"` 就能取到。
pub fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let raw = a.key.as_ref();
        if local_name(raw) == key.as_bytes() {
            Some(String::from_utf8_lossy(&a.value).into_owned())
        } else {
            None
        }
    })
}

// ---------- zip 读取 ----------

/// 读 zip 内某文本 part。
pub fn read_text(zip: &mut Package, name: &str) -> Result<String, String> {
    let mut s = String::new();
    zip.by_name(name)
        .map_err(|e| format!("读取 {name} 失败:{e}"))?
        .read_to_string(&mut s)
        .map_err(|e| format!("解码 {name} 失败:{e}"))?;
    Ok(s)
}

/// 读 zip 内某二进制 part。
pub fn read_bytes(zip: &mut Package, name: &str) -> Result<Vec<u8>, String> {
    let mut v = Vec::new();
    zip.by_name(name)
        .map_err(|e| format!("读取 {name} 失败:{e}"))?
        .read_to_end(&mut v)
        .map_err(|e| format!("读取 {name} 失败:{e}"))?;
    Ok(v)
}

/// zip 里是否存在某个 part —— 供各格式的 `can_open` 用。
pub fn has_entry(bytes: &[u8], name: &str) -> bool {
    match ZipArchive::new(Cursor::new(bytes.to_vec())) {
        Ok(mut zip) => zip.by_name(name).is_ok(),
        Err(_) => false,
    }
}

// ---------- rels ----------

/// part 路径 → 其 `.rels` 路径。`ppt/slides/slide1.xml` → `ppt/slides/_rels/slide1.xml.rels`。
pub fn rels_path_of(part_path: &str) -> String {
    match part_path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part_path}.rels"),
    }
}

/// part 路径 → 其所在目录(供 [`resolve_rel_path`] 当 base)。
pub fn dir_of(part_path: &str) -> &str {
    match part_path.rsplit_once('/') {
        Some((dir, _)) => dir,
        None => "",
    }
}

/// 相对 rels target(可能含 `../`)按 `base_dir` 归一到 zip 内绝对路径。
///
/// 这是**唯一正确通用**的实现:真正逐段处理 `.` / `..`,而不是字符串前缀替换。
/// 旧的 pptx 侧用 `strip_prefix("../")` + `replace("../", "")`,遇到
/// `../../media/x.png` 或非 `ppt/slides/` 下的 part 会算错。
pub fn resolve_rel_path(base_dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
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

/// 一条关系。
#[derive(Debug, Clone, PartialEq)]
pub struct Rel {
    pub id: String,
    /// 关系类型 URI(如 `.../relationships/image`)。
    pub ty: String,
    /// 已按所属 part 目录归一的 zip 内路径。
    pub target: String,
}

impl Rel {
    /// 关系类型是否为 `kind`(如 `"image"` / `"slideLayout"` / `"chart"`)。
    ///
    /// 按类型 URI 的**末段**比较,于是完整 URI
    /// `http://…/relationships/image` 与简写 `image` 都能命中 ——
    /// 真实文件写前者,而测试夹具为可读性写后者。
    pub fn is_kind(&self, kind: &str) -> bool {
        self.ty.rsplit('/').next() == Some(kind)
    }
}

/// 解析某个 part 的 `.rels`,target 已归一为 zip 内绝对路径。
///
/// `part_path` 是**引用方**的路径(如 `ppt/slides/slide1.xml`),用于确定 base 目录。
pub fn parse_rels(xml: &str, part_path: &str) -> Vec<Rel> {
    let base = dir_of(part_path);
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local(&e) == "Relationship" => {
                if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    out.push(Rel {
                        id,
                        ty: attr(&e, "Type").unwrap_or_default(),
                        target: resolve_rel_path(base, &t),
                    });
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// 读并解析某个 part 的 `.rels`;读不到时返回空。
pub fn rels_for(zip: &mut Package, part_path: &str) -> Vec<Rel> {
    match read_text(zip, &rels_path_of(part_path)) {
        Ok(xml) => parse_rels(&xml, part_path),
        Err(_) => Vec::new(),
    }
}

/// id → 归一化目标路径。
pub fn rels_map(rels: &[Rel]) -> HashMap<String, String> {
    rels.iter()
        .map(|r| (r.id.clone(), r.target.clone()))
        .collect()
}

/// 找首个类型为 `kind` 的关系目标(如 `"styles"` / `"worksheet"`)。
pub fn find_rel_target(rels: &[Rel], kind: &str) -> Option<String> {
    rels.iter()
        .find(|r| r.is_kind(kind))
        .map(|r| r.target.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_by_extension_is_case_insensitive() {
        assert_eq!(mime_of("a/b/image1.PNG"), "image/png");
        assert_eq!(mime_of("x.jpeg"), "image/jpeg");
        assert_eq!(mime_of("x.jpg"), "image/jpeg");
        assert_eq!(mime_of("x.svg"), "image/svg+xml");
        assert_eq!(mime_of("x.wmf"), "application/octet-stream");
    }

    #[test]
    fn emu_conversion() {
        assert_eq!(emu_to_px(9525.0), 1.0);
        assert_eq!(emu_to_px(914_400.0), 96.0);
    }

    /// 这组断言覆盖真实 OPC 包里会出现的四种 target 形态。
    /// 与旧的字符串前缀 hack 的关键差异是最后两条(多级 `..` 与非 slides 目录)。
    #[test]
    fn resolves_relative_targets() {
        let slides = dir_of("ppt/slides/slide1.xml");
        assert_eq!(slides, "ppt/slides");
        assert_eq!(
            resolve_rel_path(slides, "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            resolve_rel_path(slides, "image1.png"),
            "ppt/slides/image1.png"
        );
        // 绝对 target(以 / 开头)直接去掉前导斜杠
        assert_eq!(
            resolve_rel_path(slides, "/ppt/media/x.png"),
            "ppt/media/x.png"
        );
        // 多级 ..:旧实现的 strip_prefix("../") 只吃一层,会算错
        assert_eq!(
            resolve_rel_path("ppt/slides/sub", "../../media/x.png"),
            "ppt/media/x.png"
        );
        // presentation.xml 在 ppt/ 下,slide target 是相对它的
        assert_eq!(
            resolve_rel_path(dir_of("ppt/presentation.xml"), "slides/slide1.xml"),
            "ppt/slides/slide1.xml"
        );
        // 根级 part
        assert_eq!(
            resolve_rel_path(dir_of("[Content_Types].xml"), "xl/workbook.xml"),
            "xl/workbook.xml"
        );
    }

    #[test]
    fn rels_path_derivation() {
        assert_eq!(
            rels_path_of("ppt/slides/slide1.xml"),
            "ppt/slides/_rels/slide1.xml.rels"
        );
        assert_eq!(
            rels_path_of("presentation.xml"),
            "_rels/presentation.xml.rels"
        );
    }

    #[test]
    fn parses_rels_with_type_and_normalized_target() {
        let xml = r#"<?xml version="1.0"?><Relationships xmlns="r">
          <Relationship Id="rId1" Type="http://x/relationships/image" Target="../media/image1.png"/>
          <Relationship Id="rId2" Type="http://x/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
        </Relationships>"#;
        let rels = parse_rels(xml, "ppt/slides/slide1.xml");
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[0].target, "ppt/media/image1.png");
        assert_eq!(rels[1].target, "ppt/slideLayouts/slideLayout1.xml");
        assert_eq!(
            find_rel_target(&rels, "image").as_deref(),
            Some("ppt/media/image1.png")
        );
        // 简写与完整 URI 都能命中
        assert!(rels[0].is_kind("image"));
        assert!(Rel {
            id: "x".into(),
            ty: "image".into(),
            target: "y".into()
        }
        .is_kind("image"));
        assert_eq!(
            rels_map(&rels).get("rId2").unwrap(),
            "ppt/slideLayouts/slideLayout1.xml"
        );
    }
}
