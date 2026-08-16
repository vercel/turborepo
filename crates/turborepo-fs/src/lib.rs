//! File system utilities for used by Turborepo.
//! At the moment only used for `turbo prune` to copy over package directories.

#![deny(clippy::all)]

use std::{
    collections::{HashMap, HashSet},
    fs::{DirBuilder, FileType, Permissions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

// `fs_err` preserves paths in the error messages unlike `std::fs`
use fs_err as fs;
use ignore::WalkBuilder;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Error walking directory during recursive copy: {0}")]
    Walk(#[from] ignore::Error),
}

pub fn recursive_copy(
    src: impl AsRef<AbsoluteSystemPath>,
    dst: impl AsRef<AbsoluteSystemPath>,
    use_gitignore: bool,
    gitignore_root: Option<&AbsoluteSystemPath>,
) -> Result<(), Error> {
    CopyPlan::unplanned(use_gitignore, gitignore_root).copy(src, dst)
}

/// One path found beneath a source directory during a walk.
struct PlannedEntry {
    /// Path of the entry relative to the source directory.
    suffix: AnchoredSystemPathBuf,
    kind: PlannedEntryKind,
}

enum PlannedEntryKind {
    /// Permissions are captured during the walk so the copy can recreate the
    /// directory without a second `stat`.
    Directory(Permissions),
    File(FileType),
}

/// Copies of directories out of one repository, sharing a single walk.
///
/// Honoring the ignore files that live *above* a source directory means
/// anchoring its walk at the repository root (see [`recursive_copy`]). Walking
/// from the root once per source re-reads every directory on the way down and
/// recompiles every ignore file above it, so the cost of copying `n` packages
/// grows with `n * repository fan-out`. Planning the copies together walks the
/// repository once and hands each source the entries that belong to it.
///
/// Sources that were not planned, or that do not resolve their ignore rules
/// from a shared root, are walked on demand by [`CopyPlan::copy`].
pub struct CopyPlan {
    use_gitignore: bool,
    gitignore_root: Option<AbsoluteSystemPathBuf>,
    planned: HashMap<AbsoluteSystemPathBuf, Vec<PlannedEntry>>,
}

impl CopyPlan {
    /// A plan that walks every source on demand.
    pub fn unplanned(use_gitignore: bool, gitignore_root: Option<&AbsoluteSystemPath>) -> Self {
        Self {
            use_gitignore,
            gitignore_root: gitignore_root.map(|root| root.to_owned()),
            planned: HashMap::new(),
        }
    }

    /// Resolve the contents of every source directory that shares a walk root
    /// with the others, so that copying them performs one walk in total.
    pub fn new<'a>(
        srcs: impl IntoIterator<Item = &'a AbsoluteSystemPath>,
        use_gitignore: bool,
        gitignore_root: Option<&AbsoluteSystemPath>,
    ) -> Result<Self, Error> {
        let mut plan = Self::unplanned(use_gitignore, gitignore_root);
        let Some(root) = gitignore_root else {
            return Ok(plan);
        };

        // Only sources that resolve their ignore rules from the shared root
        // benefit from a shared walk. Anything else keeps walking itself,
        // which is what it would have done on its own anyway.
        let shared: Vec<&AbsoluteSystemPath> = srcs
            .into_iter()
            .filter(|src| walk_root_for(src, use_gitignore, Some(root)) == root)
            .filter(|src| {
                src.symlink_metadata()
                    .is_ok_and(|metadata| metadata.is_dir())
            })
            .collect();

        if !shared.is_empty() {
            collect_entries(root, &shared, use_gitignore, &mut plan.planned)?;
        }

        Ok(plan)
    }

    /// Copy `src` into `dst`, reusing the planned walk when there is one.
    pub fn copy(
        &self,
        src: impl AsRef<AbsoluteSystemPath>,
        dst: impl AsRef<AbsoluteSystemPath>,
    ) -> Result<(), Error> {
        let src = src.as_ref();
        let dst = dst.as_ref();

        if let Some(entries) = self.planned.get(src) {
            return copy_entries(src, dst, entries);
        }

        let src_metadata = src.symlink_metadata()?;
        if !src_metadata.is_dir() {
            return copy_file_with_type(src, src_metadata.file_type(), dst);
        }

        let walk_root = walk_root_for(src, self.use_gitignore, self.gitignore_root.as_deref());
        let mut entries = HashMap::new();
        collect_entries(walk_root, &[src], self.use_gitignore, &mut entries)?;
        match entries.get(src) {
            Some(entries) => copy_entries(src, dst, entries),
            None => Ok(()),
        }
    }
}

/// Where a copy of `src` has to start walking for its ignore rules to match
/// what git would do. Parent `.gitignore` files need to be evaluated from
/// their own directory so rooted patterns keep the same meaning.
fn walk_root_for<'a>(
    src: &'a AbsoluteSystemPath,
    use_gitignore: bool,
    gitignore_root: Option<&'a AbsoluteSystemPath>,
) -> &'a AbsoluteSystemPath {
    match gitignore_root {
        Some(root)
            if use_gitignore
                && src != root
                && src.as_std_path().starts_with(root.as_std_path()) =>
        {
            root
        }
        _ => src,
    }
}

