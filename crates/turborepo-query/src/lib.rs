#![allow(clippy::result_large_err)]

mod boundaries;
mod external_package;
mod file;
mod package;
mod package_graph;
mod server;
mod task;

use std::{
    collections::HashMap,
    io,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
};

use async_graphql::{http::GraphiQLSource, *};
use axum::{response, response::IntoResponse};
use external_package::ExternalPackage;
use itertools::Itertools;
use package::Package;
use package_graph::{Edge, PackageGraph};
pub use server::run_server;
use thiserror::Error;
use tokio::select;
use turbo_trace::TraceError;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_boundaries::BoundariesResult;
use turborepo_engine::Built;
use turborepo_repository::{
    change_mapper::{AllPackageChangeReason, PackageInclusionReason},
    package_graph::PackageName,
};
use turborepo_signals::SignalHandler;
use turborepo_types::TaskDefinition;

type BoundariesFuture<'a> = Pin<
    Box<
        dyn std::future::Future<Output = Result<BoundariesResult, turborepo_boundaries::Error>>
            + Send
            + 'a,
    >,
>;

/// The interface that the query layer requires from a "run" context.
///
/// This trait decouples the GraphQL query layer from the concrete `Run` type
/// in turborepo-lib, allowing the heavy async-graphql/axum/swc dependencies
/// to compile in a separate crate.
///
/// Object-safe so it can be used via `Arc<dyn QueryRun>`.
pub trait QueryRun: Send + Sync + 'static {
    fn version(&self) -> &'static str;
    fn repo_root(&self) -> &turbopath::AbsoluteSystemPath;
    fn pkg_dep_graph(&self) -> &turborepo_repository::package_graph::PackageGraph;
    fn engine(&self) -> &turborepo_engine::Engine<Built, TaskDefinition>;
    fn scm(&self) -> &turborepo_scm::SCM;
    fn root_turbo_json(&self) -> &turborepo_turbo_json::TurboJson;

    /// Calculate the set of affected packages given optional base/head git
    /// refs.
    ///
    /// This encapsulates the scope resolution and filtering logic that lives
    /// in the run builder, keeping `Opts` and `RunBuilder` out of the query
    /// crate's dependency graph.
    fn calculate_affected_packages(
        &self,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, AffectedPackagesError>;

    /// Check package boundary rules across all filtered packages.
    fn check_boundaries(&self, show_progress: bool) -> BoundariesFuture<'_>;
}

