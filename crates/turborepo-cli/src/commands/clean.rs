//! `turbo clean`: delete the resolved outputs of selected tasks and, with
//! `--cache`, the entries of the local filesystem cache.
//!
//! Tasks are planned with the same `RunBuilder` path `turbo run --dry` uses,
//! and each task's outputs come from the run cache's own resolution
//! (`repo_relative_hashable_outputs` expanded by `globwalk`), so `clean`
//! removes exactly what a cache restore would write back.
//!
//! Deleting user files demands guards beyond what caching needs. A path is
//! never removed when it is outside the repository, reached through a
//! symlink, a package directory (or an ancestor of one), inside
//! `node_modules`, `.git` or `.turbo`, or tracked by git. Directories are
//! removed only once everything inside them has been removed.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use miette::Diagnostic;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_engine::task_has_command;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_run::{Run, builder::RunBuilder};
use turborepo_scm::{RepoGitIndex, SCM};
use turborepo_signals::{SignalHandler, listeners::get_signal};
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_types::{
    TaskDefinition, TaskDefinitionExt, TaskOutputs, TaskOutputsExt,
    sharable_workspace_relative_log_file,
};

use super::CommandBase;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Run(#[from] turborepo_run::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    SignalListener(#[from] turborepo_signals::listeners::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error("Invalid output glob for {task}: {source}")]
    Glob {
        task: String,
        #[source]
        source: globwalk::GlobError,
    },
    #[error("Failed to expand outputs for {task}: {source}")]
    Walk {
        task: String,
        #[source]
        source: globwalk::WalkError,
    },
    #[error(
        "Could not read the git index, so tracked files cannot be protected. Nothing was deleted: \
         {0}"
    )]
    GitIndex(#[source] turborepo_scm::Error),
    #[error("Failed to inspect {path}: {source}")]
    Inspect {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to remove {path}: {source}")]
    Remove {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "The cache directory {path} is outside the repository root. `turbo clean` never deletes \
         outside the repository; remove it manually if intended."
    )]
    CacheOutsideRepo { path: String },
    #[error("Failed to clear the cache directory {path}: {source}")]
    Cache {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanOptions {
    /// Report what would be removed without removing anything.
    pub dry_run: bool,
    /// Clear the local filesystem cache.
    pub cache: bool,
    /// Clean the outputs of the selected tasks.
    pub clean_outputs: bool,
}

pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    options: CleanOptions,
) -> Result<(), Error> {
    let repo_root = base.repo_root.clone();

    if options.clean_outputs {
        let signal = get_signal()?;
        let handler = SignalHandler::new(signal);
        // Plan exactly like `turbo run --dry`: nothing is hashed or executed,
        // and engine validation only concerns execution (e.g. persistent
        // dependencies), so it is skipped like `turbo boundaries` does.
        let (run, _analytics) = RunBuilder::new(base.run_builder_input()?, None)?
            .skip_repo_index_and_scm_state()
            .do_not_validate_engine()
            .build(&handler, telemetry)
            .await?;

        let (plan, notices) = plan_for_run(&run)?;
        for notice in &notices {
            println!("{notice}");
        }
        print_plan(&repo_root, &plan, options.dry_run);
        if !options.dry_run {
            plan.execute()?;
        }
    }

    if options.cache {
        let cache_dir = AbsoluteSystemPathBuf::from_unknown(
            &repo_root,
            base.opts().cache_opts.cache_dir.clone(),
        );
        clean_cache_dir(&repo_root, &cache_dir, options.dry_run)?;
    }

    Ok(())
}

/// One task's outputs, as the run cache resolves them.
#[derive(Debug, Clone)]
struct OutputTarget {
    task: String,
    /// Repository-relative output globs, `!` exclusions included.
    outputs: TaskOutputs,
}

fn plan_for_run(run: &Run) -> Result<(CleanPlan, Vec<String>), Error> {
    let repo_root = run.repo_root();
    let (targets, mut notices) = output_targets(run.engine(), run.pkg_dep_graph());
    let protected = ProtectedDirectories::new(run.pkg_dep_graph());
    let candidates = collect_candidates(repo_root, &targets, &protected, &mut notices)?;
    // Only read the git index when there is something to protect.
    let tracked = if candidates.is_empty() {
        None
    } else {
        TrackedFiles::load(run.scm())?
    };
    let plan = plan_removals(repo_root, &protected, candidates, tracked.as_ref())?;
    Ok((plan, notices))
}

/// Resolves the outputs of every planned task. Tasks that never write
/// cacheable outputs are reported and skipped.
fn output_targets(
    engine: &turborepo_engine::Engine<turborepo_engine::Built, TaskDefinition>,
    package_graph: &PackageGraph,
) -> (Vec<OutputTarget>, Vec<String>) {
    let mut task_ids: Vec<&TaskId<'static>> = engine.task_ids().collect();
    task_ids.sort_by_key(|task_id| task_id.to_string());

    let mut targets = Vec::new();
    let mut skipped: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for task_id in task_ids {
        // A package without the script never runs the task, so it has no
        // outputs to clean. This is the common case and stays quiet.
        if !task_has_command(engine, package_graph, task_id) {
            continue;
        }
        let (Some(definition), Some(context)) = (
            engine.task_definition(task_id),
            package_graph.package_task_context(&PackageName::from(task_id.package())),
        ) else {
            continue;
        };
        let task = task_id.to_string();
        if definition.persistent {
            skipped.entry("persistent").or_default().push(task);
            continue;
        }
        if !definition.cache {
            skipped.entry("caching disabled").or_default().push(task);
            continue;
        }

        let namespace = context.log_namespace();
        let mut outputs =
            definition.repo_relative_hashable_outputs(task_id, context.directory(), namespace);
        // The task log is turbo's own bookkeeping, not a build artifact.
        let log_glob = repo_relative_glob(
            context.directory().as_str(),
            sharable_workspace_relative_log_file(task_id.task(), namespace).as_str(),
        );
        outputs.inclusions.retain(|glob| *glob != log_glob);
        if outputs.inclusions.is_empty() {
            skipped.entry("no outputs").or_default().push(task);
            continue;
        }
        targets.push(OutputTarget { task, outputs });
    }
    let notices = skipped
        .into_iter()
        .map(|(reason, tasks)| format!("• Skipping tasks ({reason}): {}", tasks.join(", ")))
        .collect();
    (targets, notices)
}

/// Mirrors how `repo_relative_hashable_outputs` anchors a package-relative
/// glob at the package directory.
fn repo_relative_glob(package_dir: &str, glob: &str) -> String {
    if package_dir.is_empty() {
        glob.to_owned()
    } else {
        format!("{package_dir}{}{glob}", std::path::MAIN_SEPARATOR)
    }
}

/// Expands each target's output globs with the same walk the run cache uses
/// when saving outputs. Globs that would sweep a whole package are refused.
fn collect_candidates(
    repo_root: &AbsoluteSystemPath,
    targets: &[OutputTarget],
    protected: &ProtectedDirectories,
    notices: &mut Vec<String>,
) -> Result<BTreeSet<AbsoluteSystemPathBuf>, Error> {
    let mut candidates = BTreeSet::new();
    for target in targets {
        let mut outputs = target.outputs.clone();
        outputs.inclusions.retain(|glob| {
            let sweeping = protected.is_swept_by(glob);
            if sweeping {
                notices.push(format!(
                    "• {}: not cleaning `{glob}` (it would match a whole package directory)",
                    target.task
                ));
            }
            !sweeping
        });
        if outputs.inclusions.is_empty() {
            continue;
        }

        let glob_error = |source| Error::Glob {
            task: target.task.clone(),
            source,
        };
        let inclusions = outputs.validated_inclusions().map_err(glob_error)?;
        let exclusions = outputs.validated_exclusions().map_err(glob_error)?;
        let matches =
            globwalk::globwalk(repo_root, &inclusions, &exclusions, globwalk::WalkType::All)
                .map_err(|source| Error::Walk {
                    task: target.task.clone(),
                    source,
                })?;
        candidates.extend(matches);
    }
    Ok(candidates)
}

/// Package directories, the repository root among them. Neither they nor
/// any of their ancestors may be removed.
struct ProtectedDirectories {
    directories: Vec<Vec<String>>,
}

impl ProtectedDirectories {
    fn new(package_graph: &PackageGraph) -> Self {
        let mut directories = vec![Vec::new()];
        directories.extend(
            package_graph
                .package_task_contexts()
                .map(|context| path_segments(context.directory().as_str())),
        );
        Self { directories }
    }

    #[cfg(test)]
    fn from_dirs(dirs: &[&str]) -> Self {
        let mut directories = vec![Vec::new()];
        directories.extend(dirs.iter().map(|dir| path_segments(dir)));
        Self { directories }
    }

    /// Whether `segments` names a protected directory or one of its
    /// ancestors.
    fn covers(&self, segments: &[&str]) -> bool {
        self.directories.iter().any(|directory| {
            directory.len() >= segments.len()
                && directory
                    .iter()
                    .zip(segments)
                    .all(|(protected, segment)| same_name(protected, segment))
        })
    }

    /// Whether a repository-relative output glob would match a protected
    /// directory itself or recursively sweep one: its literal base is a
    /// protected directory (or an ancestor) and the rest is empty, recursive
    /// (`**`), or a bare `*`.
    fn is_swept_by(&self, glob: &str) -> bool {
        let segments = path_segments(glob);
        let first_glob = segments
            .iter()
            .position(|segment| globwalk::is_glob_pattern(segment))
            .unwrap_or(segments.len());
        let mut base: Vec<&str> = Vec::new();
        for segment in &segments[..first_glob] {
            if segment == ".." {
                // Escaping the repository is never cleaned.
                if base.pop().is_none() {
                    return true;
                }
            } else {
                base.push(segment);
            }
        }
        if !self.covers(&base) {
            return false;
        }
        let rest = &segments[first_glob..];
        rest.is_empty()
            || rest.iter().any(|segment| segment.contains("**"))
            || (rest.len() == 1 && rest[0] == "*")
    }
}

fn path_segments(path: &str) -> Vec<String> {
    path.split(['/', std::path::MAIN_SEPARATOR])
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(str::to_owned)
        .collect()
}

/// Compare path names the way the platform's default filesystem does, so a
/// differently-cased glob cannot slip past a guard.
fn same_name(a: &str, b: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Files tracked by git. These are source, never build output, so they are
/// never removed even when an output glob matches them.
struct TrackedFiles {
    real_git_root: AbsoluteSystemPathBuf,
    index: RepoGitIndex,
}

impl TrackedFiles {
    /// `None` outside git. Fails closed: if the index of a git repository
    /// cannot be read, nothing is deleted.
    fn load(scm: &SCM) -> Result<Option<Self>, Error> {
        let Some(git_root) = scm.git_root() else {
            return Ok(None);
        };
        let Some(index) = scm.tracked_repo_index().map_err(Error::GitIndex)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            real_git_root: git_root.to_realpath()?,
            index,
        }))
    }

    fn contains(&self, real_path: &AbsoluteSystemPath) -> bool {
        self.real_git_root
            .anchor(real_path)
            .is_ok_and(|relative| self.index.is_tracked(&relative.to_unix()))
    }
}

