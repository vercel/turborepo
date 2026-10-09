//! `turbo clean`: delete the outputs of selected tasks.
//!
//! Each task's outputs are resolved by the run cache itself
//! ([`turborepo_run_cache::task_output_files`]), so `clean` removes exactly
//! the files a cache restore writes back.
//!
//! Deleting user files is fail-closed. Anything that cannot be proven to be
//! a build output is reported and kept:
//!
//! - Output patterns must be anchored inside their package. `..` segments,
//!   leading wildcards (`**/*.js`, `*.*`, `?*`) and globs that would sweep a
//!   package directory are refused. `dist/**`, `{dist,build}/**` and
//!   `*.tsbuildinfo` are allowed.
//! - Matches are resolved to their on-disk names, one directory listing at a
//!   time, so a differently cased or Unicode-normalized spelling in a glob
//!   cannot hide a tracked file. Paths reached through a symlink, inside
//!   `node_modules`, `.git` or `.turbo`, inside a nested repository or a
//!   submodule, package directories, and git-tracked files (symlinks included)
//!   are never removed.
//! - If the set of tracked files cannot be read, nothing is removed.
//! - On unix, removal walks from the repository root with directory handles
//!   that never follow symlinks, so a directory swapped for a symlink after
//!   planning fails instead of being followed.

mod names;
mod patterns;
mod plan;
mod remove;
mod targets;
mod tracked;

use std::collections::BTreeSet;

use miette::Diagnostic;
use thiserror::Error;
use turbopath::AbsoluteSystemPath;
use turborepo_cache::{CacheActions, CacheConfig, CacheOpts};
use turborepo_engine::{Built, Engine};
use turborepo_repository::package_graph::PackageGraph;
use turborepo_types::TaskDefinition;

pub use crate::{
    plan::{CleanPlan, RemovalKind, SkipReason},
    remove::{Failure, Report},
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Failed to resolve the outputs of {task}: {source}")]
    Outputs {
        task: String,
        #[source]
        source: Box<turborepo_run_cache::Error>,
    },
    #[error("Could not determine which files git tracks, so nothing was deleted: {reason}")]
    #[diagnostic(
        code(turbo::clean::tracked_files_unknown),
        help(
            "`turbo clean` relies on the git index to tell source files from build outputs. Run \
             it inside a git repository whose index can be read."
        )
    )]
    TrackedFilesUnknown { reason: String },
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

/// What a clean would do, and why some tasks or patterns were left out.
#[derive(Debug)]
pub struct Planned {
    pub plan: CleanPlan,
    /// Tasks skipped as no-ops and output patterns that were refused.
    pub notices: Vec<String>,
}

/// Plans the removal of the outputs of every task in `engine`. Nothing is
/// deleted until [`CleanPlan::execute`] is called.
pub fn plan(
    repo_root: &AbsoluteSystemPath,
    engine: &Engine<Built, TaskDefinition>,
    package_graph: &PackageGraph,
) -> Result<Planned, Error> {
    let protected = targets::ProtectedDirectories::new(package_graph);
    let (candidates, notices) = targets::output_files(engine, package_graph, &protected)?;
    let plan = plan::plan_removals(repo_root, &protected, candidates)?;
    Ok(Planned { plan, notices })
}

/// Plans the removal of explicit candidate paths, as if every one of them
/// were a task output. Used by tests and by callers that resolve outputs
/// themselves.
pub fn plan_paths(
    repo_root: &AbsoluteSystemPath,
    package_directories: &[&str],
    candidates: BTreeSet<turbopath::AbsoluteSystemPathBuf>,
) -> Result<CleanPlan, Error> {
    let protected = targets::ProtectedDirectories::from_dirs(package_directories);
    plan::plan_removals(repo_root, &protected, candidates)
}

/// Whether a package-relative output pattern may be cleaned. `None` when it
/// is allowed, otherwise the reason it is refused.
pub fn refused_pattern(pattern: &str) -> Option<&'static str> {
    patterns::refusal(pattern, &[], &targets::ProtectedDirectories::from_dirs(&[]))
}

/// Planning a clean builds a `Run`, which must not touch the cache: no local
/// cache (so no eviction from `cacheMaxAge`/`cacheMaxSize` and no
/// `.turbo/cache` directory), and no remote cache.
pub fn disable_cache_for_planning(cache_opts: &mut CacheOpts) {
    cache_opts.cache = CacheConfig {
        local: CacheActions::disabled(),
        remote: CacheActions::disabled(),
    };
    cache_opts.cache_max_age = None;
    cache_opts.cache_max_size = None;
}

#[cfg(test)]
mod tests;
