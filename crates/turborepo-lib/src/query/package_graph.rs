use std::sync::Arc;

use async_graphql::{Object, SimpleObject};
use petgraph::graph::NodeIndex;
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::{query::Array, run::Run};

pub struct PackageGraph {
    run: Arc<Run>,
    center: PackageNode,
}

impl PackageGraph {
    pub fn new(run: Arc<Run>, center: String) -> Self {
        let center = PackageNode::Workspace(PackageName::from(center));

        Self { run, center }
    }
}

#[derive(Clone)]
pub(crate) struct Node {
    idx: NodeIndex,
    run: Arc<Run>,
}

#[Object]
impl Node {
    async fn name(&self) -> Option<String> {
        self.run
            .pkg_dep_graph()
            .get_package_by_index(self.idx)
            .map(|pkg| pkg.to_string())
    }

    async fn idx(&self) -> usize {
        self.idx.index()
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct Edge {
    source: usize,
    target: usize,
}

#[Object]
impl PackageGraph {
    async fn nodes(&self) -> Array<Node> {
        let transitive_closure = self
            .run
            .pkg_dep_graph()
            .transitive_closure(Some(&self.center));
        self.run
            .pkg_dep_graph()
            .node_indices()
            .filter_map(|idx| {
                let package_node = self.run.pkg_dep_graph().get_package_by_index(idx)?;
                if !transitive_closure.contains(package_node) {
                    return None;
                }

                Some(Node {
                    idx: idx,
                    run: self.run.clone(),
                })
            })
            .collect()
    }

    async fn edges(&self) -> Array<Edge> {
        let transitive_closure = self
            .run
            .pkg_dep_graph()
            .transitive_closure(Some(&self.center));
        self.run
            .pkg_dep_graph()
            .edges()
            .iter()
            .filter_map(|edge| {
                let source_node = self
                    .run
                    .pkg_dep_graph()
                    .get_package_by_index(edge.source())?;
                let target_node = self
                    .run
                    .pkg_dep_graph()
                    .get_package_by_index(edge.target())?;

                if !transitive_closure.contains(source_node)
                    || !transitive_closure.contains(target_node)
                {
                    return None;
                }

                Some(Edge {
                    source: edge.source().index(),
                    target: edge.target().index(),
                })
            })
            .collect()
    }
}
