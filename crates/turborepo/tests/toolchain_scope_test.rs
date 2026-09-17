//! Regression coverage for lazy native toolchain discovery.
//!
//! The repository graph is built from subprocess-free scope inventories:
//! JavaScript is authoritative from the start, and each native toolchain
//! (Go, Rust, Python) contributes only its scope identities and manifests
//! until a run actually consults it. The contract under test:
//!
//! - Narrow, explicit JavaScript selections — include filters by name,
//!   directory, or glob, `package#task` arguments, `--only`, and exclusions
//!   that remove every native scope — never load a native owner, so no native
//!   subprocess runs, even when the native graph holds version-sensitive
//!   replacements or build-tag-dependent task catalogues.
//! - A queried native scope loads its owner's authoritative metadata — tasks,
//!   edges, contracts — before the final selection, even when no native task
//!   ultimately runs.
//! - Unfiltered runs, `--affected`, and other graph-wide queries may load every
//!   contributor to preserve exact native semantics.
//! - A JavaScript task's dependency on a native scope demands that scope's
//!   owner; there are no per-toolchain filter special cases.
//! - Failures are ordinary toolchain errors. There is no planning-refusal or
//!   reconciliation framework.
//!
//! On Unix, spy shims prepended to the child `PATH` make any consultation
//! observable.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{collections::BTreeSet, fs, path::Path};

use common::setup;

#[cfg(unix)]
const SPY_LOG_ENV: &str = "TOOLCHAIN_SCOPE_SPY_LOG";

fn write_file(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
}

fn write_js_sources(dir: &Path) {
    write_file(
        dir,
        "packages/js-lib/package.json",
        r#"{
  "name": "js-lib",
  "version": "0.0.0",
  "private": true,
  "scripts": {
    "build": "node -e \"require('fs').mkdirSync('dist',{recursive:true})\"",
    "js-only": "node -e \"process.stdout.write('js-only')\""
  }
}
"#,
    );
    write_file(
        dir,
        "packages/js-app/package.json",
        r#"{
  "name": "js-app",
  "version": "0.0.0",
  "private": true,
  "dependencies": {
    "js-lib": "0.0.0"
  },
  "scripts": {
    "build": "node -e \"require('fs').mkdirSync('dist',{recursive:true})\"",
    "js-only": "node -e \"process.stdout.write('js-only')\""
  }
}
"#,
    );
}

fn write_rust_sources(dir: &Path) {
    // Cargo package names come from each manifest: `rust-app` and `rust-lib`.
    // `rust-app/src/main.rs` references the crate `rust-lib` as `rust_lib`.
    write_file(
        dir,
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.metadata]
name = "toolchain-scope-rust"
"#,
    );
    write_file(
        dir,
        "Cargo.lock",
        r#"version = 4

[[package]]
name = "rust-app"
version = "0.1.0"
dependencies = [
 "rust-lib",
]

[[package]]
name = "rust-lib"
version = "0.1.0"
"#,
    );
    write_file(
        dir,
        "crates/rust-app/Cargo.toml",
        r#"[package]
name = "rust-app"
version = "0.1.0"
edition = "2021"

[dependencies]
rust-lib = { path = "../rust-lib" }
"#,
    );
    write_file(
        dir,
        "crates/rust-app/src/main.rs",
        "fn main() {\n    let _ = rust_lib::value();\n}\n",
    );
    write_file(
        dir,
        "crates/rust-lib/Cargo.toml",
        r#"[package]
name = "rust-lib"
version = "0.1.0"
edition = "2021"
"#,
    );
    write_file(
        dir,
        "crates/rust-lib/src/lib.rs",
        "pub fn value() -> u32 {\n    1\n}\n",
    );
}

#[cfg(unix)]
fn write_go_sources(dir: &Path) {
    write_file(
        dir,
        "go.work",
        "go 1.22\n\nuse (\n\t./apps/go-api\n\t./packages/go-lib\n)\n",
    );
    write_file(
        dir,
        "apps/go-api/go.mod",
        r#"module example.com/api

go 1.22

require example.com/lib v0.0.0

replace example.com/lib => ../../packages/go-lib
"#,
    );
    write_file(
        dir,
        "apps/go-api/main.go",
        "package main\n\nimport \"example.com/lib\"\n\nfunc main() {\n\t_ = lib.Value()\n}\n",
    );
    write_file(
        dir,
        "packages/go-lib/go.mod",
        "module example.com/lib\n\ngo 1.22\n",
    );
    write_file(
        dir,
        "packages/go-lib/lib.go",
        "package lib\n\nfunc Value() int {\n\treturn 1\n}\n",
    );
}

#[cfg(unix)]
fn write_python_sources(dir: &Path) {
    write_file(
        dir,
        "pyproject.toml",
        r#"[tool.turbo]
name = "toolchain-scope-python"

[tool.uv.workspace]
members = ["packages/py-app", "packages/py-lib"]
"#,
    );
    write_file(
        dir,
        "packages/py-app/pyproject.toml",
        r#"[project]
name = "py-app"
version = "0.1.0"
requires-python = ">=3.9"
dependencies = ["py-lib"]

[tool.uv.sources]
py-lib = { workspace = true }

[build-system]
requires = ["uv_build>=0.8,<2"]
build-backend = "uv_build"
"#,
    );
    write_file(dir, "packages/py-app/src/py_app/__init__.py", "VALUE = 1\n");
    write_file(
        dir,
        "packages/py-lib/pyproject.toml",
        r#"[project]
name = "py-lib"
version = "0.1.0"
requires-python = ">=3.9"

[build-system]
requires = ["uv_build>=0.8,<2"]
build-backend = "uv_build"
"#,
    );
    write_file(dir, "packages/py-lib/src/py_lib/__init__.py", "VALUE = 1\n");
}

#[cfg(unix)]
fn base_tasks() -> serde_json::Value {
    serde_json::json!({
        "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
        "js-only": {}
    })
}

/// Render `turbo.json` with all four toolchains enabled and the given tasks.
#[cfg(unix)]
fn mixed_turbo_json(tasks: serde_json::Value) -> String {
    let value = serde_json::json!({
        "$schema": "https://turborepo.dev/schema.json",
        "futureFlags": {
            "experimentalCargoWorkspaces": true,
            "experimentalPythonWorkspaces": true,
            "experimentalGoWorkspaces": true,
            "experimentalTaskCommand": true
        },
        "tasks": tasks
    });
    format!("{}\n", serde_json::to_string_pretty(&value).unwrap())
}

/// A repository registering all four toolchains, so any consultation of an
/// unselected native toolchain is observable.
#[cfg(unix)]
fn write_mixed_workspace(dir: &Path, tasks: serde_json::Value) {
    write_file(dir, ".gitignore", ".turbo\nnode_modules\ndist\n");
    write_file(
        dir,
        "package.json",
        r#"{
  "name": "toolchain-scope",
  "private": true,
  "packageManager": "npm@10.5.0",
  "workspaces": ["packages/js-app", "packages/js-lib"]
}
"#,
    );
    write_file(dir, "turbo.json", &mixed_turbo_json(tasks));
    write_js_sources(dir);
    write_rust_sources(dir);
    write_go_sources(dir);
    write_python_sources(dir);
}

