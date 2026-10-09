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
//!   `node_modules`, `.git` or `.turbo`, inside a nested repository, a
//!   submodule or a sparse-index directory, package directories, and
//!   git-tracked files (symlinks included) are never removed.
//! - Whether a match is a tracked file is decided by name and then by the
//!   filesystem: a match that is the same file (device and inode) as a tracked
//!   name in its directory is kept, whatever the filesystem's case and Unicode
//!   rules.
//! - The tracked set comes from the index git itself would use (`GIT_DIR`,
//!   `GIT_WORK_TREE`, `GIT_INDEX_FILE` and `core.worktree` are honored). If it
//!   cannot be read, is ambiguous, or does not track the repository's root
//!   `package.json` or `turbo.json`, nothing is removed.
//! - Untracked and gitignored files that match an allowed output pattern are
//!   build output and are removed.
//! - On unix, removal walks from the repository root with directory handles
//!   that never follow symlinks, and checks every directory and entry against
//!   the (device, inode) recorded when planning, so a path swapped or renamed
//!   after planning is skipped and reported instead of removed. Other platforms
//!   (Windows) only plan: deletion is refused.

mod ids;
mod names;
mod patterns;
mod plan;
mod remove;
mod targets;
mod tracked;

#[cfg(test)]
use std::collections::BTreeSet;

use miette::Diagnostic;
use thiserror::Error;
use turbopath::AbsoluteSystemPath;
use turborepo_cache::{CacheActions, CacheConfig, CacheOpts};
use turborepo_engine::{Built, Engine};
use turborepo_repository::package_graph::PackageGraph;
use turborepo_scm::GitEnvironment;
use turborepo_types::TaskDefinition;

pub use crate::{
    plan::{CleanPlan, RemovalKind, SkipReason},
    remove::{Failure, Report, ensure_deletion_supported},
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
             it inside a git repository whose index can be read and tracks the repository's \
             package.json or turbo.json. GIT_DIR, GIT_WORK_TREE, GIT_INDEX_FILE and core.worktree \
             are honored; GIT_DIR needs GIT_WORK_TREE or core.worktree."
        )
    )]
    TrackedFilesUnknown { reason: String },
    #[error(
        "`turbo clean` does not delete on Windows yet; use `turbo clean --dry-run` to list what \
         it would remove"
    )]
    #[diagnostic(code(turbo::clean::unsupported_platform))]
    DeletionUnsupported,
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
    let git = GitEnvironment::from_process().map_err(|error| Error::TrackedFilesUnknown {
        reason: error.to_string(),
    })?;
    let plan = plan::plan_removals(repo_root, &protected, candidates, &git)?;
    Ok(Planned { plan, notices })
}

/// Plans the removal of explicit candidate paths, as if every one of them
/// were a task output, with `git` as the git environment.
#[cfg(test)]
pub(crate) fn plan_paths_with(
    repo_root: &AbsoluteSystemPath,
    package_directories: &[&str],
    candidates: BTreeSet<turbopath::AbsoluteSystemPathBuf>,
    git: &GitEnvironment,
) -> Result<CleanPlan, Error> {
    let protected = targets::ProtectedDirectories::from_dirs(package_directories);
    plan::plan_removals(repo_root, &protected, candidates, git)
}

/// [`plan_paths_with`] run from `repo_root` with no git variables set.
#[cfg(test)]
pub(crate) fn plan_paths(
    repo_root: &AbsoluteSystemPath,
    package_directories: &[&str],
    candidates: BTreeSet<turbopath::AbsoluteSystemPathBuf>,
) -> Result<CleanPlan, Error> {
    let git = GitEnvironment {
        cwd: repo_root.as_std_path().to_owned(),
        ..GitEnvironment::default()
    };
    plan_paths_with(repo_root, package_directories, candidates, &git)
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
