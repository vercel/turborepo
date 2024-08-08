use std::sync::Arc;

use async_graphql::*;
use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::run::Run;

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error("package not found: {0}")]
    PackageNotFound(PackageName),
    #[error("failed to serialize result: {0}")]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Run(#[from] crate::run::Error),
}

pub struct Query {
    run: Arc<Run>,
}

impl Query {
    pub fn new(run: Run) -> Self {
        Self { run: Arc::new(run) }
    }
}

struct Package {
    run: Arc<Run>,
    name: PackageName,
}

impl Package {
    fn immediate_dependents_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .immediate_ancestors(&PackageNode::Workspace(self.name.clone()))
            .map_or(0, |pkgs| pkgs.len())
    }

    fn immediate_dependencies_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .immediate_dependencies(&PackageNode::Workspace(self.name.clone()))
            .map_or(0, |pkgs| pkgs.len())
    }

    fn dependent_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());

        self.run.pkg_dep_graph().ancestors(&node).len()
    }

    fn dependency_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());

        self.run.pkg_dep_graph().dependencies(&node).len()
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
enum PackageFields {
    Name,
    DependencyCount,
    DependentCount,
    ImmediateDependentCount,
    ImmediateDependencyCount,
}

#[derive(InputObject)]
struct FieldValuePair {
    field: PackageFields,
    value: Any,
}

/// Predicates are used to filter packages
#[derive(InputObject)]
struct PackagePredicate {
    and: Option<Vec<PackagePredicate>>,
    or: Option<Vec<PackagePredicate>>,
    equal: Option<FieldValuePair>,
    not_equal: Option<FieldValuePair>,
    greater_than: Option<FieldValuePair>,
    less_than: Option<FieldValuePair>,
    not: Option<Box<PackagePredicate>>,
}

impl PackagePredicate {
    fn check_equals(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::Name, Value::String(name)) => pkg.name.as_ref() == name,
            (PackageFields::DependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.dependency_count() == n as usize
            }
            (PackageFields::DependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.dependent_count() == n as usize
            }
            (PackageFields::ImmediateDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.immediate_dependents_count() == n as usize
            }
            (PackageFields::ImmediateDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.immediate_dependencies_count() == n as usize
            }
            _ => false,
        }
    }

    fn check_greater_than(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::DependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.dependency_count() > n as usize
            }
            (PackageFields::DependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.dependent_count() > n as usize
            }
            (PackageFields::ImmediateDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.immediate_dependents_count() > n as usize
            }
            (PackageFields::ImmediateDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.immediate_dependencies_count() > n as usize
            }
            _ => false,
        }
    }

    fn check_less_than(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::DependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.dependency_count() < n as usize
            }
            (PackageFields::DependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.dependent_count() < n as usize
            }
            (PackageFields::ImmediateDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.immediate_dependents_count() < n as usize
            }
            (PackageFields::ImmediateDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.immediate_dependencies_count() < n as usize
            }
            _ => false,
        }
    }

    fn check(&self, pkg: &Package) -> bool {
        let and = self
            .and
            .as_ref()
            .map(|predicates| predicates.iter().all(|p| p.check(pkg)));
        let or = self
            .or
            .as_ref()
            .map(|predicates| predicates.iter().any(|p| p.check(pkg)));
        let equal = self
            .equal
            .as_ref()
            .map(|pair| Self::check_equals(pkg, &pair.field, &pair.value));
        let not_equal = self
            .not_equal
            .as_ref()
            .map(|pair| !Self::check_equals(pkg, &pair.field, &pair.value));

        let greater_than = self
            .greater_than
            .as_ref()
            .map(|pair| Self::check_greater_than(pkg, &pair.field, &pair.value));

        let less_than = self
            .less_than
            .as_ref()
            .map(|pair| Self::check_greater_than(pkg, &pair.field, &pair.value));
        let not = self.not.as_ref().map(|predicate| !predicate.check(pkg));

        and.into_iter()
            .chain(or)
            .chain(equal)
            .chain(not_equal)
            .chain(greater_than)
            .chain(less_than)
            .chain(not)
            .all(|p| p)
    }
}

#[Object]
impl Query {
    async fn package(&self, name: String) -> Result<Package, Error> {
        let name = PackageName::from(name);
        Ok(Package {
            run: self.run.clone(),
            name,
        })
    }

    async fn packages(&self, filter: Option<PackagePredicate>) -> Result<Vec<Package>, Error> {
        let Some(filter) = filter else {
            return Ok(self
                .run
                .pkg_dep_graph()
                .packages()
                .map(|(name, _)| Package {
                    run: self.run.clone(),
                    name: name.clone(),
                })
                .collect());
        };

        Ok(self
            .run
            .pkg_dep_graph()
            .packages()
            .map(|(name, _)| Package {
                run: self.run.clone(),
                name: name.clone(),
            })
            .filter(|pkg| filter.check(pkg))
            .collect())
    }
}

#[Object]
impl Package {
    async fn name(&self) -> String {
        self.name.to_string()
    }

    async fn dependents(&self) -> Result<Vec<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());

        Ok(self
            .run
            .pkg_dep_graph()
            .ancestors(&node)
            .iter()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .collect())
    }

    async fn dependencies(&self) -> Result<Vec<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());

        Ok(self
            .run
            .pkg_dep_graph()
            .dependencies(&node)
            .iter()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .collect())
    }
}
