//! `affected` on Go modules without a module cache, as in a fresh CI
//! checkout: third-party imports are linked to the module versions `go.mod`
//! requires, so they select tests on a change of `go.mod` or `go.sum` and on
//! nothing else. All under the default policy.
//!
//! The repository is `fixtures/go/nocache`: the shape of a monorepo with
//! several Go modules, each with its own `go.mod` and `go.sum`; test files that
//! import an assertion library; a package of one module used by another through
//! a local `replace`; an import below two nested required modules.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::{Path, PathBuf},
    process::Command,
};

const PARSE: &str = "svc/ingest/app/parse/parse_test.go";
const PURE: &str = "svc/ingest/app/pure/pure_test.go";
const EMIT: &str = "svc/ingest/app/emit/emit_test.go";
const ROUTING: &str = "pkg/pipeline/routing/routing_test.go";
const HANDLER: &str = "svc/flags/app/handler_test.go";

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    modcache: PathBuf,
}

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

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// The fixture as a git repository with one commit, and an empty module cache
/// next to it.
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("repo");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/go/nocache");
    copy_dir(&fixture, &root);
    std::fs::write(root.join(".gitignore"), ".turbo/trace/\n").unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "base"]);
    Repo {
        _tmp: tmp,
        modcache: base.join("modcache"),
        root,
    }
}

