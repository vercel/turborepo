//! Directories that a change makes or removes
//! ([`turborepo_trace_graph::ChangeSet::dirs`]).
//!
//! A diff names files. A directory comes with its first file and goes with its
//! last, and no line of the diff says so. A lookup that asks "is `d` a
//! directory?" before it looks inside (the loader of a tsconfig, for the target
//! of an `extends` and for a project reference: `d/tsconfig.json` when `d` is a
//! directory, `d.json` when it is not) gets another answer for such a
//! directory, whatever file the diff added to it. The graph holds that answer
//! where nothing else says it, as the directory's membership of its parent
//! under its own name, and a directory that the change made or removed matches
//! those records ([`turborepo_trace_graph::Engine::match_dir`]). It matches
//! nothing else: every other lookup that has to do with the directory (a
//! listing with a pattern, a path looked for inside it) is a question of the
//! files that came or went.
//!
//! - **Removed**: a directory above a deleted path that is no directory in the
//!   working tree.
//! - **Made**: a directory above a path that may be new, which the base does
//!   not have.
//!
//! Whether a directory was there before is a question about the base, and what
//! is known of the base depends on how the change was given. The rule in every
//! mode: a directory is taken to be old only on evidence, and to be new
//! otherwise (a directory that was there and is taken for new selects the tests
//! whose lookups asked for it by name, which is what the change of an answer
//! costs; the other mistake skips them).
//!
//! | The change | A directory above a path of it was there before when |
//! |---|---|
//! | `--since`, one base | the diff states a path below it as modified or deleted, or the base's tree has it ([`Git::dirs_at_base`]: one `git` process, and none for a change whose new files all lie in directories that have other changes or that no lookup asked for) |
//! | `--since`, several merge bases | every base's tree has it. The diff is the union of the diffs against each base: "modified" says that some base has the path, and a directory that one base lacks was made since that base |
//! | paths without a status (`--changed`, a plain line of `--changed-from`), with `--since` | the base's tree has it. The kind of such a path says whether it is there now ([`Origin::bare`]): a new source file is a node of the refreshed graph like an old one |
//! | paths without a status, no base | something in it is no path of the change: the change is what was given, so the rest was there. A directory in which everything is a changed path may be new |
//! | lines with a status (`--changed-from`), no base | as for a diff: a path below it is stated as modified or deleted; else as for paths without a status |
//! | changes before the base (recordings of end-to-end specs made at an earlier commit, [`Origin::before`]) | never, above a path that they add: it was made since the recording, whatever the base has |

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::Path,
};

use turborepo_trace_graph::{Change, ChangeKind};

use crate::lockdiff::Git;

/// The directories above `path`, nearest first (`a/b/c.ts`: `a/b`, `a`).
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.rmatch_indices('/').map(|(i, _)| &path[..i])
}

/// Where the kinds of a change set come from, for the changes that no diff
/// against the base stated.
pub(crate) struct Origin<'a> {
    /// Paths given without a status: `Modified` (or `Added`, without a node in
    /// the graph) says that the path is there now, `Deleted` that it is
    /// not. Neither says what the base has.
    pub bare: &'a HashSet<String>,
    /// Changes between an earlier revision and the base: their kinds are stated
    /// against that revision. A path that they add is new for whoever
    /// starts there, and the base's tree has it.
    pub before: &'a [Change],
}

impl Origin<'_> {
    /// A change set whose kinds all come from the diff against the base.
    #[cfg(test)]
    fn diff() -> Origin<'static> {
        static NONE: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
        Origin {
            bare: NONE.get_or_init(HashSet::new),
            before: &[],
        }
    }
}

