/**
 * 前端统一日志。
 *
 * 与 Rust/WASM 侧(`crates/wasm/src/log.rs`)**格式一致**,便于把一次
 * 「打开文件 → 解析 → 首帧」在控制台里串起来看:
 *
 * ```text
 * [office-R][web ][info][a1b2c3] file.open name=demo.csv bytes=1048576
 * [office-R][wasm][info][a1b2c3] csv.parse.ok bytes=1048576 rows=20000 cols=8 ms=91.4
 * [office-R][web ][info][a1b2c3] sheet.firstFrame ms=12.7 rows=20000 cols=8
 * ```
 *
 * 三条约定:
 * 1. **绝不打印用户文件内容**——只输出文件名、字节数、行列数、耗时等元信息;
 * 2. 默认级别 `warn`(开发环境 `info`),不刷屏;
 * 3. 可通过 `?logLevel=debug` 或 `localStorage.officeR.logLevel` 调整。
 */

/** 日志级别,由细到粗。 */
export type LogLevel = "debug" | "info" | "warn" | "error" | "off";

const ORDER: Record<LogLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
  off: 4,
};

/** localStorage 中存放级别的键名。 */
export const LOG_LEVEL_STORAGE_KEY = "officeR.logLevel";

/** 结构化字段。值只能是标量 —— 这条限制本身就是「别把内容塞进日志」的护栏。 */
export type LogFields = Record<string, string | number | boolean | undefined>;

function isLevel(value: unknown): value is LogLevel {
  return typeof value === "string" && value in ORDER;
}

/**
 * 解析初始级别:URL 查询参数 > localStorage > 环境默认值。
 *
 * 放在函数里而不是模块顶层常量,是为了让测试可以重新求值。
 */
function resolveInitialLevel(): LogLevel {
  try {
    const fromQuery = new URLSearchParams(globalThis.location?.search ?? "").get("logLevel");
    if (isLevel(fromQuery)) return fromQuery;
  } catch {
    // 非浏览器环境(如 Worker / 测试)没有 location,忽略
  }
  try {
    const stored = globalThis.localStorage?.getItem(LOG_LEVEL_STORAGE_KEY);
    if (isLevel(stored)) return stored;
  } catch {
    // 隐私模式下访问 localStorage 会抛错,不能因此挂掉
  }
  return import.meta.env?.DEV ? "info" : "warn";
}

let currentLevel: LogLevel = resolveInitialLevel();

/** 级别变化的订阅者(WASM 侧靠它同步级别)。 */
const listeners = new Set<(level: LogLevel) => void>();

/** 当前级别。 */
export function getLogLevel(): LogLevel {
  return currentLevel;
}

/** 设置级别,并通知订阅者。 */
export function setLogLevel(level: LogLevel): void {
  currentLevel = level;
  for (const listener of listeners) listener(level);
}

/** 订阅级别变化;返回取消订阅的函数。 */
export function onLogLevelChange(listener: (level: LogLevel) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 该级别当前是否会被输出。热路径上可先判断,避免白白拼字符串。 */
export function isEnabled(level: Exclude<LogLevel, "off">): boolean {
  return ORDER[level] >= ORDER[currentLevel];
}

/** 把字段拼成 `k=v` 串;`undefined` 的字段自动省略。 */
function formatFields(fields?: LogFields): string {
  if (!fields) return "";
  let out = "";
  for (const key of Object.keys(fields)) {
    const value = fields[key];
    if (value === undefined) continue;
    out += `${out ? " " : ""}${key}=${value}`;
  }
  return out;
}

function emit(level: Exclude<LogLevel, "off">, traceId: string, event: string, fields?: LogFields) {
  if (!isEnabled(level)) return;
  const line = `[office-R][web ][${level}][${traceId}] ${event} ${formatFields(fields)}`.trimEnd();
  // eslint-disable-next-line no-console -- 这是日志封装本身,唯一允许直接用 console 的地方
  console[level === "debug" ? "debug" : level](line);
}

/**
 * 一个「追踪上下文」:同一次文件打开共用一个 traceId,
 * 让分散在前端与 WASM 的日志能对上号。
 */
export interface Tracer {
  readonly traceId: string;
  debug(event: string, fields?: LogFields): void;
  info(event: string, fields?: LogFields): void;
  warn(event: string, fields?: LogFields): void;
  error(event: string, fields?: LogFields): void;
}

/** 生成一个短随机 traceId(只用于日志关联,不需要密码学强度)。 */
export function newTraceId(): string {
  return Math.random().toString(36).slice(2, 8);
}

/** 创建一个 tracer。 */
export function createTracer(traceId: string = newTraceId()): Tracer {
  return {
    traceId,
    debug: (event, fields) => emit("debug", traceId, event, fields),
    info: (event, fields) => emit("info", traceId, event, fields),
    warn: (event, fields) => emit("warn", traceId, event, fields),
    error: (event, fields) => emit("error", traceId, event, fields),
  };
}

/** 不属于任何具体文件的全局日志(模块加载、WASM 初始化等)。 */
export const logger = createTracer("global");
