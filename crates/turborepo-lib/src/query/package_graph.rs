use std::sync::Arc;

use async_graphql::{Object, SimpleObject};

use crate::{query::Array, run::Run};

pub struct PackageGraph {
    run: Arc<Run>,
}

impl PackageGraph {
    pub fn new(run: Arc<Run>) -> Self {
        Self { run }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct Node {
    idx: usize,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct Edge {
    source: usize,
    target: usize,
}

#[Object]
impl PackageGraph {
    async fn nodes(&self) -> Array<Node> {
        self.run
            .pkg_dep_graph()
            .node_indices()
            .map(|idx| Node { idx: idx.index() })
            .collect()
    }

    async fn edges(&self) -> Array<Edge> {
        self.run
            .pkg_dep_graph()
            .edges()
            .iter()
            .map(|edge| Edge {
                source: edge.source().index(),
                target: edge.target().index(),
            })
            .collect()
    }
}