/// JavaScript plus Rust only, so a non-run command needs neither Go nor uv.
fn write_rust_workspace(dir: &Path) {
    write_file(dir, ".gitignore", ".turbo\nnode_modules\ndist\n");
    write_file(
        dir,
        "package.json",
        r#"{
  "name": "toolchain-scope-rust-only",
  "private": true,
  "packageManager": "npm@10.5.0",
  "workspaces": ["packages/js-app", "packages/js-lib"]
}
"#,
    );
    write_file(
        dir,
        "turbo.json",
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": {
    "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] }
  }
}
"#,
    );
    write_js_sources(dir);
    write_rust_sources(dir);
}

/// Fixture for the broad-query contract: a JavaScript package with a `dev`
/// script and a Go workspace whose only module is a library (no `main`
/// package, so no runnable `dev` target). An unfiltered `dev` query selects
/// the Go scopes too — the library module and the `go-workspace` aggregate —
/// so the Go owner is loaded even though no Go task can run.
#[cfg(unix)]
fn write_js_dev_go_library_workspace(dir: &Path) {
    write_file(dir, ".gitignore", ".turbo\nnode_modules\ndist\n");
    write_file(
        dir,
        "package.json",
        r#"{
  "name": "toolchain-scope-dev",
  "private": true,
  "packageManager": "npm@10.5.0",
  "workspaces": ["packages/js-dev"]
}
"#,
    );
    write_file(
        dir,
        "turbo.json",
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalGoWorkspaces": true },
  "tasks": {
    "dev": { "cache": false, "persistent": true }
  }
}
"#,
    );
    write_file(
        dir,
        "packages/js-dev/package.json",
        r#"{
  "name": "js-dev",
  "version": "0.0.0",
  "private": true,
  "scripts": {
    "dev": "node -e \"process.stdout.write('dev')\""
  }
}
"#,
    );
    write_file(dir, "go.work", "go 1.22\n\nuse ./packages/go-lib\n");
    write_file(
        dir,
        "packages/go-lib/go.mod",
        "module example.com/lib\n\ngo 1.22\n",
    );
    write_file(
        dir,
        "packages/go-lib/lib.go",
        "package lib\n\nfunc Value() int {\n\treturn 1\n}\n",
    );
}

/// A JavaScript workspace beside a Go workspace whose `example.com/api`
/// requires `example.com/alias` behind version-specific local replacements
/// while an active remote requirement (`example.com/remote`) could raise the
/// selected version past `v1.0.0` and flip the replacement target from
/// `go-lib` to `go-extra`. Which replacement wins is a fact only `go` can
/// decide. A JavaScript-only filter provably never consults the Go scopes,
/// so the Go owner is never loaded and the replacements cannot matter to
/// the run — the lazy-native counterpart of
/// [`write_mixed_workspace`]'s closed replacement graph, where querying the
/// Go scope does load the owner.
#[cfg(unix)]
fn write_js_go_version_sensitive_replacement_workspace(dir: &Path) {
    write_file(dir, ".gitignore", ".turbo\nnode_modules\ndist\n");
    write_file(
        dir,
        "package.json",
        r#"{
  "name": "toolchain-scope-version-alias",
  "private": true,
  "packageManager": "npm@10.5.0",
  "workspaces": ["packages/js-app", "packages/js-lib"]
}
"#,
    );
    write_file(
        dir,
        "turbo.json",
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] }
  }
}
"#,
    );
    write_js_sources(dir);
    write_file(
        dir,
        "go.work",
        "go 1.22\n\nuse (\n\t./apps/go-api\n\t./packages/go-lib\n\t./packages/go-extra\n)\n",
    );
    write_file(
        dir,
        "apps/go-api/go.mod",
        r#"module example.com/api

go 1.22

require (
	example.com/alias v1.0.0
	example.com/remote v1.0.0
)

replace (
	example.com/alias v1.0.0 => ../../packages/go-lib
	example.com/alias v1.2.0 => ../../packages/go-extra
)
"#,
    );
    write_file(
        dir,
        "apps/go-api/main.go",
        "package main\n\nfunc main() {}\n",
    );
    write_file(
        dir,
        "packages/go-lib/go.mod",
        "module example.com/lib\n\ngo 1.22\n",
    );
    write_file(
        dir,
        "packages/go-lib/lib.go",
        "package lib\n\nfunc Value() int {\n\treturn 1\n}\n",
    );
    write_file(
        dir,
        "packages/go-extra/go.mod",
        "module example.com/extra\n\ngo 1.22\n",
    );
    write_file(
        dir,
        "packages/go-extra/extra.go",
        "package extra\n\nfunc Value() int {\n\treturn 2\n}\n",
    );
}

/// A JavaScript `dev` script beside a Go module whose only `main` package
/// sits behind a build constraint (`//go:build integration` on
/// `cmd/service/main.go`), so whether the module has a runnable `dev` target
/// is a build-tag fact only `go` can resolve: the constraint never holds
/// under default tags, so a real `go` classifies the module as a library on
/// every host. Lazy discovery resolves that fact by loading the Go owner
/// whenever the module's scope is queried — and never needs to when a narrow
/// JavaScript selection provably never consults it.
///
/// `filter_using_tasks` toggles `futureFlags.filterUsingTasks`, which resolves
/// `--filter` at the task level instead of the package level.
/// `js_dev_depends_on_go_dev` adds a cross-language task dependency
/// (`js-dev#dev` → `api#dev`) to `turbo.json`, pointing at the
/// build-tag-dependent native `dev` task. The `dev` task is deliberately
/// non-persistent so that dependency is legal configuration (persistent
/// tasks cannot be depended on); every test using this fixture only
/// dry-runs, so the non-persistent `dev` semantics are irrelevant.
#[cfg(unix)]
fn write_js_dev_go_platform_constrained_workspace(
    dir: &Path,
    filter_using_tasks: bool,
    js_dev_depends_on_go_dev: bool,
) {
    write_file(dir, ".gitignore", ".turbo\nnode_modules\ndist\n");
    write_file(
        dir,
        "package.json",
        r#"{
  "name": "toolchain-scope-dev-constrained",
  "private": true,
  "packageManager": "npm@10.5.0",
  "workspaces": ["packages/js-dev"]
}
"#,
    );
    let mut future_flags = serde_json::json!({
        "experimentalGoWorkspaces": true,
        "experimentalTaskCommand": true
    });
    if filter_using_tasks {
        future_flags["filterUsingTasks"] = serde_json::Value::Bool(true);
    }
    let mut tasks = serde_json::json!({
        "build": {},
        "dev": { "cache": false }
    });
    if js_dev_depends_on_go_dev {
        tasks["js-dev#dev"] = serde_json::json!({ "dependsOn": ["api#dev"] });
    }
    let turbo_json = serde_json::json!({
        "$schema": "https://turborepo.dev/schema.json",
        "futureFlags": future_flags,
        "tasks": tasks
    });
    write_file(
        dir,
        "turbo.json",
        &format!("{}\n", serde_json::to_string_pretty(&turbo_json).unwrap()),
    );
    write_file(
        dir,
        "packages/js-dev/package.json",
        r#"{
  "name": "js-dev",
  "version": "0.0.0",
  "private": true,
  "scripts": {
    "dev": "node -e \"process.stdout.write('dev')\""
  }
}
"#,
    );
    write_file(dir, "go.work", "go 1.22\n\nuse ./packages/go-api\n");
    write_file(
        dir,
        "packages/go-api/go.mod",
        "module example.com/api\n\ngo 1.22\n",
    );
    write_file(
        dir,
        "packages/go-api/lib.go",
        "package lib\n\nfunc Value() int {\n\treturn 1\n}\n",
    );
    write_file(
        dir,
        "packages/go-api/cmd/service/main.go",
        "//go:build integration\n\npackage main\n\nfunc main() {}\n",
    );
}

