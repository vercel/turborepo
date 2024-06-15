mod walker;

use std::fmt::Display;

use itertools::Itertools;
use petgraph::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cyclic dependency detected:\n{0}")]
    CyclicDependencies(String),
    #[error("{0} depends on itself")]
    SelfDependency(String),
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
