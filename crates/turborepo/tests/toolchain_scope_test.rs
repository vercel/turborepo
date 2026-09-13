//! Regression coverage for staged toolchain discovery: a native toolchain
//! (Go, Rust, Python) that owns no finally-participating task must never be
//! invoked or required. Planning discovers every contributor without
//! subprocesses; only selected owners are fully discovered. On Unix, spy
//! shims prepended to the child `PATH` make any consultation observable.

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

/// Isolated fixture for the `dev` probe regression: a JavaScript package with a
/// `dev` script and a Go workspace whose only module is a library (no `main`
/// package, so no runnable `dev` target).
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
/// `go-lib` to `go-extra`. That edge is exactly the fact static planning
/// cannot prove without native metadata, making this the ambiguous
/// counterpart of [`write_mixed_workspace`]'s closed replacement graph.
#[cfg(unix)]
fn write_js_go_ambiguous_replacement_workspace(dir: &Path) {
    write_file(dir, ".gitignore", ".turbo\nnode_modules\ndist\n");
    write_file(
        dir,
        "package.json",
        r#"{
  "name": "toolchain-scope-ambiguous-alias",
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

/// The ambiguous counterpart of [`write_js_dev_go_library_workspace`]: the
/// same JavaScript `dev` script beside a Go module whose only `main` package
/// sits behind a build constraint (`//go:build integration` on
/// `cmd/service/main.go`), so whether the module is a runnable `main` with a
/// `dev` task or a library without one is environment-dependent — the one
/// fact static planning cannot sample without `go`. The constraint never
/// holds under default tags, so a real `go` classifies the module as a
/// library on every host: its `build` task keeps the exact library command
/// shape everywhere, which is what makes the module's `build` selection
/// provable while its `dev` selection is not.
///
/// `filter_using_tasks` toggles `futureFlags.filterUsingTasks`, which resolves
/// `--filter` at the task level instead of the package level.
/// `js_dev_depends_on_go_dev` adds a cross-language task dependency
/// (`js-dev#dev` → `example.com/api#dev`) to `turbo.json`, pointing at the
/// unprovable native `dev` task. The `dev` task is deliberately
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
        tasks["js-dev#dev"] = serde_json::json!({ "dependsOn": ["example.com/api#dev"] });
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

#[cfg(unix)]
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
// No native task in the final graph
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

/// `js-only` is declared for every package but only JavaScript packages have a
/// command for it, so no native task participates.
#[cfg(unix)]
#[test]
fn task_without_native_participants_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let (summary, output) = spy_dry_run(tempdir.path(), &["js-only"], &spy);
    let ids = task_ids(&summary);
    let combined = combined_output(&output);
    assert!(ids.contains("js-app#js-only"), "ids: {ids:?}\n{combined}");
    assert!(ids.contains("js-lib#js-only"), "ids: {ids:?}\n{combined}");
    spy.assert_no_invocations(&output, "task with no native command");
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
/// the staged path must still leave native toolchains untouched.
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

/// A requested task no package can run still builds the planning graph; it must
/// fail without consulting a native toolchain.
#[cfg(unix)]
#[test]
fn missing_task_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(&spy::all_tools());

    let output = spy_run(tempdir.path(), &["run", "doesnotexist"], &spy);
    let combined = combined_output(&output);
    assert!(
        !output.status.success(),
        "unknown task must fail:\n{combined}"
    );
    assert!(
        combined.contains("Could not find task `doesnotexist` in project"),
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

/// A change to a JavaScript package makes `--affected` select only JavaScript
/// work.
#[cfg(unix)]
#[test]
fn affected_js_change_never_invokes_native_toolchains() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    common::git(tempdir.path(), &["checkout", "-b", "feature"]);
    write_file(
        tempdir.path(),
        "packages/js-app/src/index.js",
        "module.exports = 1;\n",
    );
    common::git(tempdir.path(), &["add", "."]);
    common::git(
        tempdir.path(),
        &["commit", "-m", "change js-app", "--quiet"],
    );

    let spy = ToolchainSpy::new(&spy::all_tools());
    let (summary, output) = spy_dry_run(tempdir.path(), &["build", "--affected"], &spy);
    let ids = task_ids(&summary);
    let combined = combined_output(&output);
    assert!(ids.contains("js-app#build"), "ids: {ids:?}\n{combined}");
    assert!(
        ids.iter().all(|id| id.starts_with("js-")),
        "affected should select only JavaScript work: {ids:?}\n{combined}"
    );
    spy.assert_no_invocations(&output, "--affected JavaScript change");
}

// ---------------------------------------------------------------------------
// Native task in the final graph: the owning toolchain is required
// ---------------------------------------------------------------------------

/// Selecting a Rust task must fully discover the Rust contributor; the failing
/// `cargo` shim proves the toolchain was invoked rather than skipped.
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

/// Selecting a Go task must invoke `go`; a failed invocation must error rather
/// than silently dropping the task. The log assertion also rules out an
/// unimplemented static-discovery path passing for the wrong reason.
#[cfg(unix)]
#[test]
fn selected_go_task_requires_a_usable_go_executable() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(tempdir.path(), base_tasks());
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(
        tempdir.path(),
        &["run", "build", "--filter=example.com/api", "--dry-run=json"],
        &spy,
    );
    spy.assert_invoked("go", &output, "Go task selection");
    assert!(
        !output.status.success(),
        "selecting a Go task must not silently succeed without a usable `go`:\n{}",
        combined_output(&output)
    );
}

/// Selecting a Python task must fully discover the Python contributor. uv may
/// fall back to manifest discovery when its binary fails, so the assertion is
/// on the observed invocation.
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

/// An explicit cross-language `dependsOn` pulls `example.com/api#build` into a
/// JavaScript-filtered run, so the Go owner is selected even though `--filter`
/// never mentioned it. Contrast with
/// `js_filter_never_invokes_native_toolchains`.
#[cfg(unix)]
#[test]
fn cross_language_task_dependency_prepares_the_native_owner() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_mixed_workspace(
        tempdir.path(),
        serde_json::json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "js-only": {},
            "js-app#build": { "dependsOn": ["^build", "example.com/api#build"] }
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
        "the pulled-in Go owner must be prepared even for a JavaScript-only filter:\n{}",
        combined_output(&output)
    );
}

