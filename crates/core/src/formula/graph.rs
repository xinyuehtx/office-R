//! 依赖图:前驱提取、范围包含判定、拓扑排序。
//!
//! 这是「计算管线」的图论部分,与求值解耦:
//! - **前驱提取**:从一个公式的 AST 收集它读取的单元格与范围([`collect_precedents`]);
//! - **拓扑排序**:对**脏区子图**做 Kahn 排序,得到重算顺序,并识别处于环中的单元格
//!   ([`topo_order`])。
//!
//! 为什么范围单独存:一个 `SUM(A1:A100000)` 若把范围展开成十万条边会爆炸。
//! 这里只存 [`RangeRef`],需要判断「某格是否影响该公式」时用 [`range_contains`] 即可,
//! 脏区传播与拓扑建边都只在**脏区**这个小集合上展开范围。

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use super::ast::Node;
use super::reference::RangeRef;

/// 单元格坐标(0 基行、列)。
pub type Cell = (u32, u32);

/// 一个公式的前驱:它直接读取的单元格与范围。
#[derive(Debug, Default, Clone)]
pub struct Precedents {
    /// 显式单元格引用(已去重、已排序)。
    pub cells: Vec<Cell>,
    /// 范围引用(不展开,按需用 [`range_contains`] 判定)。
    pub ranges: Vec<RangeRef>,
}

impl Precedents {
    /// 是否没有任何前驱(纯字面量表达式,如 `=1+2`)。
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.ranges.is_empty()
    }
}

/// 从公式 AST 收集全部前驱(单元格 + 范围)。
pub fn collect_precedents(node: &Node) -> Precedents {
    let mut p = Precedents::default();
    walk(node, &mut p);
    p.cells.sort_unstable();
    p.cells.dedup();
    p
}

fn walk(node: &Node, out: &mut Precedents) {
    match node {
        Node::Ref(c) => out.cells.push((c.row, c.col)),
        Node::Range(r) => out.ranges.push(*r),
        Node::Unary(_, x) => walk(x, out),
        Node::Binary(_, a, b) => {
            walk(a, out);
            walk(b, out);
        }
        Node::Func(_, args) => {
            for a in args {
                walk(a, out);
            }
        }
        Node::Number(_) | Node::Text(_) | Node::Bool(_) | Node::Error(_) => {}
    }
}

/// 单元格是否落在范围内。
pub fn range_contains(r: &RangeRef, cell: Cell) -> bool {
    cell.0 >= r.row0 && cell.0 <= r.row1 && cell.1 >= r.col0 && cell.1 <= r.col1
}

/// 对结点集合做 Kahn 拓扑排序,`precedents_in` 给出某结点在**集合内**的前驱。
///
/// 返回 `(order, cyclic)`:`order` 是可线性化的重算顺序(前驱在前);
/// `cyclic` 是剩下无法排序的结点 —— 它们处于环中,或**下游依赖了环**。
/// 出队用最小堆保证顺序确定(便于测试与复现)。
pub fn topo_order(
    nodes: &HashSet<Cell>,
    precedents_in: impl Fn(Cell) -> Vec<Cell>,
) -> (Vec<Cell>, Vec<Cell>) {
    let mut indeg: HashMap<Cell, usize> = nodes.iter().map(|&n| (n, 0usize)).collect();
    let mut succ: HashMap<Cell, Vec<Cell>> = HashMap::new();

    for &n in nodes {
        for p in precedents_in(n) {
            if nodes.contains(&p) {
                *indeg.get_mut(&n).expect("n 在集合内") += 1;
                succ.entry(p).or_default().push(n);
            }
        }
    }

    // 入度为 0 的结点入堆(最小者先出,顺序确定)
    let mut heap: BinaryHeap<Reverse<Cell>> = indeg
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&c, _)| Reverse(c))
        .collect();

    let mut order = Vec::with_capacity(nodes.len());
    while let Some(Reverse(c)) = heap.pop() {
        order.push(c);
        if let Some(children) = succ.get(&c) {
            for &child in children {
                let e = indeg.get_mut(&child).expect("child 有入度记录");
                *e -= 1;
                if *e == 0 {
                    heap.push(Reverse(child));
                }
            }
        }
    }

    if order.len() == nodes.len() {
        return (order, Vec::new());
    }
    // 剩余(入度未清零)= 环 + 环的下游
    let ordered: HashSet<Cell> = order.iter().copied().collect();
    let mut cyclic: Vec<Cell> = nodes
        .iter()
        .copied()
        .filter(|c| !ordered.contains(c))
        .collect();
    cyclic.sort_unstable();
    (order, cyclic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::parser::parse;

    fn prec(formula: &str) -> Precedents {
        collect_precedents(&parse(formula).unwrap())
    }

    #[test]
    fn extracts_cells_and_ranges() {
        let p = prec("A1+B2*3");
        assert_eq!(p.cells, vec![(0, 0), (1, 1)]);
        assert!(p.ranges.is_empty());

        let p = prec("SUM(A1:A3)+C1");
        assert_eq!(p.cells, vec![(0, 2)]);
        assert_eq!(p.ranges.len(), 1);
    }

    #[test]
    fn dedups_repeated_refs() {
        let p = prec("A1+A1+A1");
        assert_eq!(p.cells, vec![(0, 0)]);
    }

    #[test]
    fn literal_has_no_precedents() {
        assert!(prec("1+2*3").is_empty());
    }

    #[test]
    fn range_membership() {
        let r = prec("SUM(B2:D4)").ranges[0];
        assert!(range_contains(&r, (1, 1))); // B2
        assert!(range_contains(&r, (3, 3))); // D4
        assert!(!range_contains(&r, (0, 0))); // A1
        assert!(!range_contains(&r, (4, 3))); // D5 越界
    }

    #[test]
    fn topo_orders_precedents_first() {
        // A1 → A3 → A4;A2 独立。边:A3 依赖 A1、A2;A4 依赖 A3
        let nodes: HashSet<Cell> = [(0, 0), (1, 0), (2, 0), (3, 0)].into_iter().collect();
        let prec = |c: Cell| match c {
            (2, 0) => vec![(0, 0), (1, 0)],
            (3, 0) => vec![(2, 0)],
            _ => vec![],
        };
        let (order, cyclic) = topo_order(&nodes, prec);
        assert!(cyclic.is_empty());
        let pos = |c: Cell| order.iter().position(|&x| x == c).unwrap();
        assert!(pos((0, 0)) < pos((2, 0)));
        assert!(pos((1, 0)) < pos((2, 0)));
        assert!(pos((2, 0)) < pos((3, 0)));
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn topo_detects_cycle_and_downstream() {
        // A1 ↔ A2 成环;A3 依赖 A2(环的下游)
        let nodes: HashSet<Cell> = [(0, 0), (1, 0), (2, 0)].into_iter().collect();
        let prec = |c: Cell| match c {
            (0, 0) => vec![(1, 0)],
            (1, 0) => vec![(0, 0)],
            (2, 0) => vec![(1, 0)],
            _ => vec![],
        };
        let (order, cyclic) = topo_order(&nodes, prec);
        assert!(order.is_empty());
        assert_eq!(cyclic, vec![(0, 0), (1, 0), (2, 0)]);
    }
}
