//! Early cutoff for task hashes.
//!
//! With `futureFlags.hashDependencyOutputs`, a dependency that runs a command
//! and declares `outputs` contributes an *output fingerprint* to its
//! dependents' hashes in place of its task hash. A task hash describes what a
//! task consumed; the fingerprint describes what it produced. When a
//! dependency reruns because one of its inputs changed but writes
//! byte-identical outputs, its fingerprint is unchanged and so are its
//! dependents' hashes.
//!
//! A fingerprint covers:
//! - the dependency's output files (minus its log file), keyed by
//!   repository-relative path with git blob hashes, read from disk after the
//!   dependency ran or was restored,
//! - the package configuration files (`package.json`, `turbo.json`) that decide
//!   how consumers resolve those outputs,
//! - the package's external dependency fingerprint, since consumers resolve the
//!   dependency's third-party imports through it, and
//! - the dependency's own dependencies: their fingerprints when they qualify,
//!   otherwise their task hashes.
//!
//! The last point keeps the cutoff conservative. A dependency's outputs can
//! refer to its own dependencies' files without embedding them, so a change
//! anywhere below still reaches the dependent. A dependency without outputs
//! (for example a `transit` task, or a package whose sources its consumers
//! compile) contributes its full task hash, which includes its sources, and
//! the cutoff never skips over it.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_engine::{Built, Engine, TaskDefinitionInfo, TaskNode};
use turborepo_hash::FileHashes;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageTaskContext};
use turborepo_scm::SCM;
use turborepo_task_id::TaskId;
use turborepo_types::{
    TaskCommandOverride, TaskDefinitionHashInfo, sharable_workspace_relative_log_file,
};
use xxhash_rust::xxh64::Xxh64;

use crate::{Error, file_hashes_for_inputs};

/// Reported as the hash of a task whose hash waits for its dependencies'
/// output fingerprints.
pub const OUTPUT_FINGERPRINT_DEFERRED_TASK_HASH_MESSAGE: &str =
    "Deferred because hashDependencyOutputs hashes dependency outputs after they are built.";

/// Versions the fingerprint encoding. Changing the encoding must change this.
const FINGERPRINT_DOMAIN: &[u8] = b"turborepo-dependency-output-fingerprint-v1\0";

/// Whether `task_id`'s output fingerprint stands in for its task hash in its
/// dependents' hashes.
///
/// Only a dependency that runs a command and declares outputs qualifies. A
/// task without a command produces nothing of its own, and a task without
/// outputs may be depended on for its package's sources, so both keep
/// contributing their task hash.
pub fn contributes_output_fingerprint<T: TaskDefinitionHashInfo>(
    task_id: &TaskId<'static>,
    definition: &T,
    package_graph: &PackageGraph,
) -> bool {
    !definition.outputs().inclusions.is_empty()
        && resolves_command(task_id, definition, package_graph)
}

/// Mirrors the toolchain command provider: a `command` override is
/// authoritative, otherwise the package's native task catalog decides.
fn resolves_command<T: TaskDefinitionHashInfo>(
    task_id: &TaskId<'static>,
    definition: &T,
    package_graph: &PackageGraph,
) -> bool {
    match definition.command() {
        Some(TaskCommandOverride::OptOut) => false,
        Some(TaskCommandOverride::Argv(argv)) => !argv.is_empty(),
        None => package_graph
            .package_task_context(&PackageName::from(task_id.package()))
            .and_then(|context| context.native_tasks().get(task_id.task()))
            .is_some_and(|task| task.command().is_some()),
    }
}