/// Walk `walk_root` once and record the entries belonging to each source in
/// `srcs`. Every source must be `walk_root` or live beneath it.
fn collect_entries(
    walk_root: &AbsoluteSystemPath,
    srcs: &[&AbsoluteSystemPath],
    use_gitignore: bool,
    out: &mut HashMap<AbsoluteSystemPathBuf, Vec<PlannedEntry>>,
) -> Result<(), Error> {
    let walk_from_root = !(srcs.len() == 1 && srcs[0] == walk_root);

    let mut builder = WalkBuilder::new(walk_root.as_path());
    builder
        .hidden(false)
        .parents(false)
        .require_git(false)
        .git_ignore(use_gitignore)
        .git_global(false)
        .git_exclude(use_gitignore);

    let sources: Arc<HashSet<PathBuf>> = Arc::new(
        srcs.iter()
            .map(|src| src.as_std_path().to_path_buf())
            .collect(),
    );

    if walk_from_root {
        // Directories that have to be descended into to reach a source.
        let mut approaches: HashSet<PathBuf> = HashSet::new();
        for src in srcs {
            for ancestor in src.as_std_path().ancestors().skip(1) {
                if !ancestor.starts_with(walk_root.as_std_path()) {
                    break;
                }
                approaches.insert(ancestor.to_path_buf());
            }
        }

        let filter_sources = sources.clone();
        builder.filter_entry(move |entry| {
            let path = entry.path();
            if owning_sources(&filter_sources, path).next().is_some() {
                return true;
            }
            entry
                .file_type()
                .is_some_and(|file_type| file_type.is_dir())
                && approaches.contains(path)
        });
    }

    for entry in builder.build() {
        let entry = match entry {
            Err(e) => {
                if e.io_error().is_some() {
                    // Matches go behavior where we translate path errors
                    // into skipping the path we're currently walking
                    continue;
                } else {
                    return Err(e.into());
                }
            }
            Ok(entry) => entry,
        };

        let path = AbsoluteSystemPath::from_std_path(entry.path())?;
        let mut owners = owning_sources(&sources, entry.path()).peekable();
        if owners.peek().is_none() {
            continue;
        }

        let file_type = match entry.file_type() {
            Some(file_type) => file_type,
            None => entry.metadata()?.file_type(),
        };

        // Note that we also don't currently copy broken symlinks
        if file_type.is_symlink() && path.stat().is_err() {
            // If we have a broken link, skip this entry
            continue;
        }

        let kind = if file_type.is_dir() {
            PlannedEntryKind::Directory(entry.metadata()?.permissions())
        } else if file_type.is_file() || file_type.is_symlink() {
            PlannedEntryKind::File(file_type)
        } else {
            // Skip special files (sockets, FIFOs, device nodes)
            continue;
        };

        for owner in owners {
            let owner = AbsoluteSystemPath::from_std_path(owner)?;
            let suffix = AnchoredSystemPathBuf::new(owner, path)?;
            let kind = match &kind {
                PlannedEntryKind::Directory(permissions) => {
                    PlannedEntryKind::Directory(permissions.clone())
                }
                PlannedEntryKind::File(file_type) => PlannedEntryKind::File(*file_type),
            };
            out.entry(owner.to_owned())
                .or_default()
                .push(PlannedEntry { suffix, kind });
        }
    }

    Ok(())
}

