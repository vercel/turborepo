//! `turbotrace affected` and `turbotrace fingerprint` end to end on small JS
//! repos.
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
        .expect("run turbotrace")
}

fn ok(root: &Path, args: &[&str]) -> String {
    let o = run(root, args);
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn lines(s: &str) -> Vec<&str> {
    s.lines().collect()
}

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        // Independent of the developer's signing setup.
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

/// A jest repo: a setup file, a module mapper, a snapshot and three tests.
fn jest_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "jest.config.js",
        "module.exports = {\n  setupFiles: ['<rootDir>/test/setup.js'],\n  moduleNameMapper: { \
         '^@app/(.*)$': '<rootDir>/src/$1' },\n};\n",
    );
    write(r, "src/a.js", "module.exports = require('./b');\n");
    write(r, "src/b.js", "module.exports = 1;\n");
    write(r, "src/c.js", "module.exports = 2;\n");
    write(r, "src/mapped.js", "module.exports = 3;\n");
    write(r, "test/setup.js", "global.x = 1;\n");
    write(
        r,
        "test/a.test.js",
        "require('../src/a');\ntest('a', () => {});\n",
    );
    write(
        r,
        "test/c.test.js",
        "require('../src/c');\ntest('c', () => {});\n",
    );
    write(
        r,
        "test/m.test.js",
        "require('@app/mapped');\ntest('m', () => {});\n",
    );
    write(
        r,
        "test/__snapshots__/a.test.js.snap",
        "exports[`a 1`] = `1`;\n",
    );
    d
}

fn affected(root: &Path, changed: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest"];
    for c in changed {
        args.push("--changed");
        args.push(c);
    }
    ok(root, &args).lines().map(str::to_owned).collect()
}

#[test]
fn jest_tests_and_inputs() {
    let d = jest_repo();
    let r = d.path();
    let all = ["test/a.test.js", "test/c.test.js", "test/m.test.js"];
    // Discovery through the jest adapter.
    let o = ok(
        r,
        &["affected", "--tests", "jest", "--changed", "jest.config.js"],
    );
    assert_eq!(lines(&o), all, "a config change affects every test");
    assert_eq!(affected(r, &["src/b.js"]), ["test/a.test.js"]);
    assert_eq!(affected(r, &["src/c.js"]), ["test/c.test.js"]);
    assert_eq!(
        affected(r, &["test/setup.js"]),
        all,
        "setup files are inputs of every test"
    );
    assert_eq!(
        affected(r, &["src/mapped.js"]),
        ["test/m.test.js"],
        "moduleNameMapper edge"
    );
    assert_eq!(
        affected(r, &["test/__snapshots__/a.test.js.snap"]),
        ["test/a.test.js"],
        "snapshot edge"
    );
    assert!(affected(r, &["README.md"]).is_empty());

    // Explicit entries, globs and --entries-from.
    let o = ok(r, &["affected", "test/*.test.js", "--changed", "src/b.js"]);
    assert_eq!(lines(&o), ["test/a.test.js"]);
    write(r, "entries.txt", "test/a.test.js\ntest/c.test.js\n");
    write(r, "changes.txt", "M\tsrc/c.js\n");
    let o = ok(
        r,
        &[
            "affected",
            "--entries-from",
            "entries.txt",
            "--changed-from",
            "changes.txt",
            "--since",
            "",
        ],
    );
    assert_eq!(lines(&o), ["test/c.test.js"]);

    // JSON verdicts with witness chains.
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "src/b.js",
            "--format",
            "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(v["affected"], 1);
    assert_eq!(v["unaffected"], 2);
    let w: Vec<&str> = v["results"][0]["witness"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(w, ["src/b.js", "src/a.js", "test/a.test.js"]);

    // --explain.
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "src/b.js",
            "--explain",
            "test/a.test.js",
        ],
    );
    assert!(o.contains("src/b.js → src/a.js → test/a.test.js"), "{o}");
    // --explain alone: the explained entry is the entry set.
    let o = ok(
        r,
        &[
            "affected",
            "--changed",
            "src/b.js",
            "--explain",
            "test/a.test.js",
        ],
    );
    assert!(o.contains("src/b.js → src/a.js → test/a.test.js"), "{o}");
    let o = ok(
        r,
        &[
            "affected",
            "--changed",
            "src/b.js",
            "--explain",
            "test/c.test.js",
        ],
    );
    assert_eq!(o.trim(), "unaffected test/c.test.js");

    // Playwright specs are selected from source when no build fingerprint is given
    // (tests/affected_playwright_source.rs); this repository has no Playwright
    // config.
    let o = run(
        r,
        &["affected", "--tests", "playwright", "--changed", "src/b.js"],
    );
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout), "");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("no Playwright specs"), "{err}");

    // --exit-code.
    let o = run(
        r,
        &[
            "affected",
            "test/c.test.js",
            "--changed",
            "src/b.js",
            "--exit-code",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    let o = run(
        r,
        &[
            "affected",
            "test/a.test.js",
            "--changed",
            "src/b.js",
            "--exit-code",
        ],
    );
    assert_eq!(o.status.code(), Some(1));

    // A deleted file: the graph is refreshed after the change, where a.js probes
    // it.
    std::fs::remove_file(r.join("src/b.js")).unwrap();
    write(r, "changes.txt", "D\tsrc/b.js\n");
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed-from",
            "changes.txt",
        ],
    );
    assert_eq!(lines(&o), ["test/a.test.js"]);
    // Re-adding it is seen through the new node.
    write(r, "src/b.js", "module.exports = 1;\n");
    write(r, "changes.txt", "A\tsrc/b.js\n");
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed-from",
            "changes.txt",
        ],
    );
    assert_eq!(lines(&o), ["test/a.test.js"]);

    // A new test file becomes a test after the refresh.
    write(r, "test/new.test.js", "require('../src/c');\n");
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "test/new.test.js",
        ],
    );
    assert_eq!(lines(&o), ["test/new.test.js"]);
}

