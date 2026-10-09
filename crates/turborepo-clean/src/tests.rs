//! Planning and removal over real directories. Each review finding has a
//! regression test reproducing its case.

use std::{collections::BTreeSet, process::Command, str::FromStr};

use tempfile::TempDir;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPath};

use crate::{Error, RemovalKind, SkipReason, plan_paths};

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

/// A git repository tracking `tracked`; everything else stays untracked.
fn git_repo(root: &AbsoluteSystemPath, tracked: &[&str]) {
    write(root, "README.md", "readme");
    git(root, &["init", "--quiet"]);
    git(root, &["add", "README.md"]);
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
    let report = plan.execute();

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
    plan.execute();

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
    plan.execute();

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
    let report = plan.execute();

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
    plan.execute();

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
    plan.execute();

    assert_eq!(removed(&plan), ["dist/out.js"]);
    assert!(exists(&root, "dist/vendor/lib.c"));
    assert!(exists(&root, "dist/uninitialized/d/y.c"));
    let reasons: BTreeSet<SkipReason> = plan.skipped().map(|(_, reason)| reason).collect();
    assert!(reasons.contains(&SkipReason::NestedRepository));
    assert!(reasons.contains(&SkipReason::InSubmodule));
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
    let report = plan.execute();

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
    let report = plan.execute();

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
    let report = plan.execute();
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
    assert!(crate::refused_pattern("dist/**").is_none());
    assert!(crate::refused_pattern("{dist,build}/**").is_none());
    assert!(crate::refused_pattern("*.tsbuildinfo").is_none());
    for pattern in ["*.*", "?*", "../b/*.*", "dist/*/../../**", "**/*.js"] {
        assert!(crate::refused_pattern(pattern).is_some(), "{pattern}");
    }
}