/// When Go is actually installed, the task closure is visible in the graph:
/// the JavaScript filter still executes `example.com/api#build`.
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
            "js-app#build": { "dependsOn": ["^build", "example.com/api#build"] }
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
        ids.contains("example.com/api#build"),
        "explicit task dependency must join the JavaScript selection: {ids:?}\n{combined}"
    );
}

// ---------------------------------------------------------------------------
// Review-driven regressions
// ---------------------------------------------------------------------------

/// An unqualified `dev` request with only a library-only Go module present must
/// not probe Go at all: a library has no runnable `dev` target, so it cannot
/// own a participating task.
#[cfg(unix)]
#[test]
fn dev_js_script_never_probes_a_library_only_go_module() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_library_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(tempdir.path(), &["run", "dev", "--dry-run=json"], &spy);
    assert_success(&output, "unqualified dev dry run");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let ids = task_ids(&summary);
    assert!(
        ids.contains("js-dev#dev"),
        "ids: {ids:?}\n{}",
        combined_output(&output)
    );
    // A library-only Go module may carry a commandless `dev` node, but it must
    // never make the Go owner participate, so no `go` process is spawned.
    spy.assert_no_invocations(&output, "unqualified JS dev with a library-only Go module");
}

/// The hash of an unchanged JavaScript task must not depend on whether a Rust
/// owner is prepared alongside it.
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
        "an unchanged JavaScript task hash must not change when a Rust owner is prepared"
    );
}

/// Same invariant for a Go owner: a selected Go task, whose static observation
/// reports external resolution `Unavailable`, must not perturb the JavaScript
/// task hash through global fallback inputs.
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
            "--filter=example.com/api",
            "--dry-run=json",
        ],
    );
    assert_success(&with_go, "JS+Go dry run");
    let with_go: serde_json::Value = serde_json::from_slice(&with_go.stdout).unwrap();
    assert!(
        task_ids(&with_go).contains("example.com/api#build"),
        "the Go owner must be selected: {:?}",
        task_ids(&with_go)
    );
    assert_eq!(
        task_hash(&with_go, "js-lib#build"),
        baseline,
        "an unchanged JavaScript task hash must not change when a Go owner is prepared"
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
/// selection is protected trivially: the Rust owner is never prepared, so
/// its lock never feeds any hash.
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
    // task: preparation completed and the fallback fingerprint feeds it.
    assert!(
        !task_hash(&co_selected, "rust-app#build").is_empty(),
        "the Partial Rust task must still be selected and hashed\n{combined}"
    );
}