/// Directories owned by other tools, or by turbo itself, whose contents are
/// never task output to clean: dependencies (`node_modules`, the package
/// manager's domain), the git repository, and turbo's own state (`.turbo`:
/// task logs, the default cache, local config). Task logs are left in place
/// because a cache restore rewrites them anyway; `--cache` clears the cache.
const RESERVED_DIRECTORIES: [&str; 3] = ["node_modules", ".git", ".turbo"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RemovalKind {
    File,
    /// Removed as a link; its target is never touched.
    Symlink,
    /// Removed only because everything inside it is removed too.
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SkipReason {
    OutsideRepository,
    ThroughSymlink,
    PackageDirectory,
    ReservedDirectory,
    TrackedByGit,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SkipReason::OutsideRepository => "outside the repository",
            SkipReason::ThroughSymlink => "reached through a symlink",
            SkipReason::PackageDirectory => "a package directory",
            SkipReason::ReservedDirectory => "inside node_modules, .git or .turbo",
            SkipReason::TrackedByGit => "tracked by git",
        })
    }
}

/// What `turbo clean` removes, in removal order: files and symlinks, then
/// directories deepest first.
#[derive(Debug, Default)]
struct CleanPlan {
    removals: Vec<(AbsoluteSystemPathBuf, RemovalKind)>,
    skipped: BTreeMap<AbsoluteSystemPathBuf, SkipReason>,
}

