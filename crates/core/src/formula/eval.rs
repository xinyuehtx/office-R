//! 求值器与**值层 [`Workbook`]**。
//!
//! # 值层
//!
//! [`Workbook`] 是一张**稀疏**网格,每个单元格要么是字面量([`Value`]),
//! 要么是已解析的公式([`ast::Node`])。它是 [`crate::sheet::Sheet`] 之上独立的
//! 「值/公式层」—— 符合 architecture.md 里「新增值层而非污染只读表格模型」的方向。
//!
//! # 求值策略:按需 + 记忆化 + 循环检测
//!
//! 不预先做全表拓扑排序,而是**按需递归**求值:算一个单元格时,遇到引用就递归算
//! 被引用的单元格。用一个 `cache` 记忆化(同一格只算一次),用一个 `visiting` 集合
//! 检测循环(递归回到正在计算的格 → 判定循环 → 返回 `#REF!`,既不 panic 也不死循环)。
//!
//! # 错误传播
//!
//! 错误是一等值:任何子表达式算出 [`Value::Error`],都会通过类型强制(`?`)冒泡到顶层,
//! 与 Excel 一致。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::ast::{BinOp, Node, UnOp};
use super::functions;
use super::parser::parse;
use super::reference::{CellRef, RangeRef};
use super::value::{Array, ExcelError, Value};

/// 单元格内容:字面量或公式。
#[derive(Debug, Clone)]
enum Cell {
    /// 字面量输入(数值 / 文本 / 布尔)。
    Value(Value),
    /// 公式:保存原始文本(供公式栏回显)与已解析的 AST(`Rc` 便于求值时借出)。
    Formula { src: String, ast: Rc<Node> },
}

/// 值层:承载字面量与公式的稀疏网格。
#[derive(Debug, Default, Clone)]
pub struct Workbook {
    cells: HashMap<(u32, u32), Cell>,
    /// 注入的「当前时间」序列数,供 `TODAY`/`NOW` 使用(core 不依赖系统时钟,
    /// 保持平台无关;WASM 侧由 JS 传入 `Date.now()` 换算的序列数)。
    now_serial: f64,
    rows: u32,
    cols: u32,
}

impl Workbook {
    /// 新建空工作簿。
    pub fn new() -> Self {
        Workbook::default()
    }

    /// 设置「当前时间」序列数(见字段说明)。
    pub fn set_now(&mut self, serial: f64) {
        self.now_serial = serial;
    }

    /// 已登记内容的行数上界(最大行号 + 1)。
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// 已登记内容的列数上界(最大列号 + 1)。
    pub fn cols(&self) -> u32 {
        self.cols
    }

    fn bump_dims(&mut self, row: u32, col: u32) {
        self.rows = self.rows.max(row + 1);
        self.cols = self.cols.max(col + 1);
    }

    /// 按「用户输入」语义写入一个单元格(模拟 Excel 输入框):
    /// - 以 `=` 开头 → 公式;
    /// - 能解析为数字 → 数值;
    /// - `TRUE`/`FALSE`(忽略大小写)→ 布尔;
    /// - 空串 → 清空;
    /// - 其余 → 文本。
    ///
    /// 公式文本非法时,该格存成一个「求值即报错」的公式(返回 `#NAME?`),
    /// 而不是让写入失败 —— 与 Excel「先存下再显示错误」一致。
    pub fn set_input(&mut self, row: u32, col: u32, input: &str) {
        if input.is_empty() {
            self.cells.remove(&(row, col));
            return;
        }
        self.bump_dims(row, col);
        if let Some(body) = input.strip_prefix('=') {
            let ast = match parse(body) {
                Ok(node) => Rc::new(node),
                Err(e) => Rc::new(Node::Error(e)),
            };
            self.cells.insert(
                (row, col),
                Cell::Formula {
                    src: input.to_string(),
                    ast,
                },
            );
            return;
        }
        let value = interpret_literal(input);
        self.cells.insert((row, col), Cell::Value(value));
    }

    /// 写入一个已定型的字面量值(跳过输入解析)。
    pub fn set_value(&mut self, row: u32, col: u32, value: Value) {
        if value.is_blank() {
            self.cells.remove(&(row, col));
            return;
        }
        self.bump_dims(row, col);
        self.cells.insert((row, col), Cell::Value(value));
    }