// ---------------------------------------------------------------------------
// Planning-uncertainty regressions
// ---------------------------------------------------------------------------

/// A Go edge that only native metadata can resolve — an active remote
/// requirement could raise `example.com/alias` past `v1.0.0` and flip its
/// version-specific replacement from `go-lib` to `go-extra` — must not fail
/// planning for a JavaScript-only selection: the run succeeds on a cold cache
/// with real execution, and the failing `go` spy proves zero probes. Contrast
/// with `js_filter_never_invokes_native_toolchains`, whose Go graph is exact.
#[cfg(unix)]
#[test]
fn js_only_filter_succeeds_with_remote_sensitive_go_replacements() {
    if which::which("node").is_err() {
        eprintln!("skipping: node is not on PATH");
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    write_js_go_ambiguous_replacement_workspace(tempdir.path());
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::NATIVE_TOOLS);

    let output = spy_run(tempdir.path(), &["run", "build", "--filter=js-app"], &spy);
    assert_success(&output, "JavaScript-only build over an ambiguous Go graph");
    assert!(
        tempdir.path().join("packages/js-app/dist").is_dir(),
        "the cold-cache JavaScript build must actually execute"
    );
    spy.assert_no_invocations(
        &output,
        "JavaScript-only filter with remote-sensitive replacements",
    );
}

/// A platform-constrained Go module's `dev` catalogue cannot be proven
/// without `go`, but a task-level filter (`futureFlags.filterUsingTasks`)
/// that explicitly selects the JavaScript package provably never consults
/// that catalogue, so the selection stays exact and `go` is never probed.
/// The `package#task` CLI argument form is covered separately by
/// `js_package_task_argument_never_invokes_native_toolchains` and needs no
/// flag; the two selection forms are deliberately not conflated. Contrast
/// with `dev_js_script_never_probes_a_library_only_go_module`, whose Go
/// module's catalogue is exact.
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

/// Selecting `dev` on the ambiguous Go module itself must be refused with the
/// unresolved planning fact — naming the Go toolchain, the task catalogue,
/// and the `example.com/api` scope — before preparation, so the failing `go`
/// spy proves the refusal happens without ever invoking `go` to resolve it.
#[cfg(unix)]
#[test]
fn ambiguous_native_dev_selection_is_refused_without_invoking_go() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), false, false);
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let output = spy_run(
        tempdir.path(),
        &["run", "dev", "--filter=example.com/api", "--dry-run=json"],
        &spy,
    );
    let combined = combined_output(&output);
    assert!(
        !output.status.success(),
        "a dev selection that cannot be proven exact must be refused:\n{combined}"
    );
    assert!(
        combined.contains("cannot prove this run's task selection"),
        "expected the unresolved-planning diagnostic:\n{combined}"
    );
    assert!(
        combined.contains("toolchain `go`") && combined.contains("the task catalogue"),
        "the refusal must name the Go toolchain and the unresolved fact:\n{combined}"
    );
    assert!(
        combined.contains("`example.com/api`"),
        "the refusal must name the ambiguous Go scope:\n{combined}"
    );
    spy.assert_no_invocations(&output, "ambiguous native dev selection");
}

/// The constrained module's `dev` catalogue is unprovable, but its `build`
/// task is exact: under default build tags the module classifies as a
/// library on every host, so the library command shape — `go build ./...`,
/// uncached, because a library produces no tracked output — holds whether or
/// not the constrained `main` package ever counts. An explicit native build
/// selection must therefore prepare Go and retain the real command shape
/// instead of being refused. Complements
/// `ambiguous_native_dev_selection_is_refused_without_invoking_go`, which
/// refuses the same module's unprovable `dev` selection without invoking
/// `go`. Requires a real `go`; the module declares no dependencies, so
/// preparation never touches the network.
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
        &["run", "build", "--filter=example.com/api", "--dry-run=json"],
    );
    assert_success(&output, "explicit native build selection");
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let combined = combined_output(&output);
    assert!(
        task_ids(&summary).contains("example.com/api#build"),
        "the native build task must be selected: {:?}\n{combined}",
        task_ids(&summary)
    );
    let task = dry_run_task(&summary, "example.com/api#build");
    assert_eq!(
        task["command"], "go build ./...",
        "the prepared library build must retain its real command shape\n{combined}"
    );
    assert_eq!(
        task["resolvedTaskDefinition"]["cache"], false,
        "a library build produces no tracked output, so it stays uncached\n{combined}"
    );
}

