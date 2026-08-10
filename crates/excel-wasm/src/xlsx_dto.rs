//! xlsx 结构 → JS 的**线格式**(DTO)。
//!
//! `office_core::xlsx` 的类型用元组表达可选锚点(`to: Option<(u32, u32)>`)、
//! 用 `snake_case` 命名 —— 那是内核自己的形状,不该被前端契约绑架。
//! 这里放一层薄 DTO 做映射,靠 `serde` 生成 JS 对象。
//!
//! 这层替代了此前约 180 行手写的 `js_sys::Reflect::set`:那种写法每个字段都要
//! 在循环里新建一次 JS 字符串键做哈希查找(一张 5000 个样式格的表就是 3.5 万次),
//! 而且字段名拼错只有 e2e 能发现,而 e2e 断言的是画布像素 —— 编译器和测试都挡不住。
//! 现在字段名由 `serde` 生成,并有 `tests` 里的线格式断言兜底。

use office_excel::xlsx::{BorderSide, Borders, CellFmt, XlsxChart, XlsxImage, XlsxSparkline};
use serde::Serialize;

/// 一条边框线。字段名 `w` 与前端 `BorderSide` 契约一致。
#[derive(Serialize)]
pub struct BorderSideDto {
    w: f64,
    color: String,
}

impl From<&BorderSide> for BorderSideDto {
    fn from(s: &BorderSide) -> Self {
        BorderSideDto {
            w: s.width,
            color: s.color.clone(),
        }
    }
}

/// 四边边框;无边的方向直接不出现在对象里。
#[derive(Serialize)]
pub struct BordersDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    top: Option<BorderSideDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    right: Option<BorderSideDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bottom: Option<BorderSideDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    left: Option<BorderSideDto>,
}

impl From<&Borders> for BordersDto {
    fn from(b: &Borders) -> Self {
        BordersDto {
            top: b.top.as_ref().map(Into::into),
            right: b.right.as_ref().map(Into::into),
            bottom: b.bottom.as_ref().map(Into::into),
            left: b.left.as_ref().map(Into::into),
        }
    }
}

/// 一格非默认样式(含所在行列)。
#[derive(Serialize)]
pub struct StyleDto<'a> {
    row: u32,
    col: u32,
    bold: bool,
    italic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    align: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    border: Option<BordersDto>,
}

impl<'a> StyleDto<'a> {
    pub fn new(row: u32, col: u32, f: &'a CellFmt) -> Self {
        StyleDto {
            row,
            col,
            bold: f.bold,
            italic: f.italic,
            color: f.color.as_deref(),
            fill: f.fill.as_deref(),
            align: f.align.as_deref(),
            border: f.border.as_ref().map(Into::into),
        }
    }
}

/// 内嵌图片锚点。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDto<'a> {
    media_key: &'a str,
    from_row: u32,
    from_col: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_row: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_col: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ext_w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ext_h: Option<f64>,
}

impl<'a> From<&'a XlsxImage> for ImageDto<'a> {
    fn from(i: &'a XlsxImage) -> Self {
        ImageDto {
            media_key: &i.media_key,
            from_row: i.from_row,
            from_col: i.from_col,
            to_row: i.to.map(|(r, _)| r),
            to_col: i.to.map(|(_, c)| c),
            ext_w: i.ext_px.map(|(w, _)| w),
            ext_h: i.ext_px.map(|(_, h)| h),
        }
    }
}

/// 内嵌图表。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartDto<'a> {
    from_row: u32,
    from_col: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_row: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_col: Option<u32>,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    series: &'a [Vec<f64>],
    categories: &'a [String],
}

impl<'a> From<&'a XlsxChart> for ChartDto<'a> {
    fn from(c: &'a XlsxChart) -> Self {
        ChartDto {
            from_row: c.from_row,
            from_col: c.from_col,
            to_row: c.to.map(|(r, _)| r),
            to_col: c.to.map(|(_, c)| c),
            kind: &c.kind,
            title: c.title.as_deref(),
            series: &c.series,
            categories: &c.categories,
        }
    }
}

/// 单元格内迷你图。
#[derive(Serialize)]
pub struct SparklineDto<'a> {
    row: u32,
    col: u32,
    kind: &'a str,
    values: &'a [f64],
}

impl<'a> From<&'a XlsxSparkline> for SparklineDto<'a> {
    fn from(s: &'a XlsxSparkline) -> Self {
        SparklineDto {
            row: s.row,
            col: s.col,
            kind: &s.kind,
            values: &s.values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DTO 的字段名就是**前端契约**(见 `web/src/apps/shared/sheet.ts`)。
    /// 拼错一个字母 Rust 侧照样编译通过,只有这里能挡住。
    #[test]
    fn style_wire_format_matches_frontend_contract() {
        let fmt = CellFmt {
            bold: true,
            italic: false,
            color: Some("FF0000".into()),
            fill: None,
            align: Some("center".into()),
            border: Some(Borders {
                top: Some(BorderSide {
                    width: 1.5,
                    color: "0000FF".into(),
                }),
                right: None,
                bottom: None,
                left: None,
            }),
        };
        let v = serde_json::to_value(StyleDto::new(3, 4, &fmt)).unwrap();
        assert_eq!(v["row"], 3);
        assert_eq!(v["col"], 4);
        assert_eq!(v["bold"], true);
        assert_eq!(v["italic"], false);
        assert_eq!(v["color"], "FF0000");
        assert_eq!(v["align"], "center");
        assert_eq!(v["border"]["top"]["w"], 1.5);
        assert_eq!(v["border"]["top"]["color"], "0000FF");
        // 未设置的可选字段不出现(前端按 undefined 判定)
        assert!(v.get("fill").is_none());
        assert!(v["border"].get("right").is_none());
    }

    #[test]
    fn image_and_chart_anchors_use_camel_case() {
        let img = XlsxImage {
            media_key: "xl/media/image1.png".into(),
            from_row: 1,
            from_col: 2,
            to: Some((5, 6)),
            ext_px: None,
        };
        let v = serde_json::to_value(ImageDto::from(&img)).unwrap();
        assert_eq!(v["mediaKey"], "xl/media/image1.png");
        assert_eq!(v["fromRow"], 1);
        assert_eq!(v["fromCol"], 2);
        assert_eq!(v["toRow"], 5);
        assert_eq!(v["toCol"], 6);
        assert!(v.get("extW").is_none());

        let chart = XlsxChart {
            from_row: 0,
            from_col: 0,
            to: None,
            kind: "bar".into(),
            series: vec![vec![1.0, 2.0]],
            categories: vec!["甲".into()],
            title: None,
        };
        let v = serde_json::to_value(ChartDto::from(&chart)).unwrap();
        assert_eq!(v["kind"], "bar");
        assert_eq!(v["series"][0][1], 2.0);
        assert_eq!(v["categories"][0], "甲");
        assert!(v.get("toRow").is_none());
        assert!(v.get("title").is_none());
    }

    #[test]
    fn sparkline_wire_format() {
        let sp = XlsxSparkline {
            row: 2,
            col: 3,
            kind: "line".into(),
            values: vec![1.0, 4.0, 9.0],
        };
        let v = serde_json::to_value(SparklineDto::from(&sp)).unwrap();
        assert_eq!(v["row"], 2);
        assert_eq!(v["col"], 3);
        assert_eq!(v["kind"], "line");
        assert_eq!(v["values"][2], 9.0);
    }
}