    /// 若该格是公式,返回其原始文本(供公式栏回显);否则 `None`。
    pub fn formula_src(&self, row: u32, col: u32) -> Option<&str> {
        match self.cells.get(&(row, col)) {
            Some(Cell::Formula { src, .. }) => Some(src),
            _ => None,
        }
    }

    /// 求值单个单元格,返回其 [`Value`](空格返回 [`Value::Blank`])。
    ///
    /// 每次调用新建一个求值上下文(缓存/循环检测集),因此是「一次性」求值。
    /// 若要一次算整张表,用 [`Workbook::evaluate_all`] 复用同一份缓存更高效。
    pub fn eval_cell(&self, row: u32, col: u32) -> Value {
        let mut ev = Evaluator::new(self);
        ev.cell_value(row, col)
    }

    /// 求值一个独立公式(不写入任何单元格),`body` 不含前导 `=`。
    pub fn eval_formula(&self, body: &str) -> Value {
        let ast = match parse(body) {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        let mut ev = Evaluator::new(self);
        ev.eval(&ast)
    }

    /// 一次性求值所有已登记的单元格,复用同一份缓存。
    ///
    /// 返回 `(row, col) → Value` 映射,只含非空结果。
    pub fn evaluate_all(&self) -> HashMap<(u32, u32), Value> {
        let mut ev = Evaluator::new(self);
        let mut out = HashMap::with_capacity(self.cells.len());
        // 收集坐标后排序,保证求值顺序稳定(便于测试与复现)。
        let mut coords: Vec<(u32, u32)> = self.cells.keys().copied().collect();
        coords.sort_unstable();
        for (r, c) in coords {
            let v = ev.cell_value(r, c);
            if !v.is_blank() {
                out.insert((r, c), v);
            }
        }
        out
    }
}

/// 把非公式输入解析成字面量值(数字 / 布尔 / 文本)。
fn interpret_literal(input: &str) -> Value {
    match input.to_ascii_uppercase().as_str() {
        "TRUE" => return Value::Bool(true),
        "FALSE" => return Value::Bool(false),
        _ => {}
    }
    // 只有「整串就是一个数字」才算数值,避免把 "1,2" 之类误判。
    if let Ok(n) = input.trim().parse::<f64>() {
        // 拒绝 NaN / Inf 这类边界输入,按文本处理。
        if n.is_finite() {
            return Value::Number(n);
        }
    }
    Value::Text(input.to_string())
}

/// 求值上下文:借用 [`Workbook`],自带缓存与循环检测集。
///
/// 函数库拿到 `&mut Evaluator`,可用 [`Evaluator::eval`] 求值参数(从而实现短路),
/// 也可用 [`Evaluator::flatten_arg`] 遍历范围。
pub struct Evaluator<'a> {
    wb: &'a Workbook,
    cache: HashMap<(u32, u32), Value>,
    visiting: HashSet<(u32, u32)>,
    depth: usize,
    /// 正在求值的单元格,供 `ROW()`/`COLUMN()` 无参形式使用。
    current: Option<CellRef>,
}

/// 表达式/依赖递归的最大深度,超出即判 `#NUM!`,防止栈溢出。
const MAX_DEPTH: usize = 256;

impl<'a> Evaluator<'a> {
    fn new(wb: &'a Workbook) -> Self {
        Evaluator {
            wb,
            cache: HashMap::new(),
            visiting: HashSet::new(),
            depth: 0,
            current: None,
        }
    }

    /// 正在求值的单元格(供 `ROW()`/`COLUMN()` 无参形式)。
    pub fn current_cell(&self) -> Option<CellRef> {
        self.current
    }

    /// 注入的「当前时间」序列数。
    pub fn now_serial(&self) -> f64 {
        self.wb.now_serial
    }

