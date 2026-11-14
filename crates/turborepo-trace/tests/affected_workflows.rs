//! Workflow changes in `affected`
//! (`crates/turborepo-trace/src/workflows.rs`), end to end over
//! `fixtures/tasks/gha-runs` in a temporary git repository: a changed job
//! selects the tests it runs and nothing else; edits that only steer a workflow
//! select nothing.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
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

/// The fixture as a committed git repository with a built graph.
fn repo() -> tempfile::TempDir {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/gha-runs");
    let d = tempfile::tempdir().unwrap();
    copy_dir(&src, d.path());
    std::fs::write(d.path().join(".gitignore"), ".turbo/trace\n").unwrap();
    git(d.path(), &["init", "-q", "-b", "main"]);
    git(d.path(), &["add", "-A"]);
    git(d.path(), &["commit", "-qm", "base"]);
    d
}

fn run(root: &Path, args: &[&str]) -> Vec<String> {
    let o = Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Replaces `from` by `to` in `rel` (which must contain it).
fn edit(root: &Path, rel: &str, from: &str, to: &str) {
    let p = root.join(rel);
    let s = std::fs::read_to_string(&p).unwrap();
    assert!(s.contains(from), "{rel} contains {from:?}");
    std::fs::write(p, s.replacen(from, to, 1)).unwrap();
}

/// Tests and workflow jobs affected since the base commit. Jobs are listed
/// without the uncertainty verdicts (several jobs reach a script with a
/// computed read, which makes them affected by every change under the default
/// policy).
fn since(root: &Path) -> (Vec<String>, Vec<String>) {
    let tests = run(root, &["affected", "--tests", "jest", "--since", "HEAD"]);
    let jobs = run(
        root,
        &[
            "affected",
            "--tasks",
            "gha:**",
            "--since",
            "HEAD",
            "--on-uncertain",
            "ignore",
        ],
    )
    .into_iter()
    .map(|j| j.replace("gha:.github/workflows/", ""))
    .collect();
    (tests, jobs)
}

const A: &str = "packages/a/src/a.test.js";
const B: &str = "packages/b/src/b.test.js";
const E2E: &str = "tools/e2e/flow.test.js";
const QUALITY: &str = ".github/workflows/quality.yml";
const CHECKS: &str = ".github/workflows/checks.yml";
const DEPLOY: &str = ".github/workflows/deploy.yml";

#[test]
fn a_deploy_job_edit_selects_no_tests() {
    let d = repo();
    let r = d.path();
    edit(
        r,
        DEPLOY,
        "deploy.sh production",
        "deploy.sh production --wait",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, Vec::<String>::new());
    // The job, and the job that needs it.
    assert_eq!(jobs, ["deploy.yml#deploy", "deploy.yml#notify"]);
}

#[test]
fn steering_edits_select_nothing() {
    let d = repo();
    let r = d.path();
    // A comment, the path filter, the display names, a job-level `if`, concurrency.
    edit(
        r,
        QUALITY,
        "name: Quality",
        "# CI for pull requests\nname: Quality checks",
    );
    edit(
        r,
        QUALITY,
        "      - 'packages/**'",
        "      - 'packages/**'\n      - 'tools/**'",
    );
    edit(
        r,
        QUALITY,
        "    name: Test packages",
        "    name: Unit tests\n    if: github.actor != 'bot'",
    );
    edit(
        r,
        CHECKS,
        "name: Checks",
        "name: Checks\nconcurrency: checks-${{ github.ref }}",
    );
    edit(r, DEPLOY, "branches: [main]", "branches: [main, release]");
    let (tests, jobs) = since(r);
    assert_eq!(tests, Vec::<String>::new());
    assert_eq!(jobs, Vec::<String>::new());
}

#[test]
fn the_test_job_selects_the_tests_it_runs() {
    let d = repo();
    let r = d.path();
    // The runner image of the e2e job: only the e2e tests.
    edit(
        r,
        QUALITY,
        "  e2e:\n    runs-on: ubuntu-latest",
        "  e2e:\n    runs-on: ubuntu-24.04",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, [E2E]);
    assert_eq!(jobs, ["quality.yml#e2e", "quality.yml#summary"]);
    let why = run(r, &["affected", "--since", "HEAD", "--explain", E2E]);
    assert_eq!(
        why[0],
        format!("affected {E2E}: modified {QUALITY} (read or probed) at gha:{QUALITY}#e2e → {E2E}")
    );
}

#[test]
fn a_reusable_workflow_reaches_its_callers() {
    let d = repo();
    let r = d.path();
    // The default Node version of the reusable test workflow.
    edit(r, CHECKS, "default: '22'", "default: '24'");
    let (tests, jobs) = since(r);
    // `turbo ci:checks $FILTERS` has a computed word: every test.
    assert_eq!(tests, [A, B, E2E]);
    assert_eq!(
        jobs,
        [
            "checks.yml#checks",
            "quality.yml#summary",
            "quality.yml#test-packages"
        ]
    );
}

#[test]
fn the_call_site_and_its_inputs_count() {
    let d = repo();
    let r = d.path();
    // The caller passes another input: the called jobs run differently.
    edit(
        r,
        QUALITY,
        "      shard: ${{ toJson(matrix.shard) }}",
        "      shard: ${{ toJson(matrix.shard) }}\n      node-version: '20'",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, [A, B, E2E]);
    assert_eq!(jobs, ["quality.yml#summary", "quality.yml#test-packages"]);
}

#[test]
fn a_nested_local_action_selects_the_tests_of_the_jobs_that_use_it() {
    let d = repo();
    let r = d.path();
    edit(
        r,
        ".github/actions/setup-node/action.yml",
        "actions/setup-node@v4",
        "actions/setup-node@v5",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, [A, B, E2E]);
    assert_eq!(
        jobs,
        [
            "checks.yml#checks",
            "deploy.yml#deploy",
            "deploy.yml#notify",
            "nightly.yml#smoke",
            "quality.yml#e2e",
            "quality.yml#lint",
            "quality.yml#summary",
            "quality.yml#test-packages",
            "scoring.yml#score",
        ]
    );
    // Its description is not part of any job.
    git(r, &["checkout", "-q", "."]);
    edit(
        r,
        ".github/actions/setup-node/action.yml",
        "description: Node from the input or .nvmrc",
        "description: Node",
    );
    assert_eq!(since(r), (vec![], vec![]));
}

#[test]
fn a_job_with_mapped_tests_only() {
    let d = repo();
    let r = d.path();
    // Remove the computed word: the turbo pipeline maps to the package tests.
    edit(
        r,
        CHECKS,
        "turbo ci:checks $FILTERS --continue",
        "turbo ci:checks --continue",
    );
    git(r, &["commit", "-qam", "static filters"]);
    edit(r, CHECKS, "timeout-minutes: 30", "timeout-minutes: 45");
    let (tests, _) = since(r);
    // Every package with a `test` script, through the pipeline's dependsOn.
    assert_eq!(tests, [A, B, E2E]);
    // With a filter, only that package.
    git(r, &["checkout", "-q", "."]);
    edit(
        r,
        CHECKS,
        "turbo ci:checks --continue",
        "turbo ci:checks --filter=@acme/a --continue",
    );
    git(r, &["commit", "-qam", "one package"]);
    edit(r, CHECKS, "timeout-minutes: 30", "timeout-minutes: 45");
    let (tests, _) = since(r);
    assert_eq!(tests, [A]);
}

#[test]
fn scripts_and_files_a_job_runs_count() {
    let d = repo();
    let r = d.path();
    // A module imported by the script of the `metadata` job, which the test job
    // needs.
    edit(r, ".github/scripts/weights.mjs", "a: 2", "a: 3");
    let (tests, jobs) = since(r);
    assert_eq!(tests, [A, B, E2E]);
    for j in [
        "quality.yml#metadata",
        "quality.yml#summary",
        "quality.yml#test-packages",
    ] {
        assert!(jobs.iter().any(|x| x == j), "{j} in {jobs:?}");
    }
    // The deploy script: jobs, no tests.
    git(r, &["checkout", "-q", "."]);
    edit(
        r,
        ".github/scripts/deploy.sh",
        "kubectl rollout",
        "kubectl -n web rollout",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, Vec::<String>::new());
    for j in ["deploy.yml#deploy", "deploy.yml#notify"] {
        assert!(jobs.iter().any(|x| x == j), "{j} in {jobs:?}");
    }
    // The Node version file named by `node-version-file`: every job that installs.
    git(r, &["checkout", "-q", "."]);
    edit(r, ".nvmrc", "22", "24");
    let (tests, _) = since(r);
    assert_eq!(tests, [A, B, E2E]);
}

#[test]
fn an_unmappable_launch_selects_every_test() {
    let d = repo();
    let r = d.path();
    edit(
        r,
        ".github/workflows/nightly.yml",
        "cron: '0 3 * * *'",
        "cron: '0 4 * * *'",
    );
    assert_eq!(since(r), (vec![], vec![]), "the schedule steers");
    edit(
        r,
        ".github/workflows/nightly.yml",
        "runs-on: ubuntu-latest",
        "runs-on: ubuntu-24.04",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, [A, B, E2E]);
    assert_eq!(jobs, ["nightly.yml#smoke"]);
}

#[test]
fn without_a_base_revision_every_job_of_a_changed_file_counts() {
    let d = repo();
    let r = d.path();
    // Only a comment changed, but there is no old version to compare with.
    edit(r, DEPLOY, "name: Deploy", "# deploys\nname: Deploy");
    let tests = run(r, &["affected", "--tests", "jest", "--changed", DEPLOY]);
    assert_eq!(tests, Vec::<String>::new());
    let jobs = run(
        r,
        &[
            "affected",
            "--tasks",
            "gha:**",
            "--changed",
            DEPLOY,
            "--on-uncertain",
            "ignore",
        ],
    );
    assert_eq!(
        jobs,
        [
            "gha:.github/workflows/deploy.yml#deploy",
            "gha:.github/workflows/deploy.yml#notify"
        ]
    );
    let tests = run(r, &["affected", "--tests", "jest", "--changed", QUALITY]);
    assert_eq!(
        tests,
        [A, B, E2E],
        "the file defines the job that calls the tests"
    );
    // With the base revision the same change is a comment.
    assert_eq!(since(r), (vec![], vec![]));
}

#[test]
fn source_changes_are_unchanged() {
    let d = repo();
    let r = d.path();
    edit(r, "packages/a/src/a.js", "1", "3");
    // No job runs or reads the source file: only its own test. (Jobs are still
    // reported as tasks through their `paths:` filter and whole-package
    // fallbacks, as before.)
    let (tests, _) = since(r);
    assert_eq!(tests, [A]);
}

#[test]
fn application_code_a_job_script_imports_is_an_input_not_a_definition() {
    let d = repo();
    let r = d.path();
    // `packages/b/src/b.js` is imported by `.github/scripts/run-smoke.mjs`, the
    // script of a job whose test launch can't be mapped. The edit changes what
    // that job reads, not how it runs tests: only the test the import graph
    // selects, and the job as a task.
    edit(r, "packages/b/src/b.js", "2", "4");
    let (tests, jobs) = since(r);
    assert_eq!(tests, [B]);
    assert!(jobs.iter().any(|j| j == "nightly.yml#smoke"), "{jobs:?}");
    // The same for a repository script outside `.github/` that imports a package.
    git(r, &["checkout", "-q", "."]);
    edit(r, "packages/a/src/a.js", "1", "3");
    let (tests, jobs) = since(r);
    assert_eq!(tests, [A]);
    assert!(jobs.iter().any(|j| j == "scoring.yml#score"), "{jobs:?}");
}

#[test]
fn a_job_that_only_builds_selects_no_tests_when_its_script_changes() {
    let d = repo();
    let r = d.path();
    // The script names `turbo run build`, and the job runs a computed task: neither
    // is evidence of a test launch.
    edit(r, "scripts/scoring/index.mjs", "* 10", "* 20");
    let (tests, jobs) = since(r);
    assert_eq!(tests, Vec::<String>::new());
    assert!(jobs.iter().any(|j| j == "scoring.yml#score"), "{jobs:?}");
    // A helper next to the script is CI tooling of that job too.
    git(r, &["checkout", "-q", "."]);
    edit(r, "scripts/scoring/threshold.mjs", "5", "6");
    let (tests, jobs) = since(r);
    assert_eq!(tests, Vec::<String>::new());
    assert!(jobs.iter().any(|j| j == "scoring.yml#score"), "{jobs:?}");
    // The job's own definition: still no tests.
    git(r, &["checkout", "-q", "."]);
    edit(
        r,
        ".github/workflows/scoring.yml",
        "runs-on: ubuntu-latest",
        "runs-on: ubuntu-24.04",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, Vec::<String>::new());
    assert_eq!(jobs, ["scoring.yml#score"]);
}

#[test]
fn a_test_runner_config_named_by_a_script_selects_that_script_s_tests() {
    let d = repo();
    let r = d.path();
    // `"test": "jest -c jest.ci.config.js"`: the tests' own discovery finds
    // jest.config.js, so only the script knows this file.
    edit(r, "packages/b/jest.ci.config.js", "ci: true", "ci: false");
    let (tests, _) = since(r);
    assert_eq!(tests, [B]);
}

#[test]
fn the_first_workflow_of_a_repository_reports_its_job() {
    // A repository without any task: no package script, no workflow. The graph
    // exists before the workflow is added, so `affected` sees it through a
    // refresh.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let write = |rel: &str, s: &str| {
        let p = r.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    };
    write(".gitignore", ".turbo/trace\n");
    write("package.json", r#"{"name":"root","private":true}"#);
    write("jest.config.js", "module.exports = {};\n");
    write("src/a.js", "exports.a = 1;\n");
    write("src/a.test.js", "require('./a');\ntest('a', () => {});\n");
    git(r, &["init", "-q", "-b", "main"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "base"]);
    run(r, &["graph", "build"]);
    write(
        ".github/workflows/ci.yml",
        "on: [push]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: \
         actions/checkout@v4\n      - run: npx jest\n",
    );
    let refreshed = since(r);
    assert_eq!(
        refreshed.1,
        ["ci.yml#test"],
        "the new job is a task of the refreshed graph"
    );
    assert_eq!(
        refreshed.0,
        ["src/a.test.js"],
        "and it runs the repository's jest tests"
    );
    // The same answer from a graph built after the change.
    std::fs::remove_dir_all(r.join(".turbo/trace")).unwrap();
    assert_eq!(since(r), refreshed);
    // The last workflow goes away again: no job, no test.
    std::fs::remove_dir_all(r.join(".github")).unwrap();
    assert_eq!(since(r), (vec![], vec![]));
}

/// A monorepo whose package tests run through a first-party jest wrapper, and
/// whose storage package provisions a database first (`dependsOn` in its own
/// turbo config). The provisioning script names jest in a comment only.
fn provisioned(r: &Path) {
    let write = |rel: &str, s: &str| {
        let p = r.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    };
    write(".gitignore", ".turbo/trace\n");
    write(
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*","scripts/*","tools/*"],"devDependencies":{"turbo":"2.0.0","jest":"29.0.0","tsx":"4.0.0"}}"#,
    );
    write("turbo.json", r#"{"tasks":{"test":{},"dynamo":{}}}"#);
    write(
        "jest.config.js",
        "module.exports = { testMatch: ['**/*.test.js'] };\n",
    );
    // The wrapper every package runs its tests with.
    write(
        "tools/test-dd/package.json",
        r#"{"name":"@acme/test-dd","version":"1.0.0","bin":{"jest-dd":"./bin/jest-dd.js"}}"#,
    );
    write(
        "tools/test-dd/bin/jest-dd.js",
        "#!/usr/bin/env node\nconst { runTestRunner } = require('../utils');\n\nrunTestRunner({ \
         runner: 'jest', session: 'unit-tests-jest' });\n",
    );
    write(
        "tools/test-dd/utils.js",
        "const { spawn } = require('child_process');\n\nfunction runTestRunner({ runner, session \
         }) {\n  const env = { ...process.env, SESSION: session };\n  const child = spawn('pnpm', \
         ['exec', runner, ...process.argv.slice(2)], { stdio: 'inherit', env });\n  \
         child.on('exit', (code) => process.exit(code ?? 1));\n}\n\nmodule.exports = { \
         runTestRunner };\n",
    );
    // The provisioning script: starts a database container and waits for it.
    write(
        "scripts/provision/package.json",
        r#"{"name":"@acme/provision","version":"1.0.0","scripts":{"dynamo":"tsx ./services/dynamo.ts"}}"#,
    );
    write(
        "scripts/provision/services/dynamo.ts",
        "import { execFileSync } from 'node:child_process';\n\nexport function start(): void {\n  \
         // Tables are not created here: that happens in\n  // the shared jest beforeAll table \
         setup. Probing the port gates readiness.\n  execFileSync('docker', ['compose', 'up', \
         '-d', 'dynamo']);\n}\n\nstart();\n",
    );
    // The storage package: its tests need the database.
    write(
        "packages/store/package.json",
        r#"{"name":"@acme/store","version":"1.0.0","scripts":{"test":"jest-dd"}}"#,
    );
    write(
        "packages/store/turbo.json",
        r#"{"extends":["//"],"tasks":{"test":{"dependsOn":["@acme/provision#dynamo"]}}}"#,
    );
    write("packages/store/src/store.js", "exports.get = () => 1;\n");
    write(
        "packages/store/src/store.test.js",
        "const { get } = require('./store');\ntest('get', () => expect(get()).toBe(1));\n",
    );
    // A package without that dependency.
    write(
        "packages/other/package.json",
        r#"{"name":"@acme/other","version":"1.0.0","scripts":{"test":"jest-dd"}}"#,
    );
    write("packages/other/src/other.js", "exports.x = 1;\n");
    write(
        "packages/other/src/other.test.js",
        "const { x } = require('./other');\ntest('x', () => expect(x).toBe(1));\n",
    );
    write(
        ".github/workflows/ci.yml",
        "on: [push]\njobs:\n  store:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: \
         actions/checkout@v4\n      - run: pnpm turbo run test --filter=@acme/store\n",
    );
    git(r, &["init", "-q", "-b", "main"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "base"]);
}

const STORE: &str = "packages/store/src/store.test.js";
const OTHER: &str = "packages/other/src/other.test.js";
/// A package's own jest config, as added to the storage package.
const SPREAD_CONFIG: &str = "const rootConfig = \
                             require('../../jest.config.js');\n\nmodule.exports = {\n  \
                             ...rootConfig,\n};\n";

#[test]
fn a_changed_script_selects_its_own_tests_not_those_of_its_dependencies() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    provisioned(r);
    run(r, &["graph", "build"]);
    // The package gets a jest config of its own (it spreads the root one): the
    // definition of its test script changed. The script depends on the
    // provisioning task; what that task runs is not what the script runs.
    std::fs::write(r.join("packages/store/jest.config.js"), SPREAD_CONFIG).unwrap();
    let (tests, _) = since(r);
    assert_eq!(tests, [STORE]);
    // The same change in a package without the dependency.
    std::fs::remove_file(r.join("packages/store/jest.config.js")).unwrap();
    std::fs::write(r.join("packages/other/jest.config.js"), SPREAD_CONFIG).unwrap();
    let (tests, _) = since(r);
    assert_eq!(tests, [OTHER]);
}