#[test]
fn config_edits_invalidate_framework_inputs() {
    let d = jest_repo();
    let r = d.path();
    assert_eq!(affected(r, &["src/mapped.js"]), ["test/m.test.js"]);
    // Point the mapper elsewhere: the stored framework edges must be rebuilt.
    write(r, "src/other/mapped.js", "module.exports = 4;\n");
    write(
        r,
        "jest.config.js",
        "module.exports = {\n  setupFiles: ['<rootDir>/test/setup.js'],\n  moduleNameMapper: { \
         '^@app/(.*)$': '<rootDir>/src/other/$1' },\n};\n",
    );
    assert_eq!(affected(r, &["src/other/mapped.js"]), ["test/m.test.js"]);
    // Drop the setup file from the config: it stops being an input of every test.
    write(
        r,
        "jest.config.js",
        "module.exports = {\n  moduleNameMapper: { '^@app/(.*)$': '<rootDir>/src/other/$1' \
         },\n};\n",
    );
    assert!(affected(r, &["test/setup.js"]).is_empty());
}

/// npm lockfile with dep-a → dep-b, and dep-c.
fn lock(b_version: &str, d_version: &str) -> String {
    format!(
        r#"{{
  "name": "r", "version": "1.0.0", "lockfileVersion": 3, "requires": true,
  "packages": {{
    "": {{ "name": "r", "version": "1.0.0", "dependencies": {{ "dep-a": "1.0.0", "dep-c": "1.0.0", "dep-d": "1.0.0" }} }},
    "node_modules/dep-a": {{ "version": "1.0.0", "integrity": "sha512-a", "dependencies": {{ "dep-b": "^1.0.0" }} }},
    "node_modules/dep-b": {{ "version": "{b_version}", "integrity": "sha512-b{b_version}" }},
    "node_modules/dep-c": {{ "version": "1.0.0", "integrity": "sha512-c" }},
    "node_modules/dep-d": {{ "version": "{d_version}", "integrity": "sha512-d{d_version}" }}
  }}
}}
"#
    )
}

