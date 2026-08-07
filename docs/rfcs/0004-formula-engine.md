# RFC-0004: 表格公式计算引擎(Rust 侧)

- **状态**:已实现
- **作者**:office-R team
- **创建日期**:2026-08-07
- **关联**:Spec-0004、Story-0004、[RFC-0003](./0003-csv-canvas-grid.md)

## 动机(为什么)

表格页目前只能**展示**纯文本单元格([RFC-0003](./0003-csv-canvas-grid.md)),不会**计算**。
一个真正可用的电子表格,核心能力是公式:`=SUM(A1:A10)`、`=IF(B2>60,"及格","不及格")`。

本 RFC 引入一个**平台无关、跑在 Rust/WASM 侧**的公式计算引擎:

- **为什么在 Rust 侧**:公式解析(词法/语法)、依赖求值、数百个函数的实现都是
  纯 CPU 逻辑,与项目「重 CPU 一律在 Rust」的分层一致(见 AGENTS.md)。前端只负责
  编辑公式文本与展示计算结果。
- **为什么对齐 Excel**:用户心智模型是 Excel。函数名、错误值(`#DIV/0!`)、
  运算符优先级、类型强制规则都应与 Excel 一致,才能「所见即所得」。

## 参考的开源实现

调研了两个成熟的开源公式实现(均为 TypeScript,作为**语义与函数目录**的参考,
本项目用 Rust 重新实现):

- **HyperFormula**(handsontable):独立公式引擎,418 个内置函数,13 个类别。
- **Univer**(dream-num):完整在线表格套件,`engine-formula` 覆盖
  math / statistical / logical / text / date / lookup / financial / information /
  engineering / database 等类别。

另调研了 Rust 生态的 **IronCalc**(MIT/Apache),确认「Rust 实现 Excel 兼容引擎」路线可行。

## 方案(做什么、怎么做)

在 `crates/core` 新增 `formula` 模块,是一个经典的**解释器管线** + **值层(Workbook)**:

```
公式文本 "=SUM(A1:A3)*2"
   │  tokenize            token.rs   词法:数字/字符串/布尔/错误/引用/运算符/函数
   ▼
 [Token]
   │  parse (Pratt)       parser.rs  语法:Excel 运算符优先级 → AST
   ▼
  AST (ast.rs)
   │  evaluate            eval.rs    求值:错误传播 + 类型强制 + 范围展开
   ▼
 Value (value.rs)         Blank/Number/Text/Bool/Error/Array
```

- **值模型**(`value.rs`):`Value = Blank | Number(f64) | Text | Bool | Error | Array`。
  `ExcelError` 覆盖 7 种标准错误(`#NULL! #DIV/0! #VALUE! #REF! #NAME? #NUM! #N/A`)。
  错误是一等值,沿计算链传播(与 Excel 一致)。类型强制:算术里 `TRUE→1`、
  文本数字 `"3"→3`、空单元格 `→0`,不可强制则得 `#VALUE!`。
- **引用**(`reference.rs`):A1 记法,列字母 ↔ 下标,绝对/相对(`$A$1`),范围 `A1:B2`。
- **词法/语法**(`token.rs`/`parser.rs`):Pratt 解析器,精确实现 Excel 优先级
  (`:` > 一元负号 > `%` > `^` > `* /` > `+ -` > `&` > 比较)。
- **求值器 + 值层**(`eval.rs`):`Workbook` 承载「字面量单元格 + 公式单元格」的稀疏网格。
  求值器本身**按需求值 + 记忆化 + 递归循环检测**(检测到环 → `#REF!`,不 panic、不死循环),
  是所有表达式求值的执行体。范围参数按需展开为值序列供聚合函数消费。
- **计算管线**(`graph.rs` + `eval.rs`):在求值器之上做真正的电子表格重算,见下节。
- **函数库**(`functions/`):可扩展**注册表**(`名称(大写) → 实现`),按类别分文件
  (math/stats/logical/text/datetime/lookup/info/financial)。函数拿到的是**未求值的
  AST 参数** + 求值器句柄,因此 `IF` 能短路、聚合函数能遍历范围而不必物化。

### 计算管线(依赖图 / 脏区 / 增量重算 / 循环更新)

只有「求值器」还不是电子表格 —— 编辑一个格后不能每次都全表重算。管线补上这些:

