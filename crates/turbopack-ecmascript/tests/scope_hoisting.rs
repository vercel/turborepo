#![feature(arbitrary_self_types)]
use std::hash::{BuildHasherDefault, Hash};

use anyhow::Result;
use indexmap::IndexSet;
use petgraph::{algo::has_path_connecting, graphmap::DiGraphMap};
use rustc_hash::FxHasher;
use turbopack_ecmascript::scope_hoisting::group::{split_scopes, DepGraph, EdgeData, Item};

fn register() {
    turbo_tasks::register();
    turbo_tasks_fs::register();
    turbopack_ecmascript::register();

    include!(concat!(env!("OUT_DIR"), "/register_test_scope_hoisting.rs"));
}

#[test]
fn test_1() -> Result<()> {
    let result = split(to_num_deps(vec![
        ("example", vec![("a", false), ("b", false), ("lazy", true)]),
        ("lazy", vec![("c", false), ("d", false)]),
        ("a", vec![("shared", false)]),
        ("c", vec![("shared", false), ("cjs", false)]),
        ("shared", vec![("shared2", false)]),
    ]));

    assert_eq!(result, vec![vec![0, 1, 2], vec![5, 7, 4, 3], vec![8, 6]]);

    Ok(())
}

// Test that all of a, b, c, and d are in the same scope
//
// a => b
// a => c
// b => d
// c => d
#[test]
fn test_2() -> Result<()> {
    let result = split(to_num_deps(vec![
        ("example", vec![("a", false), ("b", false), ("lazy", true)]),
        ("lazy", vec![("shared", false)]),
        ("a", vec![("shared", false), ("b", false), ("c", false)]),
        ("b", vec![("shared", false), ("d", false)]),
        ("c", vec![("shared", false), ("d", false)]),
        ("d", vec![("shared", false)]),
        ("shared", vec![("shared2", false)]),
    ]));

    assert_eq!(result, vec![vec![0, 1, 5, 2, 6], vec![3], vec![4, 7]]);

    Ok(())
}

fn to_num_deps(deps: Vec<(&str, Vec<(&str, bool)>)>) -> Deps {
    let mut map = IndexSet::new();

    for (from, to) in deps.iter() {
        if map.insert(*from) {
            eprintln!("Inserted {from} as {}", map.get_full(from).unwrap().0);
        }

        for (to, _) in to {
            if map.insert(to) {
                eprintln!("Inserted {to} as {}", map.get_full(to).unwrap().0);
            }
        }
    }

    deps.into_iter()
        .map(|(from, to)| {
            (
                map.get_full(from).unwrap().0,
                to.into_iter()
                    .map(|(to, is_lazy)| (map.get_full(to).unwrap().0, is_lazy))
                    .collect(),
            )
        })
        .collect()
}

type Deps = Vec<(usize, Vec<(usize, bool)>)>;

fn split(deps: Deps) -> Vec<Vec<usize>> {
    register();

    let mut graph = test_dep_graph(deps);

    let group = split_scopes(&mut *graph, Item(0));

    let mut data = vec![];

    for scope in group.scopes.iter() {
        let mut scope_data = vec![];

        for &module in scope.modules.iter() {
            scope_data.push(module);
        }

        data.push(scope_data);
    }

    data.into_iter()
        .map(|x| x.into_iter().map(|v| v.0).collect())
        .collect()
}

fn test_dep_graph(deps: Deps) -> Box<dyn DepGraph> {
    let mut g = InternedGraph::default();

    for (from, to) in deps {
        let from = g.node(&Item(from));

        for (to, is_lazy) in to {
            let to = g.node(&Item(to));

            g.idx_graph.add_edge(from, to, TestEdgeData { is_lazy });
        }
    }

    Box::new(TestDepGraph { graph: g })
}

pub struct TestDepGraph {
    graph: InternedGraph<Item>,
}

impl DepGraph for TestDepGraph {
    fn deps(&self, module: Item) -> Vec<Item> {
        let from = self.graph.get_node(&module);

        let dependencies = self
            .graph
            .idx_graph
            .neighbors_directed(from, petgraph::Direction::Outgoing)
            .map(|ix| *self.graph.graph_ix.get_index(ix as usize).unwrap())
            .collect();

        dependencies
    }

    fn depandants(&self, module: Item) -> Vec<Item> {
        let from = self.graph.get_node(&module);

        let dependants = self
            .graph
            .idx_graph
            .neighbors_directed(from, petgraph::Direction::Incoming)
            .map(|ix| *self.graph.graph_ix.get_index(ix as usize).unwrap())
            .collect();

        dependants
    }

    fn get_edge(&self, from: Item, to: Item) -> EdgeData {
        let from = self.graph.get_node(&from);
        let to = self.graph.get_node(&to);

        let edge_data = self.graph.idx_graph.edge_weight(from, to).unwrap();

        let is_lazy = edge_data.is_lazy;

        EdgeData { is_lazy }
    }

    fn has_path_connecting(&self, from: Item, to: Item) -> bool {
        let from = self.graph.get_node(&from);
        let to = self.graph.get_node(&to);

        has_path_connecting(&self.graph.idx_graph, from, to, None)
    }

    fn remove_edge(&mut self, from: Item, to: Item) {
        let from = self.graph.get_node(&from);
        let to = self.graph.get_node(&to);

        self.graph.idx_graph.remove_edge(from, to);
    }
}

#[derive(Debug)]
struct TestEdgeData {
    is_lazy: bool,
}

impl<T> PartialEq for InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl<T> Eq for InternedGraph<T> where T: Eq + Hash + Clone {}

#[derive(Debug)]
pub struct InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    idx_graph: DiGraphMap<u32, TestEdgeData>,
    graph_ix: IndexSet<T, BuildHasherDefault<FxHasher>>,
}

impl<T> Default for InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self {
            idx_graph: Default::default(),
            graph_ix: Default::default(),
        }
    }
}

impl<T> InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    fn node(&mut self, id: &T) -> u32 {
        self.graph_ix.get_index_of(id).unwrap_or_else(|| {
            let ix = self.graph_ix.len();
            self.graph_ix.insert_full(id.clone());
            ix
        }) as _
    }

    /// Panics if `id` is not found.
    fn get_node(&self, id: &T) -> u32 {
        self.graph_ix.get_index_of(id).unwrap() as _
    }
}