fn pkg(r: &Path, name: &str, version: &str) {
    write(
        r,
        &format!("node_modules/{name}/package.json"),
        &format!(r#"{{"name":"{name}","version":"{version}","main":"index.js"}}"#),
    );
    write(
        r,
        &format!("node_modules/{name}/index.js"),
        "module.exports = 1;\n",
    );
}

#[test]
fn lockfile_diffs_mark_packages() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, ".gitignore", "node_modules\n.turbo/trace\n");
    write(
        r,
        "package.json",
        r#"{"name":"r","version":"1.0.0","dependencies":{"dep-a":"1.0.0","dep-c":"1.0.0","dep-d":"1.0.0"}}"#,
    );
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "package-lock.json", &lock("1.0.0", "1.0.0"));
    pkg(r, "dep-a", "1.0.0");
    pkg(r, "dep-b", "1.0.0");
    pkg(r, "dep-c", "1.0.0");
    pkg(r, "dep-d", "1.0.0");
    write(r, "test/a.test.js", "require('dep-a');\n");
    write(r, "test/c.test.js", "require('dep-c');\n");
    git(r, &["init", "-q", "-b", "main"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "base"]);

    // Transitive bump of dep-b (through dep-a).
    write(r, "package-lock.json", &lock("1.1.0", "1.0.0"));
    pkg(r, "dep-b", "1.1.0");
    let o = ok(r, &["affected", "--tests", "jest", "--since", "HEAD"]);
    assert_eq!(lines(&o), ["test/a.test.js"], "closure-changed dep-a");
    // Without the old lockfile: every npm package is changed.
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "package-lock.json",
        ],
    );
    assert_eq!(lines(&o), ["test/a.test.js", "test/c.test.js"]);
    // The same with --changed-from and --since (the harness's selector template).
    write(r, "changes.txt", "M\tpackage-lock.json\n");
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed-from",
            "changes.txt",
            "--since",
            "HEAD",
        ],
    );
    assert_eq!(lines(&o), ["test/a.test.js"]);

    // A bump of a package no test loads.
    write(r, "package-lock.json", &lock("1.0.0", "1.0.1"));
    pkg(r, "dep-b", "1.0.0");
    pkg(r, "dep-d", "1.0.1");
    let o = ok(r, &["affected", "--tests", "jest", "--since", "HEAD"]);
    assert_eq!(o, "", "nothing loads dep-d");

    // A range edit in package.json without a lockfile change.
    write(r, "package-lock.json", &lock("1.0.0", "1.0.0"));
    pkg(r, "dep-d", "1.0.0");
    write(
        r,
        "package.json",
        r#"{"name":"r","version":"1.0.0","dependencies":{"dep-a":"1.0.0","dep-c":"^1.0.0","dep-d":"1.0.0"}}"#,
    );
    let o = ok(
        r,
        &[
            "affected", "--tests", "jest", "--since", "HEAD", "--format", "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    let pkgs: Vec<&str> = v["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(pkgs, ["dep-c"], "{o}");
    let hit: Vec<&str> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["entry"].as_str().unwrap())
        .collect();
    assert!(hit.contains(&"test/c.test.js"), "{o}");
}

/// Regression: a cold build over many workspace packages whose tests all share
/// one deep directory membership (the root jest config's `**/__mocks__/**`)
/// deadlocked when the membership listing (nested rayon work) ran inside a
/// `OnceLock` initializer and a waiting worker stole a job that hit the same
/// cell. Timing-dependent, so it runs a cold and several warm builds with small
/// and large pools, each under a timeout.
#[test]
fn shared_memberships_do_not_deadlock() {
    use std::time::{Duration, Instant};
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "package.json",
        r#"{"name":"root","version":"1.0.0","private":true,"workspaces":["packages/*"]}"#,
    );
    write(r, "jest.config.js", "module.exports = {};\n");
    for p in 0..40 {
        write(
            r,
            &format!("packages/p{p}/package.json"),
            &format!(r#"{{"name":"p{p}","version":"1.0.0"}}"#),
        );
        for s in 0..3 {
            write(
                r,
                &format!("packages/p{p}/src/d{s}/m{s}.js"),
                "module.exports = 1;\n",
            );
            write(
                r,
                &format!("packages/p{p}/test/t{s}.test.js"),
                &format!("require('../src/d{s}/m{s}');\ntest('t', () => {{}});\n"),
            );
        }
        write(
            r,
            &format!("packages/p{p}/src/__mocks__/m.js"),
            "module.exports = 2;\n",
        );
    }
    // Cold, then warm builds after a change: the warm path re-validates every
    // package's stored membership digests in parallel (where the hang was
    // seen).
    for threads in ["2", "16"] {
        let _ = std::fs::remove_dir_all(r.join(".turbo/trace"));
        for round in 0..4 {
            if round > 0 {
                write(
                    r,
                    &format!("packages/p{round}/src/new{threads}.js"),
                    "module.exports = 3;\n",
                );
            }
            let mut child = Command::new(support::harness())
                .args(["graph", "build", "--threads", threads])
                .env("RAYON_NUM_THREADS", threads)
                .current_dir(r)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let start = Instant::now();
            let status = loop {
                if let Some(s) = child.try_wait().unwrap() {
                    break s;
                }
                if start.elapsed() > Duration::from_secs(60) {
                    let _ = child.kill();
                    panic!("graph build hung (threads {threads}, round {round})");
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            assert!(status.success(), "threads {threads}, round {round}");
        }
    }
    // The graph built under contention is right.
    let o = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "packages/p3/src/d1/m1.js",
            "--no-refresh",
        ],
    );
    assert_eq!(lines(&o), ["packages/p3/test/t1.test.js"]);
}