    /// 求值一个单元格(带缓存与循环检测)。
    pub fn cell_value(&mut self, row: u32, col: u32) -> Value {
        if let Some(v) = self.cache.get(&(row, col)) {
            return v.clone();
        }
        let cell = match self.wb.cells.get(&(row, col)) {
            None => return Value::Blank,
            Some(c) => c,
        };
        let value = match cell {
            Cell::Value(v) => v.clone(),
            Cell::Formula { ast, .. } => {
                if self.visiting.contains(&(row, col)) {
                    // 递归回到正在计算的格 → 循环引用
                    return Value::Error(ExcelError::Ref);
                }
                let ast = Rc::clone(ast); // 脱离对 wb 的不可变借用,便于递归
                self.visiting.insert((row, col));
                let prev = self.current.replace(CellRef::new(row, col));
                let v = self.eval(&ast);
                self.current = prev;
                self.visiting.remove(&(row, col));
                v
            }
        };
        self.cache.insert((row, col), value.clone());
        value
    }

    /// 求值一个 AST 节点。
    pub fn eval(&mut self, node: &Node) -> Value {
        if self.depth >= MAX_DEPTH {
            return Value::Error(ExcelError::Num);
        }
        self.depth += 1;
        let v = self.eval_inner(node);
        self.depth -= 1;
        v
    }

    fn eval_inner(&mut self, node: &Node) -> Value {
        match node {
            Node::Number(n) => Value::Number(*n),
            Node::Text(s) => Value::Text(s.clone()),
            Node::Bool(b) => Value::Bool(*b),
            Node::Error(e) => Value::Error(*e),
            Node::Ref(cell) => self.cell_value(cell.row, cell.col),
            Node::Range(range) => Value::Array(self.range_to_array(*range)),
            Node::Unary(op, operand) => self.eval_unary(*op, operand),
            Node::Binary(op, l, r) => self.eval_binary(*op, l, r),
            Node::Func(name, args) => match functions::lookup(name) {
                Some(f) => f(self, args),
                None => Value::Error(ExcelError::Name),
            },
        }
    }

    fn eval_unary(&mut self, op: UnOp, operand: &Node) -> Value {
        let v = self.eval(operand);
        let n = match v.to_number() {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        match op {
            UnOp::Neg => Value::Number(-n),
            UnOp::Plus => Value::Number(n),
            UnOp::Percent => Value::Number(n / 100.0),
        }
    }

    fn eval_binary(&mut self, op: BinOp, l: &Node, r: &Node) -> Value {
        let lv = self.eval(l);
        let rv = self.eval(r);
        // 任一侧是错误 → 传播(左优先)
        if let Some(e) = lv.as_error() {
            return Value::Error(e);
        }
        if let Some(e) = rv.as_error() {
            return Value::Error(e);
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
                arithmetic(op, &lv, &rv)
            }
            BinOp::Concat => match (lv.to_text(), rv.to_text()) {
                (Ok(a), Ok(b)) => Value::Text(a + &b),
                (Err(e), _) | (_, Err(e)) => Value::Error(e),
            },
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                compare(op, &lv, &rv)
            }
        }
    }

    /// 把范围读成数组值(逐格求值)。
    fn range_to_array(&mut self, range: RangeRef) -> Array {
        let rows = range.rows() as usize;
        let cols = range.cols() as usize;
        let mut data = Vec::with_capacity(rows * cols);
        for r in range.row0..=range.row1 {
            for c in range.col0..=range.col1 {
                data.push(self.cell_value(r, c));
            }
        }
        Array::new(rows, cols, data)
    }

    // ---- 供函数库使用的辅助 ----

    /// 求值参数并强制为数值。
    pub fn eval_number(&mut self, node: &Node) -> Result<f64, ExcelError> {
        self.eval(node).to_number()
    }

    /// 求值参数并强制为文本。
    pub fn eval_text(&mut self, node: &Node) -> Result<String, ExcelError> {
        self.eval(node).to_text()
    }

    /// 求值参数并强制为逻辑值。
    pub fn eval_bool(&mut self, node: &Node) -> Result<bool, ExcelError> {
        self.eval(node).to_bool()
    }

    /// 把一个参数「摊平」成值序列:
    /// - 范围 → 逐格的值;
    /// - 数组 → 其元素;
    /// - 标量 → 单元素序列。
    ///
    /// 聚合函数据此遍历,再按各自规则(是否计入文本/布尔)取舍。
    pub fn flatten_arg(&mut self, node: &Node) -> Vec<Value> {
        match node {
            Node::Range(range) => {
                let mut out = Vec::with_capacity((range.rows() * range.cols()) as usize);
                for r in range.row0..=range.row1 {
                    for c in range.col0..=range.col1 {
                        out.push(self.cell_value(r, c));
                    }
                }
                out
            }
            other => match self.eval(other) {
                Value::Array(a) => a.data,
                v => vec![v],
            },
        }
    }

    /// 该参数是否是「引用型」(单元格或范围)。用于区分
    /// 「范围里的文本被忽略」与「字面量文本被强制」这类 Excel 差异。
    pub fn is_reference(node: &Node) -> bool {
        matches!(node, Node::Ref(_) | Node::Range(_))
    }

    /// 把参数解析成一个范围引用(单元格视作 1×1)。非引用返回 `None`。
    pub fn as_range(node: &Node) -> Option<RangeRef> {
        match node {
            Node::Ref(c) => Some(RangeRef::single(*c)),
            Node::Range(r) => Some(*r),
            _ => None,
        }
    }

    /// 把范围读成二维数组(供 VLOOKUP/INDEX/MATCH 等需要形状的函数)。
    pub fn array_from_range(&mut self, range: RangeRef) -> Array {
        self.range_to_array(range)
    }
}

