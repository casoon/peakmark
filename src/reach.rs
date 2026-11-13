//! Why a function is in a module: a conservative reference graph and a short
//! path from a root to it.
//!
//! [`Reach::from_bytes`] reads the functions of a core module and what refers
//! to them. [`Reach::explain`] answers for one function by index: a
//! representative path from an export or the start function, and its direct
//! callers. [`Reach::report`] does that for a list and carries the
//! assumptions with it.
//!
//! # The graph
//!
//! Nodes are the functions in index order (imported ones first, they have no
//! outgoing edges) and one pseudo node, [`Node::Table`], which stands for every
//! function reference that does not sit in code: the entries of element
//! segments and the `ref.func` of global and table initializers.
//!
//! | Edge | From, to |
//! |---|---|
//! | [`Edge::Call`] | a function to the target of its `call` or `return_call` |
//! | [`Edge::Reference`] | a function to the target of a `ref.func` in its body |
//! | [`Edge::Dispatch`] | a function to the table, if its body contains `call_indirect`, `return_call_indirect`, `call_ref`, `return_call_ref`, a `table.*` instruction or a `global.get` of a reference-typed global |
//! | [`Edge::Elements`] | the table to every function in an element segment or in an initializer |
//!
//! Roots are the functions exported by the module and its start function.
//! The table is a root as well if the module imports or exports a table, or
//! exports a global that holds a reference: the host can then reach the
//! references through it.
//!
//! # What an answer means
//!
//! The graph is conservative about indirect calls. The target of a
//! `call_indirect` is not known from the bytes, so every function in an element
//! segment is assumed callable once a reachable function can dispatch. A path
//! that has to go through the table says so ([`Reachability::ViaTable`]); a
//! direct path is always preferred.
//!
//! A function without any path is [`Reachability::NoStaticRoot`]: the graph has
//! no root for it. That is not "dead code" and not a saving. The host can call
//! only what the module exports, but function pointers kept in linear memory,
//! imported callbacks and anything the graph does not model are outside it. If
//! the module contains operators the graph does not model (stack switching),
//! the answer for a function without a path is [`Reachability::Unknown`], never
//! an empty path. Data segments hold bytes, not function references, so they add
//! no edge: a function pointer in memory is a table index and is covered by
//! the dispatch edge.
//!
//! Cycles are no problem: the search keeps a set of visited nodes, so a path
//! never repeats a node. Among several paths the shortest is taken; equal
//! lengths are decided by the order of the exports, then the start function, and
//! then by the lower function index. The result is deterministic.
//!
//! The analysis is one pass over the code section plus two breadth-first
//! searches. It reads bytes only; it does not validate them, so run it on a
//! module that has been validated (see [`crate::wasm::inspect`]).

use std::collections::VecDeque;
use std::fmt;

use serde::Serialize;
use wasmparser::{
    BinaryReaderError, ConstExpr, ElementItems, ElementKind, ExternalKind, Operator, Parser,
    Payload, TableInit, TypeRef, ValType,
};

/// How many direct callers an [`Explanation`] lists.
pub const MAX_CALLERS: usize = 5;

/// What the graph assumes. `--json` carries it beside every answer.
pub const ASSUMPTIONS: [&str; 5] = [
    "roots: exported functions and the start function; the table is a root if the module imports or exports a table or exports a global that holds a reference",
    "edges: call and return_call, ref.func in function bodies, and from the table to every function in an element segment or a global or table initializer",
    "a function that can dispatch (call_indirect, call_ref, table.*, global.get of a reference) leads to the table, and the table to all of its functions",
    "data segments, imported callbacks and function pointers kept in linear memory add no edge",
    "no static root means no root in this graph, not that the function is unused",
];

/// The module could not be read.
#[derive(Debug)]
pub struct ReachError(BinaryReaderError);

impl fmt::Display for ReachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ReachError {
    /// The parser error underneath.
    #[must_use]
    pub fn into_inner(self) -> BinaryReaderError {
        self.0
    }
}

