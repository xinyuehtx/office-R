//! **DrawingML 图表**(`chartN.xml`)解析,xlsx 与 pptx 共用。
//!
//! 只提取渲染柱/线/饼图所需的最小信息:图表类型、各系列的缓存数值(`numCache`)、
//! 类别标签(`strCache`)与标题。坐标轴刻度、图例、数据标签等**非目标**。

use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use serde::Serialize;

/// 一个图表的可渲染数据。
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct ChartData {
    /// 类型:`"bar"` / `"line"` / `"pie"`。
    pub kind: String,
    /// 各系列的数值(来自 `numCache`)。
    pub series: Vec<Vec<f64>>,
    /// 类别标签(来自首个 `cat` 的 `strCache`);可能为空。
    pub categories: Vec<String>,
    /// 标题(如有)。
    pub title: Option<String>,
}

/// 元素本地名(去命名空间前缀)。
fn local_name(raw: &[u8]) -> &[u8] {
    raw.rsplit(|&b| b == b':').next().unwrap_or(raw)
}

/// 解析 `chartN.xml`:图表类型 + 各系列 `numCache` 数值 + 首个类别 `strCache` + 标题。
/// 返回 `None` 表示没有可识别的图表(无类型或无系列)。
pub fn parse_chart_xml(xml: &str) -> Option<ChartData> {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut kind: Option<String> = None;
    let mut series: Vec<Vec<f64>> = Vec::new();
    let mut categories: Vec<String> = Vec::new();
    let mut title: Option<String> = None;

    // 上下文:区分 val/cat 里的 <c:v>,以及 title 里的 <a:t>
    let mut in_val = false;
    let mut in_cat = false;
    let mut in_title = false;
    let mut in_v = false;
    let mut cur_series: Vec<f64> = Vec::new();
    let mut v_buf = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let n = local_name(e.name().as_ref()).to_vec();
                match n.as_slice() {
                    b"barChart" | b"bar3DChart" => {
                        kind.get_or_insert_with(|| "bar".into());
                    }
                    b"lineChart" | b"areaChart" => {
                        kind.get_or_insert_with(|| "line".into());
                    }
                    b"pieChart" | b"doughnutChart" => {
                        kind.get_or_insert_with(|| "pie".into());
                    }
                    b"ser" => cur_series = Vec::new(),
                    b"val" => in_val = true,
                    b"cat" => in_cat = true,
                    b"title" => in_title = true,
                    b"v" if in_val || in_cat => {
                        in_v = true;
                        v_buf.clear();
                    }
                    b"t" if in_title => {
                        in_v = true;
                        v_buf.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_v {
                    if let Ok(s) = t.decode() {
                        v_buf.push_str(&s);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let n = local_name(e.name().as_ref()).to_vec();
                match n.as_slice() {
                    b"val" => in_val = false,
                    b"cat" => in_cat = false,
                    b"title" => in_title = false,
                    b"ser" => series.push(std::mem::take(&mut cur_series)),
                    b"v" if in_v => {
                        in_v = false;
                        if in_val {
                            if let Ok(x) = v_buf.trim().parse::<f64>() {
                                cur_series.push(x);
                            }
                        } else if in_cat && categories.len() < 4096 {
                            categories.push(v_buf.trim().to_string());
                        }
                    }
                    b"t" if in_v && in_title => {
                        in_v = false;
                        if title.is_none() && !v_buf.trim().is_empty() {
                            title = Some(v_buf.trim().to_string());
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    let kind = kind?;
    if series.is_empty() {
        return None;
    }
    // 类别在每个 ser 里都会重复出现;只保留首系列长度对应的一份
    if let Some(first) = series.first() {
        categories.truncate(first.len());
    }
    Some(ChartData {
        kind,
        series,
        categories,
        title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bar_chart() {
        let xml = r#"<c:chartSpace xmlns:c="c" xmlns:a="a"><c:chart>
          <c:title><c:tx><c:rich><a:p><a:r><a:t>销量</a:t></a:r></a:p></c:rich></c:tx></c:title>
          <c:plotArea><c:barChart><c:ser>
            <c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>一月</c:v></c:pt><c:pt idx="1"><c:v>二月</c:v></c:pt></c:strCache></c:strRef></c:cat>
            <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt></c:numCache></c:numRef></c:val>
          </c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let c = parse_chart_xml(xml).expect("chart");
        assert_eq!(c.kind, "bar");
        assert_eq!(c.series, vec![vec![10.0, 20.0]]);
        assert_eq!(c.categories, vec!["一月".to_string(), "二月".to_string()]);
        assert_eq!(c.title.as_deref(), Some("销量"));
    }

    #[test]
    fn no_series_is_none() {
        assert!(parse_chart_xml("<c:chartSpace/>").is_none());
    }
}
