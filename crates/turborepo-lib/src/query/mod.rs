mod file;
mod package;
mod server;
mod task;

use std::{
    io,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use async_graphql::{http::GraphiQLSource, *};
use axum::{response, response::IntoResponse};
use miette::Diagnostic;
use package::Package;
pub use server::run_server;
use thiserror::Error;
use tokio::select;
use turbo_trace::TraceError;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{change_mapper::AllPackageChangeReason, package_graph::PackageName};

use crate::{
    get_version,
    query::{file::File, task::RepositoryTask},
    run::{builder::RunBuilder, Run},
    signal::SignalHandler,
};

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error("failed to get file dependencies")]
    Trace(#[related] Vec<TraceError>),
    #[error("no signal handler")]
    NoSignalHandler,
    #[error("file `{0}` not found")]
    FileNotFound(String),
    #[error("failed to start GraphQL server")]
    Server(#[from] io::Error),
    #[error("package not found: {0}")]
    PackageNotFound(PackageName),
    #[error("failed to serialize result: {0}")]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Run(#[from] crate::run::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolution(#[from] crate::run::scope::filter::ResolutionError),
    #[error("failed to parse file: {0:?}")]
    Parse(swc_ecma_parser::error::Error),
}

pub struct RepositoryQuery {
    run: Arc<Run>,
}

impl RepositoryQuery {
    pub fn new(run: Arc<Run>) -> Self {
        Self { run }
    }
}

#[derive(Debug, SimpleObject)]
#[graphql(concrete(name = "RepositoryTasks", params(RepositoryTask)))]
#[graphql(concrete(name = "Packages", params(Package)))]
#[graphql(concrete(name = "ChangedPackages", params(ChangedPackage)))]
#[graphql(concrete(name = "Files", params(File)))]
#[graphql(concrete(name = "TraceErrors", params(file::TraceError)))]
pub struct Array<T: OutputType> {
    items: Vec<T>,
    length: usize,
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
            .map(|pair| Self::check_greater_than(pkg, &pair.field, &pair.value));
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

// why write few types when many work?
#[derive(SimpleObject)]
struct GlobalDepsChanged {
    // we're using slightly awkward names so we can reserve the nicer name for the "correct"
    // GraphQL type, e.g. a `file` field for the `File` type
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

impl From<turborepo_repository::change_mapper::PackageInclusionReason> for PackageChangeReason {
    fn from(value: turborepo_repository::change_mapper::PackageInclusionReason) -> Self {
        match value {
            turborepo_repository::change_mapper::PackageInclusionReason::All(
                AllPackageChangeReason::GlobalDepsChanged { file },
            ) => PackageChangeReason::GlobalDepsChanged(GlobalDepsChanged {
                file_path: file.to_string(),
            }),
            turborepo_repository::change_mapper::PackageInclusionReason::All(
                AllPackageChangeReason::DefaultGlobalFileChanged { file },
            ) => PackageChangeReason::DefaultGlobalFileChanged(DefaultGlobalFileChanged {
                file_path: file.to_string(),
            }),
            turborepo_repository::change_mapper::PackageInclusionReason::All(
                AllPackageChangeReason::LockfileChangeDetectionFailed,
            ) => {
                PackageChangeReason::LockfileChangeDetectionFailed(LockfileChangeDetectionFailed {
                    empty: false,
                })
            }
            turborepo_repository::change_mapper::PackageInclusionReason::All(
                AllPackageChangeReason::GitRefNotFound { from_ref, to_ref },
            ) => PackageChangeReason::GitRefNotFound(GitRefNotFound { from_ref, to_ref }),
            turborepo_repository::change_mapper::PackageInclusionReason::All(
                AllPackageChangeReason::LockfileChangedWithoutDetails,
            ) => {
                PackageChangeReason::LockfileChangedWithoutDetails(LockfileChangedWithoutDetails {
                    empty: false,
                })
            }
            turborepo_repository::change_mapper::PackageInclusionReason::All(
                AllPackageChangeReason::RootInternalDepChanged { root_internal_dep },
            ) => PackageChangeReason::RootInternalDepChanged(RootInternalDepChanged {
                root_internal_dep: root_internal_dep.to_string(),
            }),
            turborepo_repository::change_mapper::PackageInclusionReason::RootTask { task } => {
                PackageChangeReason::RootTask(RootTask {
                    task_name: task.to_string(),
                })
            }
            turborepo_repository::change_mapper::PackageInclusionReason::ConservativeRootLockfileChanged => {
                PackageChangeReason::ConservativeRootLockfileChanged(ConservativeRootLockfileChanged { empty: false })
            }
            turborepo_repository::change_mapper::PackageInclusionReason::LockfileChanged => {
                PackageChangeReason::LockfileChanged(LockfileChanged { empty: false })
            }
            turborepo_repository::change_mapper::PackageInclusionReason::DependencyChanged {
                dependency,
            } => PackageChangeReason::DependencyChanged(DependencyChanged {
                dependency_name: dependency.to_string(),
            }),
            turborepo_repository::change_mapper::PackageInclusionReason::DependentChanged {
                dependent,
            } => PackageChangeReason::DependentChanged(DependentChanged {
                dependent_name: dependent.to_string(),
            }),
            turborepo_repository::change_mapper::PackageInclusionReason::FileChanged { file } => {
                PackageChangeReason::FileChanged(FileChanged {
                    file_path: file.to_string(),
                })
            }
            turborepo_repository::change_mapper::PackageInclusionReason::InFilteredDirectory {
                directory,
            } => PackageChangeReason::InFilteredDirectory(InFilteredDirectory {
                directory_path: directory.to_string(),
            }),
            turborepo_repository::change_mapper::PackageInclusionReason::IncludedByFilter {
                filters,
            } => PackageChangeReason::IncludedByFilter(IncludedByFilter { filters }),
        }
    }
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
        let mut opts = self.run.opts().clone();
        opts.scope_opts.affected_range = Some((base, head));

        let mut packages = RunBuilder::calculate_filtered_packages(
            self.run.repo_root(),
            &opts,
            self.run.pkg_dep_graph(),
            self.run.scm(),
            self.run.root_turbo_json(),
        )?
        .into_iter()
        .map(|(package, reason)| {
            Ok(ChangedPackage {
                package: Package::new(self.run.clone(), package)?,
                reason: reason.into(),
            })
        })
        .filter(|package: &Result<ChangedPackage, Error>| {
            let Ok(package) = package.as_ref() else {
                return true;
            };
            filter.as_ref().map_or(true, |f| f.check(&package.package))
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
        get_version()
    }

    async fn file(&self, path: String) -> Result<File, Error> {
        let abs_path = AbsoluteSystemPathBuf::from_unknown(self.run.repo_root(), path);

        if !abs_path.exists() {
            return Err(Error::FileNotFound(abs_path.to_string()));
        }

        File::new(self.run.clone(), abs_path)
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
            .filter(|pkg| pkg.as_ref().map_or(false, |pkg| filter.check(pkg)))
            .collect::<Result<Array<_>, _>>()?;
        packages.sort_by(|a, b| a.get_name().cmp(b.get_name()));

        Ok(packages)
    }
}

pub async fn graphiql() -> impl IntoResponse {
    response::Html(GraphiQLSource::build().endpoint("/").finish())
}

pub async fn run_query_server(run: Run, signal: SignalHandler) -> Result<(), Error> {
    let subscriber = signal.subscribe().ok_or(Error::NoSignalHandler)?;
    println!("GraphiQL IDE: http://localhost:8000");
    webbrowser::open("http://localhost:8000")?;
    select! {
        biased;
        _ = subscriber.listen() => {
            println!("Shutting down GraphQL server");
            return Ok(());
        }
        result = server::run_server(None, Arc::new(run)) => {
            result?;
        }
    }

    Ok(())
}
