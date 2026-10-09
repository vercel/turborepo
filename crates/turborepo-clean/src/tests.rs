//! Planning and removal over real directories. Each review finding has a
//! regression test reproducing its case.

use std::{collections::BTreeSet, process::Command, str::FromStr};

use tempfile::TempDir;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPath};

use crate::{Error, RemovalKind, Report, SkipReason, plan_paths};

fn repo(tmp: &TempDir) -> AbsoluteSystemPathBuf {
    AbsoluteSystemPathBuf::try_from(tmp.path())
        .unwrap()
        .to_realpath()
        .unwrap()
}

fn path(root: &AbsoluteSystemPath, relative: &str) -> AbsoluteSystemPathBuf {
    root.join_unix_path(RelativeUnixPath::new(relative).unwrap())
}

fn write(root: &AbsoluteSystemPath, relative: &str, contents: &str) -> AbsoluteSystemPathBuf {
    let file = path(root, relative);
    file.ensure_dir().unwrap();
    file.create_with_contents(contents).unwrap();
    file
}

fn exists(root: &AbsoluteSystemPath, relative: &str) -> bool {
    path(root, relative).symlink_metadata().is_ok()
}

fn git(root: &AbsoluteSystemPath, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=test",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

/// A git repository tracking `tracked` and the root `package.json`;
/// everything else stays untracked.
fn git_repo(root: &AbsoluteSystemPath, tracked: &[&str]) {
    write(root, "README.md", "readme");
    write(root, "package.json", "{}");
    git(root, &["init", "--quiet"]);
    git(root, &["add", "README.md", "package.json"]);
    for file in tracked {
        git(root, &["add", "--force", file]);
    }
    git(root, &["commit", "--quiet", "-m", "init"]);
}

/// Candidates exactly as the run cache's glob walk produces them.
fn matches(
    root: &AbsoluteSystemPath,
    inclusions: &[&str],
    exclusions: &[&str],
) -> BTreeSet<AbsoluteSystemPathBuf> {
    let validate = |globs: &[&str]| -> Vec<globwalk::ValidatedGlob> {
        globs
            .iter()
            .map(|glob| globwalk::ValidatedGlob::from_str(glob).unwrap())
            .collect()
    };
    globwalk::globwalk(
        root,
        &validate(inclusions),
        &validate(exclusions),
        globwalk::WalkType::All,
    )
    .unwrap()
    .into_iter()
    .collect()
}

/// Executes a plan. Where deletion is unsupported it must be refused, and
/// nothing is removed.
fn execute(plan: &crate::CleanPlan) -> Report {
    match plan.execute() {
        Ok(report) => report,
        Err(Error::DeletionUnsupported) if !cfg!(unix) => Report::default(),
        Err(error) => panic!("{error}"),
    }
}

fn removed(plan: &crate::CleanPlan) -> Vec<String> {
    let mut paths: Vec<String> = plan.removals().map(|(path, _)| path).collect();
    paths.sort();
    paths
}

fn skipped(plan: &crate::CleanPlan) -> Vec<(String, SkipReason)> {
    plan.skipped()
        .map(|(path, reason)| (path.to_owned(), reason))
        .collect()
}

#[cfg(unix)]
#[test]
fn removes_matched_outputs_and_keeps_exclusions() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/web/dist/index.js", "");
    write(&root, "packages/web/dist/nested/chunk.js", "");
    write(&root, "packages/web/dist/keep.txt", "");
    write(&root, "packages/web/src/index.ts", "");
    git_repo(&root, &["packages/web/src/index.ts"]);

    let plan = plan_paths(
        &root,
        &["packages/web"],
        matches(
            &root,
            &["packages/web/dist/**"],
            &["packages/web/dist/keep.txt"],
        ),
    )
    .unwrap();
    let report = execute(&plan);

    assert_eq!(
        removed(&plan),
        [
            "packages/web/dist/index.js",
            "packages/web/dist/nested",
            "packages/web/dist/nested/chunk.js",
        ]
    );
    assert!(report.is_complete());
    assert_eq!((report.files, report.directories), (2, 1));
    assert!(exists(&root, "packages/web/dist/keep.txt"));
    assert!(exists(&root, "packages/web/src/index.ts"));
}

