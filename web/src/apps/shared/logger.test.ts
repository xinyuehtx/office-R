import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createTracer, getLogLevel, isEnabled, newTraceId, onLogLevelChange, setLogLevel } from "./logger";

describe("logger", () => {
  const original = getLogLevel();
  let debug: ReturnType<typeof vi.spyOn>;
  let info: ReturnType<typeof vi.spyOn>;
  let warn: ReturnType<typeof vi.spyOn>;
  let error: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    debug = vi.spyOn(console, "debug").mockImplementation(() => {});
    info = vi.spyOn(console, "info").mockImplementation(() => {});
    warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    error = vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
    setLogLevel(original);
  });

  it("按级别过滤:warn 级别下不输出 info/debug", () => {
    setLogLevel("warn");
    const tracer = createTracer("t1");
    tracer.debug("a");
    tracer.info("b");
    tracer.warn("c");
    tracer.error("d");

    expect(debug).not.toHaveBeenCalled();
    expect(info).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledOnce();
    expect(error).toHaveBeenCalledOnce();
  });

  it("off 级别下什么都不输出", () => {
    setLogLevel("off");
    const tracer = createTracer("t2");
    tracer.error("boom");
    expect(error).not.toHaveBeenCalled();
  });

  it("输出格式带上模块、级别与 traceId,便于与 WASM 侧串联", () => {
    setLogLevel("debug");
    createTracer("abc123").info("csv.parse.ok", { rows: 10, cols: 3 });

    expect(info).toHaveBeenCalledWith("[office-R][web ][info][abc123] csv.parse.ok rows=10 cols=3");
  });

  it("undefined 字段被省略,不会打出 k=undefined", () => {
    setLogLevel("debug");
    createTracer("t3").info("evt", { a: 1, b: undefined });
    expect(info).toHaveBeenCalledWith("[office-R][web ][info][t3] evt a=1");
  });

  it("没有字段时不留多余空格", () => {
    setLogLevel("debug");
    createTracer("t4").info("evt");
    expect(info).toHaveBeenCalledWith("[office-R][web ][info][t4] evt");
  });

  it("isEnabled 与当前级别一致", () => {
    setLogLevel("info");
    expect(isEnabled("debug")).toBe(false);
    expect(isEnabled("info")).toBe(true);
    expect(isEnabled("error")).toBe(true);
  });

  it("级别变化会通知订阅者(WASM 侧据此同步)", () => {
    const listener = vi.fn();
    const unsubscribe = onLogLevelChange(listener);
    setLogLevel("debug");
    expect(listener).toHaveBeenCalledWith("debug");

    unsubscribe();
    setLogLevel("error");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("traceId 短小且每次不同", () => {
    const a = newTraceId();
    const b = newTraceId();
    expect(a).toHaveLength(6);
    expect(a).not.toBe(b);
  });
});