impl std::error::Error for ReachError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<BinaryReaderError> for ReachError {
    fn from(error: BinaryReaderError) -> Self {
        Self(error)
    }
}

/// A node of the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Node {
    /// A function, by index in the function index space (imported functions first).
    Function(u32),
    /// The function references outside code: element segments and initializers.
    Table,
}

/// How a node is reached from the one before it, or from the outside for the first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// The function is exported under this name.
    Export(String),
    /// The function is the start function.
    Start,
    /// The module imports or exports the table (or exports a reference): the host can reach it.
    Host,
    /// `call` or `return_call`.
    Call,
    /// `ref.func`: the function takes the address of the target.
    Reference,
    /// The function can dispatch through a table or a reference.
    Dispatch,
    /// The function is an entry of an element segment or an initializer.
    Elements,
}

/// One step of a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hop {
    /// The node.
    pub node: Node,
    /// How it is reached; for the first hop, the root it is.
    pub edge: Edge,
}

/// What the graph says about a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reachability {
    /// A path exists from an export or the start function that does not use the table.
    Direct,
    /// Every path goes through the table: reached by indirect dispatch, which is assumed.
    ViaTable,
    /// The graph has no root for it. Not "dead", not a saving.
    NoStaticRoot,
    /// There is no path, and the module has operators the graph does not model.
    Unknown,
}

/// A direct predecessor of a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Caller {
    /// The calling function, or the table for an element entry.
    pub node: Node,
    /// [`Edge::Call`], [`Edge::Reference`] or [`Edge::Elements`]; a call wins over a reference.
    pub edge: Link,
}

/// The kind of an edge between two nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    /// `call` or `return_call`.
    Call,
    /// `ref.func`.
    Reference,
    /// Function to table.
    Dispatch,
    /// Table to function.
    Elements,
}

impl From<Link> for Edge {
    fn from(link: Link) -> Self {
        match link {
            Link::Call => Edge::Call,
            Link::Reference => Edge::Reference,
            Link::Dispatch => Edge::Dispatch,
            Link::Elements => Edge::Elements,
        }
    }
}

/// The direct predecessors of a function, in index order, the table last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Callers {
    /// How many there are.
    pub count: usize,
    /// The first [`MAX_CALLERS`].
    pub listed: Vec<Caller>,
}

/// Why one function is in the module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explanation {
    /// The function, by index in this file.
    pub function: u32,
    /// What the graph says.
    pub reachability: Reachability,
    /// A representative path from a root to the function, root first; empty
    /// unless the function is reachable.
    pub path: Vec<Hop>,
    /// The functions that call or reference it directly.
    pub callers: Callers,
}

/// What the graph could not decide.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Coverage {
    /// `false` if the module has operators the graph does not model.
    pub complete: bool,
    /// How many such operators there are.
    pub unmodelled_operators: u64,
}

/// Explanations for a list of functions, with the assumptions behind them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ReachReport {
    /// What the graph assumes, as in [`ASSUMPTIONS`].
    pub assumptions: Vec<String>,
    /// What it could not decide.
    pub coverage: Coverage,
    /// One explanation per requested function.
    pub explanations: Vec<Explanation>,
}

/// The reference graph of one module and the searches over it.
#[derive(Debug, Clone)]
pub struct Reach {
    /// Outgoing edges of every function (imported ones have none), sorted.
    edges: Vec<Vec<(u32, Link)>>,
    /// Whether the function can dispatch, by function.
    dispatches: Vec<bool>,
    /// The functions the table leads to, sorted and unique.
    entries: Vec<u32>,
    /// Predecessors of every function.
    callers: Vec<Vec<Caller>>,
    unmodelled: u64,
    direct: Tree,
    with_table: Tree,
}

/// The result of one breadth-first search from the roots.
#[derive(Debug, Clone)]
struct Tree {
    /// The node it was reached from and the edge, by node index.
    parent: Vec<Option<(usize, Link)>>,
    /// The root a node is, by node index.
    root: Vec<Option<Edge>>,
    reached: Vec<bool>,
}