#[cfg(unix)]
fn setup_mixed_workspace(dir: &Path, tasks: serde_json::Value) {
    write_mixed_workspace(dir, tasks);
    setup::setup_git(dir).unwrap();
}

fn assert_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn task_ids(summary: &serde_json::Value) -> BTreeSet<String> {
    summary["tasks"]
        .as_array()
        .expect("dry-run summary has a tasks array")
        .iter()
        .map(|task| task["taskId"].as_str().expect("taskId").to_string())
        .collect()
}

fn task_hash(summary: &serde_json::Value, task_id: &str) -> String {
    summary["tasks"]
        .as_array()
        .expect("dry-run summary has a tasks array")
        .iter()
        .find(|task| task["taskId"] == task_id)
        .unwrap_or_else(|| panic!("{task_id} missing from dry-run summary"))["hash"]
        .as_str()
        .expect("task hash")
        .to_string()
}

/// Assert the dry-run summary's exact task set, printing the summary (the
/// run's stdout) so a mismatch shows what turbo actually selected.
#[cfg(unix)]
fn assert_task_ids(summary: &serde_json::Value, expected: &BTreeSet<String>, context: &str) {
    let actual = task_ids(summary);
    assert_eq!(
        &actual,
        expected,
        "{context}: unexpected task set\nstdout:\n{}",
        serde_json::to_string_pretty(summary).expect("serialize dry-run summary")
    );
}

/// One task's JSON object from a dry-run summary, printing the summary when
/// the task is missing.
fn dry_run_task<'a>(summary: &'a serde_json::Value, task_id: &str) -> &'a serde_json::Value {
    summary["tasks"]
        .as_array()
        .expect("dry-run summary has a tasks array")
        .iter()
        .find(|task| task["taskId"] == task_id)
        .unwrap_or_else(|| {
            panic!(
                "{task_id} missing from dry-run summary\nstdout:\n{}",
                serde_json::to_string_pretty(summary).expect("serialize dry-run summary")
            )
        })
}

#[cfg(unix)]
mod spy {
    use std::{
        ffi::OsString,
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use super::SPY_LOG_ENV;

    /// Native toolchains only; used where the run must actually execute and
    /// therefore needs real `node`/`npm`.
    pub(super) const NATIVE_TOOLS: &[&str] = &["cargo", "rustc", "go", "uv", "python", "python3"];
    pub(super) const CARGO: &[&str] = &["cargo", "rustc"];
    pub(super) const GO: &[&str] = &["go"];
    pub(super) const PYTHON: &[&str] = &["uv", "python", "python3"];
    /// Every tool the mixed repository could plausibly consult.
    pub(super) fn all_tools() -> Vec<&'static str> {
        let mut tools = NATIVE_TOOLS.to_vec();
        tools.extend_from_slice(&["npm", "npx", "node", "yarn", "pnpm", "bun"]);
        tools
    }

    /// Executable shims that record every invocation and exit non-zero, so an
    /// unexpected consultation fails loudly instead of silently succeeding.
    /// Each shim appends to the file named by [`SPY_LOG_ENV`] — the variable,
    /// not a literal path — so the log lands in the spy directory regardless
    /// of the child's working directory.
    pub(super) struct ToolchainSpy {
        dir: tempfile::TempDir,
        log: PathBuf,
    }

    impl ToolchainSpy {
        pub(super) fn new(tools: &[&str]) -> Self {
            let dir = tempfile::tempdir().expect("spy directory");
            let log = dir.path().join("invocations.log");
            for tool in tools {
                let script = dir.path().join(tool);
                fs::write(
                    &script,
                    format!(
                        "#!/bin/sh\nprintf '%s\\n' \"$0 $*\" >> \"${}\"\nexit 1\n",
                        SPY_LOG_ENV
                    ),
                )
                .expect("write spy script");
                fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
                    .expect("make spy executable");
            }
            Self { dir, log }
        }

        pub(super) fn path_env(&self) -> OsString {
            let current = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![self.dir.path().to_path_buf()];
            paths.extend(std::env::split_paths(&current));
            std::env::join_paths(paths).expect("PATH can include the spy directory")
        }

        pub(super) fn log_path(&self) -> &Path {
            &self.log
        }