#[derive(Debug, Error)]
pub enum AffectedPackagesError {
    #[error(transparent)]
    Resolution(#[from] turborepo_scope::filter::ResolutionError),
    #[error("{0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Error, Debug, miette::Diagnostic)]
pub enum Error {
    #[error(transparent)]
    Boundaries(#[from] turborepo_boundaries::Error),
    #[error("Failed to get file dependencies")]
    Trace(#[related] Vec<TraceError>),
    #[error("No signal handler.")]
    NoSignalHandler,
    #[error("File `{0}` not found.")]
    FileNotFound(String),
    #[error("Failed to start GraphQL server.")]
    Server(#[from] io::Error),
    #[error("Package not found: {0}")]
    PackageNotFound(PackageName),
    #[error("Failed to serialize result: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Failed to calculate affected packages: {0}")]
    AffectedPackages(#[from] AffectedPackagesError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolution(#[from] turborepo_scope::filter::ResolutionError),
    #[error("Failed to parse file: {0:?}")]
    Parse(swc_ecma_parser::error::Error),
    #[error(transparent)]
    SignalListener(#[from] turborepo_signals::listeners::Error),
}

pub struct RepositoryQuery {
    run: Arc<dyn QueryRun>,
}

impl RepositoryQuery {
    pub fn new(run: Arc<dyn QueryRun>) -> Self {
        Self { run }
    }

    fn convert_change_reason(&self, reason: PackageInclusionReason) -> PackageChangeReason {
        match reason {
            PackageInclusionReason::All(AllPackageChangeReason::GlobalDepsChanged { file }) => {
                PackageChangeReason::GlobalDepsChanged(GlobalDepsChanged {
                    file_path: file.to_string(),
                })
            }
            PackageInclusionReason::All(AllPackageChangeReason::DefaultGlobalFileChanged {
                file,
            }) => PackageChangeReason::DefaultGlobalFileChanged(DefaultGlobalFileChanged {
                file_path: file.to_string(),
            }),
            PackageInclusionReason::All(AllPackageChangeReason::LockfileChangeDetectionFailed) => {
                PackageChangeReason::LockfileChangeDetectionFailed(LockfileChangeDetectionFailed {
                    empty: false,
                })
            }
            PackageInclusionReason::All(AllPackageChangeReason::LockfileChangedWithoutDetails) => {
                PackageChangeReason::LockfileChangedWithoutDetails(LockfileChangedWithoutDetails {
                    empty: false,
                })
            }
            PackageInclusionReason::All(AllPackageChangeReason::RootInternalDepChanged {
                root_internal_dep,
            }) => PackageChangeReason::RootInternalDepChanged(RootInternalDepChanged {
                root_internal_dep: root_internal_dep.to_string(),
            }),
            PackageInclusionReason::All(AllPackageChangeReason::GitRefNotFound {
                from_ref,
                to_ref,
            }) => PackageChangeReason::GitRefNotFound(GitRefNotFound { from_ref, to_ref }),
            PackageInclusionReason::RootTask { task } => PackageChangeReason::RootTask(RootTask {
                task_name: task.to_string(),
            }),
            PackageInclusionReason::ConservativeRootLockfileChanged => {
                PackageChangeReason::ConservativeRootLockfileChanged(
                    ConservativeRootLockfileChanged { empty: false },
                )
            }
            PackageInclusionReason::LockfileChanged { removed, added } => {
                let removed = removed
                    .into_iter()
                    .map(|package| ExternalPackage::new(self.run.clone(), package))
                    .collect::<Array<_>>();
                let added = added
                    .into_iter()
                    .map(|package| ExternalPackage::new(self.run.clone(), package))
                    .collect::<Array<_>>();
                PackageChangeReason::LockfileChanged(LockfileChanged {
                    empty: false,
                    removed,
                    added,
                })
            }
            PackageInclusionReason::DependencyChanged { dependency } => {
                PackageChangeReason::DependencyChanged(DependencyChanged {
                    dependency_name: dependency.to_string(),
                })
            }
            PackageInclusionReason::DependentChanged { dependent } => {
                PackageChangeReason::DependentChanged(DependentChanged {
                    dependent_name: dependent.to_string(),
                })
            }
            PackageInclusionReason::FileChanged { file } => {
                PackageChangeReason::FileChanged(FileChanged {
                    file_path: file.to_string(),
                })
            }
            PackageInclusionReason::InFilteredDirectory { directory } => {
                PackageChangeReason::InFilteredDirectory(InFilteredDirectory {
                    directory_path: directory.to_string(),
                })
            }
            PackageInclusionReason::IncludedByFilter { filters } => {
                PackageChangeReason::IncludedByFilter(IncludedByFilter { filters })
            }
        }
    }
}

#[derive(Debug, SimpleObject)]
#[graphql(concrete(name = "RepositoryTasks", params(task::RepositoryTask)))]
#[graphql(concrete(name = "Packages", params(Package)))]
#[graphql(concrete(name = "ChangedPackages", params(ChangedPackage)))]
#[graphql(concrete(name = "Files", params(file::File)))]
#[graphql(concrete(name = "ExternalPackages", params(ExternalPackage)))]
#[graphql(concrete(name = "Diagnostics", params(Diagnostic)))]
#[graphql(concrete(name = "Edges", params(Edge)))]
pub struct Array<T: OutputType> {
    items: Vec<T>,
    length: usize,
}

impl<T: ObjectType> From<Vec<T>> for Array<T> {
    fn from(value: Vec<T>) -> Self {
        Self {
            length: value.len(),
            items: value,
        }
    }
}

impl<T: OutputType> Deref for Array<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl<T: OutputType> DerefMut for Array<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.items
    }
}

impl<T: OutputType> FromIterator<T> for Array<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let items: Vec<_> = iter.into_iter().collect();
        let length = items.len();
        Self { items, length }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
enum PackageFields {
    Name,
    TaskName,
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
    has: Option<FieldValuePair>,
}

impl PackagePredicate {
    fn check_equals(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::Name, Value::String(name)) => pkg.get_name().as_ref() == name,
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
                let Some(n) = n.as_u64() else { return false };
                pkg.direct_dependencies_count() > n as usize
            }
            (PackageFields::DirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.direct_dependents_count() > n as usize
            }
            (PackageFields::IndirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.indirect_dependents_count() > n as usize
            }
            (PackageFields::IndirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.indirect_dependencies_count() > n as usize
            }
            (PackageFields::AllDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.all_dependents_count() > n as usize
            }
            (PackageFields::AllDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.all_dependencies_count() > n as usize
            }
            _ => false,
        }
    }

    fn check_less_than(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::DirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.direct_dependencies_count() < n as usize
            }
            (PackageFields::DirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.direct_dependents_count() < n as usize
            }
            (PackageFields::IndirectDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.indirect_dependents_count() < n as usize
            }
            (PackageFields::IndirectDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.indirect_dependencies_count() < n as usize
            }
            (PackageFields::AllDependentCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.all_dependents_count() < n as usize
            }
            (PackageFields::AllDependencyCount, Value::Number(n)) => {
                let Some(n) = n.as_u64() else { return false };
                pkg.all_dependencies_count() < n as usize
            }
            _ => false,
        }
    }

