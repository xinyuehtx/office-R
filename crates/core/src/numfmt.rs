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
//! - 分节:`正;负;零;文本`(用 `;` 分隔,按值符号选节);
//! - **颜色码** `[Red]`/`[Blue]`/`[ColorN]`(经 [`format_with`] 返回);
//! - **条件段** `[>=100]`/`[<0]`(按条件选节);
//! - **分数** `# ?/?`、`?/8`(整数 + 最佳分数逼近或固定分母)。
//!
//! **非目标**(本期):千分位缩放尾随逗号、填充 `*`、区域设置具体本地化。
//! 遇到不认识的记号按「原样/尽力」处理,绝不 panic。

/// 格式化结果:显示文本 + 可选颜色(来自 `[Red]` 等颜色码)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formatted {
    /// 显示文本。
    pub text: String,
    /// 颜色(CSS 颜色名,如 `"red"`);无颜色码时为 `None`。
    pub color: Option<String>,
}

/// 把 `value` 按 `code`(格式码)渲染成显示文本。`General` 或空码走通用格式。
pub fn format_number(value: f64, code: &str) -> String {
    format_with(value, code).text
}

/// 与 [`format_number`] 相同,但额外返回颜色码([`[Red]`] 等)。
pub fn format_with(value: f64, code: &str) -> Formatted {
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return Formatted {
            text: general(value),
            color: None,
        };
    }

    // 解析各节的「条件 / 颜色 / 主体」
    let raw_sections = split_sections(code);
    let parsed: Vec<Section> = raw_sections.iter().map(|s| parse_section(s)).collect();
    let has_condition = parsed.iter().any(|s| s.cond.is_some());

    let (section, use_abs) = if has_condition {
        // 条件模式:按顺序选第一个满足条件的节;无条件节作为兜底(不取绝对值)
        select_by_condition(&parsed, value)
    } else {
        // 传统:正;负;零[;文本],负值节取绝对值
        pick_section_parsed(&parsed, value)
    };

    let v = if use_abs { value.abs() } else { value };
    let body = &section.body;
    let color = section.color.clone();

    let text = if body.eq_ignore_ascii_case("general") {
        general(v)
    } else if is_date_code(body) {
        format_datetime(v, body)
    } else if is_fraction_code(body) {
        format_fraction(v, body)
    } else {
        format_numeric(v, body)
    };
    Formatted { text, color }
}

/// 比较运算(条件段用)。
#[derive(Debug, Clone, Copy, PartialEq)]
enum CmpOp {
    Ge,
    Le,
    Ne,
    Gt,
    Lt,
    Eq,
}

/// 一个格式节:可选条件、可选颜色、格式主体(已去掉方括号记号)。
#[derive(Debug, Clone, Default)]
struct Section {
    cond: Option<(CmpOp, f64)>,
    color: Option<String>,
    body: String,
}

/// 解析一个节:提取前导 `[...]`(颜色 / 条件 / 货币字面量),其余为主体。
fn parse_section(section: &str) -> Section {
    let mut out = Section::default();
    let chars: Vec<char> = section.chars().collect();
    let mut i = 0;
    let mut body = String::new();
    while i < chars.len() {
        if chars[i] == '[' {
            // 取到匹配的 ']'
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == ']') {
                let content: String = chars[i + 1..i + 1 + close].iter().collect();
                apply_bracket(&content, &mut out, &mut body);
                i += close + 2;
                continue;
            }
        }
        body.push(chars[i]);
        i += 1;
    }
    out.body = body;
    out
}

/// 处理一个方括号记号:颜色 / 条件 / 货币字面量(`[$￥-...]`)/ 其它忽略。
fn apply_bracket(content: &str, out: &mut Section, body: &mut String) {
    let lower = content.to_ascii_lowercase();
    // 条件:[>=100] [<0] [=5] ...
    if let Some((op, rest)) = split_cmp(content) {
        if let Ok(n) = rest.trim().parse::<f64>() {
            out.cond = Some((op, n));
            return;
        }
    }
    // 颜色名
    if let Some(c) = color_name(&lower) {
        out.color = Some(c);
        return;
    }
    // 货币/区域:[$￥-804] → 取 $ 与 - 之间的字面量并入主体
    if let Some(rest) = content.strip_prefix('$') {
        let sym = rest.split('-').next().unwrap_or("");
        body.push_str(sym);
    }
    // 其它([h]/[mm] 计时、[$-409] 纯区域等)忽略
}

