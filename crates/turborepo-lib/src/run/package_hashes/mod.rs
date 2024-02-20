use std::{collections::HashMap, sync::Arc};

use rayon::prelude::*;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPath, RelativeUnixPathBuf};
use turborepo_errors::Spanned;
use turborepo_repository::{
    discovery::PackageDiscoveryBuilder,
    package_graph::{self, PackageGraph, PackageInfo, PackageName},
    package_json::{self, PackageJson},
    package_manager,
};
use turborepo_scm::SCM;
use turborepo_telemetry::events::generic::GenericEventBuilder;

use super::task_id::TaskId;
use crate::{
    config,
    engine::{EngineBuilder, TaskNode},
    hash::FileHashes,
    run::error::Error,
    task_hash::{FileHashInputs, PackageInputsHashes},
    turbo_json::TurboJson,
    DaemonClient,
};

pub trait PackageHasher {
    /// Calculate the package-task hashes for the given tasks and packages.
    /// This will yield a hash for every pair of task and package that
    /// exists in the workspace.
    ///
    /// `tasks`: The tasks to calculate hashes for
    /// `packages`: The packages to calculate hashes for
    fn calculate_hashes(
        &self,
        run_telemetry: GenericEventBuilder,
        tasks: Vec<TaskNode>,
    ) -> impl std::future::Future<Output = Result<PackageInputsHashes, Error>> + Send;
}

impl<T: PackageHasher> PackageHasher for Arc<T> {
    fn calculate_hashes(
        &self,
        run_telemetry: GenericEventBuilder,
        tasks: Vec<TaskNode>,
    ) -> impl std::future::Future<Output = Result<PackageInputsHashes, Error>> + Send {
        self.as_ref().calculate_hashes(run_telemetry, tasks)
    }
}

/// We want to allow for lazily generating the PackageDiscovery implementation
/// to prevent unnecessary work. This trait allows us to do that.
///
/// Note: there is a blanket implementation for everything that implements
/// PackageDiscovery
pub trait PackageHasherBuilder {
    type Output: PackageHasher;
    type Error: std::error::Error;

    fn build(self) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send;
}

impl<T: PackageHasher + Send> PackageHasherBuilder for T {
    type Output = T;
    type Error = std::convert::Infallible;

    async fn build(self) -> Result<Self::Output, Self::Error> {
        Ok(self)
    }
}

pub struct LocalPackageHasherBuilder<PDB: PackageDiscoveryBuilder + Sync> {
    pub repo_root: AbsoluteSystemPathBuf,
    pub discovery: PDB,
    pub scm: SCM,
}