        pub(super) fn invocations(&self) -> Vec<String> {
            match fs::read_to_string(&self.log) {
                Ok(contents) => contents.lines().map(str::to_string).collect(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(error) => panic!("failed to read spy log: {error}"),
            }
        }

        pub(super) fn assert_no_invocations(&self, output: &std::process::Output, context: &str) {
            let invocations = self.invocations();
            assert!(
                invocations.is_empty(),
                "{context}: expected no toolchain subprocesses, but saw \
                 {invocations:?}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        pub(super) fn assert_invoked(
            &self,
            tool: &str,
            output: &std::process::Output,
            context: &str,
        ) {
            let invocations = self.invocations();
            assert!(
                invocations.iter().any(|line| line.contains(tool)),
                "{context}: expected the selected toolchain to be invoked via {tool:?}, but saw \
                 {invocations:?}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[cfg(unix)]
use spy::ToolchainSpy;

#[cfg(unix)]
fn spy_run(dir: &Path, args: &[&str], spy: &ToolchainSpy) -> std::process::Output {
    let path = spy.path_env();
    let path = path.to_string_lossy();
    let log = spy.log_path().to_string_lossy();
    common::run_turbo_with_env(
        dir,
        args,
        &[("PATH", path.as_ref()), (SPY_LOG_ENV, log.as_ref())],
    )
}

#[cfg(unix)]
fn spy_dry_run(
    dir: &Path,
    args: &[&str],
    spy: &ToolchainSpy,
) -> (serde_json::Value, std::process::Output) {
    let mut full_args = vec!["run"];
    full_args.extend_from_slice(args);
    full_args.push("--dry-run=json");
    let output = spy_run(dir, &full_args, spy);
    assert_success(&output, &format!("spied dry run {args:?}"));
    let summary = serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    (summary, output)
}

// ---------------------------------------------------------------------------
// Narrow JavaScript selections never load a native owner
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn js_filter_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let (summary, output) = spy_dry_run(tempdir.path(), &["build", "--filter=js-app"], &spy);
    // `js-app` depends on `js-lib`, so `^build` pulls js-lib#build; both stay JS.
    assert_task_ids(
        &summary,
        &BTreeSet::from(["js-app#build".to_string(), "js-lib#build".to_string()]),
        "JavaScript-only --filter",
    );
    spy.assert_no_invocations(&output, "JavaScript-only --filter");
}

/// A Go scope named `chalk` must not turn npm's `chalk: "*"` into a workspace
/// dependency. Name overlap must neither load Go nor remove npm's resolution
/// from the JavaScript task hash.
#[cfg(unix)]
#[test]
fn external_npm_dependency_matching_go_name_stays_external() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    write_file(root, ".gitignore", ".turbo\nnode_modules\ndist\n");
    write_file(
        root,
        "package.json",
        r#"{
          "name": "external-npm-with-go",
          "private": true,
          "packageManager": "npm@10.5.0",
          "workspaces": ["packages/js-app", "packages/js-lib"]
        }"#,
    );
    write_file(
        root,
        "turbo.json",
        r#"{
          "futureFlags": { "experimentalGoWorkspaces": true },
          "tasks": { "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] } }
        }"#,
    );
    write_js_sources(root);
    let app_manifest_path = root.join("packages/js-app/package.json");
    let mut app_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&app_manifest_path).unwrap()).unwrap();
    app_manifest["dependencies"]["chalk"] = serde_json::json!("*");
    fs::write(
        app_manifest_path,
        serde_json::to_vec_pretty(&app_manifest).unwrap(),
    )
    .unwrap();
    write_file(root, "go.work", "go 1.22\n\nuse ./tools/go-chalk\n");
    write_file(
        root,
        "tools/go-chalk/go.mod",
        "module example.com/other-chalk\n\ngo 1.22\n",
    );
    write_file(root, "tools/go-chalk/chalk.go", "package chalk\n");

    // Hand-authored npm v3 lockfile: no installation or registry access is needed.
    // The spy also blocks npm/node, proving discovery and hashing are
    // subprocess-free.
    let mut lockfile = serde_json::json!({
        "name": "external-npm-with-go",
        "lockfileVersion": 3,
        "requires": true,
        "packages": {
            "": {
                "name": "external-npm-with-go",
                "workspaces": ["packages/js-app", "packages/js-lib"]
            },
            "packages/js-app": {
                "version": "0.0.0",
                "dependencies": { "js-lib": "0.0.0", "chalk": "*" }
            },
            "packages/js-lib": { "version": "0.0.0" },
            "node_modules/js-app": { "resolved": "packages/js-app", "link": true },
            "node_modules/js-lib": { "resolved": "packages/js-lib", "link": true },
            "node_modules/chalk": {
                "version": "5.3.0",
                "resolved": "https://registry.npmjs.org/chalk/-/chalk-5.3.0.tgz"
            }
        }
    });
    let lockfile_path = root.join("package-lock.json");
    fs::write(
        &lockfile_path,
        serde_json::to_vec_pretty(&lockfile).unwrap(),
    )
    .unwrap();
    setup::setup_git(root).unwrap();
    let spy = ToolchainSpy::new(&spy::all_tools());
    let run = |context: &str| {
        let output = spy_run(
            root,
            &["run", "build", "--filter=js-app", "--dry-run=json"],
            &spy,
        );
        spy.assert_no_invocations(&output, context);
        assert_success(&output, context);
        let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_task_ids(
            &summary,
            &BTreeSet::from(["js-app#build".to_string(), "js-lib#build".to_string()]),
            context,
        );
        assert_eq!(
            dry_run_task(&summary, "js-app#build")["dependencies"],
            serde_json::json!(["js-lib#build"]),
            "{context}: npm chalk must not become an internal Go dependency"
        );
        summary
    };
    let baseline = run("npm chalk with a differently named Go scope");

    // Only Go metadata changes: the npm dependency and its resolution stay
    // identical.
    write_file(
        root,
        "tools/go-chalk/go.mod",
        "module example.com/chalk\n\ngo 1.22\n",
    );
    let overlapping = run("npm chalk with a Go scope also named chalk");
    assert_eq!(
        task_hash(&baseline, "js-app#build"),
        task_hash(&overlapping, "js-app#build")
    );
    let external_hash = |summary: &serde_json::Value| {
        dry_run_task(summary, "js-app#build")["hashOfExternalDependencies"]
            .as_str()
            .expect("JavaScript external dependency hash")
            .to_string()
    };
    assert_eq!(external_hash(&baseline), external_hash(&overlapping));

    // Only the external lockfile row changes; neither package.json nor Go changes.
    lockfile["packages"]["node_modules/chalk"]["version"] = serde_json::json!("5.4.1");
    lockfile["packages"]["node_modules/chalk"]["resolved"] =
        serde_json::json!("https://registry.npmjs.org/chalk/-/chalk-5.4.1.tgz");
    fs::write(
        &lockfile_path,
        serde_json::to_vec_pretty(&lockfile).unwrap(),
    )
    .unwrap();
    let changed = run("updated npm chalk resolution with a Go scope also named chalk");
    assert_ne!(
        external_hash(&overlapping),
        external_hash(&changed),
        "npm resolution must still be hashed"
    );
    assert_ne!(
        task_hash(&overlapping, "js-app#build"),
        task_hash(&changed, "js-app#build")
    );
    assert_eq!(
        task_hash(&overlapping, "js-lib#build"),
        task_hash(&changed, "js-lib#build"),
        "a lockfile change outside js-lib's dependency closure must not invalidate it"
    );
}

/// A `package#task` CLI argument selects exactly the referenced package's
/// task without any filter, so no native toolchain participates. This is the
/// argument form, which needs no future flag; resolving `--filter` at the
/// task level is a separate feature (`futureFlags.filterUsingTasks`), covered
/// by `task_level_js_dev_filter_ignores_a_platform_constrained_go_module` —
/// the two forms are deliberately not conflated.
#[cfg(unix)]
#[test]
fn js_package_task_argument_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let (summary, output) = spy_dry_run(tempdir.path(), &["js-lib#build"], &spy);
    assert_task_ids(
        &summary,
        &BTreeSet::from(["js-lib#build".to_string()]),
        "JavaScript pkg#task argument",
    );
    spy.assert_no_invocations(&output, "JavaScript pkg#task argument");
}