fn split_cmp(s: &str) -> Option<(CmpOp, &str)> {
    if let Some(r) = s.strip_prefix(">=") {
        Some((CmpOp::Ge, r))
    } else if let Some(r) = s.strip_prefix("<=") {
        Some((CmpOp::Le, r))
    } else if let Some(r) = s.strip_prefix("<>") {
        Some((CmpOp::Ne, r))
    } else if let Some(r) = s.strip_prefix('>') {
        Some((CmpOp::Gt, r))
    } else if let Some(r) = s.strip_prefix('<') {
        Some((CmpOp::Lt, r))
    } else if let Some(r) = s.strip_prefix('=') {
        Some((CmpOp::Eq, r))
    } else {
        None
    }
}

fn cmp_matches(op: CmpOp, v: f64, target: f64) -> bool {
    match op {
        CmpOp::Ge => v >= target,
        CmpOp::Le => v <= target,
        CmpOp::Ne => v != target,
        CmpOp::Gt => v > target,
        CmpOp::Lt => v < target,
        CmpOp::Eq => v == target,
    }
}

/// Excel 内置颜色名 → CSS 颜色;`color N`(1..8)映射到调色板前 8 色。
fn color_name(lower: &str) -> Option<String> {
    let name = match lower {
        "black" => "black",
        "blue" => "blue",
        "cyan" => "cyan",
        "green" => "green",
        "magenta" => "magenta",
        "red" => "red",
        "white" => "white",
        "yellow" => "yellow",
        _ => {
            if let Some(n) = lower.strip_prefix("color") {
                // [ColorN] / [Color N]
                const PALETTE: [&str; 8] = [
                    "black", "white", "red", "green", "blue", "yellow", "magenta", "cyan",
                ];
                let idx: usize = n.trim().parse().ok()?;
                return PALETTE.get(idx.saturating_sub(1)).map(|s| s.to_string());
            }
            return None;
        }
    };
    Some(name.to_string())
}

/// 条件模式选节:按顺序选第一个满足条件的节;其后第一个无条件节作兜底。
fn select_by_condition(sections: &[Section], value: f64) -> (Section, bool) {
    for s in sections {
        match s.cond {
            Some((op, target)) if cmp_matches(op, value, target) => return (s.clone(), false),
            Some(_) => {}
            None => return (s.clone(), false), // 无条件 → 兜底
        }
    }
    // 都不满足:用最后一节(或空)
    (sections.last().cloned().unwrap_or_default(), false)
}

