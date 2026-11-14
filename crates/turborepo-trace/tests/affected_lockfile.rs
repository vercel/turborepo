//! `affected --since` on lockfile changes that leave every manifest
//! alone: a project's test runner is re-resolved to another peer variant, or a
//! dependency of the runner is bumped. The shape comes from a real pnpm
//! monorepo: two packages that declare `vitest` and no DOM library were moved
//! from the `jsdom@27` variant of vitest to the `jsdom@23` variant by an
//! unrelated change, and their tests failed (`PointerEvent is not defined`).
//!
//! The repositories have no `node_modules`, like a fresh clone: packages are
//! linked through the lockfile.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run the harness")
}

fn ok(root: &Path, args: &[&str]) -> String {
    let o = run(root, args);
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
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

/// A peer suffix long enough for js-yaml to write the snapshot key in the
/// explicit form (`? key` / `: value`), as pnpm does for keys over 1024
/// characters.
fn long_peers() -> String {
    (0..90).map(|i| format!("(peer-{i:02}@1.0.0)")).collect()
}

/// The lockfile. `c_dom` is the `jsdom` of the vitest variant `packages/c` is
/// put on, `circus` the version of `jest-circus`, `pad` the version of
/// `left-pad`.
fn lock(c_dom: &str, circus: &str, pad: &str) -> String {
    let long = long_peers();
    format!(
        "\
lockfileVersion: '9.0'

settings:
  autoInstallPeers: false
  excludeLinksFromLockfile: false

importers:

  .: {{}}

  packages/a:
    devDependencies:
      jsdom:
        specifier: 27.1.0
        version: 27.1.0
      vitest:
        specifier: 4.1.9
        version: 4.1.9(jsdom@27.1.0)

  packages/b:
    devDependencies:
      jsdom:
        specifier: 23.2.0
        version: 23.2.0
      left-pad:
        specifier: ^1.0.0
        version: {pad}
      vitest:
        specifier: 4.1.9
        version: 4.1.9(jsdom@23.2.0)
      wide:
        specifier: 1.0.0
        version: 1.0.0{long}

  packages/c:
    devDependencies:
      vitest:
        specifier: 4.1.9
        version: 4.1.9(jsdom@{c_dom})

  packages/d:
    devDependencies:
      jest:
        specifier: 30.0.0
        version: 30.0.0

packages:

  jest-circus@{circus}:
    resolution: {{integrity: sha512-circus-{circus}==}}

  jest@30.0.0:
    resolution: {{integrity: sha512-jest==}}

  jsdom@23.2.0:
    resolution: {{integrity: sha512-jsdom23==}}

  jsdom@27.1.0:
    resolution: {{integrity: sha512-jsdom27==}}

  left-pad@{pad}:
    resolution: {{integrity: sha512-pad-{pad}==}}

  vitest@4.1.9:
    resolution: {{integrity: sha512-vitest==}}
    peerDependencies:
      jsdom: '*'
    peerDependenciesMeta:
      jsdom:
        optional: true

  wide@1.0.0:
    resolution: {{integrity: sha512-wide==}}

snapshots:

  jest-circus@{circus}: {{}}

  jest@30.0.0:
    dependencies:
      jest-circus: {circus}

  jsdom@23.2.0: {{}}

  jsdom@27.1.0: {{}}

  left-pad@{pad}: {{}}

  vitest@4.1.9(jsdom@23.2.0):
    optionalDependencies:
      jsdom: 23.2.0

  vitest@4.1.9(jsdom@27.1.0):
    optionalDependencies:
      jsdom: 27.1.0

  ? 'wide@1.0.0{long}'
  : dependencies:
      left-pad: {pad}
"
    )
}

/// Four projects. The vitest projects use globals and a config that imports
/// nothing, so no file of theirs names `vitest`; `packages/c` declares neither
/// `jsdom` nor anything else besides the runner. `packages/d` is a jest
/// project.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, ".gitignore", "node_modules\n.turbo/trace\n");
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    for (name, deps) in [
        ("a", r#"{"jsdom":"27.1.0","vitest":"4.1.9"}"#),
        (
            "b",
            r#"{"jsdom":"23.2.0","left-pad":"^1.0.0","vitest":"4.1.9","wide":"1.0.0"}"#,
        ),
        ("c", r#"{"vitest":"4.1.9"}"#),
    ] {
        write(
            r,
            &format!("packages/{name}/package.json"),
            &format!(r#"{{"name":"@fx/{name}","version":"1.0.0","devDependencies":{deps}}}"#),
        );
        write(
            r,
            &format!("packages/{name}/vitest.config.ts"),
            "export default {\n  test: {\n    globals: true,\n    environment: 'jsdom',\n  \
             },\n};\n",
        );
        write(
            r,
            &format!("packages/{name}/src/{name}.ts"),
            "export const value = 1;\n",
        );
        write(
            r,
            &format!("packages/{name}/src/{name}.spec.ts"),
            &format!(
                "import {{ value }} from './{name}';\n\ntest('value', () => {{\n  \
                 expect(value).toBe(1);\n}});\n"
            ),
        );
    }
    write(
        r,
        "packages/d/package.json",
        r#"{"name":"@fx/d","version":"1.0.0","devDependencies":{"jest":"30.0.0"}}"#,
    );
    write(r, "packages/d/jest.config.js", "module.exports = {};\n");
    write(r, "packages/d/src/d.js", "module.exports = 1;\n");
    write(
        r,
        "packages/d/src/d.test.js",
        "const d = require('./d');\n\ntest('d', () => {\n  expect(d).toBe(1);\n});\n",
    );
    write(r, "pnpm-lock.yaml", &lock("27.1.0", "30.0.0", "1.3.0"));
    git(r, &["init", "-q", "-b", "main"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "base"]);
    d
}

/// The tests `affected --since HEAD` selects through real paths (uncertainty
/// ignored).
fn selected(r: &Path) -> Vec<String> {
    ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--tests",
            "vitest",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

#[test]
fn a_project_moved_to_another_peer_variant_of_its_runner() {
    let d = repo();
    let r = d.path();
    assert!(selected(r).is_empty(), "nothing changed");

    // Only the importer block of packages/c changes: its vitest is now the jsdom@23
    // variant, which packages/b already used. No package is added, removed or
    // changed.
    write(r, "pnpm-lock.yaml", &lock("23.2.0", "30.0.0", "1.3.0"));
    assert_eq!(
        selected(r),
        ["packages/c/src/c.spec.ts"],
        "the tests of the project whose runner was re-resolved, and no others"
    );
    // The reason names the lockfile.
    let why = ok(
        r,
        &[
            "affected",
            "--tests",
            "vitest",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
            "--explain",
            "packages/c/src/c.spec.ts",
        ],
    );
    assert!(
        why.contains("affected packages/c/src/c.spec.ts") && why.contains("pnpm-lock.yaml"),
        "{why}"
    );
}

#[test]
fn a_dependency_of_the_runner_is_bumped() {
    let d = repo();
    let r = d.path();
    // jest-circus is a dependency of jest only; no file names either package.
    write(r, "pnpm-lock.yaml", &lock("27.1.0", "30.0.1", "1.3.0"));
    assert_eq!(selected(r), ["packages/d/src/d.test.js"]);
}

#[test]
fn a_dependency_no_test_loads_selects_nothing() {
    let d = repo();
    let r = d.path();
    // left-pad is a direct dependency of packages/b and a dependency of `wide`
    // (whose snapshot key is written in the explicit form); no file imports
    // either.
    write(r, "pnpm-lock.yaml", &lock("27.1.0", "30.0.0", "1.3.1"));
    assert!(selected(r).is_empty(), "{:?}", selected(r));
    // The lockfile was diffed, not treated as an opaque change.
    let o = run(
        r,
        &[
            "affected", "--tests", "vitest", "--since", "HEAD", "--stats",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("pnpm-lock.yaml: ") && err.contains("package(s) with a changed closure"),
        "{err}"
    );
}
