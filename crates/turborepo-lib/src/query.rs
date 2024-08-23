use std::{io, sync::Arc};

use async_graphql::{http::GraphiQLSource, *};
use async_graphql_axum::GraphQL;
use axum::{response, response::IntoResponse, routing::get, Router};
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;
use tokio::{net::TcpListener, select};
use turborepo_repository::package_graph::{PackageName, PackageNode};

use crate::{
    run::{builder::RunBuilder, Run},
    signal::SignalHandler,
};

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error("no signal handler")]
    NoSignalHandler,
    #[error("failed to start GraphQL server")]
    Server(#[from] io::Error),
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

#[derive(Debug, SimpleObject)]
struct Array<T: OutputType> {
    items: Vec<T>,
    length: usize,
}

unsafe impl<T: Send + OutputType> Send for Array<T> {}
unsafe impl<T: Sync + OutputType> Sync for Array<T> {}

impl<T: OutputType> FromIterator<T> for Array<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let items: Vec<_> = iter.into_iter().collect();
        let length = items.len();
        Self { items, length }
    }
}

struct Package {
    run: Arc<Run>,
    name: PackageName,
}

impl Package {
    fn direct_dependents_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .immediate_ancestors(&PackageNode::Workspace(self.name.clone()))
            .map_or(0, |pkgs| pkgs.len())
    }

    fn direct_dependencies_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .immediate_dependencies(&PackageNode::Workspace(self.name.clone()))
            .map_or(0, |pkgs| pkgs.len())
    }

    fn indirect_dependents_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());

        self.run.pkg_dep_graph().ancestors(&node).len() - self.direct_dependents_count()
    }

    fn indirect_dependencies_count(&self) -> usize {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());

        self.run.pkg_dep_graph().dependencies(&node).len() - self.direct_dependencies_count()
    }

    fn all_dependents_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .ancestors(&PackageNode::Workspace(self.name.clone()))
            .len()
    }

    fn all_dependencies_count(&self) -> usize {
        self.run
            .pkg_dep_graph()
            .dependencies(&PackageNode::Workspace(self.name.clone()))
            .len()
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
enum PackageFields {
    Name,
    DirectDependencyCount,
    DirectDependentCount,
    IndirectDependentCount,
    IndirectDependencyCount,
    AllDependentCount,
    AllDependencyCount,
}

#[derive(InputObject)]
struct FieldValuePair {
    field: PackageFields,
    value: Any,
}

/// Predicates are used to filter packages. If you include multiple predicates,
/// they are combined using AND. To combine predicates using OR, use the `or`
/// field.
///
/// For pairs that do not obey type safety, e.g. `NAME` `greater_than` `10`, we
/// default to `false`.
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
            (PackageFields::DirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.direct_dependencies_count() == n as usize
            }
            (PackageFields::DirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.direct_dependents_count() == n as usize
            }
            (PackageFields::IndirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.indirect_dependents_count() == n as usize
            }
            (PackageFields::IndirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.indirect_dependencies_count() == n as usize
            }
            (PackageFields::AllDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.all_dependents_count() == n as usize
            }
            (PackageFields::AllDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.all_dependencies_count() == n as usize
            }
            _ => false,
        }
    }

    fn check_greater_than(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::DirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.direct_dependencies_count() > n as usize
            }
            (PackageFields::DirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.direct_dependents_count() > n as usize
            }
            (PackageFields::IndirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.indirect_dependents_count() > n as usize
            }
            (PackageFields::IndirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.indirect_dependencies_count() > n as usize
            }
            (PackageFields::AllDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.all_dependents_count() > n as usize
            }
            (PackageFields::AllDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.all_dependencies_count() > n as usize
            }
            _ => false,
        }
    }

    fn check_less_than(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::DirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.direct_dependencies_count() < n as usize
            }
            (PackageFields::DirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.direct_dependents_count() < n as usize
            }
            (PackageFields::IndirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.indirect_dependents_count() < n as usize
            }
            (PackageFields::IndirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.indirect_dependencies_count() < n as usize
            }
            (PackageFields::AllDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.all_dependents_count() < n as usize
            }
            (PackageFields::AllDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else {
                    return false;
                };
                pkg.all_dependencies_count() < n as usize
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
    async fn affected_packages(
        &self,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<Array<Package>, Error> {
        let mut opts = self.run.opts().clone();
        opts.scope_opts.affected_range = Some((base, head));

        Ok(RunBuilder::calculate_filtered_packages(
            self.run.repo_root(),
            &opts,
            self.run.pkg_dep_graph(),
            self.run.scm(),
            self.run.root_turbo_json(),
        )?
        .into_iter()
        .map(|package| Package {
            run: self.run.clone(),
            name: package,
        })
        .sorted_by(|a, b| a.name.cmp(&b.name))
        .collect())
    }
    /// Gets a single package by name
    async fn package(&self, name: String) -> Result<Package, Error> {
        let name = PackageName::from(name);
        Ok(Package {
            run: self.run.clone(),
            name,
        })
    }

    /// Gets a list of packages that match the given filter
    async fn packages(&self, filter: Option<PackagePredicate>) -> Result<Array<Package>, Error> {
        let Some(filter) = filter else {
            return Ok(self
                .run
                .pkg_dep_graph()
                .packages()
                .map(|(name, _)| Package {
                    run: self.run.clone(),
                    name: name.clone(),
                })
                .sorted_by(|a, b| a.name.cmp(&b.name))
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
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }
}

#[Object]
impl Package {
    /// The name of the package
    async fn name(&self) -> String {
        self.name.to_string()
    }

    /// The path to the package, relative to the repository root
    async fn path(&self) -> Result<String, Error> {
        Ok(self
            .run
            .pkg_dep_graph()
            .package_info(&self.name)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?
            .package_path()
            .to_string())
    }

    /// The upstream packages that have this package as a direct dependency
    async fn direct_dependents(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self
            .run
            .pkg_dep_graph()
            .immediate_ancestors(&node)
            .iter()
            .flatten()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The downstream packages that directly depend on this package
    async fn direct_dependencies(&self) -> Result<Array<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        Ok(self
            .run
            .pkg_dep_graph()
            .immediate_dependencies(&node)
            .iter()
            .flatten()
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The downstream packages that depend on this package, transitively
    async fn all_dependents(&self) -> Result<Array<Package>, Error> {
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
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The upstream packages that this package depends on, transitively
    async fn all_dependencies(&self) -> Result<Array<Package>, Error> {
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
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The downstream packages that depend on this package, indirectly
    async fn indirect_dependents(&self) -> Result<Vec<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let immediate_dependents = self
            .run
            .pkg_dep_graph()
            .immediate_ancestors(&node)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?;

        Ok(self
            .run
            .pkg_dep_graph()
            .ancestors(&node)
            .iter()
            .filter(|package| !immediate_dependents.contains(*package))
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }

    /// The upstream packages that this package depends on, indirectly
    async fn indirect_dependencies(&self) -> Result<Vec<Package>, Error> {
        let node: PackageNode = PackageNode::Workspace(self.name.clone());
        let immediate_dependencies = self
            .run
            .pkg_dep_graph()
            .immediate_dependencies(&node)
            .ok_or_else(|| Error::PackageNotFound(self.name.clone()))?;

        Ok(self
            .run
            .pkg_dep_graph()
            .dependencies(&node)
            .iter()
            .filter(|package| !immediate_dependencies.contains(*package))
            .map(|package| Package {
                run: self.run.clone(),
                name: package.as_package_name().clone(),
            })
            .sorted_by(|a, b| a.name.cmp(&b.name))
            .collect())
    }
}

async fn graphiql() -> impl IntoResponse {
    response::Html(GraphiQLSource::build().endpoint("/").finish())
}

pub async fn run_server(run: Run, signal: SignalHandler) -> Result<(), Error> {
    let schema = Schema::new(Query::new(run), EmptyMutation, EmptySubscription);
    let app = Router::new().route("/", get(graphiql).post_service(GraphQL::new(schema)));

    let subscriber = signal.subscribe().ok_or(Error::NoSignalHandler)?;
    println!("GraphiQL IDE: http://localhost:8000");
    webbrowser::open("http://localhost:8000")?;
    select! {
        biased;
        _ = subscriber.listen() => {
            println!("Shutting down GraphQL server");
            return Ok(());
        }
        result = axum::serve(TcpListener::bind("127.0.0.1:8000").await?, app) => {
            result?;
        }
    }

    Ok(())
}