fn copy_dir(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dst).unwrap();
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

/// Regression: a CI workflow job that rewrites the
/// root `package.json` in place (`jq … > package.json.tmp && mv
/// package.json.tmp package.json`) and runs an unknown command (whole-repo
/// input) must not become the producer of `package.json`. Otherwise every
/// change reaches every test through the job.
#[test]
fn ci_job_in_place_edit_does_not_select_everything() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/tasks/outputs"),
        r,
    );
    // Control: a real dependency still selects the test.
    assert_eq!(affected(r, &["src/index.ts"]), vec!["src/index.test.ts"]);
    // An unrelated file only reaches the workflow job's whole-repo input.
    assert!(
        affected(r, &["assets/logo.svg"]).is_empty(),
        "unrelated change selected tests"
    );
}

/// M1: a jest config that default-exports an async function wrapping
/// `nextJest({ dir })(customConfig)()` (the shape of a large Next.js app) used
/// to yield zero tests, silently.
#[test]
fn next_jest_async_config_discovers_tests() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"site","version":"1.0.0"}"#);
    write(
        r,
        "jest.config.ts",
        r#"import nextJest from 'next/jest';
import type { Config } from 'jest';

const customConfig: Config = {
  setupFilesAfterEnv: ['<rootDir>/test/setup.ts'],
  testEnvironment: 'jsdom',
  testMatch: ['./**/*.test.{ts,tsx}'],
  testPathIgnorePatterns: ['/node_modules/', 'test.mocks'],
};

export default async (): Promise<Config> => {
  const config = (await nextJest({ dir: __dirname })(customConfig)()) as Config;
  const finalConfig = {
    ...config,
    transformIgnorePatterns: (config.transformIgnorePatterns ?? []).filter(
      (pattern) => !pattern.includes('node_modules'),
    ),
  };
  return finalConfig;
};
"#,
    );
    // Installed packages, as in a real checkout.
    for (pkg, main) in [("next", "jest.js"), ("jest", "index.js")] {
        write(
            r,
            &format!("node_modules/{pkg}/package.json"),
            &format!(r#"{{"name":"{pkg}","version":"1.0.0","main":"{main}"}}"#),
        );
        write(
            r,
            &format!("node_modules/{pkg}/{main}"),
            "module.exports = {};\n",
        );
    }
    write(r, "src/a.ts", "export const a = 1;\n");
    write(r, "src/b.ts", "export const b = 1;\n");
    write(r, "test/setup.ts", "export {};\n");
    write(
        r,
        "src/a.test.ts",
        "import { a } from './a';\ntest('a', () => expect(a).toBe(1));\n",
    );
    assert_eq!(affected(r, &["src/a.ts"]), ["src/a.test.ts"]);
    assert_eq!(affected(r, &["test/setup.ts"]), ["src/a.test.ts"]);
    assert!(affected(r, &["src/b.ts"]).is_empty());
}