impl CleanPlan {
    fn count(&self, kind: RemovalKind) -> usize {
        self.removals
            .iter()
            .filter(|(_, removal)| *removal == kind)
            .count()
    }

    /// The top-most removed paths: a removed directory stands in for
    /// everything inside it.
    fn roots(&self) -> Vec<(&AbsoluteSystemPath, RemovalKind)> {
        let removed: HashSet<&AbsoluteSystemPath> = self
            .removals
            .iter()
            .map(|(path, _)| path.as_ref())
            .collect();
        let mut roots: Vec<(&AbsoluteSystemPath, RemovalKind)> = self
            .removals
            .iter()
            .filter(|(path, _)| !path.parent().is_some_and(|parent| removed.contains(parent)))
            .map(|(path, kind)| (path.as_ref(), *kind))
            .collect();
        roots.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
        roots
    }

    fn execute(&self) -> Result<(), Error> {
        for (path, kind) in &self.removals {
            let result = match kind {
                RemovalKind::File => path.remove_file(),
                // `remove_file` unlinks a symlink on unix; a directory
                // symlink on Windows needs `remove_dir`, which never
                // recurses into the target.
                RemovalKind::Symlink => path.remove_file().or_else(|_| path.remove_dir()),
                // Non-recursive: fails rather than deleting anything that
                // appeared after planning.
                RemovalKind::Directory => path.remove_dir(),
            };
            match result {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(Error::Remove {
                        path: path.to_string(),
                        source,
                    });
                }
            }
        }
        Ok(())
    }
}

/// Applies the safety guards to every glob match and decides which
/// directories become empty, without touching the filesystem. Dry runs and
/// real runs share this plan.
fn plan_removals(
    repo_root: &AbsoluteSystemPath,
    protected: &ProtectedDirectories,
    candidates: BTreeSet<AbsoluteSystemPathBuf>,
    tracked: Option<&TrackedFiles>,
) -> Result<CleanPlan, Error> {
    let real_repo_root = repo_root.to_realpath()?;
    let mut plan = CleanPlan::default();
    let mut files = Vec::new();
    let mut symlinks = Vec::new();
    let mut directories = Vec::new();

    for path in candidates {
        let Some(relative) = repo_root
            .anchor(&path)
            .ok()
            .filter(|relative| !relative.as_str().is_empty())
        else {
            plan.skipped.insert(path, SkipReason::OutsideRepository);
            continue;
        };
        let segments = path_segments(relative.as_str());
        let segment_refs: Vec<&str> = segments.iter().map(String::as_str).collect();
        if segment_refs.iter().any(|segment| {
            RESERVED_DIRECTORIES
                .iter()
                .any(|name| same_name(segment, name))
        }) {
            plan.skipped.insert(path, SkipReason::ReservedDirectory);
            continue;
        }
        if protected.covers(&segment_refs) {
            plan.skipped.insert(path, SkipReason::PackageDirectory);
            continue;
        }
        let Some(real_path) = real_path_without_symlinks(&real_repo_root, &relative)? else {
            plan.skipped.insert(path, SkipReason::ThroughSymlink);
            continue;
        };
        let file_type = match path.symlink_metadata() {
            Ok(metadata) => metadata.file_type(),
            // Vanished since the walk; nothing to remove.
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            symlinks.push(path);
        } else if file_type.is_dir() {
            directories.push(path);
        } else if tracked.is_some_and(|tracked| tracked.contains(&real_path)) {
            plan.skipped.insert(path, SkipReason::TrackedByGit);
        } else {
            files.push(path);
        }
    }

    // Matches below a symlink that is itself removed are covered by removing
    // the link; only report symlink traversals that stay behind.
    plan.skipped.retain(|path, reason| {
        *reason != SkipReason::ThroughSymlink
            || !symlinks
                .iter()
                .any(|link| path.as_path().starts_with(link.as_path()))
    });

    let mut removed: HashSet<AbsoluteSystemPathBuf> = files.iter().cloned().collect();
    removed.extend(symlinks.iter().cloned());
    plan.removals
        .extend(files.into_iter().map(|path| (path, RemovalKind::File)));
    plan.removals.extend(
        symlinks
            .into_iter()
            .map(|path| (path, RemovalKind::Symlink)),
    );

    // Deepest first, so a directory sees whether its children are removed.
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        let entries =
            std::fs::read_dir(directory.as_std_path()).map_err(|source| Error::Inspect {
                path: directory.to_string(),
                source,
            })?;
        let mut empties = true;
        for entry in entries {
            let entry = entry.map_err(|source| Error::Inspect {
                path: directory.to_string(),
                source,
            })?;
            let child = AbsoluteSystemPathBuf::try_from(entry.path())?;
            if !removed.contains(&child) {
                empties = false;
                break;
            }
        }
        if empties {
            removed.insert(directory.clone());
            plan.removals.push((directory, RemovalKind::Directory));
        }
    }

    Ok(plan)
}