/// The directories that `changes` (root-relative, of the tree at `root`) made
/// or removed. `asked`: whether the graph holds the record of a lookup that
/// asked for a directory by its name (a directory that no lookup asked for is
/// left out, and the base is not asked about it: it is no change for anything).
pub(crate) fn dirs(
    root: &Path,
    changes: &[Change],
    git: Option<&Git>,
    origin: &Origin<'_>,
    asked: &dyn Fn(&str) -> bool,
) -> Vec<Change> {
    let mut out = Vec::new();
    // Removed. (A directory seen before: so were the ones above it.)
    let mut seen: HashSet<&str> = HashSet::new();
    for c in changes.iter().filter(|c| c.kind == ChangeKind::Deleted) {
        for d in ancestors(&c.path) {
            if !seen.insert(d) || root.join(d).is_dir() {
                break;
            }
            if asked(d) {
                out.push(Change::new(ChangeKind::Deleted, d));
            }
        }
    }
    // Made. What a kind says about the base: only a kind that the diff against the
    // one base stated says that a path was there (see the module docs).
    let several = git.is_some_and(|g| g.bases.len() > 1);
    let before: HashSet<&Change> = origin.before.iter().collect();
    let stated = |c: &Change| !several && !origin.bare.contains(&c.path) && !before.contains(c);
    // A path that was modified or deleted below a directory was there before, and
    // so was the directory.
    let had: HashSet<&str> = changes
        .iter()
        .filter(|c| c.kind != ChangeKind::Added && stated(c))
        .flat_map(|c| ancestors(&c.path))
        .collect();
    // The paths that may be new: added (against some base, when there are several:
    // a directory that a base lacks has nothing but such paths below it), or
    // there now and given without a status.
    let may_be_new = |c: &Change| {
        c.kind == ChangeKind::Added
            || (c.kind == ChangeKind::Modified && origin.bare.contains(&c.path))
    };
    let new: HashSet<&str> = changes
        .iter()
        .filter(|c| may_be_new(c))
        .map(|c| c.path.as_str())
        .collect();
    // Made since an earlier revision: the base's tree is not asked.
    let since_before: HashSet<&str> = origin
        .before
        .iter()
        .filter(|c| c.kind == ChangeKind::Added)
        .flat_map(|c| ancestors(&c.path))
        .collect();
    let candidates: BTreeSet<&str> = new
        .iter()
        .flat_map(|p| ancestors(p))
        .filter(|d| !had.contains(d) || since_before.contains(d))
        .collect();
    let candidates: Vec<&str> = candidates
        .into_iter()
        .filter(|d| asked(d) && root.join(d).is_dir())
        .collect();
    let (presumed, candidates): (Vec<&str>, Vec<&str>) = candidates
        .into_iter()
        .partition(|d| since_before.contains(d));
    for d in presumed {
        out.push(Change::new(ChangeKind::Added, d));
    }
    if candidates.is_empty() {
        return out;
    }
    // Unknown at the base (a failed read): taken to be new.
    let at_base = match git {
        Some(g) => g
            .dirs_at_base(&candidates)
            .unwrap_or_else(|| vec![false; candidates.len()]),
        None => {
            let mut memo = HashMap::new();
            candidates
                .iter()
                .map(|d| !only_new(root, d, &new, &mut memo))
                .collect()
        }
    };
    for (d, there) in candidates.iter().zip(at_base) {
        if !there {
            out.push(Change::new(ChangeKind::Added, *d));
        }
    }
    out
}