/// P10: an optional load in a global jest setup file (`try {
/// require.resolve(x); jest.mock(x, ...) } catch {}`) whose module doesn't
/// exist is no uncertainty, so it doesn't make every test affected; adding the
/// module later still selects (its path was probed).
#[test]
fn optional_requires_in_jest_setup_are_not_uncertain() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(
        r,
        "jest.config.js",
        "module.exports = { setupFilesAfterEnv: ['<rootDir>/tests/setup.js'] };\n",
    );
    write(
        r,
        "tests/setup.js",
        r#"try {
  require.resolve('@api/user-auth/utils/is-vercelian');
  jest.mock('@api/user-auth/utils/is-vercelian', () => ({
    ...jest.requireActual('@api/user-auth/utils/is-vercelian'),
    isMFAEnforced: jest.fn().mockReturnValue(false),
  }));
} catch {
  // @api/user-auth not installed in this workspace
}
try { require('./local-optional'); } catch (e) {}
"#,
    );
    write(r, "src/a.js", "module.exports = 1;\n");
    write(r, "src/b.js", "module.exports = 2;\n");
    write(
        r,
        "tests/a.test.js",
        "require('../src/a');\ntest('a', () => {});\n",
    );
    assert!(affected(r, &["src/b.js"]).is_empty(), "no uncertainty");
    assert_eq!(affected(r, &["src/a.js"]), ["tests/a.test.js"]);
    // Sound: the missing optional module was probed.
    write(r, "tests/local-optional.js", "module.exports = 1;\n");
    assert_eq!(
        affected(r, &["tests/local-optional.js"]),
        ["tests/a.test.js"]
    );
}