/// The real path of `relative`'s parent joined with its name, if no
/// directory between the repository root and `relative` is a symlink.
/// Returns `None` when following the path would leave the repository's
/// directory tree through a link.
fn real_path_without_symlinks(
    real_repo_root: &AbsoluteSystemPath,
    relative: &AnchoredSystemPathBuf,
) -> Result<Option<AbsoluteSystemPathBuf>, Error> {
    let expected = real_repo_root.resolve(relative);
    let (Some(expected_parent), Some(name)) = (expected.parent(), expected.file_name()) else {
        return Ok(None);
    };
    let Ok(real_parent) = expected_parent.to_realpath() else {
        return Ok(None);
    };
    if real_parent.as_str() != expected_parent.as_str() {
        return Ok(None);
    }
    Ok(Some(real_parent.join_component(name)))
}

fn display_path(repo_root: &AbsoluteSystemPath, path: &AbsoluteSystemPath) -> String {
    repo_root
        .anchor(path)
        .map(|relative| relative.to_unix().to_string())
        .unwrap_or_else(|_| path.to_string())
}

fn print_plan(repo_root: &AbsoluteSystemPath, plan: &CleanPlan, dry_run: bool) {
    for (path, reason) in &plan.skipped {
        println!(
            "• Not removing {} ({reason})",
            display_path(repo_root, path)
        );
    }

    let files = plan.count(RemovalKind::File) + plan.count(RemovalKind::Symlink);
    let directories = plan.count(RemovalKind::Directory);
    if files == 0 && directories == 0 {
        println!("No task outputs to remove.");
        return;
    }
    if dry_run {
        println!("Would remove:");
        for (path, kind) in plan.roots() {
            let suffix = if kind == RemovalKind::Directory {
                "/"
            } else {
                ""
            };
            println!("  {}{suffix}", display_path(repo_root, path));
        }
    }
    println!(
        "{} {files} {} and {directories} {}.",
        if dry_run { "Would remove" } else { "Removed" },
        if files == 1 { "file" } else { "files" },
        if directories == 1 {
            "directory"
        } else {
            "directories"
        },
    );
}