#[test]
fn planning_alone_deletes_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/web/dist/index.js", "");
    git_repo(&root, &[]);

    let plan = plan_paths(
        &root,
        &["packages/web"],
        matches(&root, &["packages/web/dist/**"], &[]),
    )
    .unwrap();

    assert_eq!((plan.file_count(), plan.directory_count()), (1, 1));
    assert_eq!(
        plan.describe(true),
        [
            "Would remove:",
            "  packages/web/dist/",
            "Would remove 1 file and 1 directory."
        ]
    );
    assert!(exists(&root, "packages/web/dist/index.js"));
}

#[test]
fn never_touches_paths_outside_the_repository_or_reserved_directories() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    let repo_root = root.join_component("repo");
    let outside = write(&root, "outside/precious.txt", "keep");
    write(&repo_root, "packages/web/node_modules/dep/index.js", "");
    write(&repo_root, "packages/web/.turbo/turbo-build.log", "");
    git_repo(&repo_root, &[]);

    let mut candidates = matches(&repo_root, &["packages/web/node_modules/**"], &[]);
    candidates.insert(outside.clone());
    candidates.insert(path(&repo_root, "packages/web/.turbo/turbo-build.log"));
    candidates.insert(path(&repo_root, "packages/web"));
    candidates.insert(path(&repo_root, "packages"));
    let plan = plan_paths(&repo_root, &["packages/web"], candidates).unwrap();
    execute(&plan);

    assert!(plan.is_empty(), "{:?}", removed(&plan));
    assert_eq!(outside.read_to_string().unwrap(), "keep");
    assert!(exists(&repo_root, "packages/web/node_modules/dep/index.js"));
    let reasons: BTreeSet<SkipReason> = plan.skipped().map(|(_, reason)| reason).collect();
    assert_eq!(
        reasons,
        BTreeSet::from([
            SkipReason::OutsideRepository,
            SkipReason::PackageDirectory,
            SkipReason::ReservedDirectory,
        ])
    );
}

