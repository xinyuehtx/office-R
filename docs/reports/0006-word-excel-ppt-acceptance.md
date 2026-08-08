# 报告-0006:Word / Excel / PPT 只读渲染 验收

- **关联**:RFC-0006
- **日期**:2026-08-08
- **状态**:已实现并通过验收(单测 + 浏览器实测)

## 概述

本期按「参考热门开源、重 CPU 在 WASM、canvas 虚拟化 + 多级缓存」的要求,完成三大应用的
只读渲染。所有解析/格式化/过滤在 Rust/WASM,视图层只负责布局与绘制。

## 交付物与测试

| 模块 | 内容 | 测试 |
| --- | --- | --- |
| `core/numfmt.rs` | Excel 数字格式码渲染(小数/千分位/百分比/货币/日期时间/科学/分节) | 13 单测 |
| `core/filter.rs` | 列过滤(文本/数值/值集/空白,多列 AND)+ 唯一值 | 9 单测(上期) |
| `core/docx.rs` | docx 读路径 → 平面化文档模型 + 图片 | 5 单测(docx-rs 构造夹具) |
| `core/pptx.rs` | zip+quick-xml 解析 PresentationML → 幻灯模型 + 图片 | 6 单测(手组 pptx 夹具) |
| `wasm/word.rs` `wasm/ppt.rs` | 模型 serde + 图片字节跨边界 | 经浏览器 e2e 覆盖 |
| `web/shared/textMeasure.ts` | 共享测量缓存(参考 pretext) | 7 单测 |
| `web/excel/grid`(freeze) | 冻结行列几何 + 四象限绘制 | 冻结几何单测 |
| `web/word` | 文档模型 + wordLayout 流式布局 + WordPage 虚拟化 | wordLayout 8 单测 |
| `web/ppt` | 幻灯模型 + slideRender + PptPage 导航/演示 | slideRender 7 单测 |

**总计**:cargo **230** 绿、web **187** 绿;`cargo fmt`/`clippy -D warnings` 干净;
`pnpm typecheck`/`build` 通过。

## 功能验收(对应本期要求)

### Excel
- ✅ **数字格式化**:`#,##0.00`、`0.00%`、`$#,##0.00`、`yyyy-mm-dd hh:mm`、分节负数括号等(单测)。
- ✅ **冻结行列**:冻结首行/首列/到选区/取消;冻结区不随对应轴滚动;分隔线可见;
  命中/选中冻结感知。**浏览器实测**:公式示例「已冻结 1 行/1 列」,滚动时表头/首列保持。

### Word(浏览器实测样例 docx)
- ✅ 标题(Heading1/2 大号加粗)、正文
- ✅ 加粗 / 斜体 / 混排、字号、颜色
- ✅ 段落对齐:左 / 居中 / 右(justify≈左)
- ✅ 列表(项目符号 + 缩进)
- ✅ 图片、**图文混排**(文字 + 右侧图片同行)
- ✅ 表格(等宽列 + 边框)
- ✅ 长文档纵向虚拟化(sticky canvas + spacer,滚动重绘切片)

### PPT(浏览器实测样例 pptx,2 页)
- ✅ 文本框(标题 + 正文,字号/加粗/颜色)
- ✅ 图片(embed → 字节 → drawImage)
- ✅ 自选图形(矩形/椭圆 + 填充色)
- ✅ 对齐(居中标题)
- ✅ 缩略图导航 + 前后翻页
- ✅ **演示模式**:全屏、方向键/空格翻页、Esc 退出

## 性能与架构要点

- **重 CPU 全在 WASM**:docx/pptx/csv 解析、数字格式化、列过滤、公式求值。
- **canvas 虚拟化**:Excel 瓦片 + GPU 平移(未冻结)/ 四象限(冻结);Word 纵向虚拟化;
  PPT 按需绘当前幻灯。
- **多级缓存**:表格窗口缓存;共享文本测量 `font→segment` 两级缓存(三页共用,字体加载失效);
  文本裁剪缓存。
- **跨边界**:模型 serde;图片字节 → Blob object URL(不走 base64)。

## 已知边界(见 RFC-0006「非目标」)

- Word:列表有序/无序未查 numbering.xml;justify 近似;无分栏/页眉页脚/修订。
- PPT:无母版/版式继承(占位符无 xfrm 则跳过)、无主题 schemeClr、动画/切换、图表/SmartArt。
- e2e:以「Rust 构造夹具 + Playwright 浏览器实测」验证在线渲染;自动化 e2e 框架(@playwright/test)为后续。
- wasm 体积:docx-rs 默认带 image 解码特性,后续可 `default-features=false` 瘦身。

## 复现

```bash
# 生成浏览器验证用夹具
cargo test -p office-core write_browser_fixture -- --ignored   # docx + pptx → /tmp/office-r-sample.{docx,pptx}
pnpm -C web dev                                                 # 上传夹具查看渲染
```