/// P13: only the tsconfig that governs a file (and its `extends` chain) is a
/// content input. Project references checked on the way (and configs above the
/// governing level) are not: editing a referenced package's tsconfig used to
/// reach every file below the referencing config.
#[test]
fn only_the_governing_tsconfig_is_an_input() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(
        r,
        "tsconfig.json",
        r#"{"files":[],"references":[{"path":"apps/site"},{"path":"packages/gate"},{"path":"packages/other"}]}"#,
    );
    write(
        r,
        "tsconfig.base.json",
        r#"{"compilerOptions":{"strict":true}}"#,
    );
    for p in ["gate", "other"] {
        write(
            r,
            &format!("packages/{p}/package.json"),
            &format!(r#"{{"name":"{p}","version":"1.0.0"}}"#),
        );
        write(
            r,
            &format!("packages/{p}/tsconfig.json"),
            r#"{"extends":"../../tsconfig.base.json","compilerOptions":{"composite":true},"include":["src"]}"#,
        );
        write(
            r,
            &format!("packages/{p}/src/index.ts"),
            "export const x = 1;\n",
        );
    }
    write(
        r,
        "apps/site/package.json",
        r#"{"name":"site","version":"1.0.0"}"#,
    );
    write(
        r,
        "apps/site/tsconfig.json",
        r#"{"extends":"./tsconfig.paths.json","include":["**/*.ts"],"references":[{"path":"../../packages/gate"},{"path":"../../packages/other"}]}"#,
    );
    write(
        r,
        "apps/site/tsconfig.paths.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    write(r, "apps/site/jest.config.ts", "export default {};\n");
    write(r, "apps/site/src/a.ts", "export const a = 1;\n");
    write(
        r,
        "apps/site/src/a.test.ts",
        "import { a } from '@/a';\ntest('a', () => {});\n",
    );
    let test = ["apps/site/src/a.test.ts"];
    // The governing config and its `extends` chain still select (sound).
    assert_eq!(affected(r, &["apps/site/tsconfig.json"]), test);
    assert_eq!(affected(r, &["apps/site/tsconfig.paths.json"]), test);
    assert_eq!(
        affected(r, &["apps/site/src/a.ts"]),
        test,
        "paths alias edge"
    );
    // The solution above the governing config is not consulted: no input.
    assert!(affected(r, &["tsconfig.json"]).is_empty());
    // References that don't govern the file, and their bases, are not inputs
    // either: an edit that leaves alone what they include selects nothing. That
    // is decided against the base revision
    // (`crates/turborepo-trace/src/tsgovern.rs`).
    write(r, ".gitignore", ".turbo/trace/\n");
    git(r, &["init", "-q"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "base"]);
    let since = |r: &Path| -> Vec<String> {
        ok(r, &["affected", "--tests", "jest", "--since", "HEAD"])
            .lines()
            .map(str::to_owned)
            .collect()
    };
    for (c, edited) in [
        (
            "packages/gate/tsconfig.json",
            r#"{"extends":"../../tsconfig.base.json","compilerOptions":{"composite":true,"strict":false},"include":["src"]}"#,
        ),
        (
            "packages/other/tsconfig.json",
            r#"{"extends":"../../tsconfig.base.json","compilerOptions":{"composite":true,"strict":false},"include":["src"]}"#,
        ),
        (
            "tsconfig.base.json",
            r#"{"compilerOptions":{"strict":false}}"#,
        ),
    ] {
        write(r, c, edited);
        assert!(since(r).is_empty(), "{c}: {:?}", since(r));
        git(r, &["checkout", "-q", "--", c]);
        // Named as changed without a base revision, nothing says what the project
        // included before: the files that consulted it count as changed.
        assert_eq!(affected(r, &[c]), test, "{c} without a base");
    }
    // A config added between the file and its governing config is probed.
    write(r, "apps/site/src/tsconfig.json", "{}\n");
    assert_eq!(affected(r, &["apps/site/src/tsconfig.json"]), test);
}

/// Scoped uncertainty: a read of a directory's files truncated at
/// `--max-glob` is covered by its directory. Edits, additions and deletions
/// inside it select the test; edits outside don't. An open uncertainty still
/// selects on any change, and `eval` is ignored unless `--strict-eval`.
#[test]
fn covered_uncertainty_selects_only_inside_its_scope() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(
        r,
        "src/scan.js",
        "const fs = require('fs');\nconst path = require('path');\nmodule.exports = \
         fs.readdirSync(path.join(__dirname, '..', 'data')).map((f) => \
         fs.readFileSync(path.join(__dirname, '..', 'data', f), 'utf8'));\n",
    );
    write(r, "src/open.js", "module.exports = (p) => require(p);\n");
    write(
        r,
        "src/ev.js",
        "module.exports = new Function('return 1');\n",
    );
    write(r, "src/plain.js", "module.exports = 1;\n");
    for f in ["a", "b", "c"] {
        write(r, &format!("data/{f}.txt"), f);
    }
    write(r, "other/x.txt", "x");
    for (t, m) in [
        ("scan", "scan"),
        ("open", "open"),
        ("eval", "ev"),
        ("plain", "plain"),
    ] {
        write(
            r,
            &format!("test/{t}.test.js"),
            &format!("require('../src/{m}');\ntest('{t}', () => {{}});\n"),
        );
    }
    ok(r, &["graph", "build", "--max-glob", "2"]);

    // Outside every scope: only the open uncertainty selects.
    assert_eq!(affected(r, &["other/x.txt"]), ["test/open.test.js"]);
    // Inside the scope: an edit, an addition, a deletion.
    let both = ["test/open.test.js", "test/scan.test.js"];
    assert_eq!(affected(r, &["data/a.txt"]), both, "edit");
    write(r, "data/d.txt", "d");
    assert_eq!(affected(r, &["data/d.txt"]), both, "add");
    std::fs::remove_file(r.join("data/b.txt")).unwrap();
    assert_eq!(affected(r, &["data/b.txt"]), both, "delete");
    // The open uncertainty selects on any change; eval only with --strict-eval.
    assert_eq!(
        affected(r, &["src/plain.js"]),
        ["test/open.test.js", "test/plain.test.js"]
    );
    let strict = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--strict-eval",
            "--changed",
            "src/plain.js",
        ],
    );
    assert_eq!(
        lines(&strict),
        [
            "test/eval.test.js",
            "test/open.test.js",
            "test/plain.test.js"
        ]
    );

    // Reported: --explain names the scope or the ignored eval, JSON counts them.
    let ex = ok(
        r,
        &[
            "affected",
            "--changed",
            "data/a.txt",
            "--explain",
            "test/scan.test.js",
            "--explain",
            "test/eval.test.js",
        ],
    );
    assert!(
        ex.contains(
            "affected test/scan.test.js: modified data/a.txt (inside the scope data/** of a \
             covered uncertainty) at src/scan.js → test/scan.test.js"
        ),
        "{ex}"
    );
    assert!(
        ex.contains("  covered src/scan.js:3 pattern_too_broad (scope data/**)"),
        "{ex}"
    );
    assert!(ex.contains("unaffected test/eval.test.js"), "{ex}");
    assert!(
        ex.contains("  ignored by default src/ev.js:1 eval_like (--strict counts it)"),
        "{ex}"
    );
    let json: serde_json::Value = serde_json::from_str(&ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "other/x.txt",
            "--format",
            "json",
        ],
    ))
    .unwrap();
    assert_eq!(json["uncertainty"]["covered"], 1, "{json}");
    assert_eq!(json["uncertainty"]["ignored"], 1, "{json}");
    assert_eq!(json["uncertainty"]["open"], 1, "{json}");
}

