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
use super::graph::{self, Cell, Precedents};
use super::parser::parse;
use super::reference::{CellRef, RangeRef};
use super::value::{Array, ExcelError, Value};

/// 单元格内容:字面量或公式。
#[derive(Debug, Clone)]
enum CellContent {
    /// 字面量输入(数值 / 文本 / 布尔)。
    Value(Value),
    /// 公式:保存原始文本(供公式栏回显)与已解析的 AST(`Rc` 便于求值时借出)。
    Formula { src: String, ast: Rc<Node> },
}

/// 迭代计算(循环引用)配置,对应 Excel 的「启用迭代计算」选项。
#[derive(Debug, Clone, Copy)]
struct Iterative {
    /// 是否允许对环做迭代求值(关闭时环内单元格得 `#REF!`)。
    enabled: bool,
    /// 最大迭代次数。
    max_iter: usize,
    /// 收敛阈值:相邻两次迭代所有环内单元格的最大数值变化小于它即停止。
    epsilon: f64,
}

impl Default for Iterative {
    fn default() -> Self {
        // 默认关闭,阈值与 Excel 默认一致(100 次 / 0.001)
        Iterative {
            enabled: false,
            max_iter: 100,
            epsilon: 1e-3,
        }
    }
}

/// 值层:承载字面量与公式的稀疏网格 + **计算管线**(依赖图 / 脏区 / 增量重算)。
#[derive(Debug, Default, Clone)]
pub struct Workbook {
    cells: HashMap<Cell, CellContent>,
    /// 注入的「当前时间」序列数,供 `TODAY`/`NOW` 使用(core 不依赖系统时钟,
    /// 保持平台无关;WASM 侧由 JS 传入 `Date.now()` 换算的序列数)。
    now_serial: f64,
    rows: u32,
    cols: u32,

    // ---- 计算管线 ----
    /// 每个公式单元格的前驱(它读取的单元格 / 范围)。
    precedents: HashMap<Cell, Precedents>,
    /// 反向边:单元格 → 直接引用它的公式单元格(依赖路径分析用)。
    dependents: HashMap<Cell, HashSet<Cell>>,
    /// 读取了范围的公式单元格集合(判断范围内某格变化影响谁时只扫这些)。
    range_readers: HashSet<Cell>,
    /// 脏区:值可能已过期、待重算的单元格。
    dirty: HashSet<Cell>,
    /// 计算值缓存(上一次重算的结果)。
    values: HashMap<Cell, Value>,
    /// 迭代计算配置。
    iterative: Iterative,
}

