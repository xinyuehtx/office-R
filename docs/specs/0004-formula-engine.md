# Spec-0004: 公式计算引擎

- **关联**:RFC-0004、Story-0004
- **状态**:已实现

精确定义公式引擎的输入/输出/边界与验收标准,是测试与实现的依据。

## 1. 值与错误

| 值类型 | 说明 |
| --- | --- |
| `Blank` | 空单元格。算术里视作 `0`,文本里视作 `""`。 |
| `Number(f64)` | 数值(日期/时间用序列数表示)。 |
| `Text(String)` | 文本。 |
| `Bool(bool)` | 逻辑值 `TRUE`/`FALSE`。 |
| `Error` | 错误值,见下。 |
| `Array` | 矩形数组(函数返回/范围展开)。 |

错误值(与 Excel 文本一致):`#NULL!`、`#DIV/0!`、`#VALUE!`、`#REF!`、`#NAME?`、`#NUM!`、`#N/A`。

**验收**:
- 错误沿链传播:`=1+#DIV/0!` → `#DIV/0!`。
- 类型强制:`="3"+2` → `5`;`=TRUE+1` → `2`;`=""&5` → `"5"`;`=1+"abc"` → `#VALUE!`。
- `1/0` → `#DIV/0!`;`SQRT(-1)` → `#NUM!`;未知函数 `=FOO()` → `#NAME?`;越界引用 → `#REF!`。

## 2. 引用

- 列字母 ↔ 下标:`A→0`、`Z→25`、`AA→26`、`XFD→16383`。
- 绝对/相对:`A1`、`$A1`、`A$1`、`$A$1` 解析等价(本期不做相对偏移重写,仅正确解析)。
- 范围:`A1:B3` 表示 2 列 × 3 行区域;`A:A`(整列)、`1:1`(整行)本期**不要求**。

**验收**:`col_to_index("AA")==26`;`CellRef::parse("$B$2")` 得到 (row=1,col=1,绝对)。

## 3. 运算符与优先级(从高到低)

1. `:`(范围) 2. 一元 `-`/`+` 3. `%`(后缀百分比) 4. `^`(幂,右结合)
5. `*` `/` 6. `+` `-` 7. `&`(文本连接) 8. 比较 `= <> < > <= >=`

**验收**:
- `=1+2*3` → `7`;`=2^3^2` → `512`(右结合);`=-2^2` → `4`(一元负号高于 `^`,与 Excel 一致);
- `=2*3%` → `0.06`;`=1<2` → `TRUE`;`="a"&"b"&"c"` → `"abc"`。

## 4. 求值语义

- `IF(cond, a, b)`:短路,只求值命中分支。
- 范围作聚合参数:`SUM(A1:A3)` 求和;文本/布尔按 Excel 规则(`SUM` 忽略文本与布尔,
  `COUNT` 只计数值,`COUNTA` 计非空)。
- **循环引用**:`A1=B1, B1=A1` → 参与环的单元格得循环错误(`#REF!`),不 panic、不死循环。
- 重复引用同一单元格只算一次(记忆化)。

## 5. 函数库(对齐 Excel,本期覆盖)

至少实现以下八大类别的常用函数(具体清单见实现与测试),**注册表可扩展**:

- **数学/三角**:SUM SUMIF SUMIFS SUMPRODUCT PRODUCT ABS SIGN INT TRUNC MOD ROUND ROUNDUP
  ROUNDDOWN CEILING FLOOR MROUND POWER SQRT EXP LN LOG LOG10 PI SIN COS TAN ASIN ACOS ATAN
  ATAN2 DEGREES RADIANS GCD LCM QUOTIENT EVEN ODD FACT COMBIN RAND RANDBETWEEN SUMSQ。
- **统计**:AVERAGE AVERAGEIF COUNT COUNTA COUNTBLANK COUNTIF COUNTIFS MAX MIN MEDIAN MODE
  STDEV STDEVP VAR VARP LARGE SMALL RANK。
- **逻辑**:IF IFS IFERROR IFNA AND OR NOT XOR TRUE FALSE SWITCH。
- **文本**:CONCAT CONCATENATE LEFT RIGHT MID LEN LOWER UPPER PROPER TRIM REPLACE SUBSTITUTE
  FIND SEARCH REPT EXACT TEXTJOIN VALUE CHAR CODE T。
- **日期时间**:DATE TIME TODAY NOW YEAR MONTH DAY HOUR MINUTE SECOND WEEKDAY EDATE EOMONTH
  DATEVALUE DAYS。
- **查找引用**:VLOOKUP HLOOKUP INDEX MATCH LOOKUP CHOOSE ROW COLUMN ROWS COLUMNS。
- **信息**:ISBLANK ISNUMBER ISTEXT ISLOGICAL ISERROR ISERR ISNA ISNONTEXT ISEVEN ISODD NA N。
- **财务**:PMT PV FV NPV NPER。

**验收**(示例):
- `=SUM(1,2,3)` → 6;`=SUM(A1:A3)`(1,2,3)→ 6;`=AVERAGE(A1:A3)` → 2。
- `=IF(1>2,"y","n")` → `"n"`;`=IFERROR(1/0,"err")` → `"err"`。
- `=LEFT("hello",2)` → `"he"`;`=CONCAT("a","b")` → `"ab"`;`=LEN("北京")` → 2。
- `=VLOOKUP(2,A1:B3,2,FALSE)` 精确匹配返回对应列。
- `=DATE(2020,1,1)` → 43831(Excel 序列数);`=YEAR(43831)` → 2020。
- `=ROUND(2.345,2)` → 2.35(四舍五入,半值远离零)。

## 6. 边界与非目标

- 非目标:动态数组溢出、跨表引用、具名区域、迭代计算开关、engineering/database/cube 类别。
- 输入超过嵌套/长度上限时返回错误而非 panic。
- 引擎**绝不 panic**:任何非法输入都映射为某个 `ExcelError`。