impl Reach {
    /// Reads the graph of a core module. The bytes should be validated.
    ///
    /// # Errors
    ///
    /// Fails if the bytes cannot be parsed as a core module.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReachError> {
        let mut builder = Builder::default();
        for payload in Parser::new(0).parse_all(bytes) {
            builder.payload(payload?)?;
        }
        Ok(builder.finish())
    }

    /// The number of functions in the index space, imported ones included.
    #[must_use]
    pub fn function_count(&self) -> usize {
        self.edges.len()
    }

    /// What the graph could not decide.
    #[must_use]
    pub fn coverage(&self) -> Coverage {
        Coverage {
            complete: self.unmodelled == 0,
            unmodelled_operators: self.unmodelled,
        }
    }

    /// Explains one function; `None` if `function` is not in the index space.
    #[must_use]
    pub fn explain(&self, function: u32) -> Option<Explanation> {
        let slot = function as usize;
        if slot >= self.edges.len() {
            return None;
        }
        let (reachability, tree) = if self.direct.reached[slot] {
            (Reachability::Direct, Some(&self.direct))
        } else if self.with_table.reached[slot] {
            (Reachability::ViaTable, Some(&self.with_table))
        } else if self.unmodelled > 0 {
            (Reachability::Unknown, None)
        } else {
            (Reachability::NoStaticRoot, None)
        };
        let callers = &self.callers[slot];
        Some(Explanation {
            function,
            reachability,
            path: tree.map_or_default(|tree| tree.path(slot)),
            callers: Callers {
                count: callers.len(),
                listed: callers.iter().take(MAX_CALLERS).copied().collect(),
            },
        })
    }

    /// Explains each of `functions` that is in the index space, with the assumptions.
    pub fn report(&self, functions: impl IntoIterator<Item = u32>) -> ReachReport {
        ReachReport {
            assumptions: ASSUMPTIONS.iter().map(ToString::to_string).collect(),
            coverage: self.coverage(),
            explanations: functions
                .into_iter()
                .filter_map(|function| self.explain(function))
                .collect(),
        }
    }
}

impl Tree {
    /// The path from the root to `node`, root first.
    fn path(&self, node: usize) -> Vec<Hop> {
        let table = self.parent.len() - 1;
        let node_of = |slot: usize| {
            if slot == table {
                Node::Table
            } else {
                Node::Function(u32::try_from(slot).expect("function index fits u32"))
            }
        };
        let mut hops = Vec::new();
        let mut current = node;
        loop {
            if let Some((from, link)) = self.parent[current] {
                hops.push(Hop {
                    node: node_of(current),
                    edge: link.into(),
                });
                current = from;
            } else {
                hops.push(Hop {
                    node: node_of(current),
                    edge: self.root[current]
                        .clone()
                        .expect("a node without a parent is a root"),
                });
                break;
            }
        }
        hops.reverse();
        hops
    }
}

#[derive(Default)]
struct Builder {
    imported_functions: u32,
    /// Whether each global, imported first, holds a reference.
    reference_globals: Vec<bool>,
    bodies: Vec<Body>,
    entries: Vec<u32>,
    roots: Vec<(u32, Edge)>,
    start: Option<u32>,
    host_table: bool,
    defined_functions: u32,
    unmodelled: u64,
}

#[derive(Default)]
struct Body {
    edges: Vec<(u32, Link)>,
    dispatches: bool,
}