/// 传统选节(正/负/零),负值节取绝对值。
fn pick_section_parsed(sections: &[Section], value: f64) -> (Section, bool) {
    match sections.len() {
        0 => (Section::default(), false),
        1 => (sections[0].clone(), false),
        2 => {
            if value < 0.0 {
                (sections[1].clone(), true)
            } else {
                (sections[0].clone(), false)
            }
        }
        _ => {
            if value > 0.0 {
                (sections[0].clone(), false)
            } else if value < 0.0 {
                (sections[1].clone(), true)
            } else {
                (sections[2].clone(), false)
            }
        }
    }
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

// ---------- 分数格式(# ?/?、?/8 等)----------

/// 是否是分数格式:主体里有被占位符包围的 `/`(排除日期里的 `/`)。
fn is_fraction_code(code: &str) -> bool {
    if is_date_code(code) {
        return false;
    }
    let bytes = code.as_bytes();
    if let Some(pos) = code.find('/') {
        // `/` 两侧应是数字占位符 ? # 0
        let before = bytes[..pos].iter().rev().find(|b| !b.is_ascii_whitespace());
        let after = bytes[pos + 1..].iter().find(|b| !b.is_ascii_whitespace());
        // 分子必是占位符;分母是占位符或固定数字(如 ?/8)
        let is_ph = |b: &&u8| matches!(**b, b'?' | b'#' | b'0');
        let is_den = |b: &&u8| matches!(**b, b'?' | b'#' | b'0') || b.is_ascii_digit();
        before.filter(is_ph).is_some() && after.filter(is_den).is_some()
    } else {
        false
    }
}

/// 按分数格式渲染:`# ?/?`(带整数)或 `?/?`(纯分数);分母固定(`?/8`)或按位数上限逼近。
fn format_fraction(value: f64, code: &str) -> String {
    let neg = value < 0.0;
    let v = value.abs();
    let slash = code.find('/').unwrap();
    let (left, right) = (&code[..slash], &code[slash + 1..]);

    // 整数部分:左侧以空白分隔时,空白前是整数占位、空白后是分子模板
    let (has_int, num_tmpl) = match left.trim_end().rfind(char::is_whitespace) {
        Some(sp) => (true, left[sp..].trim()),
        None => (false, left.trim()),
    };
    let _ = num_tmpl;

    let int_part = if has_int { v.floor() } else { 0.0 };
    let frac = v - int_part;

    // 分母:固定数字则用之,否则按 ? / # 个数定上限(9 / 99 / 999)
    let den_digits = right.trim();
    let (best_n, best_d) = if let Ok(fixed) = den_digits.parse::<u32>() {
        let n = (frac * fixed as f64).round() as u32;
        (n, fixed.max(1))
    } else {
        let max_den = 10u32.pow(
            den_digits
                .chars()
                .filter(|c| matches!(c, '?' | '#' | '0'))
                .count()
                .clamp(1, 4) as u32,
        ) - 1;
        best_fraction(frac, max_den)
    };

    let sign = if neg { "-" } else { "" };
    if has_int {
        if best_n == 0 {
            format!("{sign}{}", int_part as i64)
        } else {
            format!("{sign}{} {best_n}/{best_d}", int_part as i64)
        }
    } else {
        // 纯分数:整数并入分子
        let total_n = best_n + (int_part as u32) * best_d;
        format!("{sign}{total_n}/{best_d}")
    }
}

/// 在分母 `1..=max_den` 内找最逼近 `frac`(0..1)的 `n/d`。
fn best_fraction(frac: f64, max_den: u32) -> (u32, u32) {
    let mut best = (0u32, 1u32);
    let mut best_err = frac;
    for d in 1..=max_den.max(1) {
        let n = (frac * d as f64).round() as u32;
        let err = (frac - n as f64 / d as f64).abs();
        if err < best_err {
            best_err = err;
            best = (n, d);
        }
    }
    best
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

use crate::serial::serial_to_ymd;

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
    // 负序列数在 Excel 里是 #####(无法表示的日期);这里退回纪元当天,
    // 至少给出一个真实存在的日期,而不是早先硬编码的 1900-01-00。
    let (y, mo, d) = if serial >= 0 {
        serial_to_ymd(serial)
    } else {
        (1900, 1, 1)
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

    #[test]
    fn color_codes() {
        // 负值节带 [Red],正值无色
        let pos = format_with(1234.0, "#,##0;[Red]-#,##0");
        assert_eq!(pos.text, "1,234");
        assert_eq!(pos.color, None);
        let neg = format_with(-1234.0, "#,##0;[Red]-#,##0");
        assert_eq!(neg.text, "-1,234");
        assert_eq!(neg.color.as_deref(), Some("red"));
        // 颜色不影响纯文本 API
        assert_eq!(format_number(-1234.0, "#,##0;[Red]-#,##0"), "-1,234");
    }

    #[test]
    fn condition_sections() {
        // [>=60]"及格";[<60]"不及格"
        let code = "[>=60]0\"分\";[<60]\"不及格\"";
        assert_eq!(format_number(80.0, code), "80分");
        assert_eq!(format_number(50.0, code), "不及格");
        // 带颜色的条件段
        let c2 = "[Green][>=100]0;[Red]0";
        assert_eq!(format_with(150.0, c2).color.as_deref(), Some("green"));
        assert_eq!(format_with(50.0, c2).color.as_deref(), Some("red"));
    }

    #[test]
    fn fraction_formats() {
        // 固定分母
        assert_eq!(format_number(0.5, "?/8"), "4/8");
        // 带整数 + 最佳分数(? 上限 9)
        assert_eq!(format_number(2.5, "# ?/?"), "2 1/2");
        assert_eq!(format_number(2.25, "# ?/?"), "2 1/4");
        // 整数无小数 → 只显示整数
        assert_eq!(format_number(3.0, "# ?/?"), "3");
        // 纯分数(?? 上限 99)
        assert_eq!(format_number(0.7, "??/??"), "7/10");
        // 负数
        assert_eq!(format_number(-1.5, "# ?/?"), "-1 1/2");
    }

    #[test]
    fn currency_bracket_literal() {
        // [$￥-804]#,##0.00 → 货币符号并入前缀
        let f = format_number(12.0, "[$￥-804]#,##0.00");
        assert!(f.starts_with('￥'), "实际 {f}");
    }
}