    fn check_has(pkg: &Package, field: &PackageFields, value: &Any) -> bool {
        match (field, &value.0) {
            (PackageFields::Name, Value::String(name)) => pkg.get_name().as_str() == name,
            (PackageFields::TaskName, Value::String(name)) => pkg.get_tasks().contains_key(name),
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
            .map(|pair| Self::check_less_than(pkg, &pair.field, &pair.value));
        let not = self.not.as_ref().map(|predicate| !predicate.check(pkg));
        let has = self
            .has
            .as_ref()
            .map(|pair| Self::check_has(pkg, &pair.field, &pair.value));

        and.into_iter()
            .chain(or)
            .chain(equal)
            .chain(not_equal)
            .chain(greater_than)
            .chain(less_than)
            .chain(not)
            .chain(has)
            .all(|p| p)
    }
}

#[derive(SimpleObject)]
struct GlobalDepsChanged {
    file_path: String,
}

#[derive(SimpleObject)]
struct DefaultGlobalFileChanged {
    file_path: String,
}

#[derive(SimpleObject)]
struct LockfileChangeDetectionFailed {
    /// This is a nothing field
    empty: bool,
}

#[derive(SimpleObject)]
struct LockfileChangedWithoutDetails {
    /// This is a nothing field
    empty: bool,
}

#[derive(SimpleObject)]
struct RootInternalDepChanged {
    root_internal_dep: String,
}

#[derive(SimpleObject)]
struct NonPackageFileChanged {
    file: String,
}

#[derive(SimpleObject)]
struct GitRefNotFound {
    from_ref: Option<String>,
    to_ref: Option<String>,
}

#[derive(SimpleObject)]
struct IncludedByFilter {
    filters: Vec<String>,
}

#[derive(SimpleObject)]
struct RootTask {
    task_name: String,
}

#[derive(SimpleObject)]
struct ConservativeRootLockfileChanged {
    /// This is a nothing field
    empty: bool,
}

#[derive(SimpleObject)]
struct LockfileChanged {
    /// This is a nothing field
    empty: bool,
    removed: Array<ExternalPackage>,
    added: Array<ExternalPackage>,
}

#[derive(SimpleObject)]
struct DependencyChanged {
    dependency_name: String,
}

#[derive(SimpleObject)]
struct DependentChanged {
    dependent_name: String,
}

#[derive(SimpleObject)]
struct FileChanged {
    file_path: String,
}

#[derive(SimpleObject)]
struct InFilteredDirectory {
    directory_path: String,
}

#[derive(Union)]
enum PackageChangeReason {
    GlobalDepsChanged(GlobalDepsChanged),
    DefaultGlobalFileChanged(DefaultGlobalFileChanged),
    LockfileChangeDetectionFailed(LockfileChangeDetectionFailed),
    LockfileChangedWithoutDetails(LockfileChangedWithoutDetails),
    RootInternalDepChanged(RootInternalDepChanged),
    NonPackageFileChanged(NonPackageFileChanged),
    GitRefNotFound(GitRefNotFound),
    IncludedByFilter(IncludedByFilter),
    RootTask(RootTask),
    ConservativeRootLockfileChanged(ConservativeRootLockfileChanged),
    LockfileChanged(LockfileChanged),
    DependencyChanged(DependencyChanged),
    DependentChanged(DependentChanged),
    FileChanged(FileChanged),
    InFilteredDirectory(InFilteredDirectory),
}

#[derive(SimpleObject)]
struct ChangedPackage {
    reason: PackageChangeReason,
    #[graphql(flatten)]
    package: Package,
}

#[Object]
impl RepositoryQuery {
    async fn affected_packages(
        &self,
        base: Option<String>,
        head: Option<String>,
        filter: Option<PackagePredicate>,
    ) -> Result<Array<ChangedPackage>, Error> {
        let mut packages = self
            .run
            .calculate_affected_packages(base, head)?
            .into_iter()
            .map(|(package, reason)| {
                Ok(ChangedPackage {
                    package: Package::new(self.run.clone(), package)?,
                    reason: self.convert_change_reason(reason),
                })
            })
            .filter(|package: &Result<ChangedPackage, Error>| {
                let Ok(package) = package.as_ref() else {
                    return true;
                };
                filter.as_ref().is_none_or(|f| f.check(&package.package))
            })
            .collect::<Result<Array<_>, _>>()?;

        packages.sort_by(|a, b| a.package.get_name().cmp(b.package.get_name()));
        Ok(packages)
    }

