//! 内置函数库与**注册表**。
//!
//! 注册表是「函数名(大写)→ 实现」的哈希表,用 [`OnceLock`] 在首次查询时构建一次并缓存。
//! 新增函数只需在对应类别文件里写实现、在其 `register` 里插一行 —— **不改求值器**,
//! 这是「对齐 Excel 数百个函数」能持续推进的关键:补齐是机械式的。

mod datetime;
mod financial;
mod info;
mod logical;
mod lookup;
mod math;
mod stats;
mod text;
mod util;

pub use util::FuncImpl;

use std::collections::HashMap;
use std::sync::OnceLock;

fn registry() -> &'static HashMap<&'static str, FuncImpl> {
    static REGISTRY: OnceLock<HashMap<&'static str, FuncImpl>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut m = HashMap::new();
        math::register(&mut m);
        stats::register(&mut m);
        logical::register(&mut m);
        text::register(&mut m);
        datetime::register(&mut m);
        lookup::register(&mut m);
        info::register(&mut m);
        financial::register(&mut m);
        m
    })
}

/// 按函数名(不区分大小写,调用方通常已转大写)查实现。
pub fn lookup(name: &str) -> Option<FuncImpl> {
    registry().get(name.to_ascii_uppercase().as_str()).copied()
}

/// 已注册的函数总数(用于文档/自检)。
pub fn count() -> usize {
    registry().len()
}

/// 所有已注册的函数名(升序),便于展示「支持哪些函数」。
pub fn names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = registry().keys().copied().collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_expected_scale() {
        // 至少覆盖 120 个函数(对齐 Excel 主要类别的目标)
        assert!(count() >= 120, "已注册 {} 个函数,少于预期", count());
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert!(lookup("sum").is_some());
        assert!(lookup("SUM").is_some());
        assert!(lookup("Vlookup").is_some());
        assert!(lookup("不存在的函数").is_none());
    }

    #[test]
    fn core_functions_are_registered() {
        for name in [
            "SUM", "IF", "VLOOKUP", "DATE", "CONCAT", "AVERAGE", "PMT", "ISNUMBER",
        ] {
            assert!(lookup(name).is_some(), "缺少函数 {name}");
        }
    }
}