/// Security #4: no `git` repository means the tracked set is unknown.
#[test]
fn without_git_nothing_is_deleted() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/a/gen/t.ts", "source");

    let result = plan_paths(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/gen/**"], &[]),
    );

    assert!(matches!(result, Err(Error::TrackedFilesUnknown { .. })));
    assert!(exists(&root, "packages/a/gen/t.ts"));
}

/// Security #4: a repository whose index file is gone is not "nothing
/// tracked".
#[test]
fn a_repository_without_an_index_deletes_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/a/gen/t.ts", "source");
    git_repo(&root, &["packages/a/gen/t.ts"]);
    path(&root, ".git/index").remove_file().unwrap();

    let result = plan_paths(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/gen/**"], &[]),
    );

    assert!(matches!(result, Err(Error::TrackedFilesUnknown { .. })));
    assert!(exists(&root, "packages/a/gen/t.ts"));
}

/// Security #1 / correctness F2: a literal output spelled with another case
/// opens the tracked file on case-insensitive filesystems. It is compared by
/// its stored name, so it is kept, uncommitted edits included.
#[test]
fn a_tracked_file_spelled_with_another_case_is_kept() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/a/index.ts", "orig");
    write(&root, "packages/a/README.MD", "docs");
    git_repo(&root, &["packages/a/index.ts", "packages/a/README.MD"]);
    write(&root, "packages/a/index.ts", "UNCOMMITTED WORK");

    let mut candidates = BTreeSet::new();
    for spelled in ["packages/a/Index.ts", "packages/a/readme.md"] {
        let candidate = path(&root, spelled);
        // Only a case-insensitive filesystem resolves the other spelling.
        if candidate.symlink_metadata().is_ok() {
            candidates.insert(candidate);
        }
    }
    let plan = plan_paths(&root, &["packages/a"], candidates.clone()).unwrap();
    execute(&plan);

    assert!(plan.is_empty());
    assert_eq!(
        path(&root, "packages/a/index.ts").read_to_string().unwrap(),
        "UNCOMMITTED WORK"
    );
    assert!(exists(&root, "packages/a/README.MD"));
    if !candidates.is_empty() {
        assert_eq!(
            skipped(&plan),
            [
                ("packages/a/README.MD".to_owned(), SkipReason::TrackedByGit),
                ("packages/a/index.ts".to_owned(), SkipReason::TrackedByGit),
            ]
        );
    }
}

/// Security #1 / correctness F2: a name stored in one Unicode normalization
/// and spelled (or indexed) in the other is still tracked.
#[cfg(unix)]
#[test]
fn a_tracked_file_in_another_unicode_normalization_is_kept() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    let nfc = "caf\u{e9}.js";
    let nfd = "cafe\u{301}.js";
    write(&root, &format!("dist/{nfd}"), "tracked");
    git_repo(&root, &["dist"]);
    write(&root, "dist/generated.js", "output");

    let mut candidates = matches(&root, &["dist/**"], &[]);
    for spelled in [nfc, nfd] {
        let candidate = path(&root, &format!("dist/{spelled}"));
        if candidate.symlink_metadata().is_ok() {
            candidates.insert(candidate);
        }
    }
    let plan = plan_paths(&root, &[], candidates).unwrap();
    let report = execute(&plan);

    assert_eq!(removed(&plan), ["dist/generated.js"]);
    assert_eq!(report.files, 1);
    assert_eq!(
        path(&root, &format!("dist/{nfd}"))
            .read_to_string()
            .unwrap(),
        "tracked"
    );
}

/// Security #3 / correctness F1: committed symlinks are source too.
#[cfg(unix)]
#[test]
fn tracked_symlinks_are_kept() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/a/src/x.ts", "source");
    std::fs::create_dir_all(path(&root, "packages/a/lib").as_std_path()).unwrap();
    path(&root, "packages/a/lib/srclink")
        .symlink_to_dir("../src")
        .unwrap();
    path(&root, "packages/a/config.js")
        .symlink_to_file("src/x.ts")
        .unwrap();
    git_repo(
        &root,
        &[
            "packages/a/src/x.ts",
            "packages/a/lib/srclink",
            "packages/a/config.js",
        ],
    );

    let mut candidates = matches(&root, &["packages/a/lib/**"], &[]);
    candidates.insert(path(&root, "packages/a/config.js"));
    let plan = plan_paths(&root, &["packages/a"], candidates).unwrap();
    execute(&plan);

    assert!(plan.is_empty(), "{:?}", removed(&plan));
    assert!(exists(&root, "packages/a/lib/srclink"));
    assert!(exists(&root, "packages/a/config.js"));
    assert!(
        skipped(&plan)
            .iter()
            .all(|(_, reason)| *reason == SkipReason::TrackedByGit)
    );
}

/// Correctness F3: files inside a submodule (a gitlink, or any directory
/// holding its own `.git`) belong to another repository.
#[test]
fn submodule_and_nested_repository_contents_are_kept() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(
        &root,
        "dist/vendor/.git",
        "gitdir: ../../.git/modules/vendor",
    );
    write(&root, "dist/vendor/lib.c", "vendored");
    write(&root, "dist/uninitialized/d/y.c", "vendored");
    write(&root, "dist/out.js", "output");
    git_repo(&root, &[]);
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,5b5dbe4fa71b9ba5e09e1e0b5d2bd8ea2b0d4a4d,dist/uninitialized",
        ],
    );

    let plan = plan_paths(&root, &[], matches(&root, &["dist/**"], &[])).unwrap();
    execute(&plan);

    assert_eq!(removed(&plan), ["dist/out.js"]);
    assert!(exists(&root, "dist/vendor/lib.c"));
    assert!(exists(&root, "dist/uninitialized/d/y.c"));
    // Each protected directory is reported once, not once per file.
    assert_eq!(
        skipped(&plan),
        [
            ("dist/uninitialized/".to_owned(), SkipReason::InSubmodule),
            ("dist/vendor/".to_owned(), SkipReason::NestedRepository),
        ]
    );
}

/// Removing a symlink never touches its target, inside or outside the
/// repository.
#[cfg(unix)]
#[test]
fn removes_symlinks_without_following_them() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    let repo_root = root.join_component("repo");
    let outside_file = write(&root, "outside/precious.txt", "keep");
    write(&repo_root, "packages/web/build/index.js", "");
    git_repo(&repo_root, &[]);
    path(&repo_root, "packages/web/dist")
        .symlink_to_dir(outside_file.parent().unwrap().as_str())
        .unwrap();
    path(&repo_root, "packages/web/build/external")
        .symlink_to_dir(outside_file.parent().unwrap().as_str())
        .unwrap();

    let plan = plan_paths(
        &repo_root,
        &["packages/web"],
        matches(
            &repo_root,
            &["packages/web/dist/**", "packages/web/build/**"],
            &[],
        ),
    )
    .unwrap();
    let report = execute(&plan);

    assert_eq!(outside_file.read_to_string().unwrap(), "keep");
    assert!(!exists(&repo_root, "packages/web/dist"));
    assert!(!exists(&repo_root, "packages/web/build"));
    assert_eq!(
        plan.removals()
            .filter(|(_, kind)| *kind == RemovalKind::Symlink)
            .count(),
        2
    );
    assert!(skipped(&plan).is_empty(), "{:?}", skipped(&plan));
    assert!(report.is_complete());
}

/// Security #2: a directory swapped for a symlink after planning must not
/// lead the removal outside the repository.
#[cfg(unix)]
#[test]
fn a_parent_swapped_for_a_symlink_after_planning_is_not_followed() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    let repo_root = root.join_component("repo");
    for i in 0..20 {
        write(&repo_root, &format!("packages/a/dist/f{i}.js"), "");
        write(&root, &format!("victim/f{i}.js"), "precious");
    }
    git_repo(&repo_root, &[]);

    let plan = plan_paths(
        &repo_root,
        &["packages/a"],
        matches(&repo_root, &["packages/a/dist/**"], &[]),
    )
    .unwrap();
    assert_eq!(plan.file_count(), 20);
    std::fs::rename(
        path(&repo_root, "packages/a/dist").as_std_path(),
        path(&repo_root, "packages/a/dist.moved").as_std_path(),
    )
    .unwrap();
    path(&repo_root, "packages/a/dist")
        .symlink_to_dir(root.join_component("victim").as_str())
        .unwrap();
    let report = execute(&plan);

    for i in 0..20 {
        assert!(exists(&root, &format!("victim/f{i}.js")), "f{i}.js deleted");
    }
    assert_eq!(report.files, 0);
    assert_eq!(report.failures.len(), 20);
    assert!(!report.is_complete());
}

/// Correctness F5: a failure is reported, the rest still runs, and the
/// counts are what was actually removed.
#[cfg(unix)]
#[test]
fn continues_past_failures_and_counts_what_was_removed() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "dist/index.js", "");
    write(&root, "dist/sub/a.js", "");
    write(&root, "dist/z/locked.js", "");
    git_repo(&root, &[]);
    let locked = path(&root, "dist/z");
    std::fs::set_permissions(locked.as_std_path(), PermissionsExt::from_mode(0o555)).unwrap();

    let plan = plan_paths(&root, &[], matches(&root, &["dist/**"], &[])).unwrap();
    let report = execute(&plan);
    std::fs::set_permissions(locked.as_std_path(), PermissionsExt::from_mode(0o755)).unwrap();

    assert_eq!((plan.file_count(), plan.directory_count()), (3, 3));
    assert_eq!((report.files, report.directories), (2, 1));
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path, "dist/z/locked.js");
    assert!(exists(&root, "dist/z/locked.js"));
    assert!(!exists(&root, "dist/sub"));
    assert_eq!(
        report.describe().last().unwrap(),
        "Removed 2 files and 1 directory."
    );
}

#[test]
fn refuses_patterns_outside_the_allowlist() {
    let protected = crate::targets::ProtectedDirectories::from_dirs(&[]);
    let refused = |pattern: &str| crate::patterns::refusal(pattern, &[], &protected);
    assert!(refused("dist/**").is_none());
    assert!(refused("{dist,build}/**").is_none());
    assert!(refused("*.tsbuildinfo").is_none());
    for pattern in ["*.*", "?*", "../b/*.*", "dist/*/../../**", "**/*.js"] {
        assert!(refused(pattern).is_some(), "{pattern}");
    }
}