/// Whether everything in the directory `dir` is a path of the change that may
/// be new (`node_modules` and `.git` are no part of a change).
fn only_new(root: &Path, dir: &str, new: &HashSet<&str>, memo: &mut HashMap<String, bool>) -> bool {
    if let Some(v) = memo.get(dir) {
        return *v;
    }
    let v = match std::fs::read_dir(root.join(dir)) {
        Ok(entries) => entries.flatten().all(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name == "node_modules" || name == ".git" {
                return true;
            }
            let p = format!("{dir}/{name}");
            match e.file_type() {
                Ok(t) if t.is_dir() => only_new(root, &p, new, memo),
                _ => new.contains(p.as_str()),
            }
        }),
        Err(_) => false,
    };
    memo.insert(dir.to_owned(), v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x\n").unwrap();
    }

    fn paths(v: &[Change], kind: ChangeKind) -> Vec<&str> {
        let mut out: Vec<&str> = v
            .iter()
            .filter(|c| c.kind == kind)
            .map(|c| c.path.as_str())
            .collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn made_and_removed_directories_without_a_base() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(r, "a/old.ts");
        write(r, "a/new/x.ts");
        write(r, "a/new/deep/y.ts");
        write(r, "a/kept/z.ts");
        write(r, "a/kept/added.ts");
        write(r, "b/still/there.ts");
        let changes = [
            Change::new(ChangeKind::Added, "a/new/x.ts"),
            Change::new(ChangeKind::Added, "a/new/deep/y.ts"),
            Change::new(ChangeKind::Added, "a/kept/added.ts"),
            Change::new(ChangeKind::Deleted, "gone/dir/f.ts"),
            Change::new(ChangeKind::Deleted, "b/still/f.ts"),
            Change::new(ChangeKind::Modified, "c/m.ts"),
            Change::new(ChangeKind::Added, "c/sub/n.ts"),
        ];
        let got = dirs(r, &changes, None, &Origin::diff(), &|_| true);
        // `a` has a file that is no addition, `a/kept` too; `c` has a modified file
        // (and `c/sub` is not there in this tree: no directory).
        assert_eq!(paths(&got, ChangeKind::Added), ["a/new", "a/new/deep"]);
        assert_eq!(paths(&got, ChangeKind::Deleted), ["gone", "gone/dir"]);
        // A directory that no lookup asked for is left out.
        let got = dirs(r, &changes, None, &Origin::diff(), &|d| {
            d == "a/new/deep" || d == "gone"
        });
        assert_eq!(paths(&got, ChangeKind::Added), ["a/new/deep"]);
        assert_eq!(paths(&got, ChangeKind::Deleted), ["gone"]);
    }

    fn set(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    /// Paths without a status and no base (`--changed`): "modified" says that
    /// the path is there now. A directory in which everything is a path of
    /// the change may be new; one that holds anything else was there.
    #[test]
    fn paths_without_a_status_say_nothing_about_before() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(r, "a/new/x.ts");
        write(r, "a/new/deep/y.ts");
        write(r, "a/kept/z.ts");
        write(r, "a/kept/edited.ts");
        write(r, "a/half/one.ts");
        // A source file that is new is a node of the refreshed graph: "modified".
        let changes = [
            Change::new(ChangeKind::Modified, "a/new/x.ts"),
            Change::new(ChangeKind::Added, "a/new/deep/y.ts"),
            Change::new(ChangeKind::Modified, "a/kept/edited.ts"),
            Change::new(ChangeKind::Modified, "a/half/one.ts"),
            Change::new(ChangeKind::Deleted, "a/half/gone.ts"),
        ];
        let bare = set(&[
            "a/new/x.ts",
            "a/new/deep/y.ts",
            "a/kept/edited.ts",
            "a/half/one.ts",
            "a/half/gone.ts",
        ]);
        let origin = Origin {
            bare: &bare,
            before: &[],
        };
        let got = dirs(r, &changes, None, &origin, &|_| true);
        // `a/kept` holds a file that is no path of the change; `a` holds `a/kept`.
        // `a/half`: a path that is gone says nothing either (it may never have been
        // there), and everything that is there is a path of the change.
        assert_eq!(
            paths(&got, ChangeKind::Added),
            ["a/half", "a/new", "a/new/deep"]
        );
        // The same paths with the status a diff stated: the modified ones were there.
        let got = dirs(r, &changes, None, &Origin::diff(), &|_| true);
        assert_eq!(paths(&got, ChangeKind::Added), ["a/new/deep"]);
        // Mixed (`--changed-from` with some lines that have a status): a stated
        // modification below a directory is evidence, a path without a status is none.
        let bare = set(&["a/new/x.ts"]);
        let origin = Origin {
            bare: &bare,
            before: &[],
        };
        let got = dirs(r, &changes, None, &origin, &|_| true);
        assert_eq!(paths(&got, ChangeKind::Added), ["a/new", "a/new/deep"]);
    }

    /// Changes before the base (recordings made at an earlier commit): a
    /// directory above a path that they add was made since then, whatever
    /// else the change set says of the paths below it.
    #[test]
    fn a_directory_made_before_the_base_is_made() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(r, "a/since/x.ts");
        write(r, "a/since/other.ts");
        write(r, "a/old/y.ts");
        write(r, "a/old/other.ts");
        let before = [
            Change::new(ChangeKind::Added, "a/since/x.ts"),
            Change::new(ChangeKind::Modified, "a/old/y.ts"),
        ];
        // The diff against the base: both files modified again.
        let changes = [
            Change::new(ChangeKind::Added, "a/since/x.ts"),
            Change::new(ChangeKind::Modified, "a/since/x.ts"),
            Change::new(ChangeKind::Modified, "a/old/y.ts"),
        ];
        let none = HashSet::new();
        let origin = Origin {
            bare: &none,
            before: &before,
        };
        let got = dirs(r, &changes, None, &origin, &|d| d != "a");
        assert_eq!(paths(&got, ChangeKind::Added), ["a/since"]);
        // Without the earlier changes the modification says that `a/since` was there.
        let got = dirs(r, &changes[1..], None, &Origin::diff(), &|d| d != "a");
        assert_eq!(paths(&got, ChangeKind::Added), [] as [&str; 0]);
    }
}
