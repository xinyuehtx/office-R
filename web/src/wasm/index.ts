// office-wasm 的加载与类型封装 —— 按应用切分后的再导出层。
//
// wasm-pack 生成的产物位于 ./pkg(构建产物,不入库)。各加载器内部会 await ensureReady()。
//
// 切分的理由:这里原本是一个 382 行的文件,同时导出三个应用的加载器,并且**反向
// import 了具体应用的模型**(`imageKey` / `WordModel` / `Presentation`)。那让
// `ppt → wasm → ppt` 在包粒度上成环,也让任一应用都会把另两家的 glue 拉进 bundle。
// 现在每个应用的加载器各自成文件、各自单向依赖自己的模型。
export { ensureReady } from "./init";
export { nowSerial, parseCsv, sheetFromPacked, type PackedSheetTransfer } from "./sheet";
export { loadXlsx, type XlsxWorkbookHandle } from "./xlsx";
export { loadDocx } from "./word";
export { loadPptx } from "./ppt";