    /// Gets a single package by name
    async fn package(&self, name: String) -> Result<Package, Error> {
        let name = PackageName::from(name);
        Package::new(self.run.clone(), name)
    }

    async fn version(&self) -> &'static str {
        self.run.version()
    }

    /// Check boundaries for all packages.
    async fn boundaries(&self) -> Result<Array<Diagnostic>, Error> {
        match self.run.check_boundaries(false).await {
            Ok(result) => Ok(result
                .diagnostics
                .into_iter()
                .map(|b| b.into())
                .sorted_by(|a: &Diagnostic, b: &Diagnostic| a.message.cmp(&b.message))
                .collect()),
            Err(err) => Err(Error::Boundaries(err)),
        }
    }

    async fn package_graph(
        &self,
        center: Option<String>,
        filter: Option<PackagePredicate>,
    ) -> PackageGraph {
        PackageGraph::new(self.run.clone(), center, filter)
    }

    async fn file(&self, path: String) -> Result<file::File, Error> {
        let abs_path = AbsoluteSystemPathBuf::from_unknown(self.run.repo_root(), path);

        if !abs_path.exists() {
            return Err(Error::FileNotFound(abs_path.to_string()));
        }

        file::File::new(self.run.clone(), abs_path)
    }

    /// Gets a list of packages that match the given filter
    async fn packages(&self, filter: Option<PackagePredicate>) -> Result<Array<Package>, Error> {
        let Some(filter) = filter else {
            let mut packages = self
                .run
                .pkg_dep_graph()
                .packages()
                .map(|(name, _)| Package::new(self.run.clone(), name.clone()))
                .collect::<Result<Array<_>, _>>()?;
            packages.sort_by(|a, b| a.get_name().cmp(b.get_name()));
            return Ok(packages);
        };

        let mut packages = self
            .run
            .pkg_dep_graph()
            .packages()
            .map(|(name, _)| Package::new(self.run.clone(), name.clone()))
            .filter(|pkg| pkg.as_ref().is_ok_and(|pkg| filter.check(pkg)))
            .collect::<Result<Array<_>, _>>()?;
        packages.sort_by(|a, b| a.get_name().cmp(b.get_name()));

        Ok(packages)
    }