/// Uncertainty that something else already covers doesn't put a test on the
/// "always selected" floor: an import that
/// doesn't resolve is covered by the paths its resolution probed, and a read of
/// a path from the environment is an input from outside the repository.
/// `--strict` counts both.
#[test]
fn unresolved_imports_and_env_reads_are_not_open() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(
        r,
        "src/missing-dep.js",
        "module.exports = require('./not-there');\n",
    );
    write(
        r,
        "src/env-read.js",
        // A value derived from the variable: the env registry does not resolve it (a bare
        // `process.env.DATA_FILE` that nothing assigns would leave no record at all).
        "const fs = require('fs');\nmodule.exports = \
         fs.readFileSync(process.env.DATA_FILE.split(':')[0], 'utf8');\n",
    );
    write(
        r,
        "src/env-load.js",
        "module.exports = require(process.env.PLUGIN);\n",
    );
    write(r, "src/plain.js", "module.exports = 1;\n");
    for t in ["missing-dep", "env-read", "env-load", "plain"] {
        write(
            r,
            &format!("test/{t}.test.js"),
            &format!("require('../src/{t}');\ntest('{t}', () => {{}});\n"),
        );
    }
    write(r, "docs/readme.txt", "x");
    ok(r, &["graph", "build"]);

    // An unrelated change selects only the test that loads a module named by the
    // environment: that one executes code nothing bounds.
    assert_eq!(affected(r, &["docs/readme.txt"]), ["test/env-load.test.js"]);
    assert_eq!(
        affected(r, &["src/plain.js"]),
        ["test/env-load.test.js", "test/plain.test.js"]
    );
    // The missing module appears: the import now resolves, and its test is
    // selected.
    write(r, "src/not-there.js", "module.exports = 2;\n");
    assert_eq!(
        affected(r, &["src/not-there.js"]),
        ["test/env-load.test.js", "test/missing-dep.test.js"]
    );
    std::fs::remove_file(r.join("src/not-there.js")).unwrap();
    assert_eq!(
        affected(r, &["src/not-there.js"]),
        ["test/env-load.test.js", "test/missing-dep.test.js"],
        "deleted again"
    );

    // --strict: every record counts.
    let strict = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--strict",
            "--changed",
            "docs/readme.txt",
        ],
    );
    assert_eq!(
        lines(&strict),
        [
            "test/env-load.test.js",
            "test/env-read.test.js",
            "test/missing-dep.test.js"
        ]
    );

    // Reported: --explain says why they don't count, and the JSON counts them.
    let ex = ok(
        r,
        &[
            "affected",
            "--changed",
            "docs/readme.txt",
            "--explain",
            "test/missing-dep.test.js",
            "--explain",
            "test/env-read.test.js",
        ],
    );
    assert!(ex.contains("unaffected test/missing-dep.test.js"), "{ex}");
    assert!(
        ex.contains(
            "covered src/missing-dep.js:1 unresolved_specifier (by the paths its resolution \
             probed)"
        ),
        "{ex}"
    );
    assert!(
        ex.contains("ignored by default src/env-read.js:2 env_dependent (--strict counts it)"),
        "{ex}"
    );
    let json = ok(
        r,
        &[
            "affected",
            "--tests",
            "jest",
            "--changed",
            "docs/readme.txt",
            "--format",
            "json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["uncertainty"]["open"], 1, "{json}");
    assert_eq!(v["uncertainty"]["probed"], 1, "{json}");
    assert_eq!(v["uncertainty"]["ignored"], 1, "{json}");
}