- **依赖图 / 依赖路径分析**(`graph.rs`):从公式 AST 提取**前驱**(它读取的单元格 + 范围),
  同时维护**反向边**(后继)。范围**不展开成边**(`SUM(A1:A100000)` 只存一个 `RangeRef`),
  只在脏区这个小集合上按 `range_contains` 判定,避免大范围把图撑爆。
  暴露 `precedents(cell)` / `dependents(cell)` 供审查依赖路径。
- **脏区(dirty region)**:每次 `set_input`/`set_value` 只更新**受影响的图边**,并把
  「该格 + 其所有传递后继」标记为脏(`mark_dirty` 沿反向边 + 范围读者 BFS)。
  `dirty_cells()` 可见当前脏区。
- **增量重算 + 计算合并**(`recalculate`):对**脏区子图**做 Kahn 拓扑排序得到重算顺序
  (前驱在前),用一个喂入了「干净单元格已知值」的求值器按序求值 —— 干净格不重算、
  每个脏格**只算一次**(记忆化即计算合并)。返回 `RecalcReport { evaluated, circular }`。
- **循环更新策略**:拓扑排序识别出环(及其下游)。
  - **默认**:环内单元格得 `#REF!`(与直接惰性求值一致);
  - **迭代计算**(`set_iterative(true, max_iter, epsilon)`,对应 Excel「启用迭代计算」):
    对环做 **Jacobi 迭代**,每轮用上一轮估计值同步算新值,数值最大变化 < `epsilon`
    或达到 `max_iter` 即停。估计值放进求值器缓存,故环内互引直接命中缓存、不再递归。

一次性场景(如 WASM 从零构建整表)也走管线:首次全表皆脏,`recalculate` 一次拓扑重算即完成。
`computed_value(cell)` 取重算结果,脏/未算过时回退到惰性求值,保证始终正确。

### 与现有 `Sheet` 的关系

`Sheet`(RFC-0003)保持**只读纯文本**不变。公式引擎是它之上独立的**值层**,
符合 `crate::sheet` 与 architecture.md 里「扩展边界:新增值层而非污染表格模型」的既定方向。

对表格页的落地(演示路径):CSV 里**以 `=` 开头的单元格**被当作公式,针对整张网格
按 A1 引用求值,视图展示**计算结果**(选中单元格时公式栏显示原始公式)—— 与 Excel 的
「单元格显示值、公式栏显示式」一致。

## 取舍与备选方案

- **按需求值 + 缓存 vs. 预先拓扑排序全量重算**:MVP 选前者 —— 实现简单、天然只算用到的
  单元格、循环检测就是递归里的 visiting 集。全量拓扑重算是后续「大量公式全表重算」时的优化,
  接口已为其预留。
- **数组/溢出(spill)**:实现 `Value::Array` 供函数返回与 `SUMPRODUCT` 等消费,但**不做动态
  数组溢出**(现代 Excel 的 spill 语义),超出本期范围。
- **不引入第三方公式库**:HyperFormula/Univer 是 TS,IronCalc 是「整套表格模型」耦合较重;
  自研可精确贴合本项目的 `Sheet`/WASM 边界,且无额外体积负担。
- **函数覆盖**:本期实现引擎中核 + 八大类别约 **120+** 常用函数;注册表设计让补齐其余函数是
  **机械式**新增。完整对齐 418 个是持续工作,见「未决问题」。

## 影响

- `crates/core`:新增 `formula` 模块;`lib.rs` 导出公共 API。无破坏性改动。
- `crates/wasm`:新增公式求值绑定。
- `web/`:表格页支持公式(计算值展示 + 公式栏)。
- 依赖:仅需标准库;日期函数的「当前时间」由调用方注入(core 不依赖系统时钟,保持平台无关)。
- 文档:architecture.md / AGENTS.md 同步。

## 未决问题

- 函数覆盖率补齐到接近 Excel 全集(financial 的 IRR/RATE 迭代族、engineering、database 类别)。
- 跨工作表引用(`Sheet1!A1`)与具名区域(named ranges)。
- 动态数组溢出与 `@` 隐式交集。
- 范围依赖目前对「范围读者」做线性扫描判定;超大表可引入区间树(如 HyperFormula)进一步加速。
- 迭代计算目前是 Jacobi(整体同步);Gauss–Seidel(就地更新)通常收敛更快,可作为选项。
