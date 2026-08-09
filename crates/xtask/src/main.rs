//! 仓库开发工具(`cargo run -p xtask -- <命令>`)。
//!
//! # 为什么不是 `#[ignore]` 的测试
//!
//! 夹具生成此前挂在三个 `#[ignore]` 测试上,由 npm script 触发再从 `/tmp` 拷贝。
//! 那套做法有三个真问题:
//!
//! 1. **静默假阳性**:`cargo test <过滤器>` 匹配不到任何测试时退出码仍是 0,
//!    后面的 `cp /tmp/...` 会happily 拷走上一次遗留的旧文件 —— 拿着陈旧夹具跑出绿灯。
//! 2. **平台绑定**:写死 `/tmp` + `cp` + `cd ..`,Windows 上跑不了。
//! 3. **无谓开销**:为了造 3 个几 KB 的文件,`crates/core` 的 dev-dependencies 里
//!    钉着 `docx-rs` 的 `image` 特性,每次 `cargo test` 都要多编一套位图解码。
//!
//! 现在生成是个正经的可执行入口:失败非零退出,路径由参数给出,依赖只属于本 crate。
//!
//! # 命令
//!
//! - `fixtures <目录>`:把 `sample.{docx,xlsx,pptx}` 写进该目录。
//! - `fixtures --check <目录>`:重新生成并与目录里的文件**逐字节比对**,
//!   不一致就非零退出。夹具是入库的(clone 即可跑 e2e、`git bisect` 时拿到的是
//!   那个 commit 的夹具),这道校验保证它们不会与生成器脱节。
//!   —— 三个生成器都是确定性的(zip 条目时间戳固定),所以字节比对可行。

use std::path::Path;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "用法:cargo run -p xtask -- fixtures [--check] <目录>";
    match args.first().map(String::as_str) {
        Some("fixtures") => {
            let check = args.iter().any(|a| a == "--check");
            let dir = args.iter().skip(1).find(|a| !a.starts_with("--"));
            match dir {
                Some(d) => fixtures(Path::new(d), check),
                None => {
                    eprintln!("{usage}");
                    std::process::ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("{usage}");
            std::process::ExitCode::from(2)
        }
    }
}

/// 生成(或校验)三份夹具。
fn fixtures(dir: &Path, check: bool) -> std::process::ExitCode {
    let files = [
        ("sample.docx", docx::build()),
        ("sample.xlsx", xlsx::build()),
        ("sample.pptx", pptx::build()),
    ];
    let mut drifted = Vec::new();
    for (name, bytes) in files {
        let path = dir.join(name);
        if check {
            match std::fs::read(&path) {
                Ok(on_disk) if on_disk == bytes => {
                    println!("  ✓ {name}({} 字节)", bytes.len());
                }
                Ok(on_disk) => {
                    println!(
                        "  ✗ {name}:入库 {} 字节,重新生成 {} 字节",
                        on_disk.len(),
                        bytes.len()
                    );
                    drifted.push(name);
                }
                Err(e) => {
                    println!("  ✗ {name}:读不到({e})");
                    drifted.push(name);
                }
            }
        } else {
            if let Err(e) = std::fs::write(&path, &bytes) {
                eprintln!("写 {} 失败:{e}", path.display());
                return std::process::ExitCode::FAILURE;
            }
            println!("  写入 {}({} 字节)", path.display(), bytes.len());
        }
    }
    if drifted.is_empty() {
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!(
            "\n入库夹具与生成器不一致:{}。\n改过生成器就要一并更新夹具:\n    cargo run -p xtask -- fixtures {}",
            drifted.join("、"),
            dir.display()
        );
        std::process::ExitCode::FAILURE
    }
}

pub mod docx {
    use docx_rs::*;
    use std::io::Cursor;

    /// 生成一份用于浏览器 e2e 的 .docx 夹具:标题/居中加粗/表格/图片。
    /// 默认忽略;需要时 `cargo test -p office-core write_browser_fixture -- --ignored --nocapture`。
    /// Word 夹具:标题/居中加粗/表格/图片/页眉页脚/修订。
    pub fn build() -> Vec<u8> {
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
            .header(Header::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("office-R 文档页眉"))))
            .footer(Footer::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("第 1 页 · 页脚"))))
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
            .add_paragraph(Paragraph::new().style("Heading2").add_run(Run::new().add_text("四、修订")))
            .add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text("保留文字 "))
                    .add_insert(Insert::new(Run::new().add_text("这是插入")))
                    .add_run(Run::new().add_text(" "))
                    .add_delete(Delete::new().add_run(Run::new().add_delete_text("这是删除"))),
            )
            .build()
            .pack(&mut Cursor::new(&mut buf))
            .expect("打包");
        buf
    }
}

pub mod pptx {
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

    /// PPT 夹具:两张幻灯(图片 / 旋转矩形 + 入场动画 + fade 切换 + 图表占位)。
    pub fn build() -> Vec<u8> {
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
            // 第二张:标题 + 旋转矩形(带入场动画)+ 图表占位,并带 fade 切换。
            let slide2 = r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="r"><p:cSld><p:spTree>
             <p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/></p:nvSpPr><p:spPr><a:xfrm><a:off x="838200" y="365760"/><a:ext cx="7772400" cy="1000000"/></a:xfrm>
               <a:prstGeom prst="rect"/></p:spPr>
               <p:txBody><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="3200" b="1"><a:solidFill><a:srgbClr val="1F3864"/></a:solidFill></a:rPr><a:t>第二张幻灯</a:t></a:r></a:p></p:txBody></p:sp>
             <p:sp><p:nvSpPr><p:cNvPr id="3" name="Rot"/></p:nvSpPr><p:spPr><a:xfrm rot="2700000"><a:off x="838200" y="1800000"/><a:ext cx="2200000" cy="1200000"/></a:xfrm>
               <a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></p:spPr></p:sp>
             <p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Chart"/></p:nvGraphicFramePr><p:xfrm><a:off x="4200000" y="1800000"/><a:ext cx="3500000" cy="2600000"/></p:xfrm>
               <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic></p:graphicFrame>
           </p:spTree></p:cSld>
           <p:transition><p:fade/></p:transition>
           <p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" nodeType="tmRoot"><p:childTnLst>
             <p:seq concurrent="1"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>
               <p:par><p:cTn id="3" fill="hold" nodeType="clickEffect"><p:childTnLst>
                 <p:set><p:cBhvr><p:tgtEl><p:spTgt spid="3"/></p:tgtEl></p:cBhvr></p:set>
               </p:childTnLst></p:cTn></p:par>
               <p:par><p:cTn id="4" fill="hold" nodeType="clickEffect"><p:childTnLst>
                 <p:set><p:cBhvr><p:tgtEl><p:spTgt spid="4"/></p:tgtEl></p:cBhvr></p:set>
               </p:childTnLst></p:cTn></p:par>
             </p:childTnLst></p:cTn></p:seq>
           </p:childTnLst></p:cTn></p:par></p:tnLst></p:timing></p:sld>"#;
            put(&mut w, "ppt/slides/slide2.xml", slide2.as_bytes());
            put(&mut w, "ppt/media/image1.png", PNG);
            w.finish().unwrap();
        }
        buf
    }
}

pub mod xlsx {
    /// Excel 夹具:两张工作表 + 公式缓存值(D2 = B2*C2,缓存 14)。
    pub fn build() -> Vec<u8> {
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
}