impl Builder {
    fn payload(&mut self, payload: Payload<'_>) -> Result<(), BinaryReaderError> {
        match payload {
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    match import?.ty {
                        TypeRef::Func(_) | TypeRef::FuncExact(_) => self.imported_functions += 1,
                        TypeRef::Table(_) => self.host_table = true,
                        TypeRef::Global(global) => self
                            .reference_globals
                            .push(matches!(global.content_type, ValType::Ref(_))),
                        _ => {}
                    }
                }
            }
            Payload::FunctionSection(reader) => self.defined_functions = reader.count(),
            Payload::TableSection(reader) => {
                for table in reader {
                    if let TableInit::Expr(expr) = &table?.init {
                        self.initializer(expr)?;
                    }
                }
            }
            Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global?;
                    self.reference_globals
                        .push(matches!(global.ty.content_type, ValType::Ref(_)));
                    self.initializer(&global.init_expr)?;
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export?;
                    match export.kind {
                        ExternalKind::Func | ExternalKind::FuncExact => self
                            .roots
                            .push((export.index, Edge::Export(export.name.to_string()))),
                        ExternalKind::Table => self.host_table = true,
                        ExternalKind::Global if self.reference_globals[export.index as usize] => {
                            self.host_table = true;
                        }
                        _ => {}
                    }
                }
            }
            Payload::StartSection { func, .. } => self.start = Some(func),
            Payload::ElementSection(reader) => {
                for element in reader {
                    let element = element?;
                    if matches!(element.kind, ElementKind::Declared) {
                        continue;
                    }
                    match element.items {
                        ElementItems::Functions(functions) => {
                            for function in functions {
                                self.entries.push(function?);
                            }
                        }
                        ElementItems::Expressions(_, expressions) => {
                            for expr in expressions {
                                self.initializer(&expr?)?;
                            }
                        }
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut found = Body::default();
                for operator in body.get_operators_reader()? {
                    self.operator(&operator?, &mut found);
                }
                found.edges.sort_unstable();
                found.edges.dedup();
                self.bodies.push(found);
            }
            _ => {}
        }
        Ok(())
    }

    /// The functions a constant expression refers to are held by the table.
    fn initializer(&mut self, expr: &ConstExpr<'_>) -> Result<(), BinaryReaderError> {
        for operator in expr.get_operators_reader() {
            if let Operator::RefFunc { function_index } = operator? {
                self.entries.push(function_index);
            }
        }
        Ok(())
    }

    fn operator(&mut self, operator: &Operator<'_>, body: &mut Body) {
        match operator {
            Operator::Call { function_index } | Operator::ReturnCall { function_index } => {
                body.edges.push((*function_index, Link::Call));
            }
            Operator::RefFunc { function_index } => {
                body.edges.push((*function_index, Link::Reference));
            }
            Operator::CallIndirect { .. }
            | Operator::ReturnCallIndirect { .. }
            | Operator::CallRef { .. }
            | Operator::ReturnCallRef { .. }
            | Operator::TableGet { .. }
            | Operator::TableSet { .. }
            | Operator::TableInit { .. }
            | Operator::TableCopy { .. }
            | Operator::TableGrow { .. }
            | Operator::TableFill { .. }
            | Operator::TableSize { .. } => body.dispatches = true,
            Operator::GlobalGet { global_index }
            | Operator::GlobalAtomicGet { global_index, .. }
                if self.reference_globals[*global_index as usize] =>
            {
                body.dispatches = true;
            }
            Operator::ContNew { .. }
            | Operator::ContBind { .. }
            | Operator::Suspend { .. }
            | Operator::Resume { .. }
            | Operator::ResumeThrow { .. }
            | Operator::ResumeThrowRef { .. }
            | Operator::Switch { .. } => self.unmodelled += 1,
            _ => {}
        }
    }

    fn finish(mut self) -> Reach {
        let functions = (self.imported_functions + self.defined_functions) as usize;
        let mut edges = vec![Vec::new(); functions];
        let mut dispatches = vec![false; functions];
        for (position, body) in self.bodies.drain(..).enumerate() {
            let slot = self.imported_functions as usize + position;
            edges[slot] = body.edges;
            dispatches[slot] = body.dispatches;
        }
        self.entries.sort_unstable();
        self.entries.dedup();
        if let Some(start) = self.start {
            self.roots.push((start, Edge::Start));
        }

        let mut callers: Vec<Vec<Caller>> = vec![Vec::new(); functions];
        for (source, outgoing) in edges.iter().enumerate() {
            let node = Node::Function(u32::try_from(source).expect("function index fits u32"));
            for &(target, link) in outgoing {
                let list = &mut callers[target as usize];
                if list.last().is_none_or(|last| last.node != node) {
                    list.push(Caller { node, edge: link });
                }
            }
        }
        for &entry in &self.entries {
            callers[entry as usize].push(Caller {
                node: Node::Table,
                edge: Link::Elements,
            });
        }

        let mut reach = Reach {
            edges,
            dispatches,
            entries: self.entries,
            callers,
            unmodelled: self.unmodelled,
            direct: Tree::empty(functions),
            with_table: Tree::empty(functions),
        };
        reach.direct = reach.search(&self.roots, false, false);
        reach.with_table = reach.search(&self.roots, true, self.host_table);
        reach
    }
}

