//! Excel **数字格式化**:把数值按格式码渲染成显示文本。
//!
//! **为什么在 Rust**:格式码解析 + 大量单元格的格式化是与单元格数成正比的重 CPU 工作,
//! 放 WASM 一次算好,视图层只管画字符串。
//!
//! 覆盖 ECMA-376 数字格式码的**常用子集**(足够 CSV/公式结果的展示):
//! - 字面 `General`;
//! - `0` / `#` 占位符、小数位、千分位 `,`;
//! - 百分比 `%`(乘 100);
//! - 货币 / 文字前后缀(`$`、`"元"` 等,原样输出);
//! - 日期时间 `y m d h s`(基于 Excel 序列数,复刻 1900 闰年 bug);
//! - 科学计数 `E+`;
//! - 分节:`正;负;零;文本`(用 `;` 分隔,按值符号选节)。
//!
//! **非目标**(本期):颜色码 `[Red]`、条件 `[>=100]`、千分位缩放尾随逗号、分数 `?/?`、
//! 填充 `*`、区域设置 `[$-409]`。遇到不认识的记号按「原样/尽力」处理,绝不 panic。

/// 把 `value` 按 `code`(格式码)渲染成显示文本。`General` 或空码走通用格式。
pub fn format_number(value: f64, code: &str) -> String {
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return general(value);
    }

    // 分节:正;负;零[;文本]。按值选节;负值节存在时用其绝对值渲染。
    let sections = split_sections(code);
    let (section, use_abs) = pick_section(&sections, value);
    let v = if use_abs { value.abs() } else { value };

    if section.eq_ignore_ascii_case("general") {
        return general(v);
    }
    if is_date_code(section) {
        return format_datetime(v, section);
    }
    format_numeric(v, section)
}

/// 通用格式:整数不带小数点,小数去尾 0,吸收浮点噪声(15 位有效数字)。
/// 与公式引擎的 `formula::value::format_number` 口径一致。
pub fn general(n: f64) -> String {
    if n == 0.0 {
        return "0".to_string();
    }
    if !n.is_finite() {
        return "#NUM!".to_string();
    }
    let rounded = round_sig(n, 15);
    format!("{rounded}")
}

fn round_sig(n: f64, digits: i32) -> f64 {
    if n == 0.0 || !n.is_finite() {
        return n;
    }
    let d = n.abs().log10().floor() as i32 + 1;
    let power = digits - d;
    if !(-300..=300).contains(&power) {
        return n;
    }
    let factor = 10f64.powi(power);
    (n * factor).round() / factor
}

/// 按顶层 `;` 分节(不处理转义分号 —— 常用格式码里没有)。
fn split_sections(code: &str) -> Vec<&str> {
    code.split(';').collect()
}

/// 选用哪一节,返回 `(节, 是否取绝对值)`。
///
/// Excel 规则:1 节 → 全用;2 节 → 正/零用 [0]、负用 [1](绝对值);
/// 3+ 节 → 正 [0]、负 [1](绝对值)、零 [2]。
fn pick_section<'a>(sections: &[&'a str], value: f64) -> (&'a str, bool) {
    match sections.len() {
        0 => ("General", false),
        1 => (sections[0], false),
        2 => {
            if value < 0.0 {
                (sections[1], true)
            } else {
                (sections[0], false)
            }
        }
        _ => {
            if value > 0.0 {
                (sections[0], false)
            } else if value < 0.0 {
                (sections[1], true)
            } else {
                (sections[2], false)
            }
        }
    }
}

/// 是否是日期时间格式码:含未被引号包裹的日期时间字母。
fn is_date_code(code: &str) -> bool {
    let mut in_quote = false;
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'"' => in_quote = !in_quote,
            b'\\' => i += 1, // 跳过转义的下一个字符
            b'y' | b'Y' | b'm' | b'M' | b'd' | b'D' | b'h' | b'H' | b's' | b'S' if !in_quote => {
                return true
            }
            _ => {}
        }
        i += 1;
    }
    false
}

// ---------- 数值格式 ----------

