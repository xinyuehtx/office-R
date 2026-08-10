//! `@tengxiaohyx/office-word`:Word (.docx) 只读查看 —— 模型 + wasm 加载器。
//
// 组件(WordPage / 布局)在 Phase 5 迁入。当前先立数据层:模型类型与 wasm 加载器。
export * from "./model";
export { loadDocx } from "./wasm";
