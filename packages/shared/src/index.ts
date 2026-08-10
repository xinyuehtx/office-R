//! `@tengxiaohyx/office-shared` 的公共出口。
//
// 三个应用(word / excel / ppt)与演示站共用的叶子:日志、文本测量、图表绘制、
// 上传控件、字体常量。全部零内部依赖 —— 这是它们能住在最底层共享包的原因。
export * from "./logger";
export * from "./textMeasure";
export * from "./chartDraw";
export * from "./fonts";
export { FileUpload } from "./FileUpload";