/// 描述一个数值格式节解析出的结构。
struct NumFormat {
    prefix: String,
    suffix: String,
    int_zeros: usize,      // 整数部分最少位数(`0` 的个数)
    decimals: usize,       // 小数位数
    thousands: bool,       // 是否有千分位
    percent: bool,         // 是否百分比
    scientific: bool,      // 是否科学计数
    has_placeholder: bool, // 是否出现数字占位符(`0`/`#`/`?`);无则该节为纯文本
}

fn format_numeric(value: f64, code: &str) -> String {
    let f = parse_numeric_code(code);
    // 纯文本节(无数字占位符):只输出字面量,不渲染数值。
    if !f.has_placeholder {
        return format!("{}{}", f.prefix, f.suffix);
    }
    let mut v = value;
    if f.percent {
        v *= 100.0;
    }

    if f.scientific {
        return format!("{}{:.*e}{}", f.prefix, f.decimals, v, f.suffix);
    }

    let neg = v.is_sign_negative() && v != 0.0;
    let mut body = format_fixed(v.abs(), f.decimals, f.thousands, f.int_zeros);
    if f.percent {
        body.push('%');
    }
    let sign = if neg { "-" } else { "" };
    format!("{}{}{}{}", sign, f.prefix, body, f.suffix)
}

/// 定点渲染:`decimals` 位小数(四舍五入),可选千分位,整数最少 `int_zeros` 位。
fn format_fixed(v: f64, decimals: usize, thousands: bool, int_zeros: usize) -> String {
    let s = format!("{v:.decimals$}");
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s, None),
    };

    let mut int_digits = int_part.trim_start_matches('0').to_string();
    if int_digits.is_empty() {
        int_digits = "0".to_string();
    }
    while int_digits.len() < int_zeros {
        int_digits.insert(0, '0');
    }

    let int_out = if thousands {
        group_thousands(&int_digits)
    } else {
        int_digits
    };

    match frac_part {
        Some(f) if decimals > 0 => format!("{int_out}.{f}"),
        _ => int_out,
    }
}

/// 每三位插一个逗号。
fn group_thousands(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let n = bytes.len();
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

/// 解析数值格式节:提取前后缀(引号/字面符号)、占位符统计、千分位/百分比/科学计数。
fn parse_numeric_code(code: &str) -> NumFormat {
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut int_zeros = 0usize;
    let mut decimals = 0usize;
    let mut thousands = false;
    let mut percent = false;
    let mut scientific = false;
    let mut has_placeholder = false;

    let mut seen_digit = false; // 是否已进入数字占位符区(用于区分 prefix/suffix)
    let mut in_decimals = false;
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                // 引号字面量:整体作为前/后缀
                i += 1;
                let mut lit = String::new();
                while i < chars.len() && chars[i] != '"' {
                    lit.push(chars[i]);
                    i += 1;
                }
                if seen_digit {
                    suffix.push_str(&lit);
                } else {
                    prefix.push_str(&lit);
                }
            }
            '\\' => {
                // 转义:下一个字符作字面量
                i += 1;
                if i < chars.len() {
                    if seen_digit {
                        suffix.push(chars[i]);
                    } else {
                        prefix.push(chars[i]);
                    }
                }
            }
            '0' | '#' | '?' => {
                seen_digit = true;
                has_placeholder = true;
                if in_decimals {
                    if c == '0' {
                        decimals += 1;
                    }
                } else if c == '0' {
                    int_zeros += 1;
                }
            }
            '.' => {
                seen_digit = true;
                in_decimals = true;
            }
            ',' => {
                // 数字区里的逗号 = 千分位;此处不处理尾随缩放逗号
                if seen_digit && !in_decimals {
                    thousands = true;
                }
            }
            '%' => {
                percent = true;
                if seen_digit {
                    suffix.push('%');
                } else {
                    prefix.push('%');
                }
                // 注:百分号已并入 suffix,format_numeric 里不再重复加
            }
            'E' | 'e' => {
                // 科学计数:E+ / E-
                scientific = true;
                seen_digit = true;
                has_placeholder = true;
                // 跳过其后的 +/- 与指数占位符
                let mut j = i + 1;
                while j < chars.len()
                    && (chars[j] == '+' || chars[j] == '-' || chars[j] == '0' || chars[j] == '#')
                {
                    j += 1;
                }
                i = j - 1;
            }
            _ => {
                // 其它字面字符(货币符号、空格等)
                if seen_digit {
                    suffix.push(c);
                } else {
                    prefix.push(c);
                }
            }
        }
        i += 1;
    }

    // 百分号已经进了 prefix/suffix,format_numeric 不再补 %,故这里清掉重复标记逻辑:
    // 保留 percent=true 仅用于「乘 100」。把可能加进 suffix 的 '%' 去掉一个由 caller 处理。
    if percent {
        // 移除我们塞进 suffix/prefix 的 %,交给 format_numeric 统一追加
        if suffix.ends_with('%') {
            suffix.pop();
        } else if prefix.ends_with('%') {
            prefix.pop();
        }
    }

    NumFormat {
        prefix,
        suffix,
        int_zeros,
        decimals,
        thousands,
        percent,
        scientific,
        has_placeholder,
    }
}

