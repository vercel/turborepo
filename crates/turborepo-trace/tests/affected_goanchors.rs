//! `affected` and the location of a Go file: a test that finds a
//! directory from `runtime.Caller` names where that directory goes, not
//! everything below it (`turborepo_trace_lang_go::anchors`). All under the
//! default policy.
//!
//! The repository is `fixtures/go/anchors`, the shapes of real harnesses with
//! other names. Its two Go modules are packages of their own, so a string of a
//! test names nothing in `services/`: only the location does.
//!
//! | Package | Shape |
//! |---|---|
//! | `e2e` | a harness that takes the repository root from its own location, runs a child process there, and keeps `<root>/services/svc-a` in a struct field that another method reads with a parameter |
//! | `tools/slo` | a function that returns `<root>/services` from its own location; the test lists it and reads a file of every service |
//!
//! A runtime oracle in the trace's development repository runs the same tests
//! with `go test` under `strace` and checks that every file they open is
//! selected.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::{Path, PathBuf},
    process::Command,
};

const HARNESS: &str = "e2e/harness_test.go";
const SMOKE: &str = "e2e/smoke_test.go";
const SLO: &str = "tools/slo/slo_test.go";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    env: Vec<(&'static str, &'static str)>,
}

/// A copy of the fixture with its graph built under `env`.
fn repo(env: &[(&'static str, &'static str)]) -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap().join("repo");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/go/anchors");
    copy_dir(&fixture, &root);
    let r = Repo {
        _tmp: tmp,
        root,
        env: env.to_vec(),
    };
    r.run(&["graph", "build"]);
    r
}

impl Repo {
    fn run(&self, args: &[&str]) -> String {
        let o = Command::new(support::harness())
            .args(args)
            .current_dir(&self.root)
            .env("GOMODCACHE", self.root.join("../modcache"))
            .env_remove("GOFLAGS")
            .env_remove("GOWORK")
            .env_remove("TURBO_TRACE_DEBUG_NAMES")
            .env_remove("TURBO_TRACE_DEBUG_ANCHORS")
            .envs(self.env.iter().copied())
            .output()
            .expect("run the harness");
        assert!(
            o.status.success(),
            "harness {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8(o.stdout).unwrap()
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    /// The Go tests a change of `path` selects, default policy, sorted.
    fn affected(&self, path: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .run(&["affected", "--tests", "gotest", "--changed", path])
            .lines()
            .map(str::to_owned)
            .collect();
        out.sort();
        out
    }
}

#[test]
fn a_root_joined_with_a_literal_segment_names_that_subtree() {
    let r = repo(&[]);
    // The harness keeps `<root>/services/svc-a`: a file below it selects its tests,
    // and the test of the function that returns `<root>/services`.
    assert_eq!(
        r.affected("services/svc-a/config.json"),
        [HARNESS, SMOKE, SLO]
    );
    r.write("services/svc-a/data/added.bin", "x\n");
    assert_eq!(
        r.affected("services/svc-a/data/added.bin"),
        [HARNESS, SMOKE, SLO]
    );
    let why = r.run(&[
        "affected",
        "--tests",
        "gotest",
        "--changed",
        "services/svc-a/config.json",
        "--explain",
        SMOKE,
    ]);
    assert!(why.contains("e2e/harness_test.go:39"), "{why}");
    // Elsewhere below the root: the harness does not go there. The root itself is
    // only the working directory of a child process, the base of nothing that
    // is read.
    assert_eq!(r.affected("services/web/settings.yaml"), [SLO]);
    // An unrelated edit selects nothing.
    assert_eq!(r.affected("docs/notes.md"), [""; 0]);
    r.write("services.txt", "x\n");
    assert_eq!(r.affected("services.txt"), [""; 0]);
}

#[test]
fn without_following_the_location_names_everything_below_the_root() {
    // The coarse rule: each file reaches the root with its parent steps, and names
    // everything below it.
    let r = repo(&[("TURBO_TRACE_DEBUG_ANCHORS", "0")]);
    assert_eq!(r.affected("docs/notes.md"), [HARNESS, SMOKE, SLO]);
    assert_eq!(
        r.affected("services/web/settings.yaml"),
        [HARNESS, SMOKE, SLO]
    );
}
