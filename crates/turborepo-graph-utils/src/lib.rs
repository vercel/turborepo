mod walker;

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    hash::Hash,
};

use itertools::Itertools;
use petgraph::{
    prelude::*,
    visit::{depth_first_search, Reversed},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cyclic dependency detected:\n{0}")]
    CyclicDependencies(String),
    #[error("{0} depends on itself")]
    SelfDependency(String),
}

pub fn transitive_closure<'a, 'b, N: Hash + Eq + PartialEq + 'b, I: IntoIterator<Item = &'b N>>(
    graph: &'a Graph<N, ()>,
    node_lookup: &'a HashMap<N, petgraph::graph::NodeIndex>,
    nodes: I,
    direction: petgraph::Direction,
) -> HashSet<&'a N> {
    let indices = nodes
        .into_iter()
        .filter_map(|node| node_lookup.get(node))
        .copied();

    let mut visited = HashSet::new();

    let visitor = |event| {
        if let petgraph::visit::DfsEvent::Discover(n, _) = event {
            visited.insert(
                graph
                    .node_weight(n)
                    .expect("node index found during dfs doesn't exist"),
            );
        }
    };

    match direction {
        petgraph::Direction::Outgoing => depth_first_search(&graph, indices, visitor),
        petgraph::Direction::Incoming => depth_first_search(Reversed(&graph), indices, visitor),
    };

    visited
}

pub fn validate_graph<G: Display>(graph: &Graph<G, ()>) -> Result<(), Error> {
    // This is equivalent to AcyclicGraph.Cycles from Go's dag library
    let cycles_lines = petgraph::algo::tarjan_scc(&graph)
        .into_iter()
        .filter(|cycle| cycle.len() > 1)
        .map(|cycle| {
            let workspaces = cycle.into_iter().map(|id| graph.node_weight(id).unwrap());
            format!("\t{}", workspaces.format(", "))
        })
        .join("\n");

    if !cycles_lines.is_empty() {
        return Err(Error::CyclicDependencies(cycles_lines));
    }

    for edge in graph.edge_references() {
        if edge.source() == edge.target() {
            let node = graph
                .node_weight(edge.source())
                .expect("edge pointed to missing node");
            return Err(Error::SelfDependency(node.to_string()));
        }
    }

    Ok(())
}

pub use walker::{WalkMessage, Walker};