/// Clears the entries of the local cache directory, which must live inside
/// the repository.
fn clean_cache_dir(
    repo_root: &AbsoluteSystemPath,
    cache_dir: &AbsoluteSystemPath,
    dry_run: bool,
) -> Result<turborepo_cache::fs::ClearedCacheFiles, Error> {
    let shown = display_path(repo_root, cache_dir);
    let outside = || Error::CacheOutsideRepo {
        path: cache_dir.to_string(),
    };
    // Lexically first, then physically for an existing directory, so a
    // symlinked `.turbo` cannot lead outside the repository.
    let cache_dir = cache_dir.clean()?;
    if cache_dir.as_path() == repo_root.as_path() || !repo_root.contains(&cache_dir) {
        return Err(outside());
    }
    if let Ok(real_cache_dir) = cache_dir.to_realpath() {
        let real_repo_root = repo_root.to_realpath()?;
        if real_cache_dir == real_repo_root || !real_repo_root.contains(&real_cache_dir) {
            return Err(outside());
        }
    }

    let cleared = turborepo_cache::fs::clear_cache_dir(&cache_dir, dry_run).map_err(|source| {
        Error::Cache {
            path: cache_dir.to_string(),
            source,
        }
    })?;
    match (cleared.files, dry_run) {
        (0, _) => println!("The local cache at {shown} is already empty."),
        (files, true) => println!(
            "Would remove {files} {} ({} bytes) from the local cache at {shown}.",
            if files == 1 { "file" } else { "files" },
            cleared.bytes
        ),
        (files, false) => println!(
            "Removed {files} {} ({} bytes) from the local cache at {shown}.",
            if files == 1 { "file" } else { "files" },
            cleared.bytes
        ),
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use tempfile::TempDir;
    use turborepo_ui::ColorConfig;

    use super::*;
    use crate::Args;

    fn repo(tmp: &TempDir) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap()
    }

    fn write(repo_root: &AbsoluteSystemPath, path: &str, contents: &str) -> AbsoluteSystemPathBuf {
        let file = repo_root.join_unix_path(turbopath::RelativeUnixPath::new(path).unwrap());
        file.ensure_dir().unwrap();
        file.create_with_contents(contents).unwrap();
        file
    }

    fn exists(repo_root: &AbsoluteSystemPath, path: &str) -> bool {
        repo_root
            .join_unix_path(turbopath::RelativeUnixPath::new(path).unwrap())
            .symlink_metadata()
            .is_ok()
    }

    fn target(task: &str, inclusions: &[&str], exclusions: &[&str]) -> OutputTarget {
        let to_system = |glob: &&str| glob.replace('/', std::path::MAIN_SEPARATOR_STR);
        OutputTarget {
            task: task.to_owned(),
            outputs: TaskOutputs {
                inclusions: inclusions.iter().map(to_system).collect(),
                exclusions: exclusions.iter().map(to_system).collect(),
            },
        }
    }

    /// Plans (and optionally executes) a clean over explicit output targets.
    fn clean_targets(
        repo_root: &AbsoluteSystemPath,
        protected: &[&str],
        targets: &[OutputTarget],
        execute: bool,
    ) -> (CleanPlan, Vec<String>) {
        let protected = ProtectedDirectories::from_dirs(protected);
        let mut notices = Vec::new();
        let candidates = collect_candidates(repo_root, targets, &protected, &mut notices).unwrap();
        let tracked = TrackedFiles::load(&SCM::new(repo_root)).unwrap();
        let plan = plan_removals(repo_root, &protected, candidates, tracked.as_ref()).unwrap();
        if execute {
            plan.execute().unwrap();
        }
        (plan, notices)
    }

    fn removed_paths(repo_root: &AbsoluteSystemPath, plan: &CleanPlan) -> Vec<String> {
        let mut paths: Vec<_> = plan
            .removals
            .iter()
            .map(|(path, _)| display_path(repo_root, path))
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn removes_matched_outputs_and_keeps_exclusions() {
        let tmp = TempDir::new().unwrap();
        let repo_root = repo(&tmp);
        write(&repo_root, "packages/web/dist/index.js", "");
        write(&repo_root, "packages/web/dist/nested/chunk.js", "");
        write(&repo_root, "packages/web/dist/keep.txt", "");
        write(&repo_root, "packages/web/src/index.ts", "");

        let (plan, notices) = clean_targets(
            &repo_root,
            &["packages/web"],
            &[target(
                "web#build",
                &["packages/web/dist/**"],
                &["packages/web/dist/keep.txt"],
            )],
            true,
        );

        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(
            removed_paths(&repo_root, &plan),
            [
                "packages/web/dist/index.js",
                "packages/web/dist/nested",
                "packages/web/dist/nested/chunk.js",
            ]
        );
        assert!(!exists(&repo_root, "packages/web/dist/nested"));
        assert!(exists(&repo_root, "packages/web/dist/keep.txt"));
        assert!(exists(&repo_root, "packages/web/src/index.ts"));
    }

    #[test]
    fn removes_a_directory_once_everything_inside_is_removed() {
        let tmp = TempDir::new().unwrap();
        let repo_root = repo(&tmp);
        write(&repo_root, "packages/web/dist/index.js", "");
        write(&repo_root, "packages/web/tsconfig.tsbuildinfo", "");

        let (plan, _) = clean_targets(
            &repo_root,
            &["packages/web"],
            &[target(
                "web#build",
                &["packages/web/dist/**", "packages/web/*.tsbuildinfo"],
                &[],
            )],
            true,
        );

        assert_eq!(
            plan.roots()
                .into_iter()
                .map(|(path, _)| display_path(&repo_root, path))
                .collect::<Vec<_>>(),
            ["packages/web/dist", "packages/web/tsconfig.tsbuildinfo"]
        );
        assert!(!exists(&repo_root, "packages/web/dist"));
        assert!(exists(&repo_root, "packages/web"));
    }

    #[test]
    fn planning_alone_deletes_nothing() {
        let tmp = TempDir::new().unwrap();
        let repo_root = repo(&tmp);
        write(&repo_root, "packages/web/dist/index.js", "");

        let (plan, _) = clean_targets(
            &repo_root,
            &["packages/web"],
            &[target("web#build", &["packages/web/dist/**"], &[])],
            false,
        );

        assert_eq!(plan.count(RemovalKind::File), 1);
        assert_eq!(plan.count(RemovalKind::Directory), 1);
        assert!(exists(&repo_root, "packages/web/dist/index.js"));
    }

    #[test]
    fn refuses_globs_that_sweep_a_package_or_leave_the_repository() {
        let protected = ProtectedDirectories::from_dirs(&["packages/web", "packages/ui"]);
        let sep = std::path::MAIN_SEPARATOR;
        for glob in [
            format!("packages{sep}web{sep}**"),
            format!("packages{sep}web{sep}**/*.js"),
            format!("packages{sep}web{sep}*"),
            format!("packages{sep}web{sep}."),
            format!("packages{sep}web{sep}../ui/**"),
            format!("packages{sep}web{sep}../**"),
            format!("packages{sep}web{sep}../../../outside/**"),
            "**".to_owned(),
        ] {
            assert!(protected.is_swept_by(&glob), "{glob} should be refused");
        }
        for glob in [
            format!("packages{sep}web{sep}dist/**"),
            format!("packages{sep}web{sep}.next/**"),
            format!("packages{sep}web{sep}*.tsbuildinfo"),
            format!("packages{sep}web{sep}../ui/dist/**"),
            "dist/**".to_owned(),
        ] {
            assert!(!protected.is_swept_by(&glob), "{glob} should be allowed");
        }
    }

    #[test]
    fn a_whole_package_glob_deletes_nothing() {
        let tmp = TempDir::new().unwrap();
        let repo_root = repo(&tmp);
        write(&repo_root, "packages/web/package.json", "{}");
        write(&repo_root, "packages/web/src/index.ts", "");

        let (plan, notices) = clean_targets(
            &repo_root,
            &["packages/web"],
            &[target("web#build", &["packages/web/**"], &[])],
            true,
        );

        assert!(plan.removals.is_empty());
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(exists(&repo_root, "packages/web/src/index.ts"));
        assert!(exists(&repo_root, "packages/web/package.json"));
    }

    #[test]
    fn never_touches_paths_outside_the_repository() {
        let tmp = TempDir::new().unwrap();
        let repo_root = repo(&tmp).join_component("repo");
        let outside = write(&repo(&tmp), "outside/precious.txt", "keep");
        write(&repo_root, "packages/web/dist/index.js", "");

        let protected = ProtectedDirectories::from_dirs(&["packages/web"]);
        let candidates = BTreeSet::from([outside.clone()]);
        let plan = plan_removals(&repo_root, &protected, candidates, None).unwrap();
        plan.execute().unwrap();

        assert!(plan.removals.is_empty());
        assert_eq!(
            plan.skipped.get(&outside),
            Some(&SkipReason::OutsideRepository)
        );
        assert_eq!(outside.read_to_string().unwrap(), "keep");
    }

    #[test]
    fn never_removes_package_directories_or_dependencies() {
        let tmp = TempDir::new().unwrap();
        let repo_root = repo(&tmp);
        write(&repo_root, "packages/web/node_modules/dep/index.js", "");
        write(&repo_root, "packages/web/dist/index.js", "");

        let (plan, _) = clean_targets(
            &repo_root,
            &["packages/web"],
            &[target(
                "web#build",
                &[
                    "packages/web/node_modules/**",
                    "packages/web/dist/**",
                    "packages/web/../web",
                ],
                &[],
            )],
            true,
        );

        assert!(exists(&repo_root, "packages/web/node_modules/dep/index.js"));
        assert!(!exists(&repo_root, "packages/web/dist"));
        assert!(
            plan.skipped
                .values()
                .all(|reason| *reason == SkipReason::ReservedDirectory)
        );
    }

    #[cfg(unix)]
    #[test]
    fn removes_symlinks_without_following_them_out_of_the_package() {
        let tmp = TempDir::new().unwrap();
        let root = repo(&tmp);
        let repo_root = root.join_component("repo");
        let outside_file = write(&root, "outside/precious.txt", "keep");
        let outside_dir = outside_file.parent().unwrap().to_owned();
        write(&repo_root, "packages/web/build/index.js", "");
        // An output root that is itself a link out of the package…
        repo_root
            .join_components(&["packages", "web", "dist"])
            .symlink_to_dir(outside_dir.as_str())
            .unwrap();
        // …and a link out of the package inside a real output directory.
        repo_root
            .join_components(&["packages", "web", "build", "external"])
            .symlink_to_dir(outside_dir.as_str())
            .unwrap();

        let (plan, _) = clean_targets(
            &repo_root,
            &["packages/web"],
            &[target(
                "web#build",
                &["packages/web/dist/**", "packages/web/build/**"],
                &[],
            )],
            true,
        );

        assert_eq!(outside_file.read_to_string().unwrap(), "keep");
        assert!(!exists(&repo_root, "packages/web/dist"));
        assert!(!exists(&repo_root, "packages/web/build"));
        assert_eq!(plan.count(RemovalKind::Symlink), 2);
        assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
    }

    #[test]
    fn keeps_files_tracked_by_git() {
        let tmp = TempDir::new().unwrap();
        let repo_root = repo(&tmp);
        write(&repo_root, "packages/web/package.json", "{}");
        write(&repo_root, "packages/web/index.js", "source");
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo_root)
                .output()
                .unwrap();
            assert!(status.status.success(), "{status:?}");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);
        git(&["add", "."]);
        git(&["commit", "--quiet", "-m", "init"]);
        write(&repo_root, "packages/web/generated.js", "output");

        let (plan, _) = clean_targets(
            &repo_root,
            &["packages/web"],
            &[target("web#build", &["packages/web/*.js"], &[])],
            true,
        );

        assert!(exists(&repo_root, "packages/web/index.js"));
        assert!(!exists(&repo_root, "packages/web/generated.js"));
        assert_eq!(
            plan.skipped.values().collect::<Vec<_>>(),
            [&SkipReason::TrackedByGit]
        );
    }

    #[test]
    fn cache_dir_outside_the_repository_is_refused() {
        let tmp = TempDir::new().unwrap();
        let root = repo(&tmp);
        let repo_root = root.join_component("repo");
        repo_root.create_dir_all().unwrap();
        let outside_entry = write(&root, "shared-cache/abc.tar.zst", "");

        for cache_dir in [root.join_component("shared-cache"), repo_root.clone()] {
            assert!(matches!(
                clean_cache_dir(&repo_root, &cache_dir, false),
                Err(Error::CacheOutsideRepo { .. })
            ));
        }
        assert!(outside_entry.exists());
    }

    // End-to-end over a real workspace: tasks are planned by `RunBuilder`
    // exactly as `turbo run --dry` plans them.

    fn workspace(tmp: &TempDir) -> AbsoluteSystemPathBuf {
        let repo_root = repo(tmp);
        write(
            &repo_root,
            "package.json",
            r#"{"name": "root", "packageManager": "pnpm@9.0.0"}"#,
        );
        write(
            &repo_root,
            "pnpm-workspace.yaml",
            "packages:\n  - 'packages/*'\n",
        );
        write(
            &repo_root,
            "turbo.json",
            r#"{
                "cacheDir": "custom-cache",
                "tasks": {
                    "build": { "outputs": ["dist/**", "!dist/keep.txt"] },
                    "lint": {},
                    "dev": { "persistent": true, "outputs": ["dist/**"] },
                    "generate": { "cache": false, "outputs": ["dist/**"] }
                }
            }"#,
        );
        for package in ["web", "ui"] {
            write(
                &repo_root,
                &format!("packages/{package}/package.json"),
                &format!(
                    r#"{{"name": "{package}", "scripts": {{"build": "b", "lint": "l", "dev": "d", "generate": "g"}}}}"#
                ),
            );
            write(&repo_root, &format!("packages/{package}/src/index.ts"), "");
            write(&repo_root, &format!("packages/{package}/dist/index.js"), "");
            write(&repo_root, &format!("packages/{package}/dist/keep.txt"), "");
        }
        // The `ui` package overrides its outputs in a package configuration.
        write(
            &repo_root,
            "packages/ui/turbo.json",
            r#"{"extends": ["//"], "tasks": {"build": {"outputs": ["lib/**"]}}}"#,
        );
        write(&repo_root, "packages/ui/lib/index.js", "");
        write(&repo_root, "custom-cache/abc.tar.zst", "artifact");
        write(&repo_root, "custom-cache/abc-meta.json", "{}");
        repo_root
    }

    async fn turbo_clean(repo_root: &AbsoluteSystemPath, args: &[&str]) -> Result<(), Error> {
        let argv = ["turbo", "clean"]
            .iter()
            .chain(args)
            .map(OsString::from)
            .collect();
        let args = Args::parse_args(argv).unwrap();
        let Some(crate::cli::Command::Clean {
            dry_run,
            cache,
            ref tasks,
            ..
        }) = args.command
        else {
            panic!("expected the clean command");
        };
        let options = CleanOptions {
            dry_run,
            cache,
            clean_outputs: !tasks.is_empty(),
        };
        let base = CommandBase::new(
            args.clone(),
            repo_root.to_owned(),
            "test",
            ColorConfig::new(true),
        )
        .unwrap();
        run(base, CommandEventBuilder::new("clean"), options).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleans_resolved_outputs_for_the_filtered_packages() {
        let tmp = TempDir::new().unwrap();
        let repo_root = workspace(&tmp);

        turbo_clean(&repo_root, &["build", "--filter=web"])
            .await
            .unwrap();

        assert!(!exists(&repo_root, "packages/web/dist/index.js"));
        assert!(exists(&repo_root, "packages/web/dist/keep.txt"));
        assert!(exists(&repo_root, "packages/web/src/index.ts"));
        // Outside the filter: untouched.
        assert!(exists(&repo_root, "packages/ui/lib/index.js"));
        assert!(exists(&repo_root, "packages/ui/dist/index.js"));
        // `--cache` was not passed.
        assert!(exists(&repo_root, "custom-cache/abc.tar.zst"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn uses_package_configurations_to_resolve_outputs() {
        let tmp = TempDir::new().unwrap();
        let repo_root = workspace(&tmp);

        turbo_clean(&repo_root, &["build"]).await.unwrap();

        assert!(!exists(&repo_root, "packages/web/dist/index.js"));
        assert!(!exists(&repo_root, "packages/ui/lib"));
        // `ui#build` outputs `lib/**` only, so its `dist` is not an output.
        assert!(exists(&repo_root, "packages/ui/dist/index.js"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dry_run_deletes_nothing() {
        let tmp = TempDir::new().unwrap();
        let repo_root = workspace(&tmp);

        turbo_clean(&repo_root, &["build", "--dry", "--cache"])
            .await
            .unwrap();
        turbo_clean(&repo_root, &["build", "--dry-run"])
            .await
            .unwrap();

        assert!(exists(&repo_root, "packages/web/dist/index.js"));
        assert!(exists(&repo_root, "packages/ui/lib/index.js"));
        assert!(exists(&repo_root, "custom-cache/abc.tar.zst"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tasks_without_outputs_persistent_or_uncached_are_no_ops() {
        let tmp = TempDir::new().unwrap();
        let repo_root = workspace(&tmp);

        turbo_clean(&repo_root, &["lint", "dev", "generate"])
            .await
            .unwrap();

        assert!(exists(&repo_root, "packages/web/dist/index.js"));
        assert!(exists(&repo_root, "packages/ui/dist/index.js"));
        assert!(exists(&repo_root, "packages/ui/lib/index.js"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_flag_clears_only_the_configured_cache_dir() {
        let tmp = TempDir::new().unwrap();
        let repo_root = workspace(&tmp);
        write(&repo_root, ".turbo/cache/default.tar.zst", "");
        // An entry another `turbo run` is still writing.
        write(&repo_root, "custom-cache/.def.tar.zst.123.0.tmp", "partial");

        turbo_clean(&repo_root, &["--cache"]).await.unwrap();

        assert!(exists(&repo_root, "custom-cache/.def.tar.zst.123.0.tmp"));

        assert!(!exists(&repo_root, "custom-cache/abc.tar.zst"));
        assert!(!exists(&repo_root, "custom-cache/abc-meta.json"));
        assert!(exists(&repo_root, "custom-cache"));
        // Not the configured cache directory, and no task was selected.
        assert!(exists(&repo_root, ".turbo/cache/default.tar.zst"));
        assert!(exists(&repo_root, "packages/web/dist/index.js"));
    }

    /// `turbo run build` → `turbo clean build` → `turbo run build` must be a
    /// cache hit that writes the cleaned outputs back. With an output watcher
    /// (`turbo watch`), the restore is skipped when the watcher reports the
    /// outputs unchanged, so the watcher has to observe the deletions.
    mod restore_after_clean {
        use std::{collections::HashSet, pin::Pin, sync::Arc, time::Duration};

        use turborepo_cache::{AsyncCache, CacheActions, CacheConfig, CacheOpts, LazyScmState};
        use turborepo_filewatch::{
            FileSystemWatcher,
            cookies::CookieWriter,
            globwatcher::{GlobSet, GlobWatcher},
        };
        use turborepo_repository::package_json::PackageJson;
        use turborepo_run_cache::{OutputWatcher, OutputWatcherError, RunCache, TaskCacheContext};
        use turborepo_telemetry::events::task::PackageTaskEventBuilder;
        use turborepo_types::RunCacheOpts;

        use super::*;

        /// The same adapter `turbo watch` uses to back the run cache's
        /// output tracking with a file-system `GlobWatcher`.
        struct GlobOutputWatcher(Arc<GlobWatcher>);

        type Boxed<T> = Pin<Box<dyn std::future::Future<Output = T> + Send>>;

        impl OutputWatcher for GlobOutputWatcher {
            fn get_changed_outputs(
                &self,
                hash: String,
                output_globs: Vec<String>,
            ) -> Boxed<Result<HashSet<String>, OutputWatcherError>> {
                let watcher = self.0.clone();
                Box::pin(async move {
                    watcher
                        .get_changed_globs(
                            hash,
                            output_globs.into_iter().collect(),
                            Duration::from_secs(2),
                        )
                        .await
                        .map_err(|e| OutputWatcherError(Box::new(e)))
                })
            }

            fn notify_outputs_written(
                &self,
                hash: String,
                output_globs: Vec<String>,
                output_exclusion_globs: Vec<String>,
                _time_saved: u64,
            ) -> Boxed<Result<(), OutputWatcherError>> {
                let watcher = self.0.clone();
                Box::pin(async move {
                    let globs = GlobSet::from_raw(output_globs, output_exclusion_globs)
                        .map_err(|e| OutputWatcherError(Box::new(e)))?;
                    watcher
                        .watch_globs(hash, globs, Duration::from_secs(2))
                        .await
                        .map_err(|e| OutputWatcherError(Box::new(e)))
                })
            }
        }

        fn task_handle() -> turborepo_log::grouping::TaskHandle {
            let logger = Arc::new(turborepo_log::Logger::new(vec![]));
            turborepo_log::grouping::GroupingLayer::new(
                logger,
                turborepo_log::grouping::GroupingMode::Passthrough,
            )
            .task("web#build")
        }

        async fn build_then_clean_then_build(watch: bool) {
            let tmp = TempDir::new().unwrap();
            let repo_root = workspace(&tmp);
            let log = write(
                &repo_root,
                "packages/web/.turbo/turbo-build.log",
                "build output\n",
            );
            let index_js = repo_root.join_components(&["packages", "web", "dist", "index.js"]);
            index_js.create_with_contents("built").unwrap();

            // Keep the file system watcher alive for the whole test.
            let mut fs_watcher = None;
            let output_watcher: Option<Arc<dyn OutputWatcher>> = if watch {
                let watcher = FileSystemWatcher::new_with_default_cookie_dir(&repo_root).unwrap();
                let cookie_writer = CookieWriter::new(
                    watcher.cookie_dir(),
                    Duration::from_secs(2),
                    watcher.watch(),
                );
                let glob_watcher =
                    GlobWatcher::new(repo_root.clone(), cookie_writer, watcher.source());
                fs_watcher = Some(watcher);
                Some(Arc::new(GlobOutputWatcher(Arc::new(glob_watcher))))
            } else {
                None
            };

            let cache_opts = CacheOpts {
                cache_dir: ".turbo/cache".into(),
                cache: CacheConfig {
                    local: CacheActions::enabled(),
                    remote: CacheActions::disabled(),
                },
                workers: 1,
                remote_cache_opts: None,
                cache_max_age: None,
                cache_max_size: None,
            };
            let async_cache = AsyncCache::new(
                &cache_opts,
                &repo_root,
                None,
                None,
                None,
                LazyScmState::resolved(None),
            )
            .unwrap();
            let run_cache = Arc::new(RunCache::new(
                async_cache.clone(),
                &repo_root,
                RunCacheOpts::default(),
                &cache_opts,
                output_watcher,
                ColorConfig::new(true),
                false,
            ));
            let root_json = PackageJson::load(&repo_root.join_component("package.json")).unwrap();
            let graph = PackageGraph::builder(&repo_root, root_json)
                .build()
                .await
                .unwrap();
            let context = graph
                .package_task_context(&PackageName::from("web"))
                .unwrap();
            let definition = TaskDefinition {
                outputs: TaskOutputs {
                    inclusions: vec!["dist/**".to_string()],
                    exclusions: vec!["dist/keep.txt".to_string()],
                },
                ..TaskDefinition::default()
            };
            let task_cache = || {
                run_cache
                    .task_cache(TaskCacheContext {
                        task_definition: &definition,
                        package_context: &context,
                        task_id: TaskId::new("web", "build"),
                        hash: "web-build-hash",
                    })
                    .unwrap()
            };
            let telemetry = PackageTaskEventBuilder::new("web", "build");

            // turbo run build: the task ran and its outputs were cached.
            task_cache()
                .save_outputs(Duration::from_millis(10), &telemetry)
                .await
                .unwrap();
            async_cache.wait().await.unwrap();

            // turbo clean build
            turbo_clean(&repo_root, &["build", "--filter=web"])
                .await
                .unwrap();
            assert!(!index_js.exists());
            assert_eq!(
                log.read_to_string().unwrap(),
                "build output\n",
                "task logs are left for the cache restore to rewrite"
            );

            // turbo run build: a cache hit that writes the outputs back.
            log.create_with_contents("stale log\n").unwrap();
            let hit = task_cache()
                .restore_outputs(&mut task_handle(), None, &telemetry)
                .await
                .unwrap();
            assert!(hit.is_some(), "expected a cache hit");
            assert_eq!(index_js.read_to_string().unwrap(), "built");
            assert_eq!(log.read_to_string().unwrap(), "build output\n");
            drop(fs_watcher);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn without_an_output_watcher() {
            build_then_clean_then_build(false).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn with_the_watch_mode_output_watcher() {
            build_then_clean_then_build(true).await;
        }
    }
}