/// The sources that contain `path`, nearest first. A source contains itself.
fn owning_sources<'a>(
    sources: &'a HashSet<PathBuf>,
    path: &'a Path,
) -> impl Iterator<Item = &'a Path> {
    path.ancestors()
        .filter(move |ancestor| sources.contains(*ancestor))
}

fn copy_entries(
    src: &AbsoluteSystemPath,
    dst: &AbsoluteSystemPath,
    entries: &[PlannedEntry],
) -> Result<(), Error> {
    for entry in entries {
        let target = dst.resolve(&entry.suffix);
        match &entry.kind {
            PlannedEntryKind::Directory(permissions) => make_dir_copy(&target, permissions)?,
            PlannedEntryKind::File(file_type) => {
                copy_file_with_type(src.resolve(&entry.suffix), *file_type, &target)?
            }
        }
    }

    Ok(())
}

fn make_dir_copy(
    dir: impl AsRef<AbsoluteSystemPath>,
    #[allow(unused_variables)] src_permissions: &Permissions,
) -> Result<(), Error> {
    let dir = dir.as_ref();
    let mut builder = DirBuilder::new();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(src_permissions.mode());
    }
    builder.recursive(true);
    builder.create(dir.as_path())?;
    Ok(())
}

pub fn copy_file(
    from: impl AsRef<AbsoluteSystemPath>,
    to: impl AsRef<AbsoluteSystemPath>,
) -> Result<(), Error> {
    let from = from.as_ref();
    let metadata = from.symlink_metadata()?;
    copy_file_with_type(from, metadata.file_type(), to)
}