#[derive(thiserror::Error, Debug)]
pub enum LocalPackageHasherBuilderError {
    #[error("package.json not found")]
    MissingPackageJson(#[from] package_json::Error),
    #[error("turbo.json not found")]
    MissingTurboJson(#[from] config::Error),
    #[error("unable to build package graph: {0}")]
    PackageGraphError(#[from] package_graph::Error),
}

impl<PDB> PackageHasherBuilder for LocalPackageHasherBuilder<PDB>
where
    PDB: PackageDiscoveryBuilder + Sync + Send,
    PDB::Output: Send + Sync,
    PDB::Error: Into<package_manager::Error>,
{
    type Output = LocalPackageHashes;
    type Error = LocalPackageHasherBuilderError;

    async fn build(self) -> Result<Self::Output, Self::Error> {
        let package_json_path = self.repo_root.join_component("package.json");
        let root_package_json = PackageJson::load(&package_json_path)?;
        let root_turbo_json = TurboJson::load(
            &self.repo_root,
            AnchoredSystemPath::empty(),
            &root_package_json,
            false,
        )?;

        let pkg_dep_graph = PackageGraph::builder(&self.repo_root, root_package_json)
            .with_package_discovery(self.discovery)
            .build()
            .await?;

        let engine = EngineBuilder::new(&self.repo_root, &pkg_dep_graph, false)
            .with_root_tasks(root_turbo_json.pipeline.keys().cloned())
            .with_tasks(
                root_turbo_json
                    .pipeline
                    .keys()
                    .map(|name| Spanned::new(name.clone())),
            )
            .with_turbo_jsons(Some(
                [(PackageName::Root, root_turbo_json)].into_iter().collect(),
            ))
            .with_workspaces(
                pkg_dep_graph
                    .packages()
                    .map(|(name, _)| name.to_owned())
                    .collect(),
            )
            .build()
            .unwrap();

        Ok(LocalPackageHashes::new(
            self.scm,
            pkg_dep_graph
                .packages()
                .map(|(name, info)| (name.to_owned(), info.to_owned()))
                .collect(),
            engine
                .task_definitions()
                .iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned().into()))
                .collect(),
            self.repo_root,
        ))
    }
}

#[derive(Clone)]
pub struct LocalPackageHashes {
    scm: SCM,
    workspaces: HashMap<PackageName, PackageInfo>,
    task_definitions: HashMap<TaskId<'static>, FileHashInputs>,
    repo_root: AbsoluteSystemPathBuf,
}

impl LocalPackageHashes {
    pub fn new(
        scm: SCM,
        workspaces: HashMap<PackageName, PackageInfo>,
        task_definitions: HashMap<TaskId<'static>, FileHashInputs>,
        repo_root: AbsoluteSystemPathBuf,
    ) -> Self {
        tracing::debug!(
            "creating new local package hasher with {} definitions across {} workspaces",
            task_definitions.len(),
            workspaces.len()
        );

        Self {
            scm,
            workspaces,
            task_definitions,
            repo_root,
        }
    }
}

impl PackageHasher for LocalPackageHashes {
    async fn calculate_hashes(
        &self,
        run_telemetry: GenericEventBuilder,
        tasks: Vec<TaskNode>,
    ) -> Result<PackageInputsHashes, Error> {
        tracing::debug!("running local package hash discovery in {}", self.repo_root);
        let package_inputs_hashes = PackageInputsHashes::calculate_file_hashes(
            &self.scm,
            tasks.par_iter(),
            &self.workspaces,
            &self.task_definitions,
            &self.repo_root,
            &run_telemetry,
        )?;
        Ok(package_inputs_hashes)
    }
}

impl<T: PackageHasher + Send + Sync> PackageHasher for Option<T> {
    async fn calculate_hashes(
        &self,
        run_telemetry: GenericEventBuilder,
        tasks: Vec<TaskNode>,
    ) -> Result<PackageInputsHashes, Error> {
        tracing::debug!("hashing packages using optional strategy");

        match self {
            Some(d) => d.calculate_hashes(run_telemetry, tasks).await,
            None => {
                tracing::debug!("no strategy available");
                Err(Error::PackageHashingUnavailable)
            }
        }
    }
}

/// Attempts to run the `primary` strategy for an amount of time
/// specified by `timeout` before falling back to `fallback`
pub struct FallbackPackageHasher<P, F> {
    primary: P,
    fallback: F,
    timeout: std::time::Duration,
}

impl<P: PackageHasher, F: PackageHasher> FallbackPackageHasher<P, F> {
    pub fn new(primary: P, fallback: F, timeout: std::time::Duration) -> Self {
        Self {
            primary,
            fallback,
            timeout,
        }
    }
}

impl<A: PackageHasher + Send + Sync, B: PackageHasher + Send + Sync> PackageHasher
    for FallbackPackageHasher<A, B>
{
    async fn calculate_hashes(
        &self,
        run_telemetry: GenericEventBuilder,
        tasks: Vec<TaskNode>,
    ) -> Result<PackageInputsHashes, Error> {
        tracing::debug!("discovering packages using fallback strategy");

        tracing::debug!("attempting primary strategy");
        match tokio::time::timeout(
            self.timeout,
            self.primary
                .calculate_hashes(run_telemetry.clone(), tasks.clone()),
        )
        .await
        {
            Ok(Ok(packages)) => Ok(packages),
            Ok(Err(err1)) => {
                tracing::debug!("primary strategy failed, attempting fallback strategy");
                match self.fallback.calculate_hashes(run_telemetry, tasks).await {
                    Ok(packages) => Ok(packages),
                    // if the backup is unavailable, return the original error
                    Err(Error::PackageHashingUnavailable) => Err(err1),
                    Err(err2) => Err(err2),
                }
            }
            Err(_) => {
                tracing::debug!("primary strategy timed out, attempting fallback strategy");
                self.fallback.calculate_hashes(run_telemetry, tasks).await
            }
        }
    }
}

pub struct DaemonPackageHasher<C> {
    daemon: DaemonClient<C>,
}

impl<C: Clone + Send + Sync> PackageHasher for DaemonPackageHasher<C> {
    async fn calculate_hashes(
        &self,
        _run_telemetry: GenericEventBuilder,
        tasks: Vec<TaskNode>,
    ) -> Result<PackageInputsHashes, Error> {
        // clone to avoid using a mutex or a mutable reference
        let mut daemon = self.daemon.clone();
        let package_hashes = daemon.discover_package_hashes(tasks).await;

        package_hashes
            .map(|resp| {
                let file_hashes: HashMap<_, _> = resp
                    .file_hashes
                    .into_iter()
                    .map(|fh| (fh.relative_path, fh.hash))
                    .collect();

                let (expanded_hashes, hashes) = resp
                    .package_hashes
                    .into_iter()
                    .filter_map(|ph| Some((ph.task_id?, ph.hash, ph.files)))
                    .map(|(task_id, hash, files)| {
                        (
                            (
                                TaskId::new(&task_id.package, &task_id.task).into_owned(),
                                FileHashes(
                                    files
                                        .into_iter()
                                        .filter_map(|f| {
                                            file_hashes.get(&f).map(|hash| {
                                                (
                                                    RelativeUnixPathBuf::new(f).unwrap(),
                                                    hash.to_owned(),
                                                )
                                            })
                                        })
                                        .collect(),
                                ),
                            ),
                            (TaskId::from_owned(task_id.package, task_id.task), hash),
                        )
                    })
                    .unzip();

                PackageInputsHashes {
                    expanded_hashes,
                    hashes,
                }
            })
            .map_err(|_| Error::PackageHashingUnavailable)
    }
}

impl<C> DaemonPackageHasher<C> {
    pub fn new(daemon: DaemonClient<C>) -> Self {
        Self { daemon }
    }
}