impl Tree {
    fn empty(functions: usize) -> Self {
        Self {
            parent: vec![None; functions + 1],
            root: vec![None; functions + 1],
            reached: vec![false; functions + 1],
        }
    }
}

impl Reach {
    /// A breadth-first search from the roots. Without `table`, the table node
    /// is never entered: the result holds the paths made of calls and
    /// references only. With it, dispatch edges are followed, and the table is
    /// a root itself if `host_table` is set.
    fn search(&self, roots: &[(u32, Edge)], table: bool, host_table: bool) -> Tree {
        let table_slot = self.edges.len();
        let mut tree = Tree::empty(self.edges.len());
        let mut queue = VecDeque::new();
        for (function, edge) in roots {
            let slot = *function as usize;
            if !tree.reached[slot] {
                tree.reached[slot] = true;
                tree.root[slot] = Some(edge.clone());
                queue.push_back(slot);
            }
        }
        if table && host_table {
            tree.reached[table_slot] = true;
            tree.root[table_slot] = Some(Edge::Host);
            queue.push_back(table_slot);
        }
        while let Some(from) = queue.pop_front() {
            let mut visit = |to: usize, link: Link| {
                if !tree.reached[to] {
                    tree.reached[to] = true;
                    tree.parent[to] = Some((from, link));
                    queue.push_back(to);
                }
            };
            if from == table_slot {
                for &entry in &self.entries {
                    visit(entry as usize, Link::Elements);
                }
                continue;
            }
            for &(target, link) in &self.edges[from] {
                visit(target as usize, link);
            }
            if table && self.dispatches[from] {
                visit(table_slot, Link::Dispatch);
            }
        }
        tree
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    fn reach(wat: &str) -> Reach {
        Reach::from_bytes(&wat::parse_str(wat).unwrap()).unwrap()
    }

    fn explain(reach: &Reach, function: u32) -> Explanation {
        reach.explain(function).unwrap()
    }

    fn functions(path: &[Hop]) -> Vec<Option<u32>> {
        path.iter()
            .map(|hop| match hop.node {
                Node::Function(index) => Some(index),
                Node::Table => None,
            })
            .collect()
    }

    fn export(name: &str) -> Edge {
        Edge::Export(name.to_string())
    }

    #[test]
    fn a_direct_call_has_a_path_from_the_export() {
        let reach = reach(
            r#"(module
                (func $main (export "main") call $a)
                (func $a call $b)
                (func $b))"#,
        );
        let explanation = explain(&reach, 2);
        assert_eq!(explanation.reachability, Reachability::Direct);
        assert_eq!(functions(&explanation.path), [Some(0), Some(1), Some(2)]);
        assert_eq!(explanation.path[0].edge, export("main"));
        assert_eq!(explanation.path[2].edge, Edge::Call);
        assert_eq!(explanation.callers.count, 1);
        assert_eq!(explanation.callers.listed[0].node, Node::Function(1));
    }