fn copy_file_with_type(
    from: impl AsRef<AbsoluteSystemPath>,
    from_type: FileType,
    to: impl AsRef<AbsoluteSystemPath>,
) -> Result<(), Error> {
    let from = from.as_ref();
    let to = to.as_ref();
    if from_type.is_symlink() {
        let target = from.read_link()?;
        to.ensure_dir()?;
        if to.symlink_metadata().is_ok() {
            to.remove_file()?;
        }
        to.symlink_to_file(target)?;
        Ok(())
    } else {
        to.ensure_dir()?;
        fs::copy(from.as_path(), to.as_path())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    fn tmp_dir() -> Result<(tempfile::TempDir, AbsoluteSystemPathBuf), Error> {
        let tmp_dir = tempfile::tempdir()?;
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())?;
        Ok((tmp_dir, dir))
    }

    #[test]
    fn test_copy_missing_file() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        let err = match copy_file(src_file, dst_file) {
            Err(Error::Path(err)) => err,
            Err(err) => panic!("expected path error, got {err}"),
            Ok(()) => panic!("expected path error, got success"),
        };
        assert!(err.is_io_error(io::ErrorKind::NotFound));
        Ok(())
    }

    #[test]
    fn test_basic_copy_file() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // src exists, dst doesn't
        src_file.create_with_contents("src")?;

        copy_file(&src_file, &dst_file)?;
        assert_file_matches(&src_file, &dst_file)?;
        Ok(())
    }

    #[test]
    fn test_symlinks() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_symlink = src_dir.join_component("symlink");

        let (_target_tmp, target_dir) = tmp_dir()?;
        let src_target = target_dir.join_component("target");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // create symlink target
        src_target.create_with_contents("target")?;
        src_symlink.symlink_to_file(src_target.as_path())?;

        copy_file(&src_symlink, &dst_file)?;
        assert_target_matches(&dst_file, &src_target)?;
        Ok(())
    }

    #[test]
    fn test_symlink_to_dir() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_symlink = src_dir.join_component("symlink");

        let (_target_tmp, target_dir) = tmp_dir()?;
        let src_target = target_dir.join_component("target");

        let target_a = src_target.join_component("a");
        target_a.ensure_dir()?;
        target_a.create_with_contents("solid")?;

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // create symlink target
        src_symlink.symlink_to_dir(src_target.as_path())?;

        copy_file(&src_symlink, &dst_file)?;
        assert_target_matches(&dst_file, &src_target)?;

        let target = dst_file.read_link()?;
        assert_eq!(target.read_dir()?.count(), 1);

        Ok(())
    }

    #[test]
    fn test_copy_file_with_perms() -> Result<(), Error> {
        let (_src_tmp, src_dir) = tmp_dir()?;
        let src_file = src_dir.join_component("src");

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        let dst_file = dst_dir.join_component("dest");

        // src exists, dst doesn't
        src_file.create_with_contents("src")?;
        src_file.set_readonly()?;

        copy_file(&src_file, &dst_file)?;
        assert_file_matches(&src_file, &dst_file)?;
        assert!(dst_file.is_readonly()?);
        Ok(())
    }

    #[test]
    fn test_recursive_copy() -> Result<(), Error> {
        // Directory layout:
        //
        // <src>/
        //   b
        //   child/
        //     a
        //     link -> ../b
        //     broken -> missing
        //     circle -> ../child
        //     other -> ../sibling
        //   sibling/
        //     c
        let (_src_tmp, src_dir) = tmp_dir()?;

        let sibling_dir = src_dir.join_component("sibling");
        let c_path = sibling_dir.join_component("c");
        c_path.ensure_dir()?;
        c_path.create_with_contents("right here")?;

        let child_dir = src_dir.join_component("child");
        let a_path = child_dir.join_component("a");
        a_path.ensure_dir()?;
        a_path.create_with_contents("hello")?;

        let b_path = src_dir.join_component("b");
        b_path.create_with_contents("bFile")?;

        let link_path = child_dir.join_component("link");
        link_path.symlink_to_file(["..", "b"].join(std::path::MAIN_SEPARATOR_STR))?;

        let broken_link_path = child_dir.join_component("broken");
        broken_link_path.symlink_to_file("missing")?;

        let circle_path = child_dir.join_component("circle");
        circle_path.symlink_to_dir(["..", "child"].join(std::path::MAIN_SEPARATOR_STR))?;

        let other_path = child_dir.join_component("other");
        other_path.symlink_to_dir(["..", "sibling"].join(std::path::MAIN_SEPARATOR_STR))?;

        let (_dst_tmp, dst_dir) = tmp_dir()?;

        recursive_copy(&src_dir, &dst_dir, true, None)?;

        // Ensure double copy doesn't error
        recursive_copy(&src_dir, &dst_dir, true, None)?;

        let dst_child_path = dst_dir.join_component("child");
        let dst_a_path = dst_child_path.join_component("a");
        assert_file_matches(&a_path, dst_a_path)?;

        let dst_b_path = dst_dir.join_component("b");
        assert_file_matches(&b_path, dst_b_path)?;

        let dst_link_path = dst_child_path.join_component("link");
        assert_target_matches(
            dst_link_path,
            ["..", "b"].join(std::path::MAIN_SEPARATOR_STR),
        )?;

        let dst_broken_path = dst_child_path.join_component("broken");
        assert!(!dst_broken_path.as_path().exists());

        let dst_circle_path = dst_child_path.join_component("circle");
        let dst_circle_metadata = fs::symlink_metadata(dst_circle_path)?;
        assert!(dst_circle_metadata.is_symlink());

        let num_files = fs::read_dir(dst_child_path.as_path())?.count();
        // We don't copy the broken symlink so there are only 4 entries
        assert_eq!(num_files, 4);

        let dst_other_path = dst_child_path.join_component("other");

        let dst_other_metadata = fs::symlink_metadata(dst_other_path.as_path())?;
        assert!(dst_other_metadata.is_symlink());

        let dst_c_path = dst_other_path.join_component("c");

        assert_file_matches(&c_path, dst_c_path)?;

        Ok(())
    }

    #[test_case(true)]
    #[test_case(false)]
    fn test_recursive_copy_gitignore(use_gitignore: bool) -> Result<(), Error> {
        // Directory layout:
        //
        // <src>/
        //   .gitignore
        //   invisible.txt <- ignored
        //   dist/ <- ignored
        //     output.txt
        //   child/
        //     seen.txt
        //     .hidden
        let (_src_tmp, src_dir) = tmp_dir()?;
        // Need to create this for `.gitignore` to be respected
        src_dir.join_component(".git").create_dir_all()?;
        src_dir
            .join_component(".gitignore")
            .create_with_contents("invisible.txt\ndist/\n")?;
        src_dir
            .join_component("invisible.txt")
            .create_with_contents("not here")?;
        let output = src_dir.join_components(&["dist", "output.txt"]);
        output.ensure_dir()?;
        output.create_with_contents("hi!")?;

        let child = src_dir.join_component("child");
        let seen = child.join_component("seen.txt");
        seen.ensure_dir()?;
        seen.create_with_contents("here")?;
        let hidden = child.join_component(".hidden");
        hidden.create_with_contents("polo")?;

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        recursive_copy(&src_dir, &dst_dir, use_gitignore, None)?;

        assert!(dst_dir.join_component(".gitignore").exists());
        assert_eq!(
            !dst_dir.join_component("invisible.txt").exists(),
            use_gitignore
        );
        assert_eq!(!dst_dir.join_component("dist").exists(), use_gitignore);
        assert!(dst_dir.join_component("child").exists());
        assert!(dst_dir.join_components(&["child", "seen.txt"]).exists());
        assert!(dst_dir.join_components(&["child", ".hidden"]).exists());

        Ok(())
    }

    #[test_case(true)]
    #[test_case(false)]
    fn test_recursive_copy_respects_repo_gitignore(use_gitignore: bool) -> Result<(), Error> {
        let (_root_tmp, root) = tmp_dir()?;
        root.join_component(".git").create_dir_all()?;
        root.join_component(".gitignore")
            .create_with_contents("node_modules\n/apps/web/coverage\n*.log\n")?;

        let web = root.join_components(&["apps", "web"]);
        web.create_dir_all()?;
        web.join_component("package.json")
            .create_with_contents("{}")?;

        let node_modules = web.join_components(&["node_modules", "dep", "index.js"]);
        node_modules.ensure_dir()?;
        node_modules.create_with_contents("dep")?;

        let coverage = web.join_components(&["coverage", "report.txt"]);
        coverage.ensure_dir()?;
        coverage.create_with_contents("report")?;

        let nested_coverage = web.join_components(&["src", "api", "coverage", "keep.js"]);
        nested_coverage.ensure_dir()?;
        nested_coverage.create_with_contents("keep")?;

        web.join_components(&["src", "api", ".gitignore"])
            .create_with_contents("ignored.js\n")?;
        web.join_components(&["src", "api", "ignored.js"])
            .create_with_contents("ignored")?;

        let keep_log = web.join_components(&["logs", "keep.log"]);
        keep_log.ensure_dir()?;
        web.join_components(&["logs", ".gitignore"])
            .create_with_contents("!keep.log\n")?;
        keep_log.create_with_contents("keep")?;
        web.join_components(&["logs", "drop.log"])
            .create_with_contents("drop")?;

        let (_dst_tmp, dst_dir) = tmp_dir()?;
        recursive_copy(&web, &dst_dir, use_gitignore, Some(&root))?;

        assert!(dst_dir.join_component("package.json").exists());
        assert!(
            dst_dir
                .join_components(&["src", "api", "coverage", "keep.js"])
                .exists()
        );

        assert_eq!(
            !dst_dir
                .join_components(&["node_modules", "dep", "index.js"])
                .exists(),
            use_gitignore
        );
        assert_eq!(
            !dst_dir
                .join_components(&["coverage", "report.txt"])
                .exists(),
            use_gitignore
        );
        assert_eq!(
            !dst_dir
                .join_components(&["src", "api", "ignored.js"])
                .exists(),
            use_gitignore
        );
        assert!(dst_dir.join_components(&["logs", "keep.log"]).exists());
        assert_eq!(
            !dst_dir.join_components(&["logs", "drop.log"]).exists(),
            use_gitignore
        );

        Ok(())
    }

    /// A repository whose ignore rules exercise every place the walk root
    /// matters: a root ignore file, a package-level ignore file, a negation
    /// that only the nearest ignore file can express, and an anchored root
    /// pattern that must not match the same relative path in another package.
    fn ignore_fixture() -> Result<(tempfile::TempDir, AbsoluteSystemPathBuf), Error> {
        let (tmp, root) = tmp_dir()?;
        root.join_component(".git").create_dir_all()?;
        root.join_component(".gitignore")
            .create_with_contents("node_modules\n/apps/web/coverage\n*.log\n")?;

        for package in fixture_packages() {
            let dir = root.join_components(&package);
            dir.create_dir_all()?;
            dir.join_component("package.json")
                .create_with_contents("{}")?;
            let source = dir.join_components(&["src", "index.ts"]);
            source.ensure_dir()?;
            source.create_with_contents("export {}")?;
            let dep = dir.join_components(&["node_modules", "dep", "index.js"]);
            dep.ensure_dir()?;
            dep.create_with_contents("dep")?;
            let coverage = dir.join_components(&["coverage", "report.txt"]);
            coverage.ensure_dir()?;
            coverage.create_with_contents("report")?;
            let keep = dir.join_components(&["logs", "keep.log"]);
            keep.ensure_dir()?;
            keep.create_with_contents("keep")?;
            dir.join_components(&["logs", ".gitignore"])
                .create_with_contents("!keep.log\n")?;
            dir.join_components(&["logs", "drop.log"])
                .create_with_contents("drop")?;
        }

        Ok((tmp, root))
    }

    fn fixture_packages() -> [[&'static str; 2]; 3] {
        [["apps", "web"], ["apps", "docs"], ["packages", "ui"]]
    }

    /// Every path under `dir`, relative to it, sorted.
    fn listing(dir: &AbsoluteSystemPath) -> Result<Vec<String>, Error> {
        fn collect(
            base: &AbsoluteSystemPath,
            dir: &AbsoluteSystemPath,
            paths: &mut Vec<String>,
        ) -> Result<(), Error> {
            for entry in fs::read_dir(dir.as_path())? {
                let entry = entry?;
                let path = AbsoluteSystemPathBuf::try_from(entry.path().as_path())?;
                paths.push(AnchoredSystemPathBuf::new(base, &path)?.to_string());
                if entry.file_type()?.is_dir() {
                    collect(base, &path, paths)?;
                }
            }
            Ok(())
        }

        let mut paths = Vec::new();
        collect(dir, dir, &mut paths)?;
        paths.sort();
        Ok(paths)
    }

    #[test_case(true)]
    #[test_case(false)]
    fn test_copy_plan_matches_individual_copies(use_gitignore: bool) -> Result<(), Error> {
        let (_root_tmp, root) = ignore_fixture()?;
        let packages: Vec<AbsoluteSystemPathBuf> = fixture_packages()
            .iter()
            .map(|package| root.join_components(package))
            .collect();

        let plan = CopyPlan::new(
            packages.iter().map(|package| package.as_ref()),
            use_gitignore,
            Some(&root),
        )?;

        for package in &packages {
            let (_planned_tmp, planned_dst) = tmp_dir()?;
            let (_single_tmp, single_dst) = tmp_dir()?;

            plan.copy(package, &planned_dst)?;
            recursive_copy(package, &single_dst, use_gitignore, Some(&root))?;

            assert_eq!(
                listing(&planned_dst)?,
                listing(&single_dst)?,
                "planned copy of {package} should match an individual copy"
            );
        }

        Ok(())
    }

    #[test]
    fn test_copy_plan_honors_ignore_files_above_the_package() -> Result<(), Error> {
        let (_root_tmp, root) = ignore_fixture()?;
        let web = root.join_components(&["apps", "web"]);
        let docs = root.join_components(&["apps", "docs"]);

        let plan = CopyPlan::new([web.as_ref(), docs.as_ref()], true, Some(&root))?;

        let (_web_tmp, web_dst) = tmp_dir()?;
        plan.copy(&web, &web_dst)?;
        let (_docs_tmp, docs_dst) = tmp_dir()?;
        plan.copy(&docs, &docs_dst)?;

        for dst in [&web_dst, &docs_dst] {
            assert!(dst.join_components(&["src", "index.ts"]).exists());
            // `node_modules` from the root ignore file
            assert!(!dst.join_component("node_modules").exists());
            // `*.log` from the root ignore file, and the nearer negation of it
            assert!(dst.join_components(&["logs", "keep.log"]).exists());
            assert!(!dst.join_components(&["logs", "drop.log"]).exists());
        }

        // `/apps/web/coverage` is anchored at the root, so it must not take
        // the identically named directory in another package with it.
        assert!(
            !web_dst
                .join_components(&["coverage", "report.txt"])
                .exists()
        );
        assert!(
            docs_dst
                .join_components(&["coverage", "report.txt"])
                .exists()
        );

        Ok(())
    }

    #[test]
    fn test_copy_plan_honors_ignore_files_without_git_metadata() -> Result<(), Error> {
        let (_root_tmp, root) = ignore_fixture()?;
        root.join_component(".git").remove_dir_all()?;
        let web = root.join_components(&["apps", "web"]);

        let plan = CopyPlan::new([web.as_ref()], true, Some(&root))?;
        let (_web_tmp, web_dst) = tmp_dir()?;
        plan.copy(&web, &web_dst)?;

        assert!(web_dst.join_components(&["src", "index.ts"]).exists());
        assert!(!web_dst.join_component("node_modules").exists());

        Ok(())
    }

    #[test]
    fn test_copy_plan_walks_unplanned_sources() -> Result<(), Error> {
        let (_root_tmp, root) = ignore_fixture()?;
        let web = root.join_components(&["apps", "web"]);
        let ui = root.join_components(&["packages", "ui"]);

        // Only `web` is planned; `ui` has to fall back to its own walk.
        let plan = CopyPlan::new([web.as_ref()], true, Some(&root))?;

        let (_ui_tmp, planned_dst) = tmp_dir()?;
        plan.copy(&ui, &planned_dst)?;
        let (_single_tmp, single_dst) = tmp_dir()?;
        recursive_copy(&ui, &single_dst, true, Some(&root))?;

        assert_eq!(listing(&planned_dst)?, listing(&single_dst)?);
        assert!(!planned_dst.join_component("node_modules").exists());

        Ok(())
    }

    #[test]
    fn test_copy_plan_copies_nested_packages_into_both_sources() -> Result<(), Error> {
        let (_root_tmp, root) = ignore_fixture()?;
        let web = root.join_components(&["apps", "web"]);
        let nested = web.join_components(&["nested", "pkg"]);
        nested.create_dir_all()?;
        nested
            .join_component("package.json")
            .create_with_contents("{}")?;

        let plan = CopyPlan::new([web.as_ref(), nested.as_ref()], true, Some(&root))?;

        let (_web_tmp, web_dst) = tmp_dir()?;
        plan.copy(&web, &web_dst)?;
        let (_nested_tmp, nested_dst) = tmp_dir()?;
        plan.copy(&nested, &nested_dst)?;

        assert!(
            web_dst
                .join_components(&["nested", "pkg", "package.json"])
                .exists()
        );
        assert!(nested_dst.join_component("package.json").exists());

        Ok(())
    }

    fn assert_file_matches(
        a: impl AsRef<AbsoluteSystemPath>,
        b: impl AsRef<AbsoluteSystemPath>,
    ) -> Result<(), Error> {
        let a = a.as_ref();
        let b = b.as_ref();
        let a_contents = fs::read_to_string(a.as_path())?;
        let b_contents = fs::read_to_string(b.as_path())?;
        assert_eq!(a_contents, b_contents);
        Ok(())
    }

    fn assert_target_matches(
        link: impl AsRef<AbsoluteSystemPath>,
        expected: impl AsRef<Path>,
    ) -> Result<(), Error> {
        let link = link.as_ref();
        let path = link.read_link()?;
        assert_eq!(path.as_path(), expected.as_ref());
        Ok(())
    }
}
