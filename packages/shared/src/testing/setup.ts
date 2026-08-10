import "@testing-library/jest-dom/vitest";
import { setLogLevel } from "../logger";

// 测试里默认关掉日志:断言看的是行为,不是控制台输出。
// 需要验证日志本身的用例(logger.test.ts)会自己临时打开级别。
setLogLevel("off");

// jsdom 没有实现 2D 上下文,调用 getContext 会打印一大段 "Not implemented"。
// 渲染器本来就能处理「拿不到上下文」的情况(退化为不绘制),
// 这里直接返回 null,让测试输出保持干净。
if (typeof HTMLCanvasElement !== "undefined") {
  HTMLCanvasElement.prototype.getContext = () => null;
}

// jsdom 的 Blob/File 未实现 arrayBuffer(),为上传逻辑补齐(仅测试环境)。
if (typeof Blob !== "undefined" && !Blob.prototype.arrayBuffer) {
  Blob.prototype.arrayBuffer = function arrayBuffer() {
    return new Promise<ArrayBuffer>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as ArrayBuffer);
      reader.onerror = () => reject(reader.error);
      reader.readAsArrayBuffer(this);
    });
  };
}