/// A `package#task` CLI argument narrows the selection to the referenced
/// package even without `--filter`, so the ambiguous Go module's uncertain
/// `dev` catalogue is provably never consulted: the run must succeed without
/// probing `go`. Contrast an unqualified `dev` request, whose task set
/// genuinely cannot be proven. The argument form needs no future flag;
/// `js_package_task_argument_never_invokes_native_toolchains` covers it
/// against an exact native graph.
#[cfg(unix)]
#[test]
fn qualified_js_dev_argument_ignores_the_ambiguous_go_catalogue() {
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

/// An exclude-only filter provably removes the ambiguous Go module from the
/// selection, so its uncertain `dev` catalogue is never consulted and the
/// JavaScript `dev` run must succeed without probing `go`. The exclusion —
/// not a repository-wide catalogue domain — bounds what the run can ask
/// about. The Go workspace aggregate keeps a commandless `dev` node — the
/// root `dev` task config applies to every scope, and exclude-only selects
/// all of them minus the excluded module — so it may appear, but only as a
/// `<NONEXISTENT>` phantom that executes nothing.
#[cfg(unix)]
#[test]
fn excluding_the_ambiguous_go_module_keeps_a_js_dev_run_exact() {
    let tempdir = tempfile::tempdir().unwrap();
    write_js_dev_go_platform_constrained_workspace(tempdir.path(), false, false);
    setup::setup_git(tempdir.path()).unwrap();
    let spy = ToolchainSpy::new(spy::GO);

    let (summary, output) =
        spy_dry_run(tempdir.path(), &["dev", "--filter=!example.com/api"], &spy);
    let ids = task_ids(&summary);
    let combined = combined_output(&output);
    assert!(
        ids.contains("js-dev#dev"),
        "the JavaScript dev task must be selected: {ids:?}\n{combined}"
    );
    assert!(
        !ids.contains("example.com/api#dev"),
        "the excluded module must not participate: {ids:?}\n{combined}"
    );
    assert!(
        ids.iter()
            .all(|id| id.starts_with("js-") || id == "go-workspace#dev"),
        "only JavaScript work and the commandless aggregate may remain: {ids:?}\n{combined}"
    );
    let aggregate = dry_run_task(&summary, "go-workspace#dev");
    assert_eq!(
        aggregate["command"], "<NONEXISTENT>",
        "the aggregate's phantom dev node must stay commandless\n{combined}"
    );
    spy.assert_no_invocations(&output, "exclude-only JavaScript dev filter");
}

/// A cross-language `dependsOn` on the ambiguous module's unprovable `dev`
/// task must refuse the run with the unresolved-planning diagnostic: the
/// static catalogue never contributed that task, so the dependency is a
/// phantom that preparation cannot recover, and the run must not silently
/// execute the JavaScript task while ignoring it. The refusal happens before
/// preparation, so `go` is never invoked. Contrast
/// `task_level_js_dev_filter_ignores_a_platform_constrained_go_module`: the
/// same command and flag without the dependency succeeds.
#[cfg(unix)]
#[test]
fn cross_language_dev_dependency_on_an_uncertain_catalogue_is_refused() {
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
    assert!(
        !output.status.success(),
        "a dependency on an unprovable native task must be refused:\n{combined}"
    );
    assert!(
        combined.contains("cannot prove this run's task selection"),
        "expected the unresolved-planning diagnostic:\n{combined}"
    );
    assert!(
        combined.contains("toolchain `go`") && combined.contains("the task catalogue"),
        "the refusal must name the Go toolchain and the unresolved fact:\n{combined}"
    );
    assert!(
        combined.contains("`example.com/api`"),
        "the refusal must name the ambiguous Go scope:\n{combined}"
    );
    spy.assert_no_invocations(&output, "cross-language dev dependency");
}

// ---------------------------------------------------------------------------
// Non-run commands
// ---------------------------------------------------------------------------

/// Non-run commands share the graph builder and must keep working: `ls` skips
/// external dependencies and still reports native packages, without needing a
/// native toolchain executable.
#[test]
fn non_run_command_discovers_native_packages() {
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