/// 一次增量重算的报告。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecalcReport {
    /// 本次按依赖顺序重算的单元格(前驱在前)。每个只算一次(计算合并)。
    pub evaluated: Vec<Cell>,
    /// 处于循环引用(或其下游)的单元格:关闭迭代计算时它们得 `#REF!`。
    pub circular: Vec<Cell>,
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
            self.set_content((row, col), None);
            return;
        }
        self.bump_dims(row, col);
        let content = if let Some(body) = input.strip_prefix('=') {
            let ast = match parse(body) {
                Ok(node) => Rc::new(node),
                Err(e) => Rc::new(Node::Error(e)),
            };
            CellContent::Formula {
                src: input.to_string(),
                ast,
            }
        } else {
            CellContent::Value(interpret_literal(input))
        };
        self.set_content((row, col), Some(content));
    }

    /// 写入一个已定型的字面量值(跳过输入解析)。
    pub fn set_value(&mut self, row: u32, col: u32, value: Value) {
        if value.is_blank() {
            self.set_content((row, col), None);
            return;
        }
        self.bump_dims(row, col);
        self.set_content((row, col), Some(CellContent::Value(value)));
    }

    /// 统一的单元格写入入口:更新内容 + 维护依赖图 + 传播脏区。
    ///
    /// `content == None` 表示清空该格。这是计算管线的关键钩子:每次编辑都在这里
    /// **只**更新受影响的图边,并把「该格 + 其所有(传递)后继」标记为脏,
    /// 供 [`Workbook::recalculate`] 增量重算。
    fn set_content(&mut self, cell: Cell, content: Option<CellContent>) {
        self.detach(cell); // 摘掉旧公式留下的边
        match content {
            Some(c) => {
                if let CellContent::Formula { ast, .. } = &c {
                    self.attach(cell, ast);
                }
                self.cells.insert(cell, c);
            }
            None => {
                self.cells.remove(&cell);
            }
        }
        self.mark_dirty(cell);
    }

    /// 摘除某格作为公式时建立的前驱边(改写 / 删除前调用)。
    fn detach(&mut self, cell: Cell) {
        if let Some(prev) = self.precedents.remove(&cell) {
            for p in prev.cells {
                if let Some(set) = self.dependents.get_mut(&p) {
                    set.remove(&cell);
                }
            }
        }
        self.range_readers.remove(&cell);
    }

    /// 依据新公式的 AST 建立前驱边(单元格入反向表,含范围者入 `range_readers`)。
    fn attach(&mut self, cell: Cell, ast: &Node) {
        let prec = graph::collect_precedents(ast);
        for p in &prec.cells {
            self.dependents.entry(*p).or_default().insert(cell);
        }
        if !prec.ranges.is_empty() {
            self.range_readers.insert(cell);
        }
        self.precedents.insert(cell, prec);
    }

    /// 把 `start` 及其所有(传递)后继标记为脏。
    ///
    /// 后继 = 直接引用它的公式(反向边)+ 范围覆盖它的公式(扫 `range_readers`)。
    fn mark_dirty(&mut self, start: Cell) {
        let mut stack = vec![start];
        while let Some(c) = stack.pop() {
            if !self.dirty.insert(c) {
                continue; // 已在脏区,避免重复展开
            }
            if let Some(deps) = self.dependents.get(&c) {
                stack.extend(deps.iter().copied());
            }
            for &f in &self.range_readers {
                if let Some(prec) = self.precedents.get(&f) {
                    if prec.ranges.iter().any(|r| graph::range_contains(r, c)) {
                        stack.push(f);
                    }
                }
            }
        }
    }

    /// 若该格是公式,返回其原始文本(供公式栏回显);否则 `None`。
    pub fn formula_src(&self, row: u32, col: u32) -> Option<&str> {
        match self.cells.get(&(row, col)) {
            Some(CellContent::Formula { src, .. }) => Some(src),
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

    // ==================== 计算管线 ====================

    /// 一个公式单元格的**前驱**(依赖路径分析:它读取哪些单元格 / 范围)。
    pub fn precedents(&self, row: u32, col: u32) -> Precedents {
        self.precedents
            .get(&(row, col))
            .cloned()
            .unwrap_or_default()
    }

    /// 直接依赖某格的公式单元格(**后继**;含范围覆盖它的公式),已排序。
    pub fn dependents(&self, row: u32, col: u32) -> Vec<Cell> {
        let cell = (row, col);
        let mut out: HashSet<Cell> = self.dependents.get(&cell).cloned().unwrap_or_default();
        for &f in &self.range_readers {
            if let Some(prec) = self.precedents.get(&f) {
                if prec.ranges.iter().any(|r| graph::range_contains(r, cell)) {
                    out.insert(f);
                }
            }
        }
        let mut v: Vec<Cell> = out.into_iter().collect();
        v.sort_unstable();
        v
    }

    /// 当前**脏区**(值可能过期、待重算的单元格),已排序。
    pub fn dirty_cells(&self) -> Vec<Cell> {
        let mut v: Vec<Cell> = self.dirty.iter().copied().collect();
        v.sort_unstable();
        v
    }

    /// 开启/配置**迭代计算**(循环引用)。关闭时环内单元格得 `#REF!`。
    pub fn set_iterative(&mut self, enabled: bool, max_iter: usize, epsilon: f64) {
        self.iterative = Iterative {
            enabled,
            max_iter: max_iter.max(1),
            epsilon: epsilon.max(0.0),
        };
    }

    /// 取某格「已重算的计算值」;脏或未算过时退回一次性惰性求值,保证始终正确。
    pub fn computed_value(&self, row: u32, col: u32) -> Value {
        let cell = (row, col);
        if !self.dirty.contains(&cell) {
            if let Some(v) = self.values.get(&cell) {
                return v.clone();
            }
        }
        self.eval_cell(row, col)
    }

    /// **增量重算**:只对脏区按依赖拓扑序重算,每格算一次(计算合并);
    /// 环内单元格按迭代策略处理。返回本次重算报告。
    pub fn recalculate(&mut self) -> RecalcReport {
        if self.dirty.is_empty() {
            return RecalcReport::default();
        }
        let dirty_set: HashSet<Cell> = self.dirty.iter().copied().collect();

        // 1) 依赖路径分析:对脏区子图拓扑排序,得到重算顺序与环集合
        let (order, circular) =
            graph::topo_order(&dirty_set, |c| self.precedents_within(c, &dirty_set));

        let iterative = self.iterative;
        let mut results: HashMap<Cell, Value> = HashMap::new();
        {
            // 2) 建求值器,喂入所有「干净」单元格的已知值,避免重算它们(增量的关键)
            let mut ev = Evaluator::new(self);
            for (cell, val) in &self.values {
                if !dirty_set.contains(cell) {
                    ev.prime(*cell, val.clone());
                }
            }
            // 3) 无环脏格:按拓扑序求值,记忆化保证每格只算一次(计算合并)
            for &cell in &order {
                let v = ev.cell_value(cell.0, cell.1);
                results.insert(cell, v);
            }
            // 4) 环(及其下游):默认 #REF!,或按迭代策略收敛
            if !circular.is_empty() {
                if iterative.enabled {
                    iterate_cycle(&mut ev, &circular, iterative, &mut results);
                } else {
                    for &cell in &circular {
                        results.insert(cell, Value::Error(ExcelError::Ref));
                    }
                }
            }
        }

        // 5) 提交结果、清空脏区
        for (cell, val) in results {
            self.values.insert(cell, val);
        }
        self.dirty.clear();
        RecalcReport {
            evaluated: order,
            circular,
        }
    }

    /// 计算某格在**给定集合内**的前驱(单元格前驱 ∩ 集合,再并入范围覆盖到的集合内单元格)。
    fn precedents_within(&self, cell: Cell, set: &HashSet<Cell>) -> Vec<Cell> {
        let Some(prec) = self.precedents.get(&cell) else {
            return Vec::new();
        };
        let mut out: Vec<Cell> = prec
            .cells
            .iter()
            .copied()
            .filter(|c| set.contains(c))
            .collect();
        if !prec.ranges.is_empty() {
            // 范围只在脏区这个小集合上展开,避免 A:A 这类大范围爆炸
            for &d in set {
                if prec.ranges.iter().any(|r| graph::range_contains(r, d)) {
                    out.push(d);
                }
            }
            out.sort_unstable();
            out.dedup();
        }
        out
    }
}

/// 取数值(仅 `Number`),用于迭代收敛判定。
fn num_of(v: &Value) -> Option<f64> {
    if let Value::Number(n) = v {
        Some(*n)
    } else {
        None
    }
}

/// 对环内单元格做 **Jacobi 迭代**:每轮用上一轮的估计值同步算出新值,
/// 直到所有数值变化都小于阈值,或达到最大轮数。非数值不参与收敛判定。
///
/// 估计值放进求值器缓存,故环内互相引用时直接命中缓存、不再递归 —— 这是让
/// 「循环更新」可控收敛(而非无限递归)的关键。
fn iterate_cycle(
    ev: &mut Evaluator,
    cyclic: &[Cell],
    cfg: Iterative,
    results: &mut HashMap<Cell, Value>,
) {
    // 初值 0:环内引用先读到 0,再逐轮逼近不动点
    for &c in cyclic {
        ev.prime(c, Value::Number(0.0));
        results.insert(c, Value::Number(0.0));
    }
    for _ in 0..cfg.max_iter {
        // 先用「上一轮」的缓存算出本轮全部新值(Jacobi:同步更新)
        let mut next: Vec<(Cell, Value)> = Vec::with_capacity(cyclic.len());
        let mut max_delta = 0.0f64;
        for &c in cyclic {
            let v = match ev.formula_ast(c) {
                Some(ast) => ev.eval(&ast),
                None => Value::Blank,
            };
            if let (Some(a), Some(b)) = (num_of(&v), results.get(&c).and_then(num_of)) {
                max_delta = max_delta.max((a - b).abs());
            }
            next.push((c, v));
        }
        // 应用本轮结果(下一轮读取)
        for (c, v) in next {
            ev.prime(c, v.clone());
            results.insert(c, v);
        }
        if max_delta < cfg.epsilon {
            break;
        }
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

    /// 预置一个单元格的已知值到缓存(供增量重算「喂入干净值 / 迭代估计值」)。
    /// 之后对该格的引用会直接命中缓存、不再递归求值。
    fn prime(&mut self, cell: Cell, value: Value) {
        self.cache.insert(cell, value);
    }

    /// 取某格公式的 AST(供迭代计算逐次求值);非公式返回 `None`。
    fn formula_ast(&self, cell: Cell) -> Option<Rc<Node>> {
        match self.wb.cells.get(&cell) {
            Some(CellContent::Formula { ast, .. }) => Some(Rc::clone(ast)),
            _ => None,
        }
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
            CellContent::Value(v) => v.clone(),
            CellContent::Formula { ast, .. } => {
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

    // ---- 计算管线 ----

    /// 造一张链式表:A1=1,A3=A1+A2,A4=A3*2,A2=2(独立)。
    fn pipeline_wb() -> Workbook {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "1"); // A1
        wb.set_input(1, 0, "2"); // A2
        wb.set_input(2, 0, "=A1+A2"); // A3
        wb.set_input(3, 0, "=A3*2"); // A4
        wb
    }

    #[test]
    fn precedents_and_dependents_expose_dependency_paths() {
        let wb = pipeline_wb();
        // A3 依赖 A1、A2
        assert_eq!(wb.precedents(2, 0).cells, vec![(0, 0), (1, 0)]);
        // A1 的后继是 A3;A3 的后继是 A4
        assert_eq!(wb.dependents(0, 0), vec![(2, 0)]);
        assert_eq!(wb.dependents(2, 0), vec![(3, 0)]);
        // A2 只被 A3 依赖
        assert_eq!(wb.dependents(1, 0), vec![(2, 0)]);
    }

    #[test]
    fn recalculate_orders_precedents_before_dependents() {
        let mut wb = pipeline_wb();
        let report = wb.recalculate();
        let pos = |c: Cell| report.evaluated.iter().position(|&x| x == c).unwrap();
        assert!(pos((0, 0)) < pos((2, 0)), "A1 应在 A3 之前");
        assert!(pos((2, 0)) < pos((3, 0)), "A3 应在 A4 之前");
        assert!(report.circular.is_empty());
        assert_eq!(wb.computed_value(2, 0), Value::Number(3.0));
        assert_eq!(wb.computed_value(3, 0), Value::Number(6.0));
    }

    #[test]
    fn edit_marks_only_affected_cells_dirty() {
        let mut wb = pipeline_wb();
        wb.recalculate();
        assert!(wb.dirty_cells().is_empty(), "重算后应无脏区");

        // 改 A1 → 脏区应恰好是 {A1, A3, A4}(A2 不受影响)
        wb.set_input(0, 0, "10");
        assert_eq!(wb.dirty_cells(), vec![(0, 0), (2, 0), (3, 0)]);

        let report = wb.recalculate();
        // 计算合并:本次只重算这三格,且各一次
        assert_eq!(report.evaluated.len(), 3);
        assert_eq!(wb.computed_value(3, 0), Value::Number(24.0)); // (10+2)*2
        assert!(wb.dirty_cells().is_empty());
    }

    #[test]
    fn range_edit_propagates_dirty_to_range_reader() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "1"); // A1
        wb.set_input(1, 0, "2"); // A2
        wb.set_input(2, 0, "3"); // A3
        wb.set_input(3, 0, "=SUM(A1:A3)"); // A4
        wb.recalculate();
        assert_eq!(wb.computed_value(3, 0), Value::Number(6.0));

        // 改范围内的 A2 → A4(范围读者)应变脏
        wb.set_input(1, 0, "20");
        assert!(wb.dirty_cells().contains(&(3, 0)));
        wb.recalculate();
        assert_eq!(wb.computed_value(3, 0), Value::Number(24.0));
    }

    #[test]
    fn circular_reference_defaults_to_ref_error() {
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "=B1"); // A1
        wb.set_input(0, 1, "=A1+1"); // B1
        let report = wb.recalculate();
        assert_eq!(report.circular, vec![(0, 0), (0, 1)]);
        assert_eq!(wb.computed_value(0, 0), Value::Error(ExcelError::Ref));
        assert_eq!(wb.computed_value(0, 1), Value::Error(ExcelError::Ref));
    }

    #[test]
    fn iterative_calculation_converges_on_cycle() {
        // A1 = B1;B1 = A1/2 + 3 → 不动点 A1 = B1 = 6
        let mut wb = Workbook::new();
        wb.set_input(0, 0, "=B1");
        wb.set_input(0, 1, "=A1/2+3");
        wb.set_iterative(true, 200, 1e-9);
        wb.recalculate();
        let a1 = wb.computed_value(0, 0).to_number().unwrap();
        let b1 = wb.computed_value(0, 1).to_number().unwrap();
        assert!((a1 - 6.0).abs() < 1e-3, "A1 应收敛到 6,实际 {a1}");
        assert!((b1 - 6.0).abs() < 1e-3, "B1 应收敛到 6,实际 {b1}");
    }

    #[test]
    fn recalculate_is_noop_when_clean() {
        let mut wb = pipeline_wb();
        wb.recalculate();
        let report = wb.recalculate();
        assert!(report.evaluated.is_empty() && report.circular.is_empty());
    }
}