/// `--only` prunes the engine to the selected package's own tasks: without
/// `futureFlags.filterUsingTasks` the topological `^build` dependency
/// `js-lib#build` is not retained. (Task-level filtering keeps such
/// dependencies runnable — see `task_filter_package_scope_test`, which runs
/// with the flag.) The point here is that no native toolchain joins the
/// selection.
#[cfg(unix)]
#[test]
fn only_flag_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let (summary, output) = spy_dry_run(
        tempdir.path(),
        &["build", "--filter=js-app", "--only"],
        &spy,
    );
    assert_task_ids(
        &summary,
        &BTreeSet::from(["js-app#build".to_string()]),
        "--only JavaScript selection",
    );
    spy.assert_no_invocations(&output, "--only JavaScript selection");
}

#[cfg(unix)]
#[test]
fn task_filtering_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let (summary, output) =
        spy_dry_run(tempdir.path(), &["build", "--filter=./packages/js-*"], &spy);
    assert_task_ids(
        &summary,
        &BTreeSet::from(["js-app#build".to_string(), "js-lib#build".to_string()]),
        "directory glob filter",
    );
    spy.assert_no_invocations(&output, "directory glob filter");
}

/// `--parallel` removes inter-package dependencies after graph construction;
/// the lazy path must still leave native toolchains untouched.
#[cfg(unix)]
#[test]
fn parallel_js_only_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let (summary, output) = spy_dry_run(
        tempdir.path(),
        &["build", "--filter=js-app", "--parallel"],
        &spy,
    );
    let ids = task_ids(&summary);
    let combined = combined_output(&output);
    assert!(ids.contains("js-app#build"), "ids: {ids:?}\n{combined}");
    assert!(
        ids.iter().all(|id| id.starts_with("js-")),
        "no native task should be selected: {ids:?}\n{combined}"
    );
    spy.assert_no_invocations(&output, "--parallel JavaScript selection");
}

/// A `command: null` opt-out leaves the selected task without a command, so it
/// cannot make a native toolchain participate.
#[cfg(unix)]
#[test]
fn command_opt_out_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(
        tempdir.path(),
        serde_json::json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "js-only": {},
            "js-lib#build": { "command": null }
        }),
    );
    let spy = ToolchainSpy::new(&spy::all_tools());

    let (_, output) = spy_dry_run(tempdir.path(), &["build", "--filter=js-lib"], &spy);
    spy.assert_no_invocations(&output, "command opt-out");
}

/// A requested task no package can run must fail without consulting a native
/// toolchain — via the `package#task` argument form, which names its owner
/// exactly: `js-app` is a JavaScript package whose task catalogue is
/// authoritative without any subprocess, so the unknown task is answered
/// against that known owner. An unqualified task name is a repo-wide
/// catalogue question that any native scope may own, so validating it
/// demands native metadata instead; see
/// `unqualified_missing_task_fails_after_loading_real_native_metadata`.
#[cfg(unix)]
#[test]
fn missing_task_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let output = spy_run(tempdir.path(), &["run", "js-app#doesnotexist"], &spy);
    let combined = combined_output(&output);
    assert!(
        !output.status.success(),
        "unknown task must fail:\n{combined}"
    );
    assert!(
        combined.contains("Could not find task `js-app#doesnotexist` in project"),
        "expected a missing-task diagnostic:\n{combined}"
    );
    spy.assert_no_invocations(&output, "missing task");
}

/// Real execution, not just a dry run: with native shims installed but
/// `node`/`npm` real, a JavaScript-only run executes and never touches the
/// native toolchains.
#[cfg(unix)]
#[test]
fn real_js_execution_never_invokes_native_toolchains() {
    if which::which("node").is_err() {
        eprintln!("skipping: node is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(spy::NATIVE_TOOLS);

    let output = spy_run(tempdir.path(), &["run", "build", "--filter=js-app"], &spy);
    assert_success(&output, "real JavaScript build");
    assert!(
        tempdir.path().join("packages/js-app/dist").is_dir(),
        "the JavaScript build must actually execute"
    );
    spy.assert_no_invocations(&output, "real JavaScript execution");
}

/// A Go edge that only native metadata can resolve — an active remote
/// requirement could raise `example.com/alias` past `v1.0.0` and flip its
/// version-specific replacement from `go-lib` to `go-extra` — is irrelevant
/// to a JavaScript-only selection: the filter provably never consults the Go
/// scopes, so the Go owner is never loaded. The run succeeds on a cold cache
/// with real execution, and the failing `go` spy proves zero probes. Contrast
/// with `js_filter_never_invokes_native_toolchains`, whose Go graph is exact
/// once the owner is loaded.
#[cfg(unix)]
#[test]
fn js_only_filter_succeeds_with_remote_sensitive_go_replacements() {
    if which::which("node").is_err() {
        eprintln!("skipping: node is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    write_js_go_version_sensitive_replacement_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::NATIVE_TOOLS);

    let output = spy_run(tempdir.path(), &["run", "build", "--filter=js-app"], &spy);
    assert_success(
        &output,
        "JavaScript-only build over a version-sensitive Go graph",
    );
    assert!(
        tempdir.path().join("packages/js-app/dist").is_dir(),
        "the cold-cache JavaScript build must actually execute"
    );
    spy.assert_no_invocations(
        &output,
        "JavaScript-only filter with remote-sensitive replacements",
    );
}

/// A task-level filter (`futureFlags.filterUsingTasks`) that explicitly
/// selects the JavaScript package never consults the Go scope's task
/// catalogue, so the Go owner is never loaded — even though the module's
/// `dev` classification depends on build tags only `go` can resolve. The
/// `package#task` CLI argument form is covered separately by
/// `js_package_task_argument_never_invokes_native_toolchains` and needs no
/// flag; the two selection forms are deliberately not conflated. Contrast
/// `unqualified_dev_query_loads_the_go_owner`, whose unfiltered request does
/// load the Go owner.
#[cfg(unix)]
#[test]
fn task_level_js_dev_filter_ignores_a_platform_constrained_go_module() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), true, false);
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let (summary, output) = spy_dry_run(tempdir.path(), &["dev", "--filter=js-dev"], &spy);
    assert_task_ids(
        &summary,
        &BTreeSet::from(["js-dev#dev".to_string()]),
        "task-level JavaScript dev filter",
    );
    spy.assert_no_invocations(&output, "task-level JavaScript dev filter");
}

/// A `package#task` CLI argument narrows the selection to the referenced
/// package even without `--filter`, so the run provably never consults the
/// Go scopes: the Go owner is never loaded and the build-tag-dependent `dev`
/// catalogue is never asked about. The argument form needs no future flag;
/// `js_package_task_argument_never_invokes_native_toolchains` covers it
/// against the fully loaded mixed repository.
#[cfg(unix)]
#[test]
fn qualified_js_dev_argument_never_loads_the_go_owner() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), false, false);
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let (summary, output) = spy_dry_run(tempdir.path(), &["js-dev#dev"], &spy);
    assert_task_ids(
        &summary,
        &BTreeSet::from(["js-dev#dev".to_string()]),
        "qualified JavaScript dev argument",
    );
    spy.assert_no_invocations(&output, "qualified JavaScript dev argument");
}