impl Repo {
    fn harness(&self, args: &[&str]) -> String {
        let o = Command::new(support::harness())
            .args(args)
            .current_dir(&self.root)
            .env("GOMODCACHE", &self.modcache)
            .env_remove("GOFLAGS")
            .env_remove("GOWORK")
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

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    /// Replaces `from` by `to` in a file; the text must be there.
    fn edit(&self, rel: &str, from: &str, to: &str) {
        let text = self.read(rel);
        assert!(text.contains(from), "{rel} has no {from:?}");
        self.write(rel, &text.replace(from, to));
    }

    /// The Go tests the working tree's changes select, default policy, sorted.
    fn selected(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .harness(&["affected", "--tests", "gotest", "--since", "HEAD"])
            .lines()
            .map(str::to_owned)
            .collect();
        v.sort();
        v
    }

    /// Back to the committed state.
    fn reset(&self) {
        git(&self.root, &["checkout", "-q", "--", "."]);
        git(&self.root, &["clean", "-qfd"]);
    }
}

fn sorted(mut v: Vec<&str>) -> Vec<&str> {
    v.sort();
    v
}

#[test]
fn third_party_imports_without_a_module_cache_leave_no_floor() {
    let r = repo();
    // No test is selected on every change.
    let floor = r.harness(&["floor", "--tests", "gotest"]);
    assert!(
        floor.starts_with("uncertainty floor: 0 of 5 entries"),
        "{floor}"
    );
    assert_eq!(r.selected(), [""; 0], "nothing changed");
    // An unrelated edit selects nothing; an edit of a package selects its tests
    // only.
    r.write("docs/notes.md", "edited\n");
    assert_eq!(r.selected(), [""; 0]);
    r.reset();
    r.edit(
        "svc/ingest/app/parse/parse.go",
        "Fields(line)",
        "Fields(line )",
    );
    assert_eq!(r.selected(), [PARSE]);
    r.reset();
    // The import is an edge to the module version of go.mod, as with a module
    // cache.
    let deps = r.harness(&["deps", PARSE]);
    assert!(
        deps.lines()
            .any(|l| l == "go:github.com/acme-test/assert@v1.2.0"),
        "{deps}"
    );
    // An import below two nested modules may come from either: both are linked.
    let deps = r.harness(&["deps", "svc/ingest/app/emit/emit.go"]);
    for module in [
        "go:cloud.acme.test/go/storage@v1.30.1",
        "go:cloud.acme.test/go@v0.110.0",
    ] {
        assert!(deps.lines().any(|l| l == module), "{module} in {deps}");
    }
}

#[test]
fn a_bumped_requirement_selects_the_tests_of_its_module() {
    let r = repo();
    // A direct requirement: go.mod and go.sum change together, as `go get` leaves
    // them.
    r.edit("svc/ingest/go.mod", "assert v1.2.0", "assert v1.3.0");
    r.edit("svc/ingest/go.sum", "assert v1.2.0", "assert v1.3.0");
    let sel = r.selected();
    // Every test of the module: nothing in the repository says which module depends
    // on the bumped one, so no import of the module is known to be unaffected.
    // Not the tests of the other modules, which have their own go.mod
    // (svc/flags requires the same library).
    assert_eq!(sel, sorted(vec![EMIT, PARSE, PURE]));
    let why = r.harness(&[
        "affected",
        "--tests",
        "gotest",
        "--since",
        "HEAD",
        "--explain",
        PARSE,
    ]);
    assert!(why.contains("svc/ingest/go.mod"), "{why}");
    r.reset();

    // An indirect requirement (no first-party file imports the module).
    r.edit("svc/ingest/go.mod", "deep v0.3.0", "deep v0.4.0");
    r.edit("svc/ingest/go.sum", "deep v0.3.0", "deep v0.4.0");
    assert_eq!(r.selected(), sorted(vec![EMIT, PARSE, PURE]));
}

#[test]
fn a_go_sum_change_alone_selects_the_tests_of_its_module() {
    let r = repo();
    // The hash of an indirect module changes; go.mod is untouched. The module is no
    // node of the graph, so only the file itself can carry the change.
    r.edit("svc/ingest/go.sum", "DEEPZIPHASH0", "DEEPZIPHASH1");
    assert_eq!(r.selected(), sorted(vec![EMIT, PARSE, PURE]));
    r.reset();
    // The hash of a directly imported module.
    r.edit("svc/flags/go.sum", "ASSERTZIPHASH0", "ASSERTZIPHASH1");
    assert_eq!(r.selected(), [HANDLER]);
    r.reset();
    // The same lines in another order change no build.
    let sum = r.read("svc/ingest/go.sum");
    let mut lines: Vec<&str> = sum.lines().collect();
    lines.reverse();
    r.write("svc/ingest/go.sum", &format!("{}\n", lines.join("\n")));
    assert_eq!(r.selected(), [""; 0]);
}

#[test]
fn a_local_replace_is_a_first_party_edge() {
    let r = repo();
    // Code of the replaced module: its own tests and the tests of its importers.
    r.edit(
        "pkg/pipeline/routing/routing.go",
        "routes[kind]",
        "routes[ kind]",
    );
    assert_eq!(r.selected(), sorted(vec![EMIT, ROUTING]));
    r.reset();
    // Its go.mod: its own tests, and every test of the module that replaces it (the
    // requirements of a replaced module enter that module's build list).
    r.edit("pkg/pipeline/go.mod", "yaml v1.0.0", "yaml v1.1.0");
    r.edit("pkg/pipeline/go.sum", "yaml v1.0.0", "yaml v1.1.0");
    assert_eq!(r.selected(), sorted(vec![EMIT, PARSE, PURE, ROUTING]));
}

#[test]
fn a_workspace_shares_one_build_list() {
    let r = repo();
    r.write(
        "go.work",
        "go 1.22\n\nuse (\n\t./pkg/pipeline\n\t./svc/flags\n\t./svc/ingest\n)\n",
    );
    // Adding go.work changes how every module below it resolves.
    assert_eq!(
        r.selected(),
        sorted(vec![EMIT, HANDLER, PARSE, PURE, ROUTING])
    );
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-qm", "workspace"]);
    assert_eq!(r.selected(), [""; 0]);
    let floor = r.harness(&["floor", "--tests", "gotest"]);
    assert!(
        floor.starts_with("uncertainty floor: 0 of 5 entries"),
        "{floor}"
    );
    // A requirement of one workspace module is part of the build list of all of
    // them.
    r.edit("svc/flags/go.mod", "assert v1.2.0", "assert v1.4.0");
    r.edit("svc/flags/go.sum", "assert v1.2.0", "assert v1.4.0");
    assert_eq!(
        r.selected(),
        sorted(vec![EMIT, HANDLER, PARSE, PURE, ROUTING])
    );
    r.reset();
    // Code still selects by import.
    r.edit("svc/ingest/app/pure/pure.go", "2 * n", "n * 2");
    assert_eq!(r.selected(), [PURE]);
}

#[test]
fn a_module_cache_gives_the_same_edges() {
    let r = repo();
    // The module is downloaded: the import resolves to its files, which belong to
    // the same opaque package as without the cache.
    let m = r.modcache.join("github.com/acme-test/assert@v1.2.0");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(m.join("go.mod"), "module github.com/acme-test/assert\n").unwrap();
    std::fs::write(m.join("assert.go"), "package assert\n").unwrap();
    let deps = r.harness(&["deps", PARSE]);
    assert!(
        deps.lines()
            .any(|l| l == "go:github.com/acme-test/assert@v1.2.0"),
        "{deps}"
    );
    assert_eq!(r.selected(), [""; 0]);
    r.edit("svc/flags/go.mod", "assert v1.2.0", "assert v1.3.0");
    r.edit("svc/flags/go.sum", "assert v1.2.0", "assert v1.3.0");
    assert_eq!(r.selected(), [HANDLER]);
}
