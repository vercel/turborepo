//! Deciding what to remove. Nothing here touches the filesystem except to
//! read it, so dry runs and real runs share one plan.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    Error,
    names::{DiskNames, EntryKind, Resolution, fold},
    targets::{ProtectedDirectories, path_segments},
    tracked::TrackedFiles,
};

/// Directories owned by other tools, or by turbo itself, whose contents are
/// never task output: dependencies, the git repository, and turbo's state
/// (task logs, the default cache, local config).
const RESERVED_DIRECTORIES: [&str; 3] = ["node_modules", ".git", ".turbo"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemovalKind {
    File,
    /// Removed as a link; its target is never touched.
    Symlink,
    /// Removed only because everything inside it is removed too.
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    OutsideRepository,
    ThroughSymlink,
    AmbiguousName,
    Unreadable,
    PackageDirectory,
    ReservedDirectory,
    NestedRepository,
    InSubmodule,
    TrackedByGit,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SkipReason::OutsideRepository => "outside the repository",
            SkipReason::ThroughSymlink => "reached through a symlink",
            SkipReason::AmbiguousName => "several files on disk match its name",
            SkipReason::Unreadable => "its directory cannot be read",
            SkipReason::PackageDirectory => "a package directory",
            SkipReason::ReservedDirectory => "inside node_modules, .git or .turbo",
            SkipReason::NestedRepository => "inside a nested git repository",
            SkipReason::InSubmodule => "inside a git submodule",
            SkipReason::TrackedByGit => "tracked by git",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Removal {
    /// The stored name of each component below the repository root.
    pub(crate) components: Vec<String>,
    pub(crate) kind: RemovalKind,
}

impl Removal {
    pub(crate) fn display(&self) -> String {
        self.components.join("/")
    }
}

/// What `turbo clean` removes, in removal order: files and symlinks, then
/// directories deepest first.
#[derive(Debug)]
pub struct CleanPlan {
    pub(crate) real_root: AbsoluteSystemPathBuf,
    pub(crate) removals: Vec<Removal>,
    skipped: BTreeMap<String, SkipReason>,
}

impl CleanPlan {
    fn empty(real_root: AbsoluteSystemPathBuf) -> Self {
        Self {
            real_root,
            removals: Vec::new(),
            skipped: BTreeMap::new(),
        }
    }

    /// Files and symlinks to remove.
    pub fn file_count(&self) -> usize {
        self.removals
            .iter()
            .filter(|removal| removal.kind != RemovalKind::Directory)
            .count()
    }

    pub fn directory_count(&self) -> usize {
        self.removals.len() - self.file_count()
    }

    pub fn is_empty(&self) -> bool {
        self.removals.is_empty()
    }

    /// Paths that match an output but are kept, repository-relative.
    pub fn skipped(&self) -> impl Iterator<Item = (&str, SkipReason)> {
        self.skipped
            .iter()
            .map(|(path, reason)| (path.as_str(), *reason))
    }

    /// Every path the plan removes, repository-relative, in removal order.
    pub fn removals(&self) -> impl Iterator<Item = (String, RemovalKind)> + '_ {
        self.removals
            .iter()
            .map(|removal| (removal.display(), removal.kind))
    }

    /// The top-most removed paths: a removed directory stands in for
    /// everything inside it.
    pub fn roots(&self) -> Vec<(String, RemovalKind)> {
        let directories: HashSet<&[String]> = self
            .removals
            .iter()
            .filter(|removal| removal.kind == RemovalKind::Directory)
            .map(|removal| removal.components.as_slice())
            .collect();
        let mut roots: Vec<(String, RemovalKind)> = self
            .removals
            .iter()
            .filter(|removal| {
                removal
                    .components
                    .split_last()
                    .is_none_or(|(_, parent)| !directories.contains(parent))
            })
            .map(|removal| (removal.display(), removal.kind))
            .collect();
        roots.sort();
        roots
    }

    /// The lines `turbo clean` prints before acting on the plan.
    pub fn describe(&self, dry_run: bool) -> Vec<String> {
        let mut lines: Vec<String> = self
            .skipped()
            .map(|(path, reason)| format!("• Not removing {path} ({reason})"))
            .collect();
        if self.is_empty() {
            lines.push("No task outputs to remove.".to_owned());
        } else if dry_run {
            lines.push("Would remove:".to_owned());
            for (path, kind) in self.roots() {
                let suffix = if kind == RemovalKind::Directory {
                    "/"
                } else {
                    ""
                };
                lines.push(format!("  {path}{suffix}"));
            }
            lines.push(format!(
                "Would remove {}.",
                counts(self.file_count(), self.directory_count())
            ));
        }
        lines
    }
}

pub(crate) fn counts(files: usize, directories: usize) -> String {
    format!(
        "{files} {} and {directories} {}",
        if files == 1 { "file" } else { "files" },
        if directories == 1 {
            "directory"
        } else {
            "directories"
        }
    )
}

/// Applies the guards to every candidate and decides which directories
/// become empty.
pub(crate) fn plan_removals(
    repo_root: &AbsoluteSystemPath,
    protected: &ProtectedDirectories,
    candidates: BTreeSet<AbsoluteSystemPathBuf>,
) -> Result<CleanPlan, Error> {
    let real_root = repo_root.to_realpath()?;
    if candidates.is_empty() {
        return Ok(CleanPlan::empty(real_root));
    }
    // Fail closed: without the tracked set nothing can be told apart from
    // source, so nothing is removed.
    let tracked = TrackedFiles::load(&real_root)?;
    let mut plan = CleanPlan::empty(real_root.clone());
    let mut names = DiskNames::new(&real_root);
    let mut files = BTreeSet::new();
    let mut symlinks = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut through_symlinks = Vec::new();

    for path in candidates {
        let spelled = match repo_root.anchor(&path) {
            Ok(relative) => path_segments(relative.as_str()),
            Err(_) => Vec::new(),
        };
        if spelled.is_empty() || spelled.iter().any(|segment| segment == "..") {
            plan.skipped
                .insert(path.to_string(), SkipReason::OutsideRepository);
            continue;
        }
        let shown = spelled.join("/");
        let (components, kind, in_nested_repository) = match names.resolve(&spelled) {
            Resolution::Found {
                components,
                kind,
                in_nested_repository,
            } => (components, kind, in_nested_repository),
            // Vanished since the walk; nothing to remove.
            Resolution::Missing => continue,
            Resolution::Ambiguous => {
                plan.skipped.insert(shown, SkipReason::AmbiguousName);
                continue;
            }
            Resolution::ThroughSymlink => {
                through_symlinks.push(shown);
                continue;
            }
            Resolution::Unreadable => {
                plan.skipped.insert(shown, SkipReason::Unreadable);
                continue;
            }
        };
        let shown = components.join("/");
        let reason = if components.iter().any(|component| {
            RESERVED_DIRECTORIES
                .iter()
                .any(|reserved| fold(component) == *reserved)
        }) {
            Some(SkipReason::ReservedDirectory)
        } else if protected.covers(&components) {
            Some(SkipReason::PackageDirectory)
        } else if in_nested_repository {
            Some(SkipReason::NestedRepository)
        } else {
            // Before looking at the kind: a tracked symlink is source too.
            tracked.protects(&components)
        };
        if let Some(reason) = reason {
            plan.skipped.insert(shown, reason);
            continue;
        }
        match kind {
            EntryKind::File => files.insert(components),
            EntryKind::Symlink => symlinks.insert(components),
            EntryKind::Directory => directories.insert(components),
        };
    }

    // Matches below a symlink that is itself removed are covered by removing
    // the link; only report traversals that stay behind.
    let folded_links: Vec<String> = symlinks
        .iter()
        .map(|link: &Vec<String>| fold(&link.join("/")))
        .collect();
    for shown in through_symlinks {
        let folded = fold(&shown);
        let covered = folded_links.iter().any(|link| {
            folded
                .strip_prefix(link.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if !covered {
            plan.skipped.insert(shown, SkipReason::ThroughSymlink);
        }
    }

    let mut removed: HashSet<Vec<String>> = files.iter().cloned().collect();
    removed.extend(symlinks.iter().cloned());
    plan.removals
        .extend(files.into_iter().map(|components| Removal {
            components,
            kind: RemovalKind::File,
        }));
    plan.removals
        .extend(symlinks.into_iter().map(|components| Removal {
            components,
            kind: RemovalKind::Symlink,
        }));

    // Deepest first, so a directory sees whether its children are removed.
    let mut directories: Vec<Vec<String>> = directories.into_iter().collect();
    directories.sort_by_key(|components| std::cmp::Reverse(components.len()));
    for directory in directories {
        let Some(listing) = names.listing(&directory) else {
            continue;
        };
        let empties = listing.names().all(|name| {
            let mut child = directory.clone();
            child.push(name.to_owned());
            removed.contains(&child)
        });
        if empties {
            removed.insert(directory.clone());
            plan.removals.push(Removal {
                components: directory,
                kind: RemovalKind::Directory,
            });
        }
    }

    Ok(plan)
}