/// An exclude-only filter is a near-repository-wide query: it keeps every
/// scope that is not excluded, including the `go-workspace` aggregate, so
/// excluding only the Go module would still select a Go scope and load its
/// owner. Excluding every Go scope — the module and the aggregate — leaves a
/// provably JavaScript-only selection, so the Go owner is never loaded and
/// the build-tag-dependent `dev` catalogue is never asked about.
#[cfg(unix)]
#[test]
fn excluding_every_go_scope_keeps_a_js_dev_run_native_free() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), false, false);
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let (summary, output) = spy_dry_run(
        tempdir.path(),
        &["dev", "--filter=!api", "--filter=!go-workspace"],
        &spy,
    );
    let ids = task_ids(&summary);
    let combined = combined_output(&output);
    assert!(
        ids.contains("js-dev#dev"),
        "the JavaScript dev task must be selected: {ids:?}\n{combined}"
    );
    assert!(
        ids.iter().all(|id| id.starts_with("js-")),
        "no Go scope may participate once every Go scope is excluded: {ids:?}\n{combined}"
    );
    spy.assert_no_invocations(&output, "exclude-only JavaScript dev filter");
}

// ---------------------------------------------------------------------------
// Native owners load on demand
// ---------------------------------------------------------------------------

/// Selecting a Rust task loads the Rust owner's authoritative metadata; the
/// failing `cargo` shim proves the toolchain was invoked rather than skipped.
#[cfg(unix)]
#[test]
fn selected_cargo_task_invokes_cargo() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(spy::CARGO);

    let output = spy_run(
        tempdir.path(),
        &["run", "build", "--filter=rust-app", "--dry-run=json"],
        &spy,
    );
    spy.assert_invoked("cargo", &output, "Rust task selection");
}

/// Selecting a Go task loads the Go owner, so the run depends on a usable
/// `go`: a failed invocation must error with the ordinary toolchain
/// diagnostic rather than silently dropping the task or refusing the plan.
/// The log assertion also rules out the run passing for the wrong reason.
#[cfg(unix)]
#[test]
fn selected_go_task_requires_a_usable_go_executable() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(
        tempdir.path(),
        &["run", "build", "--filter=api", "--dry-run=json"],
        &spy,
    );
    let combined = combined_output(&output);
    spy.assert_invoked("go", &output, "Go task selection");
    assert!(
        !output.status.success(),
        "selecting a Go task must not silently succeed without a usable `go`:\n{combined}"
    );
    assert!(
        combined.contains("`go work edit -json` failed"),
        "an unusable `go` must surface the ordinary Go toolchain error, not a planning \
         refusal:\n{combined}"
    );
}

/// Selecting a Python task loads the Python owner's authoritative metadata.
/// uv may fall back to manifest discovery when its binary fails, so the
/// assertion is on the observed invocation.
#[cfg(unix)]
#[test]
fn selected_python_task_invokes_uv() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(spy::PYTHON);

    let output = spy_run(
        tempdir.path(),
        &["run", "build", "--filter=py-app", "--dry-run=json"],
        &spy,
    );
    spy.assert_invoked("uv", &output, "Python task selection");
}

/// An explicit cross-language `dependsOn` pulls `api#build` into
/// a JavaScript-filtered run, so the Go owner is loaded even though
/// `--filter` never mentioned it. Contrast with
/// `js_filter_never_invokes_native_toolchains`.
#[cfg(unix)]
#[test]
fn cross_language_task_dependency_loads_the_native_owner() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(
        tempdir.path(),
        serde_json::json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "js-only": {},
            "js-app#build": { "dependsOn": ["^build", "api#build"] }
        }),
    );
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(
        tempdir.path(),
        &["run", "build", "--filter=js-app", "--dry-run=json"],
        &spy,
    );
    spy.assert_invoked("go", &output, "cross-language task dependency");
    assert!(
        !output.status.success(),
        "the pulled-in Go owner must be loaded even for a JavaScript-only filter:\n{}",
        combined_output(&output)
    );
}

/// When Go is actually installed, the task closure is visible in the graph:
/// the JavaScript filter still executes `api#build`.
#[cfg(unix)]
#[test]
fn cross_language_task_dependency_appears_in_the_selected_graph() {
    if which::which("go").is_err() {
        eprintln!("skipping: go is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(
        tempdir.path(),
        serde_json::json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "js-only": {},
            "js-app#build": { "dependsOn": ["^build", "api#build"] }
        }),
    );

    let output = common::run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=js-app", "--dry-run=json"],
    );
    assert_success(&output, "cross-language dry run");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let ids = task_ids(&summary);
    let combined = combined_output(&output);
    assert!(ids.contains("js-app#build"), "ids: {ids:?}\n{combined}");
    assert!(
        ids.contains("api#build"),
        "explicit task dependency must join the JavaScript selection: {ids:?}\n{combined}"
    );
}