#[test]
fn a_comment_that_names_a_test_runner_starts_no_tests() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    provisioned(r);
    // The provisioning script is the definition of its task; its text names jest in
    // a comment, which is not a launch.
    edit(
        r,
        "scripts/provision/services/dynamo.ts",
        "'up', '-d'",
        "'up', '--wait', '-d'",
    );
    let (tests, _) = since(r);
    assert_eq!(tests, Vec::<String>::new());
    // With the launch in code, the script starts jest and its edit selects the
    // tests.
    git(r, &["checkout", "-q", "."]);
    edit(
        r,
        "scripts/provision/services/dynamo.ts",
        "start();",
        "start();\nexecFileSync('jest', ['--ci']);",
    );
    git(r, &["commit", "-qam", "the script runs jest"]);
    edit(
        r,
        "scripts/provision/services/dynamo.ts",
        "'--ci'",
        "'--ci', '--bail'",
    );
    let (tests, _) = since(r);
    assert_eq!(tests, [OTHER, STORE]);
}

#[test]
fn a_changed_job_still_selects_what_its_tasks_depend_on() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    provisioned(r);
    // The storage tests also wait for another package's tests (a contrived
    // dependency, to have a dependency task that runs tests).
    edit(
        r,
        "packages/store/turbo.json",
        r#""@acme/provision#dynamo""#,
        r#""@acme/provision#dynamo", "@acme/other#test""#,
    );
    git(r, &["commit", "-qam", "store tests depend on other tests"]);
    // The job's environment is that of everything turbo starts for it.
    edit(
        r,
        ".github/workflows/ci.yml",
        "runs-on: ubuntu-latest",
        "runs-on: ubuntu-24.04",
    );
    let (tests, jobs) = since(r);
    assert_eq!(tests, [OTHER, STORE]);
    assert_eq!(jobs, ["ci.yml#store"]);
    // The script's own definition: its tests only.
    git(r, &["checkout", "-q", "."]);
    std::fs::write(r.join("packages/store/jest.config.js"), SPREAD_CONFIG).unwrap();
    let (tests, _) = since(r);
    assert_eq!(tests, [STORE]);
}