fn git_output(root: &AbsoluteSystemPath, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// Security round 2 #1 / correctness round 2 N0: a collapsed sparse-index
/// directory (`packages/a/gen/`) protects everything beneath it, even when
/// files reappear on disk at its tracked paths.
#[test]
fn sparse_index_directories_protect_everything_beneath_them() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/a/gen/deep/t1.ts", "1");
    write(&root, "packages/a/gen/t2.ts", "2");
    write(&root, "packages/a/src/x.ts", "source");
    git_repo(&root, &["packages"]);
    git(
        &root,
        &["sparse-checkout", "init", "--cone", "--sparse-index"],
    );
    git(&root, &["sparse-checkout", "set", "packages/a/src"]);
    assert!(
        git_output(&root, &["ls-files", "--sparse"])
            .lines()
            .any(|line| line == "packages/a/gen/"),
        "expected a collapsed sparse directory"
    );
    write(&root, "packages/a/gen/deep/t1.ts", "LOCAL");
    write(&root, "packages/a/gen/t2.ts", "LOCAL");

    let plan = plan_paths(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/gen/**"], &[]),
    )
    .unwrap();
    execute(&plan);

    assert!(plan.is_empty(), "{:?}", removed(&plan));
    assert_eq!(
        skipped(&plan),
        [("packages/a/gen/".to_owned(), SkipReason::InSubmodule)]
    );
    assert!(exists(&root, "packages/a/gen/deep/t1.ts"));
    assert!(exists(&root, "packages/a/gen/t2.ts"));
}

/// Security round 2 #2: an index that does not track this repository's root
/// manifest (here, a dotfiles `~/.git` above an untracked project) cannot
/// tell its source from its outputs.
#[test]
fn an_ancestor_repository_that_does_not_track_the_project_deletes_nothing() {
    let tmp = TempDir::new().unwrap();
    let home = repo(&tmp);
    write(&home, ".bashrc", "dotfiles");
    git(&home, &["init", "--quiet"]);
    git(&home, &["add", ".bashrc"]);
    git(&home, &["commit", "--quiet", "-m", "dotfiles"]);
    let project = home.join_component("project");
    write(&project, "package.json", "{}");
    write(&project, "packages/a/gen/t.ts", "HANDWRITTEN");

    let result = plan_paths(
        &project,
        &["packages/a"],
        matches(&project, &["packages/a/gen/**"], &[]),
    );

    let Err(Error::TrackedFilesUnknown { reason }) = result else {
        panic!("expected the tracked set to be unknown: {result:?}");
    };
    assert!(reason.contains("package.json"), "{reason}");
    assert!(exists(&project, "packages/a/gen/t.ts"));
}

/// Security round 2 #2: with `GIT_DIR` and `GIT_WORK_TREE`, the files their
/// index tracks are protected, not those of the `.git` found above.
#[test]
fn the_repository_named_by_git_dir_is_used() {
    let tmp = TempDir::new().unwrap();
    let home = repo(&tmp);
    write(&home, ".bashrc", "dotfiles");
    git(&home, &["init", "--quiet"]);
    git(&home, &["add", ".bashrc"]);
    git(&home, &["commit", "--quiet", "-m", "dotfiles"]);
    let project = home.join_component("project");
    write(&project, "package.json", "{}");
    write(&project, "packages/a/gen/t.ts", "HANDWRITTEN");
    write(&project, "packages/a/gen/out.js", "built");
    git(&home, &["init", "--quiet", "--bare", "store.git"]);
    let output = Command::new("git")
        .args(["add", "package.json", "packages/a/gen/t.ts"])
        .env("GIT_DIR", home.join_component("store.git").as_str())
        .env("GIT_WORK_TREE", project.as_str())
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let git_env = turborepo_scm::GitEnvironment {
        cwd: project.as_std_path().to_owned(),
        git_dir: Some("../store.git".into()),
        work_tree: Some(".".into()),
        ..Default::default()
    };
    let plan = crate::plan_paths_with(
        &project,
        &["packages/a"],
        matches(&project, &["packages/a/gen/**"], &[]),
        &git_env,
    )
    .unwrap();
    execute(&plan);

    assert_eq!(removed(&plan), ["packages/a/gen/out.js"]);
    assert_eq!(
        skipped(&plan),
        [("packages/a/gen/t.ts".to_owned(), SkipReason::TrackedByGit)]
    );
    assert!(exists(&project, "packages/a/gen/t.ts"));
}

/// Security round 2 #3: on APFS `STRAẞE.ts` and `straße.ts` are one file,
/// but no name folding equates `ẞ` and `ß` with `ss`-folding `ß`. The
/// filesystem decides instead: same device and inode as a tracked name in
/// the same directory means tracked.
#[test]
fn a_tracked_file_the_filesystem_equates_with_the_match_is_kept() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    let indexed = "packages/a/gen/stra\u{df}e.ts";
    let on_disk = "packages/a/gen/STRA\u{1e9e}E.ts";
    write(&root, indexed, "committed");
    git_repo(&root, &[indexed]);
    std::fs::rename(
        path(&root, indexed).as_std_path(),
        path(&root, on_disk).as_std_path(),
    )
    .unwrap();
    if path(&root, indexed).symlink_metadata().is_err() {
        // A filesystem that tells the two names apart: nothing to protect.
        return;
    }
    write(&root, indexed, "UNCOMMITTED EDIT");
    assert_ne!(
        crate::names::fold("STRA\u{1e9e}E.ts"),
        crate::names::fold("stra\u{df}e.ts"),
        "folding alone would have protected it"
    );

    let plan = plan_paths(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/gen/**"], &[]),
    )
    .unwrap();
    execute(&plan);

    assert!(plan.is_empty(), "{:?}", removed(&plan));
    assert_eq!(
        skipped(&plan),
        [(on_disk.to_owned(), SkipReason::TrackedByGit)]
    );
    assert_eq!(
        path(&root, indexed).read_to_string().unwrap(),
        "UNCOMMITTED EDIT"
    );
}

/// Security round 2 #4: a directory renamed into a planned path after
/// planning (`src` moved to `dist/b`) is not emptied in its place: every
/// directory and entry is checked against its planned (device, inode).
#[cfg(unix)]
#[test]
fn a_directory_renamed_into_a_planned_path_is_not_emptied() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/a/src/index.js", "TRACKED");
    git_repo(&root, &["packages/a/src/index.js"]);
    write(&root, "packages/a/src/index.js", "uncommitted");
    write(&root, "packages/a/dist/a/f.js", "built");
    write(&root, "packages/a/dist/b/index.js", "built");

    let plan = plan_paths(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/dist/**"], &[]),
    )
    .unwrap();
    assert!(removed(&plan).contains(&"packages/a/dist/b/index.js".to_owned()));
    std::fs::rename(
        path(&root, "packages/a/dist/b").as_std_path(),
        path(&root, "packages/a/dist.b.old").as_std_path(),
    )
    .unwrap();
    std::fs::rename(
        path(&root, "packages/a/src").as_std_path(),
        path(&root, "packages/a/dist/b").as_std_path(),
    )
    .unwrap();
    let report = execute(&plan);

    assert_eq!(
        path(&root, "packages/a/dist/b/index.js")
            .read_to_string()
            .unwrap(),
        "uncommitted"
    );
    assert!(!exists(&root, "packages/a/dist/a/f.js"));
    assert_eq!(
        report
            .failures
            .iter()
            .map(|failure| failure.path.as_str())
            .collect::<Vec<_>>(),
        ["packages/a/dist/b/index.js"]
    );
    assert!(!report.is_complete());
}

/// The same check for a single file replaced after planning.
#[cfg(unix)]
#[test]
fn a_file_replaced_after_planning_is_kept() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "dist/out.js", "built");
    git_repo(&root, &[]);

    let plan = plan_paths(&root, &[], matches(&root, &["dist/**"], &[])).unwrap();
    path(&root, "dist/out.js").remove_file().unwrap();
    write(&root, "dist/out.js", "someone else's");
    let report = execute(&plan);

    assert_eq!(
        path(&root, "dist/out.js").read_to_string().unwrap(),
        "someone else's"
    );
    assert_eq!(report.failures.len(), 1);
}

/// Correctness round 2 N1: `node_modules` inside an output stays protected,
/// and is reported once rather than once per file.
#[test]
fn reserved_directories_inside_outputs_are_reported_once() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "apps/web/.next/standalone/server.js", "built");
    for file in ["react/index.js", "react/package.json", ".pnpm/x/y.js"] {
        write(
            &root,
            &format!("apps/web/.next/standalone/node_modules/{file}"),
            "dep",
        );
    }
    git_repo(&root, &[]);

    let plan = plan_paths(
        &root,
        &["apps/web"],
        matches(&root, &["apps/web/.next/**"], &[]),
    )
    .unwrap();

    assert_eq!(
        skipped(&plan),
        [(
            "apps/web/.next/standalone/node_modules/".to_owned(),
            SkipReason::ReservedDirectory
        )]
    );
    assert_eq!(removed(&plan), ["apps/web/.next/standalone/server.js"]);
    assert_eq!(
        plan.describe(true)[0],
        "• Not removing apps/web/.next/standalone/node_modules/ (node_modules, .git and .turbo \
         are never cleaned)"
    );
}

/// Security round 2 #5 / correctness round 2 N7: only unix deletes.
#[test]
fn deletion_is_refused_where_unsupported() {
    assert_eq!(crate::ensure_deletion_supported().is_ok(), cfg!(unix));
}

/// A repository at `root` whose git working tree is `packages/a`, tracking
/// its `package.json` and `gen/t.ts`, as in the security round 3 F1
/// fixtures. `configure` moves the working tree.
fn package_worktree(root: &AbsoluteSystemPath, configure: impl FnOnce(&AbsoluteSystemPath)) {
    write(root, "package.json", "{}");
    write(root, "turbo.json", "{}");
    write(root, "packages/a/package.json", "{}");
    write(root, "packages/a/gen/t.ts", "HANDWRITTEN");
    git(root, &["init", "--quiet"]);
    configure(root);
    let package = path(root, "packages/a");
    git(&package, &["add", "package.json", "gen/t.ts"]);
    git(&package, &["commit", "--quiet", "-m", "init"]);
    write(root, "packages/a/gen/t.ts", "HANDWRITTEN\nUNCOMMITTED");
}

fn assert_nothing_deleted(root: &AbsoluteSystemPath) {
    let result = plan_paths(
        root,
        &["packages/a"],
        matches(root, &["packages/a/gen/**"], &[]),
    );
    assert!(
        matches!(result, Err(Error::TrackedFilesUnknown { .. })),
        "{result:?}"
    );
    assert!(exists(root, "packages/a/gen/t.ts"));
}

/// Security round 3 F1: `core.worktree` in `config.worktree` (with
/// `extensions.worktreeConfig`) makes `packages/a` git's working tree. Its
/// index does not describe the repository root, so nothing is deleted.
#[test]
fn a_working_tree_set_in_config_worktree_deletes_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    package_worktree(&root, |root| {
        git(root, &["config", "extensions.worktreeConfig", "true"]);
        git(
            root,
            &["config", "--worktree", "core.worktree", "../packages/a"],
        );
    });
    assert_nothing_deleted(&root);
}

/// Security round 3 F1: git reads a config file that starts with a UTF-8
/// byte order mark, `core.worktree` included.
#[test]
fn a_working_tree_set_after_a_byte_order_mark_deletes_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    package_worktree(&root, |root| {
        let config = path(root, ".git/config");
        let original = std::fs::read_to_string(config.as_std_path()).unwrap();
        std::fs::write(
            config.as_std_path(),
            format!("\u{feff}[core]\n\tworktree = ../packages/a\n{original}"),
        )
        .unwrap();
    });
    assert_nothing_deleted(&root);
}

/// Correctness round 3 R3-1: in a `pre-commit` hook of a partial commit,
/// `GIT_INDEX_FILE` is a temporary index without the files staged only in
/// the repository's own index. Those are protected too.
#[cfg(unix)]
#[test]
fn files_staged_outside_a_partial_commit_are_kept() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "y.txt", "y");
    git_repo(&root, &["y.txt"]);
    write(&root, "packages/a/dist/staged.js", "STAGED");
    write(&root, "packages/a/dist/out.js", "built");
    git(&root, &["add", "--force", "packages/a/dist/staged.js"]);
    write(&root, "y.txt", "y2");
    // The hook keeps a copy of the temporary index git hands it.
    let hook = write(
        &root,
        "hooks/pre-commit",
        "#!/bin/sh\ncp \"$GIT_INDEX_FILE\" .git/partial-index\n",
    );
    std::fs::set_permissions(hook.as_std_path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let hooks_path = format!("core.hooksPath={}", path(&root, "hooks"));
    git(
        &root,
        &[
            "-c",
            &hooks_path,
            "commit",
            "--quiet",
            "-m",
            "partial",
            "y.txt",
        ],
    );

    let git_env = turborepo_scm::GitEnvironment {
        cwd: root.as_std_path().to_owned(),
        index_file: Some(".git/partial-index".into()),
        ..Default::default()
    };
    let plan = crate::plan_paths_with(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/dist/**"], &[]),
        &git_env,
    )
    .unwrap();
    execute(&plan);

    assert_eq!(removed(&plan), ["packages/a/dist/out.js"]);
    assert_eq!(
        skipped(&plan),
        [(
            "packages/a/dist/staged.js".to_owned(),
            SkipReason::TrackedByGit
        )]
    );
    assert!(exists(&root, "packages/a/dist/staged.js"));
}

/// Correctness round 3 R3-2: a tracked `turbo.jsonc` identifies the
/// repository on its own.
#[test]
fn a_tracked_turbo_jsonc_identifies_the_repository() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "package.json", "{}");
    write(&root, "turbo.jsonc", "{}");
    write(&root, "packages/a/gen/out.js", "built");
    git(&root, &["init", "--quiet"]);
    git(&root, &["add", "turbo.jsonc"]);
    git(&root, &["commit", "--quiet", "-m", "init"]);

    let plan = plan_paths(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/gen/**"], &[]),
    )
    .unwrap();
    assert_eq!(removed(&plan), ["packages/a/gen", "packages/a/gen/out.js"]);
}

/// Security round 3 F1 (defense in depth): an index that lists the root
/// manifest, but whose record of it does not match the file on disk and
/// which git does not confirm, may describe another working tree.
#[test]
fn a_root_manifest_the_index_does_not_describe_deletes_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp);
    write(&root, "packages/a/gen/t.ts", "HANDWRITTEN");
    git_repo(&root, &[]);
    // Edited since it was staged: the recorded stat data is stale.
    write(&root, "package.json", "{\"name\": \"edited\"}");
    // The index git uses does not track it at all.
    let other = path(&root, ".git/other-index");
    let output = Command::new("git")
        .args(["add", "README.md"])
        .env("GIT_INDEX_FILE", other.as_str())
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let git_env = turborepo_scm::GitEnvironment {
        cwd: root.as_std_path().to_owned(),
        index_file: Some(other.as_std_path().into()),
        ..Default::default()
    };
    let result = crate::plan_paths_with(
        &root,
        &["packages/a"],
        matches(&root, &["packages/a/gen/**"], &[]),
        &git_env,
    );
    let Err(Error::TrackedFilesUnknown { reason }) = result else {
        panic!("expected the tracked set to be unknown: {result:?}");
    };
    assert!(reason.contains("does not match"), "{reason}");
    assert!(exists(&root, "packages/a/gen/t.ts"));
}