/// 算术运算(四则 + 幂),两侧已确保非错误。
fn arithmetic(op: BinOp, lv: &Value, rv: &Value) -> Value {
    let a = match lv.to_number() {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let b = match rv.to_number() {
        Ok(n) => n,
        Err(e) => return Value::Error(e),
    };
    let r = match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => {
            if b == 0.0 {
                return Value::Error(ExcelError::Div0);
            }
            a / b
        }
        BinOp::Pow => {
            // 负底数配非整数指数在实数域无定义 → #NUM!
            if a < 0.0 && b.fract() != 0.0 {
                return Value::Error(ExcelError::Num);
            }
            a.powf(b)
        }
        _ => unreachable!("非算术运算不应进入 arithmetic"),
    };
    if r.is_nan() || r.is_infinite() {
        return Value::Error(ExcelError::Num);
    }
    Value::Number(r)
}

/// 比较运算,返回布尔值。两侧已确保非错误。
///
/// Excel 语义:数值按大小;文本**忽略大小写**按字典序;跨类型按
/// `数值 < 文本 < 布尔` 排序;空单元格按对方类型退化(与数值比作 0、与文本比作 "")。
fn compare(op: BinOp, lv: &Value, rv: &Value) -> Value {
    use std::cmp::Ordering;
    let ord = cmp_values(lv, rv);
    let result = match op {
        BinOp::Eq => ord == Ordering::Equal,
        BinOp::Ne => ord != Ordering::Equal,
        BinOp::Lt => ord == Ordering::Less,
        BinOp::Gt => ord == Ordering::Greater,
        BinOp::Le => ord != Ordering::Greater,
        BinOp::Ge => ord != Ordering::Less,
        _ => unreachable!("非比较运算不应进入 compare"),
    };
    Value::Bool(result)
}

/// 类型的排序等级:数值 < 文本 < 布尔。
fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        _ => 0,
    }
}

/// 比较两个值,产出全序(供比较运算符使用)。
pub fn cmp_values(lv: &Value, rv: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // 空单元格按对方类型退化
    let (l, r) = normalize_blanks(lv, rv);
    match (&l, &r) {
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Value::Text(a), Value::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => type_rank(&l).cmp(&type_rank(&r)),
    }
}