/// The constrained module's `dev` classification depends on build tags, but
/// its `build` task does not: under default build tags the module classifies
/// as a library on every host, so the library command shape — `go build
/// ./...`, uncached, because a library produces no tracked output — holds
/// whether or not the constrained `main` package ever counts. An explicit
/// native build selection loads the Go owner and retains the real command
/// shape. Complements
/// `constrained_go_module_dev_selection_resolves_build_tags_with_go`, which
/// resolves the build-tag-dependent `dev` classification. Requires a real
/// `go`; the module declares no dependencies, so discovery never touches the
/// network.
#[cfg(unix)]
#[test]
fn constrained_go_module_build_selection_retains_the_real_command() {
    if which::which("go").is_err() {
        eprintln!("skipping: go is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), false, false);
    setup::setup_git(tempdir.path()).unwrap();

    let output = common::run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=api", "--dry-run=json"],
    );
    assert_success(&output, "explicit native build selection");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let combined = combined_output(&output);
    assert!(
        task_ids(&summary).contains("api#build"),
        "the native build task must be selected: {:?}\n{combined}",
        task_ids(&summary)
    );
    let task = dry_run_task(&summary, "api#build");
    assert_eq!(
        task["command"], "go build ./...",
        "the library build must retain its real command shape\n{combined}"
    );
    assert_eq!(
        task["resolvedTaskDefinition"]["cache"], false,
        "a library build produces no tracked output, so it stays uncached\n{combined}"
    );
}

/// Querying the constrained module's own `dev` task loads the Go owner, and
/// the owner resolves build tags with the real `go` command: under default
/// tags the `//go:build integration` constraint never holds, so the module
/// classifies as a library and its `dev` stays commandless rather than
/// guessing a runnable target. Complements
/// `constrained_go_module_build_selection_retains_the_real_command`, which
/// keeps the module's real library `build` command. Requires a real `go`;
/// the module declares no dependencies, so discovery never touches the
/// network.
#[cfg(unix)]
#[test]
fn constrained_go_module_dev_selection_resolves_build_tags_with_go() {
    if which::which("go").is_err() {
        eprintln!("skipping: go is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), false, false);
    setup::setup_git(tempdir.path()).unwrap();

    let output = common::run_turbo(
        tempdir.path(),
        &["run", "dev", "--filter=api", "--dry-run=json"],
    );
    assert_success(
        &output,
        "native dev selection over a build-tag-constrained module",
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let combined = combined_output(&output);
    let dev = dry_run_task(&summary, "api#dev");
    assert_eq!(
        dev["command"], "<NONEXISTENT>",
        "actual `go` must resolve the build tags: the constrained main never counts under default \
         tags, so the module's dev stays commandless\n{combined}"
    );
}

/// Querying the constrained module's `dev` catalogue loads the Go owner, so
/// the run depends on a usable `go`: the failing spy proves the owner was
/// invoked, and the failure is the ordinary Go toolchain error — never a
/// planning refusal. With a real `go` the same query resolves the build tags
/// and succeeds; see
/// `constrained_go_module_dev_selection_resolves_build_tags_with_go`.
#[cfg(unix)]
#[test]
fn constrained_go_module_dev_selection_demands_a_usable_go_executable() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), false, false);
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(
        tempdir.path(),
        &["run", "dev", "--filter=api", "--dry-run=json"],
        &spy,
    );
    let combined = combined_output(&output);
    spy.assert_invoked("go", &output, "native dev selection");
    assert!(
        !output.status.success(),
        "a dev selection whose owner cannot run must fail:\n{combined}"
    );
    assert!(
        combined.contains("`go work edit -json` failed"),
        "the failure must be the ordinary Go toolchain error, not a planning refusal:\n{combined}"
    );
}

/// A cross-language `dependsOn` on the constrained module's `dev` task must
/// demand the Go owner even for a task-level JavaScript-only filter
/// (`futureFlags.filterUsingTasks`): the dependency edge reaches a Go scope,
/// whose task catalogue only the owner can answer, so `go` is loaded before
/// the final selection. The failing spy proves the demand; the failure is the
/// ordinary toolchain error. Contrast
/// `task_level_js_dev_filter_ignores_a_platform_constrained_go_module`: the
/// same command without the dependency never loads Go.
#[cfg(unix)]
#[test]
fn cross_language_dev_dependency_loads_the_go_owner() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), true, true);
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(
        tempdir.path(),
        &["run", "dev", "--filter=js-dev", "--dry-run=json"],
        &spy,
    );
    let combined = combined_output(&output);
    spy.assert_invoked("go", &output, "cross-language dev dependency");
    assert!(
        !output.status.success(),
        "the demanded Go owner must be loaded even for a JavaScript-only filter:\n{combined}"
    );
    assert!(
        combined.contains("`go work edit -json` failed"),
        "the failure must be the ordinary Go toolchain error, not a planning refusal:\n{combined}"
    );
}

// ---------------------------------------------------------------------------
// Broad queries may load every contributor
// ---------------------------------------------------------------------------

/// An unqualified `dev` request is a repository-wide query: it selects every
/// scope, including the library-only Go module and the `go-workspace`
/// aggregate, so the Go owner is loaded to preserve exact native semantics —
/// even though no Go task can run, because a library has no runnable `dev`
/// target. The failing spy proves the load; the failure is the ordinary
/// toolchain error.
#[cfg(unix)]
#[test]
fn unqualified_dev_query_loads_the_go_owner() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_library_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(tempdir.path(), &["run", "dev", "--dry-run=json"], &spy);
    let combined = combined_output(&output);
    spy.assert_invoked("go", &output, "unqualified dev query");
    assert!(
        !output.status.success(),
        "an unqualified query whose contributor cannot run must fail:\n{combined}"
    );
    assert!(
        combined.contains("`go work edit -json` failed"),
        "the failure must be the ordinary Go toolchain error, not a planning refusal:\n{combined}"
    );
}

/// `js-only` is declared for every package but only JavaScript packages have
/// a command for it, so no native task would run — yet the request is
/// unqualified, a repository-wide query that may load every contributor to
/// preserve exact native semantics. Native metadata may therefore load even
/// though no native task ultimately participates. Owner loading is unordered,
/// so with failing spies the first loaded contributor aborts the run; at
/// least one native consultation must be observed.
#[cfg(unix)]
#[test]
fn unqualified_task_query_may_load_every_native_contributor() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(spy::NATIVE_TOOLS);

    let output = spy_run(tempdir.path(), &["run", "js-only", "--dry-run=json"], &spy);
    let combined = combined_output(&output);
    let invocations = spy.invocations();
    assert!(
        !invocations.is_empty(),
        "an unqualified query may load every native contributor, so a native toolchain must have \
         been consulted, but saw {invocations:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "the failing native spy must fail the run with its ordinary toolchain error:\n{combined}"
    );
}

/// `--affected` answers depend on the whole graph, so it is a complete-graph
/// query: every contributor is loaded up front, even when the change itself
/// touches only JavaScript. The failing spy proves the Go owner is loaded
/// before affectedness is resolved; the failure is the ordinary toolchain
/// error.
#[cfg(unix)]
#[test]
fn affected_query_loads_the_go_owner() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_library_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();
    common::git(tempdir.path(), &["checkout", "-b", "feature"]);
    write_file(
        tempdir.path(),
        "packages/js-dev/src/index.js",
        "module.exports = 1;\n",
    );
    common::git(tempdir.path(), &["add", "."]);
    common::git(
        tempdir.path(),
        &["commit", "-m", "change js-dev", "--quiet"],
    );

    let spy = ToolchainSpy::new(spy::GO);
    let output = spy_run(
        tempdir.path(),
        &["run", "dev", "--affected", "--dry-run=json"],
        &spy,
    );
    let combined = combined_output(&output);
    spy.assert_invoked(
        "go",
        &output,
        "affected query over a JavaScript-only change",
    );
    assert!(
        !output.status.success(),
        "the failing native spy must fail the run with its ordinary toolchain error:\n{combined}"
    );
}

