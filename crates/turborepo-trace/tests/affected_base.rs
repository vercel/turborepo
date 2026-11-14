//! `affected --since <rev>`: the base is the merge base of `<rev>` and `HEAD`,
//! so the changes are the branch's own
//! (`crates/turborepo-trace/src/lockdiff.rs`, `Git`). Each test builds a small
//! git history: a branch that merged main into itself, a rebased branch, a
//! branch behind main, a pull request's merge ref, a shallow checkout, a
//! criss-cross history, and a revision that is an ancestor of `HEAD`.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

use serde_json::Value;

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn git_out(root: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args([
            "-c",
            "protocol.file.allow=always",
            "-c",
            "advice.detachedHead=false",
        ])
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .output()
        .unwrap()
}

fn git(root: &Path, args: &[&str]) -> String {
    let o = git_out(root, args);
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

fn harness(root: &Path, args: &[&str]) -> Output {
    Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .env_remove("TURBO_TRACE_DEBUG_EXACT_BASE")
        .env_remove("TURBO_TRACE_DEBUG_MODE")
        .output()
        .expect("run the harness")
}

/// The selected jest tests since `rev`, as the module names they test (`a`,
/// `b`, ...), and what the command printed on stderr.
fn selected(root: &Path, rev: &str, extra: &[&str]) -> (Vec<String>, String) {
    let mut args = vec!["affected", "--tests", "jest", "--since", rev];
    args.extend_from_slice(extra);
    let o = harness(root, &args);
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert!(o.status.success(), "harness {args:?} failed: {err}");
    let names = String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            l.strip_prefix("src/")
                .and_then(|l| l.strip_suffix(".test.js"))
                .unwrap_or(l)
                .to_owned()
        })
        .collect();
    (names, err)
}

const MODULES: [&str; 5] = ["a", "b", "c", "d", "m"];

/// Rewrites a module: a change of `src/<name>.js`, which selects
/// `src/<name>.test.js`.
fn edit(root: &Path, name: &str, value: &str) {
    write(
        root,
        &format!("src/{name}.js"),
        &format!("exports.value = '{value}';\n"),
    );
}

fn commit(root: &Path, message: &str) -> String {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", message]);
    git(root, &["rev-parse", "HEAD"])
}

/// One package with five modules, each with a jest test of its own, committed
/// on `main`.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"app","private":true,"scripts":{"test":"jest"}}"#,
    );
    write(
        r,
        "jest.config.js",
        "module.exports = { testMatch: ['**/*.test.js'] };\n",
    );
    write(r, ".gitignore", ".turbo/trace/\n");
    for m in MODULES {
        edit(r, m, "0");
        write(
            r,
            &format!("src/{m}.test.js"),
            &format!(
                "const {{ value }} = require('./{m}');\ntest('{m}', () => \
                 expect(value).toBeDefined());\n"
            ),
        );
    }
    git(r, &["init", "-q", "-b", "main"]);
    commit(r, "base");
    d
}