/// 把 [`Value::Blank`] 按对方类型转换成可比较的具体值。
fn normalize_blanks(lv: &Value, rv: &Value) -> (Value, Value) {
    let conv = |blank_side: &Value, other: &Value| -> Value {
        match other {
            Value::Number(_) => Value::Number(0.0),
            Value::Text(_) => Value::Text(String::new()),
            Value::Bool(_) => Value::Bool(false),
            _ => blank_side.clone(),
        }
    };
    match (lv.is_blank(), rv.is_blank()) {
        (true, true) => (Value::Number(0.0), Value::Number(0.0)),
        (true, false) => (conv(lv, rv), rv.clone()),
        (false, true) => (lv.clone(), conv(rv, lv)),
        (false, false) => (lv.clone(), rv.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 求值一个独立公式(自动补上前导逻辑),便于断言。
    fn ev(formula: &str) -> Value {
        let wb = Workbook::new();
        wb.eval_formula(formula.trim_start_matches('='))
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("1+2*3"), Value::Number(7.0));
        assert_eq!(ev("2^3^2"), Value::Number(512.0));
        assert_eq!(ev("-2^2"), Value::Number(4.0));
        assert_eq!(ev("2*3%"), Value::Number(0.06));
        assert_eq!(ev("(1+2)*3"), Value::Number(9.0));
    }

    #[test]
    fn coercion_rules() {
        assert_eq!(ev("\"3\"+2"), Value::Number(5.0));
        assert_eq!(ev("TRUE+1"), Value::Number(2.0));
        assert_eq!(ev("\"\"&5"), Value::Text("5".into()));
        assert_eq!(ev("1&2"), Value::Text("12".into()));
        assert_eq!(ev("1+\"abc\""), Value::Error(ExcelError::Value));
    }

    #[test]
    fn division_and_domain_errors() {
        assert_eq!(ev("1/0"), Value::Error(ExcelError::Div0));
        assert_eq!(ev("(-1)^0.5"), Value::Error(ExcelError::Num));
    }

    #[test]
    fn error_literals_propagate() {
        assert_eq!(ev("1+#DIV/0!"), Value::Error(ExcelError::Div0));
        assert_eq!(ev("#N/A&\"x\""), Value::Error(ExcelError::Na));
    }

    #[test]
    fn comparisons() {
        assert_eq!(ev("1<2"), Value::Bool(true));
        assert_eq!(ev("2<=2"), Value::Bool(true));
        assert_eq!(ev("3<>3"), Value::Bool(false));
        assert_eq!(
            ev("\"abc\"=\"ABC\""),
            Value::Bool(true),
            "文本比较忽略大小写"
        );
        assert_eq!(ev("\"a\"<\"b\""), Value::Bool(true));
    }

    #[test]
    fn unknown_function_is_name_error() {
        assert_eq!(ev("FOO()"), Value::Error(ExcelError::Name));
    }

    #[test]
    fn references_resolve_across_cells() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "1"); // A1
        wb.set_input(1, 0, "2"); // A2
        wb.set_input(2, 0, "=A1+A2"); // A3
        assert_eq!(wb.eval_cell(2, 0), Value::Number(3.0));
    }

    #[test]
    fn blank_cell_is_zero_in_arithmetic() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "=Z9+5"); // Z9 空
        assert_eq!(wb.eval_cell(0, 0), Value::Number(5.0));
    }

    #[test]
    fn circular_reference_yields_ref_error_without_hanging() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "=B1"); // A1 = B1
        wb.set_input(0, 1, "=A1"); // B1 = A1
        assert_eq!(wb.eval_cell(0, 0), Value::Error(ExcelError::Ref));
        assert_eq!(wb.eval_cell(0, 1), Value::Error(ExcelError::Ref));
    }

    #[test]
    fn chain_of_formulas_is_memoized() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "1");
        for r in 1..50 {
            wb.set_input(r, 0, &format!("=A{}+1", r)); // A(r+1) = A(r)+1
        }
        assert_eq!(wb.eval_cell(49, 0), Value::Number(50.0));
    }

    #[test]
    fn input_interpretation() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "42");
        wb.set_input(0, 1, "hello");
        wb.set_input(0, 2, "TRUE");
        assert_eq!(wb.eval_cell(0, 0), Value::Number(42.0));
        assert_eq!(wb.eval_cell(0, 1), Value::Text("hello".into()));
        assert_eq!(wb.eval_cell(0, 2), Value::Bool(true));
    }

    #[test]
    fn formula_src_is_kept_for_formula_bar() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "=1+2");
        assert_eq!(wb.formula_src(0, 0), Some("=1+2"));
        wb.set_input(0, 1, "plain");
        assert_eq!(wb.formula_src(0, 1), None);
    }
}