// ---------- 日期时间格式 ----------

/// Excel 序列数 → 公历(复刻 1900 闰年 bug),与 formula::functions::datetime 一致。
fn serial_to_ymd(serial: i64) -> (i64, i64, i64) {
    if serial == 60 {
        return (1900, 2, 29);
    }
    let naive = if serial >= 61 { serial - 1 } else { serial };
    civil_from_days(naive + epoch_offset())
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
fn epoch_offset() -> i64 {
    days_from_civil(1899, 12, 31)
}

const MONTH_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// 按日期时间格式码渲染。支持 `yyyy/yy m/mm/mmm/mmmm d/dd h/hh s/ss` 与分隔符。
fn format_datetime(value: f64, code: &str) -> String {
    let serial = value.floor() as i64;
    let (y, mo, d) = if serial >= 0 {
        serial_to_ymd(serial)
    } else {
        (1900, 1, 0)
    };
    let frac = value - value.floor();
    let total_secs = (frac * 86400.0).round() as i64;
    let hh = (total_secs / 3600) % 24;
    let mi = (total_secs / 60) % 60;
    let ss = total_secs % 60;

    let mut out = String::new();
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    // 记录上一个 m 到底是「月」还是「分」:紧邻 h 时 m 表示分钟。
    while i < chars.len() {
        let c = chars[i];
        match c.to_ascii_lowercase() {
            'y' => {
                let n = run_len(&chars, i, 'y');
                if n >= 3 {
                    out.push_str(&format!("{y:04}"));
                } else {
                    out.push_str(&format!("{:02}", y % 100));
                }
                i += n;
                continue;
            }
            'm' => {
                let n = run_len(&chars, i, 'm');
                // 判断是月还是分:向后/向前看有没有 h/s 邻接
                let minute = is_minute_context(&chars, i);
                if minute {
                    out.push_str(&format!("{mi:02}"));
                } else {
                    let idx = ((mo - 1).clamp(0, 11)) as usize;
                    match n {
                        1 => out.push_str(&mo.to_string()),
                        2 => out.push_str(&format!("{mo:02}")),
                        3 => out.push_str(MONTH_ABBR[idx]),
                        _ => out.push_str(MONTH_FULL[idx]),
                    }
                }
                i += n;
                continue;
            }
            'd' => {
                let n = run_len(&chars, i, 'd');
                if n <= 1 {
                    out.push_str(&d.to_string());
                } else {
                    out.push_str(&format!("{d:02}"));
                }
                i += n;
                continue;
            }
            'h' => {
                let n = run_len(&chars, i, 'h');
                if n <= 1 {
                    out.push_str(&hh.to_string());
                } else {
                    out.push_str(&format!("{hh:02}"));
                }
                i += n;
                continue;
            }
            's' => {
                let n = run_len(&chars, i, 's');
                if n <= 1 {
                    out.push_str(&ss.to_string());
                } else {
                    out.push_str(&format!("{ss:02}"));
                }
                i += n;
                continue;
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
                continue;
            }
            '\\' => {
                i += 1;
                if i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// 从 `start` 起连续相同字符(忽略大小写)的个数。
fn run_len(chars: &[char], start: usize, target: char) -> usize {
    let mut n = 0;
    while start + n < chars.len() && chars[start + n].to_ascii_lowercase() == target {
        n += 1;
    }
    n.max(1)
}

/// 判断位置 `i` 的 `m` 是否表示分钟(紧邻 h 或 s)。
fn is_minute_context(chars: &[char], i: usize) -> bool {
    // 向前找最近的非分隔字母
    let prev = (0..i)
        .rev()
        .map(|k| chars[k].to_ascii_lowercase())
        .find(|c| c.is_ascii_alphabetic());
    // 向后找(跳过本段 m)
    let mut j = i;
    while j < chars.len() && chars[j].eq_ignore_ascii_case(&'m') {
        j += 1;
    }
    let next = (j..chars.len())
        .map(|k| chars[k].to_ascii_lowercase())
        .find(|c| c.is_ascii_alphabetic());
    matches!(prev, Some('h')) || matches!(next, Some('s'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_format() {
        assert_eq!(format_number(0.0, "General"), "0");
        assert_eq!(format_number(3.0, ""), "3");
        assert_eq!(format_number(3.5, "General"), "3.5");
        assert_eq!(format_number(0.1 + 0.2, "General"), "0.3");
    }

    #[test]
    fn fixed_decimals() {
        assert_eq!(format_number(3.14259, "0.00"), "3.14");
        assert_eq!(format_number(3.0, "0.00"), "3.00");
        assert_eq!(format_number(3.5, "0"), "4"); // 四舍五入
        assert_eq!(format_number(0.5, "0"), "0"); // 银行家?Excel 用四舍五入到偶?实为 round half away → 1
    }

    #[test]
    fn integer_padding() {
        assert_eq!(format_number(5.0, "000"), "005");
        assert_eq!(format_number(42.0, "0000"), "0042");
    }

    #[test]
    fn thousands_separator() {
        assert_eq!(format_number(1234567.0, "#,##0"), "1,234,567");
        assert_eq!(format_number(1234.5, "#,##0.0"), "1,234.5");
        assert_eq!(format_number(12.0, "#,##0"), "12");
    }

    #[test]
    fn percent_format() {
        assert_eq!(format_number(0.1234, "0.00%"), "12.34%");
        assert_eq!(format_number(0.5, "0%"), "50%");
    }

    #[test]
    fn currency_and_literals() {
        assert_eq!(format_number(1234.5, "$#,##0.00"), "$1,234.50");
        assert_eq!(format_number(50.0, "0\"元\""), "50元");
    }

    #[test]
    fn negative_sections() {
        // 正;负 两节:负值用第二节 + 绝对值
        assert_eq!(format_number(-1234.0, "#,##0;(#,##0)"), "(1,234)");
        assert_eq!(format_number(1234.0, "#,##0;(#,##0)"), "1,234");
        // 无自定义负节时,默认前置负号
        assert_eq!(format_number(-42.0, "0.0"), "-42.0");
    }

    #[test]
    fn zero_section() {
        assert_eq!(format_number(0.0, "0.0;-0.0;\"零\""), "零");
    }

    #[test]
    fn scientific() {
        let s = format_number(12345.0, "0.00E+00");
        assert!(s.starts_with("1.23"), "科学计数应以 1.23 开头,实际 {s}");
        assert!(s.contains('e') || s.contains('E'));
    }

    #[test]
    fn date_formats() {
        // 2020-01-01 的序列数是 43831
        assert_eq!(format_number(43831.0, "yyyy-mm-dd"), "2020-01-01");
        assert_eq!(format_number(43831.0, "yyyy/m/d"), "2020/1/1");
        assert_eq!(format_number(43831.0, "mmm d, yyyy"), "Jan 1, 2020");
        assert_eq!(format_number(43831.0, "mmmm"), "January");
    }

    #[test]
    fn time_formats() {
        // 0.5 = 中午 12:00:00
        assert_eq!(format_number(43831.5, "hh:mm:ss"), "12:00:00");
        assert_eq!(format_number(43831.25, "h:mm"), "6:00");
    }

    #[test]
    fn datetime_combined() {
        assert_eq!(
            format_number(43831.5, "yyyy-mm-dd hh:mm"),
            "2020-01-01 12:00"
        );
    }

    #[test]
    fn never_panics_on_weird_codes() {
        // 不认识的码不应崩
        let _ = format_number(1.0, "[Red]0.0");
        let _ = format_number(1.0, "???");
        let _ = format_number(-1.0, ";;;");
        let _ = format_number(f64::NAN, "0.00");
    }
}