#[test]
fn a_branch_that_merged_main_into_itself() {
    let d = repo();
    let r = d.path();
    // The branch changes `a` and `b`; main changes `m` and, differently, `b`.
    git(r, &["checkout", "-q", "-b", "feature"]);
    edit(r, "a", "feature");
    edit(r, "b", "feature");
    commit(r, "feature: a, b");
    git(r, &["checkout", "-q", "main"]);
    edit(r, "m", "main");
    edit(r, "b", "main");
    let merged_in = commit(r, "main: m, b");
    // The branch merges main: a conflict in `b`, resolved with content of its own.
    git(r, &["checkout", "-q", "feature"]);
    assert!(!git_out(r, &["merge", "main"]).status.success());
    edit(r, "b", "resolved");
    commit(r, "merge main into feature");
    // Main moves on after the merge.
    git(r, &["checkout", "-q", "main"]);
    edit(r, "c", "main later");
    commit(r, "main: c");
    git(r, &["checkout", "-q", "feature"]);

    // The branch's own commit and its conflict resolution: not `m`, which the merge
    // commit brought in, and not `c`, which main gained afterwards.
    let (tests, err) = selected(r, "main", &[]);
    assert_eq!(tests, ["a", "b"]);
    assert!(
        err.contains(&format!(
            "base {} (the merge base of main and HEAD)",
            &merged_in[..12]
        )),
        "{err}"
    );
    // The base is in the JSON output, and it is what every consumer compared with.
    let o = harness(
        r,
        &[
            "affected", "--tests", "jest", "--since", "main", "--format", "json",
        ],
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["base"]["given"], "main");
    assert_eq!(v["base"]["commit"], merged_in.as_str());
    assert_eq!(v["base"]["exact"], false);
    let changed: Vec<&str> = v["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(changed, ["src/a.js", "src/b.js"]);
    let o = harness(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--since",
            "main",
            "--format",
            "selection",
        ],
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["base"], "main");
    assert_eq!(v["base_commit"], merged_in.as_str());

    // Exactly the revision, for a caller that resolved the base itself: the two
    // trees, so what main gained since counts too. The flag and the variable do
    // the same.
    let (tests, err) = selected(r, "main", &["--exact-base"]);
    assert_eq!(tests, ["a", "b", "c"]);
    assert!(!err.contains("merge base"), "{err}");
    let o = Command::new(support::harness())
        .args(["affected", "--tests", "jest", "--since", "main"])
        .current_dir(r)
        .env("TURBO_TRACE_DEBUG_EXACT_BASE", "1")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout).lines().count(), 3);
    // With the merge base passed exactly, the answer is the first one.
    let (tests, _) = selected(r, &merged_in, &["--exact-base"]);
    assert_eq!(tests, ["a", "b"]);
}

#[test]
fn a_rebased_branch_a_branch_behind_main_and_an_ancestor() {
    let d = repo();
    let r = d.path();
    let fork = git(r, &["rev-parse", "HEAD"]);
    git(r, &["checkout", "-q", "-b", "feature"]);
    edit(r, "a", "feature");
    commit(r, "feature: a");
    // A change that is taken back is no change: no commit is walked.
    edit(r, "d", "tried");
    commit(r, "feature: d");
    edit(r, "d", "0");
    commit(r, "feature: d back");
    git(r, &["checkout", "-q", "main"]);
    edit(r, "m", "main");
    let main_tip = commit(r, "main: m");
    git(r, &["checkout", "-q", "feature"]);

    // Behind main: what main gained since the fork is not a change of the branch.
    let (tests, err) = selected(r, "main", &[]);
    assert_eq!(tests, ["a"]);
    assert!(err.contains(&format!("base {}", &fork[..12])), "{err}");
    // A revision that is an ancestor of HEAD is its own merge base.
    assert_eq!(selected(r, &fork, &[]).0, ["a"]);
    assert_eq!(selected(r, &fork, &["--exact-base"]).0, ["a"]);
    // Uncommitted and untracked changes count, as before.
    edit(r, "c", "not committed");
    write(
        r,
        "src/new.test.js",
        "test('new', () => expect(1).toBe(1));\n",
    );
    assert_eq!(selected(r, "main", &[]).0, ["a", "c", "new"]);
    git(r, &["checkout", "-q", "--", "src/c.js"]);
    std::fs::remove_file(r.join("src/new.test.js")).unwrap();

    // Rebased onto the newer main: only its own commits.
    git(r, &["rebase", "-q", "main"]);
    let (tests, err) = selected(r, "main", &[]);
    assert_eq!(tests, ["a"]);
    assert!(err.contains(&format!("base {}", &main_tip[..12])), "{err}");
}

