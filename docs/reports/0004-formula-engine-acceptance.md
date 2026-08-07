# 报告-0004:公式计算引擎 验收

- **关联**:RFC-0004、Spec-0004、Story-0004
- **日期**:2026-08-07
- **状态**:已实现并通过验收

## 概述

在 `crates/core` 新增平台无关的**公式计算引擎**,把表格里以 `=` 开头的单元格按 **Excel 语义**
求值。引擎是「词法 → 语法 → 求值」的解释器管线 + 独立的值/公式层 [`Workbook`],
并接入 WASM 与表格页:网格显示**计算值**,公式栏回显**原始公式**。

参考的开源实现(仅作**语义与函数目录**参考,引擎为 Rust 自研):
[HyperFormula](https://github.com/handsontable/hyperformula)(418 函数)、
[Univer](https://github.com/dream-num/univer);另以 Rust 生态的 IronCalc 佐证路线可行。

## 交付物

| 层 | 内容 |
| --- | --- |
| `crates/core/src/formula/value.rs` | `Value`/`ExcelError`、类型强制、General 数字格式化 |
| `.../reference.rs` | A1 引用、列字母↔下标、范围、绝对/相对 |
| `.../token.rs` `ast.rs` `parser.rs` | 词法器、AST、Pratt 优先级解析器 |
| `.../eval.rs` | 求值器 + 值层 `Workbook`(按需求值 / 记忆化 / 循环检测)+ **计算管线** |
| `.../graph.rs` | **依赖图**:前驱提取、范围包含判定、拓扑排序(Kahn) |
| `.../functions/` | 可扩展注册表 + 8 大类别 **140** 个函数 |
| `.../mod.rs` | 公共 API + `evaluate_sheet`(整表求值,供 WASM) |
| `crates/wasm/src/csv_sheet.rs` | `parseCsvPacked` 接入公式求值 + `formulas` 回传 |
| `web/`(excel 页 / wasm 封装 / worker) | 公式栏、公式数元信息、「加载公式示例」、`SheetHandle.formula()` |
| `docs/` | RFC/Spec/Story/本报告 + architecture.md / AGENTS.md 同步 |

## 引擎能力(对齐 Excel)

- **运算符优先级**:`:` > 一元 `-` > `%` > `^`(右结合)> `* /` > `+ -` > `&` > 比较。
  验证:`-2^2=4`、`2^3^2=512`、`2*3%=0.06`。
- **类型强制**:`"3"+2=5`、`TRUE+1=2`、`""&5="5"`、`1+"abc"=#VALUE!`。
- **错误一等值 + 传播**:`1+#DIV/0!=#DIV/0!`;域错误 `SQRT(-1)=#NUM!`、`1/0=#DIV/0!`、
  未知函数 `FOO()=#NAME?`。
- **引用/范围**:`A1`、`$A$1`、`A1:B3`;列 `A..XFD`(0..16383)。
- **循环引用**:参与环的单元格得 `#REF!`,**不 panic、不死循环**;重复引用记忆化只算一次。

### 已实现函数(140)

- **数学/三角(43)**:ABS ACOS ASIN ATAN ATAN2 CEILING COMBIN COS DEGREES EVEN EXP FACT
  FLOOR GCD INT LCM LN LOG LOG10 MOD MROUND ODD PI POWER PRODUCT QUOTIENT RADIANS RAND
  RANDBETWEEN ROUND ROUNDDOWN ROUNDUP SIGN SIN SQRT SUM SUMIF SUMIFS SUMPRODUCT SUMSQ TAN TRUNC
- **统计(19)**:AVERAGE AVERAGEA AVERAGEIF COUNT COUNTA COUNTBLANK COUNTIF COUNTIFS LARGE
  MAX MEDIAN MIN MODE RANK SMALL STDEV STDEVP VAR VARP
- **逻辑(11)**:AND FALSE IF IFERROR IFNA IFS NOT OR SWITCH TRUE XOR
- **文本(23)**:CHAR CODE CONCAT CONCATENATE EXACT FIND LEFT LEN LOWER MID PROPER REPLACE
  REPT RIGHT SEARCH SUBSTITUTE T TEXTJOIN TRIM UNICHAR UNICODE UPPER VALUE
- **日期时间(15)**:DATE DATEVALUE DAY DAYS EDATE EOMONTH HOUR MINUTE MONTH NOW SECOND
  TIME TODAY WEEKDAY YEAR
- **查找引用(10)**:CHOOSE COLUMN COLUMNS HLOOKUP INDEX LOOKUP MATCH ROW ROWS VLOOKUP
- **信息(15)**:ERROR.TYPE ISBLANK ISERR ISERROR ISEVEN ISLOGICAL ISNA ISNONTEXT ISNUMBER
  ISODD ISREF ISTEXT N NA TYPE
- **财务(5)**:FV NPER NPV PMT PV

## 计算管线(依赖图 / 脏区 / 增量重算 / 循环更新)

在求值器之上实现真正的电子表格重算,回应「计算合并 / 脏区 / 依赖路径分析 / 更新与循环更新策略」:

| 能力 | 实现 | 验证 |
| --- | --- | --- |
| **依赖路径分析** | 从 AST 提取前驱(单元格+范围)+ 维护反向边;`precedents()`/`dependents()` 暴露 | ✅ `precedents(A3)={A1,A2}`、`dependents(A1)={A3}` |
| **脏区跟踪** | 编辑只更新受影响图边,把「该格+传递后继」标脏;`dirty_cells()` 可见 | ✅ 改 A1 → 脏区恰为 `{A1,A3,A4}`,A2 不脏 |
| **增量重算 + 计算合并** | 脏区子图 Kahn 拓扑排序,喂入干净值后按序求值,每格只算一次 | ✅ 改 A1 后 `evaluated.len()==3`,顺序前驱在前 |
| **范围依赖** | 范围不展开成边,建列索引(宽范围回退)+ 脏区按包含判定 | ✅ 改范围内 A2 → `SUM(A1:A3)` 变脏并重算;多窄范围只脏对应列;宽范围回退仍正确 |
| **循环更新(默认)** | 拓扑识别环及下游 → `#REF!` | ✅ `A1=B1, B1=A1+1` → 两格 `#REF!` |
| **循环更新(迭代)** | `set_iterative` 开启迭代,Jacobi / Gauss–Seidel 可选,`epsilon`/`max_iter` 收敛 | ✅ `A1=B1, B1=A1/2+3` 两法都收敛到 6 |

一次性构建(WASM `evaluate_sheet`)也走管线:首次全表皆脏,一次拓扑重算完成;
`computed_value` 取结果,脏/未算过时回退惰性求值保证正确。

## 验收场景(对应 Story-0004)

| 场景 | 结果 |
| --- | --- |
| 基本算术与引用 `=SUM(A1:A3)*2` | ✅ 通过(单测 + 浏览器) |
| 条件与文本 `=IF(B2>=60,…)` | ✅ 通过 |
| 错误优雅呈现 `=1/0`→`#DIV/0!`、`=FOO()`→`#NAME?` | ✅ 通过,页面不崩 |
| 公式栏回显原始公式 | ✅ 通过(浏览器验证:选中 D2 显示 `=B2*C2`,格内显示 `14`) |
| 循环引用不卡死 | ✅ 通过(`#REF!`,无死循环) |

**浏览器端到端**:`pnpm -C web dev` 打开表格页 → 「加载公式示例」→ 网格显示计算值
(如 D2=`14`)、元信息「8 个已求值」、选中公式格公式栏显示 `ƒ =B2*C2`。用真实 WASM 验证。

## 质量门禁

- `cargo fmt --check` ✅ `cargo clippy --all-targets -D warnings` ✅ `cargo test --all` ✅(**194**,含依赖图/管线/索引/迭代)
- `pnpm -C web typecheck` ✅ `pnpm -C web test` ✅(156)`pnpm -C web build` ✅
- WASM 体积:公式引擎零新增依赖,只用标准库。

## 已知边界(非目标,见 RFC-0004)

- 动态数组溢出(spill)、跨工作表引用、具名区域。
- financial 的 IRR/RATE 迭代族、engineering/database/cube 类别。
- `TODAY`/`NOW` 的时区取前端注入值;数字格式化(`TEXT` 的格式码)未实现。
- 覆盖率相对 Excel 全集(~400+)约为常用子集;注册表设计使补齐为机械式新增。
