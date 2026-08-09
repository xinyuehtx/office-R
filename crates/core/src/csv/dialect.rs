//! 分隔符(方言)嗅探。
//!
//! `.csv` 这个扩展名并不保证分隔符是逗号:中文 / 欧洲地区 Excel 导出的
//! “CSV” 常用分号,数据管道里也常见制表符与竖线。用户不该为此手动选择,
//! 所以这里做一次轻量嗅探;同时保留显式配置(见 [`super::CsvOptions::delimiter`])。
//!
//! 思路很朴素但足够稳:对每个候选分隔符,统计**样本行**中它在引号外出现的次数,
//! 「每行出现次数一致且大于 0」的候选最可能是真正的分隔符 —— 因为表格是规整的。

/// 候选分隔符 —— 定义在 [`crate::format`](识别入口),这里转发。
pub use crate::format::CANDIDATES;

/// 默认分隔符。
pub const DEFAULT_DELIMITER: u8 = b',';

/// 嗅探时最多参考的行数。表头之后几行足以判断,读全文没有额外收益。
const SAMPLE_LINES: usize = 20;

/// 分隔符的来源,用于向用户说明「为什么这么切」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelimiterSource {
    /// 调用方显式指定。
    Explicit,
    /// 由内容嗅探得出。
    Sniffed,
    /// 嗅探不出任何分隔符(如单列文件),退回默认逗号。
    Fallback,
}

/// 嗅探文本使用的分隔符。
///
/// 返回 `None` 表示样本中找不到任何候选分隔符(例如只有一列的文件),
/// 由调用方决定退回 [`DEFAULT_DELIMITER`]。
pub fn sniff(text: &str) -> Option<u8> {
    let lines = sample_lines(text);
    if lines.is_empty() {
        return None;
    }

    let mut best: Option<(u8, u32)> = None;
    for candidate in CANDIDATES {
        let counts: Vec<usize> = lines
            .iter()
            .map(|line| count_outside_quotes(line, candidate))
            .collect();
        let Some(score) = score(&counts) else {
            continue;
        };
        // 同分时保持 CANDIDATES 的优先级:只有严格更高才替换
        if best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((candidate, score));
        }
    }
    best.map(|(delimiter, _)| delimiter)
}

/// 给候选分隔符打分。返回 `None` 表示该候选完全没出现过。
///
/// 分数 = 每行平均出现次数,行间次数完全一致时翻倍 ——
/// 「规整」比「出现得多」更能说明它是分隔符。
fn score(counts: &[usize]) -> Option<u32> {
    let total: usize = counts.iter().sum();
    if total == 0 {
        return None;
    }
    let average = (total / counts.len()) as u32;
    if average == 0 {
        // 出现过但平均不足一次/行:大概率是正文里的标点,不是分隔符
        return Some(1);
    }
    let consistent = counts.windows(2).all(|w| w[0] == w[1]);
    Some(if consistent { average * 10 } else { average })
}

/// 取样本行:跳过空行,最多 [`SAMPLE_LINES`] 行。
///
/// 注意这里按物理行切分,字段内嵌换行会被切开;对**统计**用途无妨,
/// 真正的解析仍由 `csv` crate 完成。
fn sample_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .take(SAMPLE_LINES)
        .collect()
}

/// 统计某字节在双引号**之外**出现的次数。
///
/// 引号内的分隔符是普通字符(如 `"上海,中国"`),计入会让嗅探完全跑偏。
fn count_outside_quotes(line: &str, needle: u8) -> usize {
    let mut in_quotes = false;
    let mut count = 0;
    for byte in line.bytes() {
        match byte {
            // 连续两个引号(转义)会连续切换两次状态,等价于没切换,无需特判
            b'"' => in_quotes = !in_quotes,
            b if b == needle && !in_quotes => count += 1,
            _ => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_comma() {
        assert_eq!(sniff("a,b,c\n1,2,3\n"), Some(b','));
    }

    #[test]
    fn sniffs_semicolon_for_european_style() {
        assert_eq!(sniff("姓名;年龄;城市\n张三;18;北京\n"), Some(b';'));
    }

    #[test]
    fn sniffs_tab() {
        assert_eq!(sniff("a\tb\tc\n1\t2\t3\n"), Some(b'\t'));
    }

    #[test]
    fn sniffs_pipe() {
        assert_eq!(sniff("a|b|c\n1|2|3\n"), Some(b'|'));
    }

    #[test]
    fn prefers_regular_delimiter_over_punctuation_in_text() {
        // 分号只是正文里的标点,逗号才是分隔符
        let text = "id,note\n1,\"foo; bar\"\n2,baz; qux\n";
        assert_eq!(sniff(text), Some(b','));
    }

    #[test]
    fn ignores_delimiters_inside_quotes() {
        // 引号内有大量逗号,但真正的分隔符是分号
        let text = "a;b\n\"x,y,z,w\";1\n\"p,q,r,s\";2\n";
        assert_eq!(sniff(text), Some(b';'));
    }

    #[test]
    fn single_column_file_has_no_delimiter() {
        assert_eq!(sniff("hello\nworld\n"), None);
    }

    #[test]
    fn empty_text_has_no_delimiter() {
        assert_eq!(sniff(""), None);
        assert_eq!(sniff("\n\n\n"), None);
    }

    #[test]
    fn count_outside_quotes_handles_escaped_quotes() {
        // "" 是转义引号,不应让后续内容被误判为在引号内
        assert_eq!(count_outside_quotes(r#""a""b",c"#, b','), 1);
    }
}