#[test]
fn the_merge_ref_of_a_pull_request() {
    let d = repo();
    let r = d.path();
    git(r, &["checkout", "-q", "-b", "feature"]);
    edit(r, "a", "feature");
    commit(r, "feature: a");
    git(r, &["checkout", "-q", "main"]);
    edit(r, "m", "main");
    let main_tip = commit(r, "main: m");
    // What a CI system checks out: a merge of the pull request into the current
    // main, with main as the first parent.
    git(r, &["checkout", "-q", "--detach", "main"]);
    git(
        r,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "merge pull request",
            "feature",
        ],
    );
    assert_eq!(git(r, &["rev-parse", "HEAD^1"]), main_tip);

    // The pull request's changes, as applied on top of the latest main.
    for rev in ["main", "HEAD^1"] {
        let (tests, err) = selected(r, rev, &[]);
        assert_eq!(tests, ["a"], "{rev}");
        assert!(err.contains(&format!("base {}", &main_tip[..12])), "{err}");
    }
    // The pull request's own head would compare the wrong way round: main's change.
    assert_eq!(selected(r, "HEAD^2", &[]).0, ["m"]);

    // As a CI system fetches it, with depth 2: the merge commit and its two
    // parents, each a cut. The first parent is an ancestor of HEAD, so it is
    // the base whatever is cut.
    git(r, &["branch", "pr-merge"]);
    let shallow = tempfile::tempdir().unwrap();
    let s = shallow.path();
    let url = format!("file://{}", r.display());
    git(
        s,
        &[
            "clone", "-q", "--depth", "2", "--branch", "pr-merge", &url, ".",
        ],
    );
    assert_eq!(git(s, &["rev-parse", "--is-shallow-repository"]), "true");
    let (tests, err) = selected(s, "HEAD^1", &[]);
    assert_eq!(tests, ["a"]);
    assert!(err.contains(&format!("base {}", &main_tip[..12])), "{err}");
    // With depth 1 the parents are not there, and the message says what to fetch.
    let one = tempfile::tempdir().unwrap();
    git(
        one.path(),
        &[
            "clone", "-q", "--depth", "1", "--branch", "pr-merge", &url, ".",
        ],
    );
    let out = harness(
        one.path(),
        &["affected", "--tests", "jest", "--since", "HEAD^1"],
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success() && err.contains("a depth of at least 2"),
        "{err}"
    );
}