/// The direct dependencies of `task_id` whose output fingerprints replace
/// their task hashes in `task_id`'s hash. Empty when the task hashes all of
/// its dependencies by task hash: when none qualify, or when the task
/// configures explicit `dependencyOutputs` inputs, which keep their own
/// semantics.
pub fn output_fingerprint_dependencies<T>(
    engine: &Engine<Built, T>,
    task_id: &TaskId<'static>,
    package_graph: &PackageGraph,
) -> Vec<TaskId<'static>>
where
    T: TaskDefinitionInfo + TaskDefinitionHashInfo + Clone,
{
    let Some(definition) = engine.task_definition(task_id) else {
        return Vec::new();
    };
    if definition.inputs().has_dependency_outputs() {
        return Vec::new();
    }
    let mut producers = engine
        .dependencies(task_id)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|node| match node {
            TaskNode::Task(dependency) => Some(dependency),
            TaskNode::Root => None,
        })
        .filter(|dependency| {
            engine
                .task_definition(dependency)
                .is_some_and(|definition| {
                    contributes_output_fingerprint(dependency, definition, package_graph)
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    producers.sort();
    producers.dedup();
    producers
}

/// Globs selecting a task's outputs for fingerprinting: its declared
/// inclusions and exclusions, minus its log file. Turborepo writes the log
/// file itself, and it records timing and cache status that differ between
/// otherwise identical runs.
pub fn output_fingerprint_globs<T: TaskDefinitionHashInfo>(
    task_id: &TaskId<'static>,
    definition: &T,
    package_context: &PackageTaskContext<'_>,
) -> Vec<String> {
    let outputs = definition.outputs();
    let log_file =
        sharable_workspace_relative_log_file(task_id.task(), package_context.log_namespace());
    outputs
        .inclusions
        .iter()
        .cloned()
        .chain(outputs.exclusions.iter().map(|glob| format!("!{glob}")))
        .chain(std::iter::once(format!("!{log_file}")))
        .collect()
}

/// Hashes the files a task produced, as they are on disk now, keyed by
/// repository-relative Unix path so the result is identical on every machine.
/// The package's configuration files are included alongside the outputs.
pub fn output_file_hashes<T: TaskDefinitionHashInfo>(
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    task_id: &TaskId<'static>,
    definition: &T,
    package_context: &PackageTaskContext<'_>,
) -> Result<FileHashes, Error> {
    let globs = output_fingerprint_globs(task_id, definition, package_context);
    let package_dir = package_context.directory();
    // The task has already run or been restored, so the repo index captured
    // at startup may hold stale hashes for the files it wrote.
    let hashes = file_hashes_for_inputs(scm, repo_root, package_dir, &globs, false, None)?;
    let package_root = repo_root.resolve(package_dir);
    let mut by_repo_path = BTreeMap::new();
    for (path, hash) in hashes.0.iter() {
        let full_path = package_root.join_unix_path(path);
        let repo_relative = AnchoredSystemPathBuf::relative_path_between(repo_root, &full_path);
        by_repo_path.insert(repo_relative.to_unix(), *hash);
    }
    Ok(FileHashes(by_repo_path.into_iter().collect()))
}

/// Encodes the parts of an output fingerprint unambiguously and hashes them.
pub(crate) fn fingerprint(
    task_id: &TaskId<'_>,
    external_deps_hash: Option<&str>,
    output_hashes: &FileHashes,
    dependency_hashes: &[Arc<str>],
) -> String {
    fn framed(hasher: &mut Xxh64, bytes: &[u8]) {
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }

    let mut hasher = Xxh64::new(0);
    hasher.update(FINGERPRINT_DOMAIN);
    framed(&mut hasher, task_id.to_string().as_bytes());
    match external_deps_hash {
        Some(hash) => {
            hasher.update(&[1]);
            framed(&mut hasher, hash.as_bytes());
        }
        None => hasher.update(&[0]),
    }
    hasher.update(&(output_hashes.0.len() as u64).to_le_bytes());
    for (path, hash) in &output_hashes.0 {
        framed(&mut hasher, path.as_str().as_bytes());
        framed(&mut hasher, hash.as_bytes());
    }
    hasher.update(&(dependency_hashes.len() as u64).to_le_bytes());
    for hash in dependency_hashes {
        framed(&mut hasher, hash.as_bytes());
    }
    hex::encode(hasher.digest().to_be_bytes())
}

/// Output fingerprints of a task's dependencies, keyed by dependency.
pub type OutputFingerprints = HashMap<TaskId<'static>, Arc<str>>;
