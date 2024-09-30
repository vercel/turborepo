use std::sync::Arc;

use async_graphql::{Object, SimpleObject};
use itertools::Itertools;
use petgraph::graph::NodeIndex;
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::{
    query::{package::Package, Array, PackagePredicate},
    run::Run,
};

pub struct PackageGraph {
    run: Arc<Run>,
    center: Option<PackageNode>,
    filter: Option<PackagePredicate>,
}

impl PackageGraph {
    pub fn new(run: Arc<Run>, center: Option<String>, filter: Option<PackagePredicate>) -> Self {
        let center = center.map(|center| PackageNode::Workspace(PackageName::from(center)));

        Self {
            run,
            center,
            filter,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Node {
    idx: NodeIndex,
    run: Arc<Run>,
}

#[derive(Debug, Clone, SimpleObject, Hash, PartialEq, Eq)]
pub(crate) struct Edge {
    source: String,
    target: String,
}

#[Object]
impl PackageGraph {
    async fn nodes(&self) -> Array<Package> {
        let transitive_closure = self
            .center
            .as_ref()
            .map(|center| self.run.pkg_dep_graph().transitive_closure(Some(center)));
        self.run
            .pkg_dep_graph()
            .node_indices()
            .filter_map(|idx| {
                let package_node = self.run.pkg_dep_graph().get_package_by_index(idx)?;
                if let Some(closure) = transitive_closure.as_ref() {
                    if !closure.contains(package_node) {
                        return None;
                    }
                }

                let package = Package {
                    run: self.run.clone(),
                    name: package_node.as_package_name().clone(),
                };

                if let Some(filter) = &self.filter {
                    if !filter.check(&package) {
                        return None;
                    }
                }

                Some(package)
            })
            .collect()
    }

    async fn edges(&self) -> Array<Edge> {
        let transitive_closure = self
            .center
            .as_ref()
            .map(|center| self.run.pkg_dep_graph().transitive_closure(Some(center)));
        self.run
            .pkg_dep_graph()
            .edges()
            .iter()
            .filter_map(|edge| {
                if edge.source() == edge.target() {
                    return None;
                }
                let source_node = self
                    .run
                    .pkg_dep_graph()
                    .get_package_by_index(edge.source())?;
                let target_node = self
                    .run
                    .pkg_dep_graph()
                    .get_package_by_index(edge.target())?;

                if let Some(closure) = transitive_closure.as_ref() {
                    if !closure.contains(source_node) || !closure.contains(target_node) {
                        return None;
                    }
                }

                Some(Edge {
                    source: source_node.as_package_name().to_string(),
                    target: target_node.as_package_name().to_string(),
                })
            })
            .dedup()
            .collect()
    }
}