#[test]
fn several_merge_bases_count_together() {
    let d = repo();
    let r = d.path();
    // Two lines that each merge the other: `main` and `feature` then have two merge
    // bases.
    git(r, &["checkout", "-q", "-b", "feature"]);
    edit(r, "a", "feature");
    let f1 = commit(r, "feature: a");
    git(r, &["checkout", "-q", "main"]);
    edit(r, "m", "main");
    let m1 = commit(r, "main: m");
    git(
        r,
        &["merge", "-q", "--no-ff", "-m", "main merges feature", &f1],
    );
    git(r, &["checkout", "-q", "feature"]);
    git(
        r,
        &["merge", "-q", "--no-ff", "-m", "feature merges main", &m1],
    );
    edit(r, "c", "feature");
    commit(r, "feature: c");
    let mut bases: Vec<String> = git(r, &["merge-base", "--all", "main", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    bases.sort();
    assert_eq!(bases.len(), 2);

    // The union of the changes against each base: against `f1` the branch gained
    // `m` and `c`, against `m1` it gained `a` and `c`. Never less than against
    // any one of them.
    let (tests, err) = selected(r, "main", &[]);
    assert_eq!(tests, ["a", "c", "m"]);
    assert!(err.contains("2 merge bases of main and HEAD"), "{err}");
    let o = harness(
        r,
        &[
            "affected", "--tests", "jest", "--since", "main", "--format", "json",
        ],
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let mut listed: Vec<String> = v["base"]["merge_bases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_str().unwrap().to_owned())
        .collect();
    listed.sort();
    assert_eq!(listed, bases);
    let o = harness(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--since",
            "main",
            "--format",
            "selection",
        ],
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["merge_bases"].as_array().unwrap().len(), 2);
}

#[test]
fn a_shallow_checkout_never_guesses_a_base() {
    // The repository a CI system clones from: main moved on after the branch
    // forked, and the branch merged main once.
    let origin = repo();
    let o = origin.path();
    git(o, &["checkout", "-q", "-b", "feature"]);
    edit(o, "a", "feature");
    commit(o, "feature: a");
    git(o, &["checkout", "-q", "main"]);
    edit(o, "m", "main");
    let m1 = commit(o, "main: m");
    git(o, &["checkout", "-q", "feature"]);
    git(
        o,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "feature merges main",
            "main",
        ],
    );
    edit(o, "b", "feature");
    commit(o, "feature: b");
    git(o, &["checkout", "-q", "main"]);
    edit(o, "c", "main later");
    commit(o, "main: c");
    let url = format!("file://{}", o.display());

    // Depth 1 of the branch and of main: no history that connects them.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    git(
        r,
        &[
            "clone", "-q", "--depth", "1", "--branch", "feature", &url, ".",
        ],
    );
    git(
        r,
        &[
            "fetch",
            "-q",
            "--depth",
            "1",
            "origin",
            "main:refs/remotes/origin/main",
        ],
    );
    let out = harness(
        r,
        &["affected", "--tests", "jest", "--since", "origin/main"],
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "{err}");
    assert!(out.stdout.is_empty());
    assert!(
        err.contains("the merge base of \"origin/main\" and HEAD is not in this shallow checkout")
            && err.contains("git fetch --deepen")
            && err.contains("1 commit(s) of HEAD's history"),
        "{err}"
    );
    // The selection document: everything runs, and it says why. Exit code 0.
    let out = harness(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--since",
            "origin/main",
            "--format",
            "selection",
            "--mode",
            "enforce",
        ],
    );
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (&v["run"], &v["reason"]["code"]),
        (&Value::from("all"), &Value::from("diff-unreadable"))
    );
    assert!(
        v["reason"]["detail"]
            .as_str()
            .unwrap()
            .contains("shallow checkout")
    );
    assert!(v.get("base_commit").is_none());
    // A revision that is not there at all.
    let out = harness(r, &["affected", "--tests", "jest", "--since", "HEAD~1"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success() && err.contains("is not in this shallow checkout"),
        "{err}"
    );

    // Deeper, but still cut off between the branch and the merge base: a common
    // commit is found, and a later one could lie behind the cut. Not trusted.
    git(r, &["fetch", "-q", "--depth", "3", "origin", "feature"]);
    git(
        r,
        &[
            "fetch",
            "-q",
            "--depth",
            "2",
            "origin",
            "main:refs/remotes/origin/main",
        ],
    );
    assert_eq!(git(r, &["merge-base", "origin/main", "HEAD"]), m1);
    let out = harness(
        r,
        &["affected", "--tests", "jest", "--since", "origin/main"],
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains("history is cut off at") && err.contains("--exact-base"),
        "{err}"
    );
    // The caller knows the base: no history is needed for the comparison itself.
    assert_eq!(selected(r, &m1, &["--exact-base"]).0, ["a", "b"]);

    // With the history: the branch's own changes.
    git(r, &["fetch", "-q", "--unshallow", "origin"]);
    git(
        r,
        &["fetch", "-q", "origin", "main:refs/remotes/origin/main"],
    );
    let (tests, err) = selected(r, "origin/main", &[]);
    assert_eq!(tests, ["a", "b"]);
    assert!(err.contains(&format!("base {}", &m1[..12])), "{err}");
}

#[test]
fn histories_without_a_common_commit() {
    let d = repo();
    let r = d.path();
    git(r, &["checkout", "-q", "--orphan", "other"]);
    edit(r, "a", "other");
    commit(r, "another root");
    let out = harness(r, &["affected", "--tests", "jest", "--since", "main"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success());
    assert!(err.contains("have no common ancestor"), "{err}");
    assert_eq!(selected(r, "main", &["--exact-base"]).0, ["a"]);
}
