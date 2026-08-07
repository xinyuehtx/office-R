//! WASM 侧的统一日志。
//!
//! 与前端 `web/src/apps/shared/logger.ts` **格式一致**,便于把一次文件打开
//! 在控制台里串起来看:
//!
//! ```text
//! [office-R][wasm][info][a1b2c3] csv.parse rows=200000 cols=12 ms=412.5
//! [office-R][web ][info][a1b2c3] sheet.firstFrame ms=18.2
//! ```
//!
//! 两条铁律:
//! 1. **绝不打印单元格内容**——只输出计数、耗时、编码名等统计量,避免泄露用户数据;
//! 2. 默认级别为 `warn`,不刷屏;需要排查时由前端调用 `setLogLevel("debug")` 打开。

use std::sync::atomic::{AtomicU8, Ordering};

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = debug)]
    fn console_debug(message: &str);
    #[wasm_bindgen(js_namespace = console, js_name = info)]
    fn console_info(message: &str);
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn console_warn(message: &str);
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}

/// 日志级别。数值越小越详细。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    /// 细节追踪。
    Debug = 0,
    /// 关键阶段。
    Info = 1,
    /// 可继续但需留意。
    Warn = 2,
    /// 失败。
    Error = 3,
    /// 全部关闭。
    Off = 4,
}

impl Level {
    fn from_name(name: &str) -> Option<Level> {
        match name {
            "debug" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" => Some(Level::Warn),
            "error" => Some(Level::Error),
            "off" | "silent" => Some(Level::Off),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
            Level::Off => "off",
        }
    }
}

/// 默认 `warn`:生产环境不刷屏。
static LEVEL: AtomicU8 = AtomicU8::new(Level::Warn as u8);

/// 设置 WASM 侧日志级别:`debug` / `info` / `warn` / `error` / `off`。
///
/// 由前端 logger 在初始化时同步过来,保证两侧级别一致。
#[wasm_bindgen(js_name = setLogLevel)]
pub fn set_log_level(level: &str) {
    if let Some(level) = Level::from_name(level) {
        LEVEL.store(level as u8, Ordering::Relaxed);
    }
}

/// 当前是否会输出该级别的日志。热路径上可先判断再拼接字符串。
pub fn enabled(level: Level) -> bool {
    level as u8 >= LEVEL.load(Ordering::Relaxed)
}

/// 输出一条日志。`event` 是点分事件名(如 `csv.parse`),`fields` 是 `k=v` 串。
pub fn log(level: Level, trace_id: &str, event: &str, fields: &str) {
    if !enabled(level) {
        return;
    }
    let line = format!(
        "[office-R][wasm][{}][{}] {} {}",
        level.label(),
        trace_id,
        event,
        fields
    );
    match level {
        Level::Debug => console_debug(&line),
        Level::Info => console_info(&line),
        Level::Warn => console_warn(&line),
        Level::Error => console_error(&line),
        Level::Off => {}
    }
}

/// 当前时间戳(毫秒)。
///
/// `std::time::Instant` 在 `wasm32-unknown-unknown` 上不可用,只能借道 JS。
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}
