//! The adapters to Turborepo: the JavaScript workspace members come from
//! turbo's package graph ([`Packages`]), and the changed files from turbo's
//! SCM layer (`SCM::changed_files`, the same code as `turbo run --affected`)
//! with a status layer on top ([`ScmChanges`]). The base ref is resolved and
//! validated by turbo too (`GitRepo::resolve_base`). `turbo trace` starts the
//! same `changed_files` queries while turbo builds its package graph
//! ([`prefetch_requests`]), so the git scans overlap the build.
//!
//! turbo lists changed paths without a status (`git diff --name-only
//! --no-renames`, plus uncommitted and untracked files). The selection needs to
//! know which paths were added, modified or deleted, and which were symbolic
//! links at the base: the status layer reads that from the trees, a path's
//! existence at the base (`git ls-tree`) and in the working tree. A rename is a
//! deletion and an addition, as in the crate's own diff (`crate::lockdiff`).

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use turbopath::AbsoluteSystemPath;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_scm::{GitRepo, SCM, git::ChangedFilesRequest};
use turborepo_trace_graph::{Change, ChangeKind, ChangeList, ChangeSource};
use turborepo_trace_store::workspace::{ListedMembers, Members};

use crate::lockdiff::{Git, git};

/// The JavaScript workspace members of turbo's package graph: the directories
/// of its `package.json` packages, relative to the repository root,
/// `/`-separated.
pub(crate) fn js_member_dirs(graph: &PackageGraph) -> Vec<String> {
    let mut dirs: Vec<String> = graph
        .package_scope_directories()
        .filter(|(name, _)| *name != PackageName::Root)
        .filter(|(name, _)| {
            graph
                .package_view(name)
                .is_some_and(|v| v.is_package_json_scope())
        })
        .map(|(_, dir)| dir.to_unix().to_string())
        .filter(|d| !d.is_empty())
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// What the library needs of turbo's package graph, taken once so that the
/// graph (and the rest of the run) can be dropped before the trace's heavy
/// phase: the JavaScript workspace members and the package name of each
/// package directory.
#[derive(Clone, Debug, Default)]
pub struct Packages {
    members: Vec<String>,
    names: BTreeMap<String, String>,
}

impl Packages {
    /// The members and names of `graph`.
    pub fn new(graph: &PackageGraph) -> Packages {
        let mut names = BTreeMap::new();
        for (name, dir) in graph.package_scope_directories() {
            names
                .entry(dir.to_unix().to_string())
                .or_insert_with(|| name.to_string());
        }
        Packages {
            members: js_member_dirs(graph),
            names,
        }
    }
}

/// The JavaScript members of `packages` as the store's [`Members`].
pub(crate) fn members(packages: &Packages) -> Members {
    Members(std::sync::Arc::new(ListedMembers(packages.members.clone())))
}

/// turbo's package name for a package directory (`.` or `""`: the root, `//`).
pub(crate) fn package_name(packages: &Packages, dir: &str) -> Option<String> {
    if dir.is_empty() || dir == "." {
        return Some("//".to_owned());
    }
    packages.names.get(dir).cloned()
}

/// The changes of `turbo trace affected` from turbo's SCM layer, computed when
/// the source is made (`SCM` stays with the caller).
#[derive(Debug)]
pub(crate) struct ScmChanges {
    /// The base revision as resolved (`--base`, else the GitHub Actions base,
    /// else `main` or `master`).
    base: Option<String>,
    changes: Result<ChangeList, String>,
}

impl ScmChanges {
    /// The changes from `base` to `head` (default: the working tree, with
    /// uncommitted and untracked files). With `head`, the files that differ
    /// between `head` and the working tree count too: the graph is read
    /// from the working tree.
    pub(crate) fn new(
        root: &AbsoluteSystemPath,
        scm: &SCM,
        base: Option<&str>,
        head: Option<&str>,
    ) -> ScmChanges {
        let resolved = match head
            .map(validate_ref)
            .transpose()
            .and_then(|_| resolve_base(scm, base))
        {
            Ok(b) => b,
            Err(e) => {
                return ScmChanges {
                    base: None,
                    changes: Err(e),
                };
            }
        };
        // The debug variable of the engine's tooling (`EXACT_BASE`): the base as
        // given, for clones without history.
        let exact = crate::lockdiff::exact_base(false);
        let git = match Git::new(root.as_std_path(), &resolved, exact) {
            Ok(g) => g,
            Err(e) => {
                return ScmChanges {
                    base: Some(resolved),
                    changes: Err(format!("{e:#}")),
                };
            }
        };
        // The queries of `prefetch_requests`, with the same arguments: the base as
        // given (turbo resolves it again, to the same ref).
        let changes = queries(base, head, !exact)
            .into_iter()
            .try_fold(BTreeSet::new(), |mut out, q| {
                changed_paths(root, scm, &q, &mut out).map(|()| out)
            })
            .and_then(|paths| {
                with_status(root.as_std_path(), &git, &paths).map_err(|e| format!("{e:#}"))
            });
        // The changes are against the merge base, and so must be the base's
        // contents: the merge base itself when there is one (`main` may have
        // moved on since), the ref when there are several (a criss-cross
        // history), which `affected` resolves to the same merge bases.
        let base = match git.bases.as_slice() {
            [one] => one.clone(),
            _ => resolved,
        };
        ScmChanges {
            base: Some(base),
            changes,
        }
    }
}

impl ChangeSource for ScmChanges {
    fn base(&self) -> Option<String> {
        self.base.clone()
    }
    fn changes(&self) -> Result<ChangeList, String> {
        self.changes.clone()
    }
}

/// A ref git would read as an option is refused, as turbo refuses it.
pub(crate) fn validate_ref(r: &str) -> Result<(), String> {
    GitRepo::validate_git_ref(r).map_err(|e| e.to_string())
}

/// The base ref as turbo's SCM layer resolves it (`GitRepo::resolve_base`):
/// the given one (validated), else the GitHub Actions base, else `main`, else
/// `master`.
pub(crate) fn resolve_base(scm: &SCM, given: Option<&str>) -> Result<String, String> {
    match scm {
        SCM::Git(repo) => repo
            .resolve_base(
                given.filter(|b| !b.is_empty()),
                turborepo_scm::git::CIEnv::new(),
            )
            .map_err(|e| match e {
                turborepo_scm::Error::UnableToResolveRef => "no base: pass --base (neither main \
                                                             nor master exists, and no GitHub \
                                                             Actions base resolves)"
                    .to_owned(),
                e => e.to_string(),
            }),
        SCM::Manual(_) => Err("not a git repository".to_owned()),
    }
}

/// One `SCM::changed_files` query: from `from` (`None`: the base turbo
/// resolves) to `to` (`None`: HEAD, and with `uncommitted` the working
/// tree).
struct Query {
    from: Option<String>,
    to: Option<String>,
    uncommitted: bool,
    merge_base: bool,
}

/// The queries of the changes from `base` to `head`: from the merge base of
/// `base` and HEAD to the working tree; with `head`, from that merge base to
/// `head`, and from `head` to the working tree. From `base` itself without
/// `merge_base`.
fn queries(base: Option<&str>, head: Option<&str>, merge_base: bool) -> Vec<Query> {
    let base = base.filter(|b| !b.is_empty()).map(str::to_owned);
    match head {
        None => vec![Query {
            from: base,
            to: None,
            uncommitted: true,
            merge_base,
        }],
        Some(h) => vec![
            Query {
                from: base,
                to: Some(h.to_owned()),
                uncommitted: false,
                merge_base,
            },
            Query {
                from: Some(h.to_owned()),
                to: None,
                uncommitted: true,
                merge_base: false,
            },
        ],
    }
}

/// The `SCM::changed_files` queries [`ScmChanges`] will make for the changes
/// from `base` to `head` at `root`, for `SCM::prefetch_changes` (through
/// `RunBuilder::prefetch_changed_files`): started before turbo's package
/// graph is built, they overlap it.
pub fn prefetch_requests(
    root: &AbsoluteSystemPath,
    base: Option<&str>,
    head: Option<&str>,
) -> Vec<ChangedFilesRequest> {
    let exact = crate::lockdiff::exact_base(false);
    queries(base, head, !exact)
        .into_iter()
        .map(|q| ChangedFilesRequest {
            turbo_root: root.to_owned(),
            from_commit: q.from,
            to_commit: q.to,
            include_uncommitted: q.uncommitted,
            merge_base: q.merge_base,
        })
        .collect()
}

/// Adds the paths of one `SCM::changed_files` query to `out`.
fn changed_paths(
    root: &AbsoluteSystemPath,
    scm: &SCM,
    q: &Query,
    out: &mut BTreeSet<String>,
) -> Result<(), String> {
    match scm.changed_files(
        root,
        q.from.as_deref(),
        q.to.as_deref(),
        q.uncommitted,
        false,
        q.merge_base,
    ) {
        Ok(Ok(files)) => {
            out.extend(files.iter().map(|f| f.to_unix().to_string()));
            Ok(())
        }
        Ok(Err(range)) => Err(format!(
            "cannot diff {} against {}",
            range.from_ref.as_deref().unwrap_or("the base"),
            range.to_ref.as_deref().unwrap_or("HEAD")
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// The status of each changed path: from its existence at each merge base of
/// `base` and HEAD (one, or several in a criss-cross history, as the crate's
/// own diff) and in the working tree. A path at neither is not a change.
fn with_status(root: &Path, g: &Git, paths: &BTreeSet<String>) -> anyhow::Result<ChangeList> {
    let mut changes = Vec::new();
    let mut links = BTreeSet::new();
    let full = |p: &str| format!("{}{p}", g.prefix);
    let list: Vec<&String> = paths.iter().collect();
    for b in &g.bases {
        let mut at_base: std::collections::HashMap<String, bool> = Default::default();
        for chunk in list.chunks(512) {
            let mut args = vec![
                "ls-tree".to_owned(),
                "-z".to_owned(),
                b.clone(),
                "--".to_owned(),
            ];
            args.extend(chunk.iter().map(|p| full(p)));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = git(&g.toplevel, &args)?;
            // `<mode> SP <type> SP <object> TAB <path>`
            for entry in out.split('\0').filter(|e| !e.is_empty()) {
                // A directory at the base is no file there: a file in its place
                // now is added.
                if let Some((meta, path)) = entry.split_once('\t')
                    && !meta.starts_with("040000 ")
                    && let Some(rel) = path.strip_prefix(&g.prefix)
                {
                    at_base.insert(rel.to_owned(), meta.starts_with("120000 "));
                }
            }
        }
        for p in &list {
            let base_link = at_base.get(p.as_str()).copied();
            // A directory in its place now: the file (or link) is deleted, as git
            // says (the files below it are added).
            let now = std::fs::symlink_metadata(root.join(p.as_str())).is_ok_and(|m| !m.is_dir());
            let kind = match (base_link.is_some(), now) {
                (true, true) => ChangeKind::Modified,
                (false, true) => ChangeKind::Added,
                (true, false) => ChangeKind::Deleted,
                (false, false) => continue,
            };
            if base_link == Some(true) {
                links.insert((*p).clone());
            }
            changes.push(Change::new(kind, p.as_str()));
        }
    }
    changes.sort();
    changes.dedup();
    Ok(ChangeList {
        changes,
        links_at_base: links.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        path::{Path, PathBuf},
    };

    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{package_graph::PackageGraph, package_json::PackageJson};
    use turborepo_trace_store::{
        smallfiles::SmallFiles,
        workspace::{DeclaredMembers, WorkspaceMembers},
    };

    /// The repositories of the fixture corpora: directories whose
    /// `package.json` declares workspaces, or that have a
    /// `pnpm-workspace.yaml`.
    fn workspace_roots() -> Vec<PathBuf> {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let corpora = [
            crates.join("turborepo-trace-testkit/fixtures"),
            crates.join("../turborepo-tests/integration/fixtures"),
        ];
        let mut out = Vec::new();
        let mut stack: Vec<PathBuf> = corpora.into_iter().filter(|c| c.is_dir()).collect();
        while let Some(d) = stack.pop() {
            let manifest = d.join("package.json");
            let declares = std::fs::read_to_string(&manifest)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .is_some_and(|v| v.get("workspaces").is_some());
            if declares || (manifest.is_file() && d.join("pnpm-workspace.yaml").is_file()) {
                out.push(d.clone());
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name();
                if e.file_type().is_ok_and(|t| t.is_dir())
                    && name != "node_modules"
                    && name != ".git"
                {
                    stack.push(p);
                }
            }
        }
        out.sort();
        out
    }

    /// The `package.json` files below `root`, root-relative, as the graph walk
    /// lists them (no `node_modules`, no links followed).
    fn manifests(root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                let Ok(t) = e.file_type() else { continue };
                if t.is_dir() && e.file_name() != "node_modules" && e.file_name() != ".git" {
                    stack.push(p);
                } else if t.is_file() && e.file_name() == "package.json" {
                    let rel = p.strip_prefix(root).unwrap_or(&p);
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        out.sort();
        out
    }

    /// The JavaScript workspace members of turbo's package graph are the
    /// packages the crate's own discovery finds, on every repository of the
    /// fixtures.
    #[tokio::test]
    async fn package_roots_match_turbo_package_graph() {
        let mut compared = 0;
        let mut skipped = Vec::new();
        let mut differ = Vec::new();
        for root in workspace_roots() {
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            let files = manifests(&root);
            let ours: BTreeSet<String> = DeclaredMembers
                .js_members(&root, &files, &SmallFiles::in_memory())
                .into_iter()
                .collect();
            let Ok(abs) = AbsoluteSystemPathBuf::try_from(root.as_path()) else {
                skipped.push(format!("{}: not an absolute path", root.display()));
                continue;
            };
            let graph = match PackageJson::load(&abs.join_component("package.json")) {
                Ok(pj) => {
                    PackageGraph::builder(&abs, pj)
                        .with_allow_no_package_manager(true)
                        .build()
                        .await
                }
                Err(e) => {
                    skipped.push(format!("{}: {e}", root.display()));
                    continue;
                }
            };
            let graph = match graph {
                Ok(g) => g,
                Err(e) => {
                    skipped.push(format!("{}: {e}", root.display()));
                    continue;
                }
            };
            let walked: BTreeSet<&str> = files
                .iter()
                .filter_map(|f| f.strip_suffix("/package.json"))
                .collect();
            let theirs: BTreeSet<String> = super::js_member_dirs(&graph)
                .into_iter()
                .filter(|d| walked.contains(d.as_str()))
                .collect();
            compared += 1;
            if ours != theirs {
                differ.push(format!(
                    "{}: only ours {:?}, only turbo {:?}",
                    root.display(),
                    ours.difference(&theirs).collect::<Vec<_>>(),
                    theirs.difference(&ours).collect::<Vec<_>>()
                ));
            }
        }
        eprintln!(
            "compared {compared} repositories; skipped {}:",
            skipped.len()
        );
        for s in &skipped {
            eprintln!("  skipped {s}");
        }
        assert!(compared >= 10, "too few repositories compared: {compared}");
        assert!(
            differ.is_empty(),
            "package roots differ:\n{}",
            differ.join("\n")
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let o = std::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// The crate's own changes for `--since <base>`: the diff against the
    /// merge base and the untracked files.
    fn ours(root: &Path, base: &str) -> (BTreeSet<(String, String)>, Vec<String>) {
        let g = crate::lockdiff::Git::new(root, base, false).unwrap();
        let mut c = g.tracked_changes().unwrap();
        c.extend(g.untracked_with(root, None, None, None).unwrap());
        let set = c
            .into_iter()
            .map(|c| (c.kind.as_str().to_owned(), c.path))
            .collect();
        (set, g.links_at_base())
    }

    /// The changes through turbo's SCM layer and the status layer.
    fn theirs(
        root: &Path,
        base: &str,
        head: Option<&str>,
    ) -> (BTreeSet<(String, String)>, Vec<String>) {
        let abs = AbsoluteSystemPathBuf::try_from(root).unwrap();
        let scm = turborepo_scm::SCM::new(&abs);
        let src = super::ScmChanges::new(&abs, &scm, Some(base), head);
        let list = turborepo_trace_graph::ChangeSource::changes(&src).unwrap();
        let set = list
            .changes
            .into_iter()
            .map(|c| (c.kind.as_str().to_owned(), c.path))
            .collect();
        (set, list.links_at_base)
    }

    /// A repository on `main` with files, directories and a link, and a branch
    /// `work` that `main` gained a commit after (the merge base is the fork
    /// point).
    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = std::fs::canonicalize(d.path()).unwrap();
        git(&r, &["init", "-q"]);
        write(
            &r,
            "package.json",
            r#"{"name":"r","workspaces":["packages/*"]}"#,
        );
        write(&r, "packages/a/package.json", r#"{"name":"a"}"#);
        write(&r, "packages/a/src/x.js", "module.exports = 1;\n");
        write(&r, "packages/a/src/y.js", "module.exports = 2;\n");
        write(&r, "packages/a/old/z.js", "module.exports = 3;\n");
        write(&r, "packages/b/package.json", r#"{"name":"b"}"#);
        write(&r, "packages/b/index.js", "require('a');\n");
        write(&r, "keep.txt", "keep\n");
        #[cfg(unix)]
        std::os::unix::fs::symlink("packages/a/src/x.js", r.join("link.js")).unwrap();
        git(&r, &["add", "-A"]);
        git(&r, &["commit", "-qm", "base"]);
        git(&r, &["checkout", "-qb", "work"]);
        // Committed on the branch: a modification, a deletion, a rename, a new
        // directory, a removed directory, a link retargeted.
        write(&r, "packages/a/src/x.js", "module.exports = 10;\n");
        git(&r, &["rm", "-q", "packages/a/src/y.js"]);
        git(&r, &["mv", "packages/b/index.js", "packages/b/main.js"]);
        write(&r, "packages/c/package.json", r#"{"name":"c"}"#);
        write(&r, "packages/c/lib/n.js", "module.exports = 4;\n");
        git(&r, &["rm", "-rq", "packages/a/old"]);
        #[cfg(unix)]
        {
            std::fs::remove_file(r.join("link.js")).unwrap();
            std::os::unix::fs::symlink("packages/b/main.js", r.join("link.js")).unwrap();
        }
        git(&r, &["add", "-A"]);
        git(&r, &["commit", "-qm", "work"]);
        // `main` moves on: its later changes are not the branch's.
        git(&r, &["checkout", "-q", "main"]);
        write(&r, "keep.txt", "changed on main\n");
        git(&r, &["commit", "-qam", "main moves"]);
        git(&r, &["checkout", "-q", "work"]);
        // Uncommitted: a modification, a staged addition, an untracked file in a new
        // directory, a deleted tracked file, a file replaced by a directory.
        write(&r, "packages/c/lib/n.js", "module.exports = 5;\n");
        write(&r, "packages/c/staged.js", "1;\n");
        git(&r, &["add", "packages/c/staged.js"]);
        write(&r, "untracked/dir/u.js", "2;\n");
        std::fs::remove_file(r.join("packages/b/package.json")).unwrap();
        d
    }

    /// The status layer over turbo's SCM layer gives the changes the crate's
    /// own diff gives: adds, deletes, renames (as a deletion and an
    /// addition), directories made and removed, links at the base,
    /// committed and uncommitted changes.
    #[test]
    fn changes_match_the_engine_diff() {
        let d = repo();
        let r = std::fs::canonicalize(d.path()).unwrap();
        let (a, la) = ours(&r, "main");
        let (b, lb) = theirs(&r, "main", None);
        assert_eq!(a, b, "own diff (left) vs turbo's SCM + status (right)");
        assert_eq!(la, lb, "links at the base");
        assert!(a.contains(&("deleted".to_owned(), "packages/b/index.js".to_owned())));
        assert!(a.contains(&("added".to_owned(), "packages/b/main.js".to_owned())));
        assert!(a.contains(&("added".to_owned(), "untracked/dir/u.js".to_owned())));
        assert!(!a.iter().any(|(_, p)| p == "keep.txt"), "main's own change");
    }

    /// With the base at HEAD, only the working tree's changes.
    #[test]
    fn changes_match_the_engine_diff_from_head() {
        let d = repo();
        let r = std::fs::canonicalize(d.path()).unwrap();
        assert_eq!(ours(&r, "HEAD"), theirs(&r, "HEAD", None));
    }

    /// A clean tree against its own commit: no change either way.
    #[test]
    fn no_changes_on_a_clean_tree() {
        let d = repo();
        let r = std::fs::canonicalize(d.path()).unwrap();
        git(&r, &["add", "-A"]);
        git(&r, &["commit", "-qm", "all"]);
        let (b, _) = theirs(&r, "HEAD", None);
        assert!(b.is_empty(), "{b:?}");
        assert_eq!(ours(&r, "HEAD").0, b);
    }

    /// The engine's changes for `turbo trace affected` without `--head`: the
    /// diff against the merge base, and as untracked files the files of the
    /// graph build's walk that git does not track (`(kind, path)` pairs, the
    /// graph directory left out, as `affected` leaves it out).
    fn walked(root: &Path, base: &str) -> BTreeSet<(String, String)> {
        let loc = crate::graph::GraphLoc {
            root: Some(root.to_path_buf()),
            graph: None,
            members: Default::default(),
        };
        let (_, report) = crate::graph::open_report(&loc, false, false).unwrap();
        let report = report.unwrap();
        let g = crate::lockdiff::Git::new(root, base, false).unwrap();
        let mut c = g.tracked_changes().unwrap();
        c.extend(
            g.untracked_with(
                root,
                Some((&report.walked, &report.walked_links)),
                None,
                None,
            )
            .unwrap(),
        );
        let graph_dir = format!("{}/", turborepo_trace_store::DEFAULT_GRAPH_PATH);
        c.into_iter()
            .filter(|c| !c.path.starts_with(&graph_dir))
            .map(|c| (c.kind.as_str().to_owned(), c.path))
            .collect()
    }

    /// turbo's SCM layer, with the graph directory left out.
    fn theirs_without_graph(root: &Path, base: &str) -> BTreeSet<(String, String)> {
        let graph_dir = format!("{}/", turborepo_trace_store::DEFAULT_GRAPH_PATH);
        theirs(root, base, None)
            .0
            .into_iter()
            .filter(|(_, p)| !p.starts_with(&graph_dir))
            .collect()
    }

    /// Untracked files from the walk are the ones git lists: source files and
    /// others, in new directories and in existing ones, at any depth; ignored
    /// files are in neither; with the committed and uncommitted changes of
    /// [`repo`] around them.
    #[test]
    fn untracked_files_from_the_walk_match_git() {
        let d = repo();
        let r = std::fs::canonicalize(d.path()).unwrap();
        write(&r, ".gitignore", "ignored/\n*.log\n");
        write(&r, "ignored/i.js", "3;\n");
        write(&r, "packages/a/debug.log", "x\n");
        write(&r, "packages/a/src/new.ts", "export const n = 1;\n");
        write(&r, "packages/a/data.json", "{}\n");
        write(&r, "notes.txt", "n\n");
        write(&r, "deep/a/b/c/d.mjs", "export default 1;\n");
        let a = walked(&r, "main");
        let b = theirs_without_graph(&r, "main");
        assert_eq!(
            a, b,
            "the engine with the walk (left) vs turbo's SCM (right)"
        );
        for p in [
            "packages/a/src/new.ts",
            "packages/a/data.json",
            "notes.txt",
            "deep/a/b/c/d.mjs",
            ".gitignore",
            "untracked/dir/u.js",
        ] {
            assert!(
                a.contains(&("added".to_owned(), p.to_owned())),
                "{p}: {a:?}"
            );
        }
        assert!(
            !a.iter()
                .any(|(_, p)| p.starts_with("ignored/") || p.ends_with(".log")),
            "{a:?}"
        );
    }

    /// A clean tree: no change from the walk either.
    #[test]
    fn untracked_files_from_the_walk_on_a_clean_tree() {
        let d = repo();
        let r = std::fs::canonicalize(d.path()).unwrap();
        write(&r, ".gitignore", ".turbo/\n");
        git(&r, &["add", "-A"]);
        git(&r, &["commit", "-qm", "all"]);
        let a = walked(&r, "HEAD");
        assert!(a.is_empty(), "{a:?}");
        assert_eq!(a, theirs_without_graph(&r, "HEAD"));
    }

    /// `--head`: the branch's commits from the base to `head`, and what differs
    /// between `head` and the working tree (the graph is read from there):
    /// the same set as against the base directly.
    #[test]
    fn head_and_working_tree() {
        let d = repo();
        let r = std::fs::canonicalize(d.path()).unwrap();
        let (a, _) = ours(&r, "main");
        let (b, _) = theirs(&r, "main", Some("HEAD"));
        assert_eq!(a, b);
    }

    /// A small repository on `main`: one commit.
    fn change_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = std::fs::canonicalize(d.path()).unwrap();
        git(&r, &["init", "-q"]);
        write(&r, "package.json", r#"{"name":"r"}"#);
        for (rel, text) in files {
            write(&r, rel, text);
        }
        git(&r, &["add", "-A"]);
        git(&r, &["commit", "-qm", "base"]);
        d
    }

    /// An untracked file that a `.ignore` file hides is a change: git does not
    /// read `.ignore` files (the graph's own walk does), and the changes are
    /// git's, as for `turbo run --affected`.
    #[test]
    fn untracked_file_hidden_by_dot_ignore_is_changed() {
        let d = change_repo(&[(".ignore", "hidden/\n*.gen.js\n"), ("src/x.js", "1;\n")]);
        let r = std::fs::canonicalize(d.path()).unwrap();
        write(&r, "hidden/h.js", "2;\n");
        write(&r, "src/y.gen.js", "3;\n");
        let (b, _) = theirs(&r, "HEAD", None);
        for p in ["hidden/h.js", "src/y.gen.js"] {
            assert!(
                b.contains(&("added".to_owned(), p.to_owned())),
                "{p}: {b:?}"
            );
        }
    }

    /// A file replaced by a directory of the same name. git (and
    /// the crate's own diff) says the file is deleted; the status layer sees
    /// a path that exists at the base and "exists" now (as a directory) and
    /// calls it modified.
    #[test]
    fn file_replaced_by_a_directory_is_deleted() {
        let d = change_repo(&[("src/data", "module.exports = 1;\n")]);
        let r = std::fs::canonicalize(d.path()).unwrap();
        std::fs::remove_file(r.join("src/data")).unwrap();
        write(&r, "src/data/note.txt", "x\n");
        let (a, _) = ours(&r, "HEAD");
        let (b, _) = theirs(&r, "HEAD", None);
        assert!(a.contains(&("deleted".to_owned(), "src/data".to_owned())));
        assert_eq!(a, b, "own diff (left) vs turbo's SCM + status (right)");
    }

    /// A link replaced by a directory. git says the link is
    /// deleted (and it was a link at the base); the status layer says
    /// modified.
    #[cfg(unix)]
    #[test]
    fn link_replaced_by_a_directory_is_deleted() {
        let d = change_repo(&[("real/index.js", "module.exports = 1;\n")]);
        let r = std::fs::canonicalize(d.path()).unwrap();
        std::os::unix::fs::symlink("real", r.join("data")).unwrap();
        git(&r, &["add", "-A"]);
        git(&r, &["commit", "-qm", "link"]);
        std::fs::remove_file(r.join("data")).unwrap();
        write(&r, "data/note.txt", "x\n");
        let (a, la) = ours(&r, "HEAD");
        let (b, lb) = theirs(&r, "HEAD", None);
        assert!(a.contains(&("deleted".to_owned(), "data".to_owned())));
        assert_eq!(a, b, "own diff (left) vs turbo's SCM + status (right)");
        assert_eq!(la, lb, "links at the base");
    }

    /// The base a change source names is the commit the changes
    /// and their statuses were computed against. `affected` takes it as it is
    /// (`Git::new(root, base, exact = true)`) and reads the base's files from
    /// it. `ScmChanges::base` gives the ref (`main`), not the merge base: once
    /// `main` has moved on, the statuses are those of the merge base and the
    /// contents those of `main`'s tip.
    #[test]
    fn the_source_base_is_the_merge_base() {
        let d = change_repo(&[("src/x.js", "module.exports = 1;\n")]);
        let r = std::fs::canonicalize(d.path()).unwrap();
        git(&r, &["checkout", "-qb", "work"]);
        write(&r, "src/x.js", "module.exports = 2;\n");
        git(&r, &["commit", "-qam", "work"]);
        git(&r, &["checkout", "-q", "main"]);
        write(&r, "src/x.js", "module.exports = 2;\n");
        git(&r, &["commit", "-qam", "the same edit on main"]);
        git(&r, &["checkout", "-q", "work"]);
        let abs = AbsoluteSystemPathBuf::try_from(r.as_path()).unwrap();
        let scm = turborepo_scm::SCM::new(&abs);
        let src = super::ScmChanges::new(&abs, &scm, Some("main"), None);
        let base = turborepo_trace_graph::ChangeSource::base(&src).unwrap();
        // What `affected` does with it.
        let read_from = crate::lockdiff::Git::new(&r, &base, true).unwrap().rev;
        let merge_base = crate::lockdiff::Git::new(&r, "main", false).unwrap().rev;
        assert_eq!(
            read_from, merge_base,
            "the base's files are read from {read_from} (main's tip), the changes are against \
             {merge_base}"
        );
    }

    /// An unknown base is an error (which selects every test), never a guess.
    #[test]
    fn unknown_base_is_an_error() {
        let d = repo();
        let r = std::fs::canonicalize(d.path()).unwrap();
        let abs = AbsoluteSystemPathBuf::try_from(r.as_path()).unwrap();
        let scm = turborepo_scm::SCM::new(&abs);
        let src = super::ScmChanges::new(&abs, &scm, Some("no-such-ref"), None);
        assert!(turborepo_trace_graph::ChangeSource::changes(&src).is_err());
    }
}
