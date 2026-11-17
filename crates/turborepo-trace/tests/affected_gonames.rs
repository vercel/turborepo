//! `affected` and Go reads with a computed path: `os.ReadFile(path)`
//! on a parameter, a struct field or the result of a helper is bounded by the
//! path-like strings of the test's code, like the same reads in JS, instead of
//! selecting the test on every change. All under the default policy.
//!
//! The repository is `fixtures/go/names`, the shapes of real sites with other
//! names:
//!
//! | Package | Shape |
//! |---|---|
//! | `config` | a loader that reads its `path` parameter; its test writes the file below `t.TempDir()` and loads it |
//! | `extract` | a test that reads back, below `t.TempDir()`, what the code under test wrote |
//! | `tools/schema` | a test that reads a file found from `runtime.Caller`, through two helpers |
//! | `data/loader` | a struct field joined with a parameter; the test passes `"../fixtures"` |
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::{Path, PathBuf},
    process::Command,
};

const CONFIG: &str = "config/config_test.go";
const EXTRACT: &str = "extract/extract_test.go";
const SCHEMA: &str = "tools/schema/schema_test.go";
const LOADER: &str = "data/loader/loader_test.go";

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
}

/// A copy of the fixture with its graph built.
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap().join("repo");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../turborepo-trace-testkit/fixtures/go/names");
    copy_dir(&fixture, &root);
    let r = Repo { _tmp: tmp, root };
    r.run(&["graph", "build"], &[]);
    r
}

impl Repo {
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> String {
        let o = Command::new(support::harness())
            .args(args)
            .current_dir(&self.root)
            .env("GOMODCACHE", self.root.join("../modcache"))
            .env_remove("GOFLAGS")
            .env_remove("GOWORK")
            .env_remove("TURBO_TRACE_DEBUG_NAMES")
            .envs(env.iter().copied())
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

    /// The Go tests a change of `path` selects, default policy.
    fn affected(&self, path: &str) -> Vec<String> {
        self.run(&["affected", "--tests", "gotest", "--changed", path], &[])
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[test]
fn computed_reads_in_go_leave_no_floor() {
    let r = repo();
    let floor = r.run(&["floor", "--tests", "gotest"], &[]);
    assert!(
        floor.starts_with("uncertainty floor: 0 of 4 entries"),
        "{floor}"
    );
    // Without the names the reads are open and every test that reaches one is on
    // the floor.
    let open = r.run(
        &["floor", "--tests", "gotest"],
        &[("TURBO_TRACE_DEBUG_NAMES", "0")],
    );
    assert!(
        open.starts_with("uncertainty floor: 4 of 4 entries"),
        "{open}"
    );
    // An unrelated edit selects nothing.
    assert_eq!(r.affected("docs/notes.md"), [""; 0]);
    // Code still selects by import.
    assert_eq!(r.affected("config/config.go"), [CONFIG]);
}

#[test]
fn a_file_the_strings_of_a_test_name_selects_it() {
    let r = repo();
    // A directory given as a relative path and a file name given to a method: the
    // read is a struct field joined with a parameter, in another file.
    let golden = "data/fixtures/golden.json";
    assert_eq!(r.affected(golden), [LOADER]);
    let why = r.run(
        &[
            "affected",
            "--tests",
            "gotest",
            "--changed",
            golden,
            "--explain",
            LOADER,
        ],
        &[],
    );
    assert!(why.contains("named by the string"), "{why}");
    r.write("data/fixtures/added.json", "{}\n");
    assert_eq!(r.affected("data/fixtures/added.json"), [LOADER]);
}

#[test]
fn a_literal_path_from_the_location_of_a_file_is_a_reference() {
    let r = repo();
    // `filepath.Join(filepath.Dir(thisFile), "schemas", "job.schema.json")`,
    // returned by a helper and read through another: an edge to exactly that
    // file.
    let schema = "tools/schema/schemas/job.schema.json";
    assert_eq!(r.affected(schema), [SCHEMA]);
    let deps = r.run(&["deps", SCHEMA], &[]);
    assert!(deps.lines().any(|l| l == schema), "{deps}");
    // The file does not use its location for anything else: a sibling of the schema
    // that no string names is not a dependency through the location. (It still
    // is one through the working directory, see below, so look outside the
    // package directory.)
    r.write("tools/other/notes.txt", "x\n");
    assert_eq!(r.affected("tools/other/notes.txt"), [""; 0]);
}

#[test]
fn a_file_written_below_a_temp_dir_names_no_repository_file() {
    let r = repo();
    // `config_test.go` writes `settings.yaml` below `t.TempDir()` and loads it: the
    // repository's own `defaults/settings.yaml` is not what it reads. The failure
    // message that mentions the name is no path either.
    assert_eq!(r.affected("defaults/settings.yaml"), [""; 0]);
    // The same for a file that a test only looks for below a temporary directory.
    r.write("docs/doomed.txt", "x\n");
    assert_eq!(r.affected("docs/doomed.txt"), [""; 0]);
}

#[test]
fn a_test_with_a_computed_read_reads_below_its_package_directory() {
    let r = repo();
    // `go test` runs in the package directory, and a relative path needs no string
    // to name what is there. A file below the directory of a test with a read
    // that can't be evaluated selects that test, and no other.
    r.write("config/nested/extra.txt", "x\n");
    assert_eq!(r.affected("config/nested/extra.txt"), [CONFIG]);
    r.write("extract/nested/extra.txt", "x\n");
    assert_eq!(r.affected("extract/nested/extra.txt"), [EXTRACT]);
    // A test file with a parent step (`"../fixtures"`) reaches one directory
    // further up.
    r.write("data/other/extra.txt", "x\n");
    assert_eq!(r.affected("data/other/extra.txt"), [LOADER]);
    r.write("tools/schema/extra/notes.txt", "x\n");
    assert_eq!(r.affected("tools/schema/extra/notes.txt"), [SCHEMA]);
}
