# Story-0006:查看 Word / Excel / PPT

- **关联**:RFC-0006、Spec-0006
- **状态**:已实现

## 用户故事

> 作为用户,我希望在同一个应用里**在线查看** Word 文档、Excel 表格(带冻结与格式化)、
> PPT 演示(带演示模式),不必装 Office 就能看清内容与排版。

## 验收场景

### Excel
- **When** 点「冻结首行/首列」**Then** 横竖滚动时表头行 / 首列保持可见,分隔线提示冻结边界。
- **When** 单元格是数字且带格式码 **Then** 按 Excel 规则显示(千分位/百分比/货币/日期)。

### Word
- **Given** 一份含标题、加粗/斜体、居中/右对齐、列表、图片、表格、图文混排的 .docx
- **When** 上传 **Then** canvas 按流式布局渲染以上全部;长文档滚动流畅(虚拟化)。

### PPT
- **Given** 一份多页 .pptx(文本框、图片、自选图形)
- **When** 上传 **Then** 主视图渲染当前幻灯,缩略图可切换;
- **When** 点「演示」**Then** 全屏播放,方向键 / 空格翻页,Esc 退出。

## 网站落地

三个标签页(文档 / 表格 / 演示)各自上传对应文件即时渲染。解析在 Rust/WASM,
渲染用 canvas(虚拟化 + 测量缓存)。可用 `cargo test ... write_browser_fixture --ignored`
生成样例 docx/pptx 后在 `pnpm -C web dev` 里查看。
