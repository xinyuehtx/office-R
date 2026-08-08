# Spec-0006:Word / Excel / PPT 只读渲染

- **关联**:RFC-0006、报告-0006
- **状态**:已实现

## 1. Excel

### 数字格式化 `numfmt::format_number(value, code) -> String`
- `General`/空 → 通用格式(整数无小数点、去尾 0、吸收浮点噪声)。
- 占位符 `0`/`#`,小数位、千分位 `,`、百分比 `%`(×100)、前后缀(`$`、`"元"`)、
  科学计数 `E+`、日期时间 `y/m/d/h/s`、分节 `正;负;零[;文本]`。
- **验收**:`#,##0.00`→`1,234.50`;`0.00%`→`12.34%`;`yyyy-mm-dd`→`2020-01-01`;
  负节 `#,##0;(#,##0)`→`(1,234)`;非法码不 panic。

### 冻结行列
- `GridLayout` 增 `frozenRows/frozenCols/frozenWidth/frozenHeight`(夹到总行列内)。
- `cellScreenRect`/`hitTest`:冻结轴不减滚动。
- 冻结时四象限全量重绘 + 分隔线;未冻结走原瓦片路径。
- **验收**:冻结几何单测;浏览器实测滚动时冻结区固定。

## 2. Word `docx::parse(bytes) -> ParsedDoc`
- 模型:`Block = Paragraph | Table`;`Paragraph{heading, align, list, inlines}`;
  `Inline = Text(Run) | Image | Break`;`Run{text,bold,italic,underline,size_pt,color}`。
- 标题:pStyle `Heading1..6`/`Title`;对齐:jc → left/center/right/justify;
  列表:numPr → level;图片:Drawing/Pic → id + EMU 尺寸,字节从 docx-rs `images` 取。
- 渲染:流式布局带绝对 y;sticky canvas + spacer 纵向虚拟化;CJK 逐字折行。
- **验收**:docx 单测(docx-rs 构造夹具)+ wordLayout 单测 + 浏览器实测。

## 3. PPT `pptx::parse(bytes) -> ParsedPpt`
- 模型:`Presentation{width_px,height_px,slides}`;`Slide{shapes}`;
  `Shape{x,y,width,height,geom,fill,image,paragraphs}`。
- 解析:presentation.xml(sldSz EMU + sldIdLst 顺序)→ rels → slideN spTree;
  slide rels 解析图片字节。EMU÷9525;字号百分之一磅。元素名栈区分 spPr/rPr。
- 渲染:`fitScale` 等比铺入画布;形状几何/填充、文本折行+对齐、图片;
  缩略图导航 + 翻页 + 全屏演示(方向键/Esc)。
- **验收**:pptx 单测(手组夹具)+ slideRender 单测 + 浏览器实测(2 页 + 演示模式)。

## 4. 共享文本测量
- `font→segment` 两级缓存;OffscreenCanvas 优先;字体加载失效清缓存;
  `fit`(二分裁剪省略号)、`wrap`(折行)。
- **验收**:textMeasure 单测(测量/缓存/裁剪/折行/兜底)。

## 5. 非目标
见 RFC-0006「取舍与非目标」。