/// An unqualified task name is a repo-wide catalogue question — any scope,
/// including a native one, may own the task — so validating it demands the
/// Go owner's real metadata even when the package filter selects only
/// JavaScript. With `go` present the metadata loads and the genuinely
/// unknown task fails with the ordinary missing-task diagnostic: neither a
/// suppressed empty run nor a toolchain error. (With an unusable `go`, the
/// same query fails with the ordinary toolchain error instead.) Contrast
/// `missing_task_never_invokes_native_toolchains`: the qualified
/// `package#task` form is exact against its known JavaScript owner and never
/// consults native metadata.
#[cfg(unix)]
#[test]
fn unqualified_missing_task_fails_after_loading_real_native_metadata() {
    if which::which("go").is_err() {
        eprintln!("skipping: go is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_library_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();

    let output = common::run_turbo(tempdir.path(), &["run", "doesnotexist", "--filter=js-dev"]);
    let combined = combined_output(&output);
    assert!(
        !output.status.success(),
        "an unknown unqualified task must fail once the catalogue is answered exactly:\n{combined}"
    );
    assert!(
        combined.contains("Could not find task `doesnotexist` in project"),
        "expected the ordinary missing-task diagnostic after real native metadata \
         loaded:\n{combined}"
    );
}

// ---------------------------------------------------------------------------
// Hash stability under native co-selection
// ---------------------------------------------------------------------------

/// The hash of an unchanged JavaScript task must not depend on whether a Rust
/// owner is loaded alongside it.
#[test]
fn unchanged_js_task_hash_is_stable_with_a_selected_rust_owner() {
    let tempdir = tempfile::tempdir().unwrap();
    write_rust_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();

    let js_only = common::run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=js-lib", "--dry-run=json"],
    );
    assert_success(&js_only, "JS-only dry run");
    let js_only: serde_json::Value = serde_json::from_slice(&js_only.stdout).unwrap();
    let baseline = task_hash(&js_only, "js-lib#build");

    let with_rust = common::run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=js-lib",
            "--filter=rust-app",
            "--dry-run=json",
        ],
    );
    assert_success(&with_rust, "JS+Rust dry run");
    let with_rust: serde_json::Value = serde_json::from_slice(&with_rust.stdout).unwrap();
    assert!(
        task_ids(&with_rust).contains("rust-app#build"),
        "the Rust owner must be selected: {:?}",
        task_ids(&with_rust)
    );
    assert_eq!(
        task_hash(&with_rust, "js-lib#build"),
        baseline,
        "an unchanged JavaScript task hash must not change when a Rust owner is loaded"
    );
}

/// Same invariant for a Go owner: a selected Go task must not perturb the
/// JavaScript task hash through global fallback inputs — the Go domain's
/// failures route consumer-scoped fallbacks, and a successful Go discovery
/// contributes no JavaScript-facing hash input.
#[cfg(unix)]
#[test]
fn unchanged_js_task_hash_is_stable_with_a_selected_go_owner() {
    if which::which("go").is_err() {
        eprintln!("skipping: go is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());

    let js_only = common::run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=js-lib", "--dry-run=json"],
    );
    assert_success(&js_only, "JS-only dry run");
    let js_only: serde_json::Value = serde_json::from_slice(&js_only.stdout).unwrap();
    let baseline = task_hash(&js_only, "js-lib#build");

    let with_go = common::run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=js-lib",
            "--filter=api",
            "--dry-run=json",
        ],
    );
    assert_success(&with_go, "JS+Go dry run");
    let with_go: serde_json::Value = serde_json::from_slice(&with_go.stdout).unwrap();
    assert!(
        task_ids(&with_go).contains("api#build"),
        "the Go owner must be selected: {:?}",
        task_ids(&with_go)
    );
    assert_eq!(
        task_hash(&with_go, "js-lib#build"),
        baseline,
        "an unchanged JavaScript task hash must not change when a Go owner is loaded"
    );
}

/// An out-of-date `Cargo.lock` fails `cargo metadata --locked`, so a
/// co-selected Rust owner contributes a `Partial` external resolution: the
/// lockfile-aware closure is unavailable and hashing falls back to the
/// domain's conservative inputs. Those fallbacks are consumer-scoped, so the
/// unchanged JavaScript task's hash must not move when the Rust owner joins
/// the selection, while the Rust task itself still participates and hashes
/// through the fallback fingerprint. (No cache-policy assertion here:
/// `PackageResolutionState::cache_eligible` has no production caller, so
/// there is no Partial-task caching behavior to pin.) A JavaScript-only
/// selection is protected trivially: the Rust owner is never loaded, so its
/// lock never feeds any hash.
#[test]
fn unchanged_js_task_hash_with_partial_rust_resolution() {
    let tempdir = tempfile::tempdir().unwrap();
    write_rust_workspace(tempdir.path());
    // Lock `rust-lib` at a version no manifest declares, so
    // `cargo metadata --locked` refuses to validate the lock.
    write_file(
        tempdir.path(),
        "Cargo.lock",
        r#"version = 4

[[package]]
name = "rust-app"
version = "0.1.0"
dependencies = [
 "rust-lib",
]

[[package]]
name = "rust-lib"
version = "0.2.0"
"#,
    );
    setup::setup_git(tempdir.path()).unwrap();

    let js_only = common::run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=js-lib", "--dry-run=json"],
    );
    assert_success(&js_only, "JS-only dry run with an invalid Rust lock");
    let js_only: serde_json::Value = serde_json::from_slice(&js_only.stdout).unwrap();
    let baseline = task_hash(&js_only, "js-lib#build");

    let co_output = common::run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=js-lib",
            "--filter=rust-app",
            "--dry-run=json",
        ],
    );
    assert_success(
        &co_output,
        "co-selection must tolerate the invalid lock via a Partial resolution",
    );
    let combined = combined_output(&co_output);
    let co_selected: serde_json::Value = serde_json::from_slice(&co_output.stdout).unwrap();
    assert!(
        task_ids(&co_selected).contains("rust-app#build"),
        "the Rust owner must be selected: {:?}\n{combined}",
        task_ids(&co_selected)
    );
    assert_eq!(
        task_hash(&co_selected, "js-lib#build"),
        baseline,
        "a Partial Rust resolution must not move the JavaScript task's hash: consumer-scoped \
         fallbacks preserve JavaScript-owned behavior"
    );
    // The Partial resolution still yields a participating, hashable Rust
    // task: the owner loaded and the fallback fingerprint feeds it.
    assert!(
        !task_hash(&co_selected, "rust-app#build").is_empty(),
        "the Partial Rust task must still be selected and hashed\n{combined}"
    );
}

// ---------------------------------------------------------------------------
// Non-run commands
// ---------------------------------------------------------------------------

/// Non-run commands share the run builder, so `ls` is an unfiltered,
/// repository-wide query: it loads the Cargo owner, whose authoritative
/// discovery lists the crates — exactly like an unfiltered `turbo run`, `ls`
/// requires a usable Cargo.
#[test]
fn non_run_command_discovers_native_packages() {
    if which::which("cargo").is_err() {
        eprintln!("skipping: cargo is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    write_rust_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();

    let output = common::run_turbo(tempdir.path(), &["ls", "--output", "json"]);
    assert_success(&output, "turbo ls");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("ls emits JSON");
    let names: BTreeSet<String> = json["packages"]["items"]
        .as_array()
        .expect("packages items")
        .iter()
        .map(|item| item["name"].as_str().expect("package name").to_string())
        .collect();
    assert!(names.contains("rust-app"), "names: {names:?}");
    assert!(names.contains("rust-lib"), "names: {names:?}");
}
