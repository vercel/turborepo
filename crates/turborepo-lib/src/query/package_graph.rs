use std::sync::Arc;

use async_graphql::{Object, SimpleObject};
use itertools::Itertools;
use petgraph::graph::NodeIndex;
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::{
    query::{package::Package, Array, Error, PackagePredicate},
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
    async fn nodes(&self) -> Result<Array<Package>, Error> {
        let direct_dependencies = self
            .center
            .as_ref()
            .and_then(|center| self.run.pkg_dep_graph().immediate_dependencies(center));

        let mut nodes = self
            .run
            .pkg_dep_graph()
            .node_indices()
            .filter_map(|idx| {
                let package_node = self.run.pkg_dep_graph().get_package_by_index(idx)?;
                if let Some(center) = &self.center {
                    if center == package_node {
                        return Some(Package::new(
                            self.run.clone(),
                            package_node.as_package_name().clone(),
                        ));
                    }
                }

                if matches!(package_node, PackageNode::Root)
                    || matches!(package_node, PackageNode::Workspace(PackageName::Root))
                {
                    return None;
                }
                if let Some(dependencies) = direct_dependencies.as_ref() {
                    if !dependencies.contains(package_node) {
                        return None;
                    }
                }

                let package =
                    match Package::new(self.run.clone(), package_node.as_package_name().clone()) {
                        Ok(package) => package,
                        Err(err) => {
                            return Some(Err(err));
                        }
                    };

                if let Some(filter) = &self.filter {
                    if !filter.check(&package) {
                        return None;
                    }
                }

                Some(Ok(package))
            })
            .collect::<Result<Array<_>, _>>()?;

        nodes.sort_by(|a, b| a.get_name().cmp(b.get_name()));

        Ok(nodes)
    }

    async fn edges(&self) -> Array<Edge> {
        let direct_dependencies = self
            .center
            .as_ref()
            .and_then(|center| self.run.pkg_dep_graph().immediate_dependencies(center));
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

                if matches!(
                    source_node,
                    PackageNode::Root | PackageNode::Workspace(PackageName::Root)
                ) || matches!(
                    target_node,
                    PackageNode::Root | PackageNode::Workspace(PackageName::Root)
                ) {
                    return None;
                }

                if let Some(center) = &self.center {
                    if center == source_node || center == target_node {
                        return Some(Edge {
                            source: source_node.as_package_name().to_string(),
                            target: target_node.as_package_name().to_string(),
                        });
                    }
                }
                if let Some(dependencies) = direct_dependencies.as_ref() {
                    if !dependencies.contains(source_node) || !dependencies.contains(target_node) {
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