    #[test]
    fn a_helper_shared_by_two_exports_takes_the_shortest_path_and_names_both_callers() {
        let reach = reach(
            r#"(module
                (func (export "first") call $mid)
                (func (export "second") call $helper)
                (func $mid call $helper)
                (func $helper))"#,
        );
        let explanation = explain(&reach, 3);
        assert_eq!(functions(&explanation.path), [Some(1), Some(3)]);
        assert_eq!(explanation.path[0].edge, export("second"));
        let callers: Vec<_> = explanation.callers.listed.iter().map(|c| c.node).collect();
        assert_eq!(callers, [Node::Function(1), Node::Function(2)]);
    }

    #[test]
    fn equal_paths_are_decided_by_the_order_of_the_exports() {
        let reach = reach(
            r#"(module
                (func (export "a") call $h)
                (func (export "b") call $h)
                (func $h))"#,
        );
        assert_eq!(explain(&reach, 2).path[0].edge, export("a"));
    }

    #[test]
    fn recursion_does_not_loop_and_is_listed_as_a_caller() {
        let reach = reach(
            r#"(module
                (func $a (export "a") call $b)
                (func $b call $a call $b call $c)
                (func $c call $c))"#,
        );
        let explanation = explain(&reach, 2);
        assert_eq!(functions(&explanation.path), [Some(0), Some(1), Some(2)]);
        let callers: Vec<_> = explanation.callers.listed.iter().map(|c| c.node).collect();
        assert_eq!(callers, [Node::Function(1), Node::Function(2)]);
        assert_eq!(explain(&reach, 1).callers.count, 2);
    }

    #[test]
    fn the_start_function_is_a_root() {
        let reach = reach(
            "(module
                (func $init call $setup)
                (func $setup)
                (start $init))",
        );
        let explanation = explain(&reach, 1);
        assert_eq!(explanation.reachability, Reachability::Direct);
        assert_eq!(explanation.path[0].edge, Edge::Start);
        assert_eq!(functions(&explanation.path), [Some(0), Some(1)]);
    }

    #[test]
    fn a_function_nobody_refers_to_has_no_static_root() {
        let reach = reach(
            r#"(module
                (func (export "main"))
                (func $orphan)
                (func $caller_of_orphan call $orphan))"#,
        );
        for function in [1, 2] {
            let explanation = explain(&reach, function);
            assert_eq!(explanation.reachability, Reachability::NoStaticRoot);
            assert_eq!(explanation.path.len(), 0);
        }
        assert_eq!(explain(&reach, 1).callers.count, 1);
        assert!(reach.coverage().complete);
    }

    #[test]
    fn a_table_entry_is_reached_through_the_table_when_a_function_dispatches() {
        let reach = reach(
            r#"(module
                (type $t (func))
                (table 2 funcref)
                (elem (i32.const 0) $slot)
                (func (export "run") (call_indirect (type $t) (i32.const 0)))
                (func $slot call $deeper)
                (func $deeper))"#,
        );
        let slot = explain(&reach, 1);
        assert_eq!(slot.reachability, Reachability::ViaTable);
        assert_eq!(functions(&slot.path), [Some(0), None, Some(1)]);
        assert_eq!(slot.path[1].edge, Edge::Dispatch);
        assert_eq!(slot.path[2].edge, Edge::Elements);
        assert_eq!(slot.callers.listed[0].node, Node::Table);

        let deeper = explain(&reach, 2);
        assert_eq!(deeper.reachability, Reachability::ViaTable);
        assert_eq!(functions(&deeper.path), [Some(0), None, Some(1), Some(2)]);
    }

    #[test]
    fn a_function_with_a_direct_path_is_not_reported_as_indirect() {
        let reach = reach(
            r#"(module
                (type $t (func))
                (table 1 funcref)
                (elem (i32.const 0) $both)
                (func (export "run") call $both (call_indirect (type $t) (i32.const 0)))
                (func $both))"#,
        );
        assert_eq!(explain(&reach, 1).reachability, Reachability::Direct);
    }

    #[test]
    fn a_table_that_nothing_dispatches_through_leads_nowhere() {
        let reach = reach(
            r#"(module
                (table 1 funcref)
                (elem (i32.const 0) $slot)
                (func (export "run"))
                (func $slot))"#,
        );
        assert_eq!(explain(&reach, 1).reachability, Reachability::NoStaticRoot);
    }

    #[test]
    fn an_exported_table_is_a_root() {
        let reach = reach(
            r#"(module
                (table (export "table") 1 funcref)
                (elem (i32.const 0) $slot)
                (func $slot))"#,
        );
        let explanation = explain(&reach, 0);
        assert_eq!(explanation.reachability, Reachability::ViaTable);
        assert_eq!(functions(&explanation.path), [None, Some(0)]);
        assert_eq!(explanation.path[0].edge, Edge::Host);
    }

    #[test]
    fn ref_func_in_a_function_body_is_an_edge() {
        let reach = reach(
            r#"(module
                (func (export "run") (drop (ref.func $target)))
                (func $target))"#,
        );
        let explanation = explain(&reach, 1);
        assert_eq!(explanation.reachability, Reachability::Direct);
        assert_eq!(explanation.path[1].edge, Edge::Reference);
        assert_eq!(explanation.callers.listed[0].edge, Link::Reference);
    }

    #[test]
    fn ref_func_in_a_global_initializer_is_reached_when_a_function_reads_references() {
        let reach = reach(
            r#"(module
                (global $g funcref (ref.func $target))
                (func (export "run") (drop (global.get $g)))
                (func $target)
                (func $unread))"#,
        );
        let target = explain(&reach, 1);
        assert_eq!(target.reachability, Reachability::ViaTable);
        assert_eq!(functions(&target.path), [Some(0), None, Some(1)]);
        assert_eq!(explain(&reach, 2).reachability, Reachability::NoStaticRoot);
    }

    #[test]
    fn ref_func_in_a_global_initializer_is_not_reached_by_a_function_that_reads_numbers() {
        let reach = reach(
            r#"(module
                (global $n i32 (i32.const 1))
                (global $g funcref (ref.func $target))
                (func (export "run") (drop (global.get $n)))
                (func $target))"#,
        );
        assert_eq!(explain(&reach, 1).reachability, Reachability::NoStaticRoot);
    }

    #[test]
    fn imported_functions_come_first_and_are_leaves() {
        let reach = reach(
            r#"(module
                (import "env" "log" (func $log))
                (func (export "run") call $log))"#,
        );
        assert_eq!(reach.function_count(), 2);
        let log = explain(&reach, 0);
        assert_eq!(functions(&log.path), [Some(1), Some(0)]);
        assert_eq!(explain(&reach, 1).path[0].edge, export("run"));
    }

    #[test]
    fn only_the_first_callers_are_listed_but_all_are_counted() {
        let mut callers = String::new();
        for i in 0..7 {
            let _ = write!(callers, r#"(func (export "e{i}") call $h)"#);
        }
        let reach = reach(&format!("(module {callers} (func $h))"));
        let explanation = explain(&reach, 7);
        assert_eq!(explanation.callers.count, 7);
        assert_eq!(explanation.callers.listed.len(), MAX_CALLERS);
    }

    #[test]
    fn an_index_outside_the_module_has_no_explanation() {
        assert!(reach("(module (func))").explain(1).is_none());
    }

    #[test]
    fn the_report_carries_the_assumptions_and_skips_unknown_indexes() {
        let reach = reach(r#"(module (func (export "run")))"#);
        let report = reach.report([0, 9]);
        assert_eq!(report.explanations.len(), 1);
        assert_eq!(report.assumptions.len(), ASSUMPTIONS.len());
        assert!(report.coverage.complete);
    }

    #[test]
    fn a_module_the_graph_cannot_model_answers_unknown_and_never_no_root() {
        let bytes = wat::parse_str(
            r#"(module
                (type $f (func))
                (type $c (cont $f))
                (func (export "run"))
                (func $orphan (param (ref null $c)) (resume $c (local.get 0)))
                (func $other))"#,
        )
        .unwrap();
        let reach = Reach::from_bytes(&bytes).unwrap();
        assert_eq!(reach.coverage().unmodelled_operators, 1);
        assert!(!reach.coverage().complete);
        assert_eq!(explain(&reach, 2).reachability, Reachability::Unknown);
        assert_eq!(explain(&reach, 2).path.len(), 0);
        // What has a path keeps it.
        assert_eq!(explain(&reach, 0).reachability, Reachability::Direct);
    }

    #[test]
    fn bytes_that_are_not_a_module_are_an_error() {
        assert!(Reach::from_bytes(b"not wasm").is_err());
    }
}