    async fn external_dependencies(&self) -> Result<Array<ExternalPackage>, Error> {
        let pkg_dep_graph = self.run.pkg_dep_graph();
        let all_package_names: Vec<_> = pkg_dep_graph.packages().map(|(name, _)| name).collect();
        let mut packages = pkg_dep_graph
            .transitive_external_dependencies(all_package_names)
            .into_iter()
            .map(|pkg| ExternalPackage::new(self.run.clone(), pkg.clone()))
            .collect::<Array<_>>();
        packages.sort_by_key(|pkg| pkg.human_name());
        Ok(packages)
    }
}

pub async fn graphiql() -> impl IntoResponse {
    response::Html(
        GraphiQLSource::build()
            .version("5.0.0-rc.1")
            .endpoint("/")
            .finish(),
    )
}

pub async fn run_query_server(run: Arc<dyn QueryRun>, signal: SignalHandler) -> Result<(), Error> {
    let subscriber = signal.subscribe().ok_or(Error::NoSignalHandler)?;
    println!("GraphiQL IDE: http://localhost:8000");
    webbrowser::open("http://localhost:8000")?;
    select! {
        biased;
        _ = subscriber.listen() => {
            println!("Shutting down GraphQL server");
            return Ok(());
        }
        result = server::run_server(None, run) => {
            result?;
        }
    }

    Ok(())
}

#[derive(SimpleObject, Debug, Default)]
pub struct Diagnostic {
    pub message: String,
    pub reason: Option<String>,
    pub path: Option<String>,
    pub import: Option<String>,
    pub start: Option<usize>,
    pub end: Option<usize>,
}

/// An error with source location information from a GraphQL query.
pub struct QueryErrorLocation {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// The result of executing a GraphQL query.
pub struct QueryResult {
    pub result_json: String,
    pub errors: Vec<QueryErrorLocation>,
}

/// The standard GraphQL introspection query used by `turbo query --schema`.
pub const SCHEMA_QUERY: &str = "query IntrospectionQuery {
  __schema {
    queryType {
      name
    }
    mutationType {
      name
    }
    subscriptionType {
      name
    }
    types {
      ...FullType
    }
    directives {
      name
      description
      locations
      args {
        ...InputValue
      }
    }
  }
}

fragment FullType on __Type {
  kind
  name
  description
  fields(includeDeprecated: true) {
    name
    description
    args {
      ...InputValue
    }
    type {
      ...TypeRef
    }
    isDeprecated
    deprecationReason
  }
  inputFields {
    ...InputValue
  }
  interfaces {
    ...TypeRef
  }
  enumValues(includeDeprecated: true) {
    name
    description
    isDeprecated
    deprecationReason
  }
  possibleTypes {
    ...TypeRef
  }
}

fragment InputValue on __InputValue {
  name
  description
  type {
    ...TypeRef
  }
  defaultValue
}

fragment TypeRef on __Type {
  kind
  name
  ofType {
    kind
    name
    ofType {
      kind
      name
      ofType {
        kind
        name
        ofType {
          kind
          name
          ofType {
            kind
            name
            ofType {
              kind
              name
              ofType {
                kind
                name
              }
            }
          }
        }
      }
    }
  }
}";

/// Execute a GraphQL query against the repository and return the result as
/// JSON.
///
/// This is the high-level entry point that hides the `async-graphql` schema
/// machinery from callers. `variables_json` is an optional raw JSON string
/// for query variables.
pub async fn execute_query(
    run: Arc<dyn QueryRun>,
    query: &str,
    variables_json: Option<&str>,
) -> Result<QueryResult, Error> {
    let schema = Schema::new(RepositoryQuery::new(run), EmptyMutation, EmptySubscription);

    let variables: Variables = variables_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(Error::Serde)?
        .unwrap_or_default();

    let request = Request::new(query).variables(variables);
    let result = schema.execute(request).await;

    let result_json = serde_json::to_string_pretty(&result).map_err(Error::Serde)?;

    let errors = result
        .errors
        .into_iter()
        .filter_map(|e| {
            let loc = e.locations.first()?;
            Some(QueryErrorLocation {
                message: e.message,
                line: loc.line,
                column: loc.column,
            })
        })
        .collect();

    Ok(QueryResult {
        result_json,
        errors,
    })
}
