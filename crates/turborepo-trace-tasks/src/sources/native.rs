//! turbo's native packages: Cargo crates and uv projects that are turbo
//! packages of their own, behind the root `turbo.json`'s `futureFlags`
//! (`experimentalCargoWorkspaces`, `experimentalPythonWorkspaces`).
//!
//! Such a package has no `package.json` scripts: turbo derives its tasks from
//! the toolchain. Without this source, the `turbo.json` definitions of these
//! tasks ([`super::turbo`], which only augment a task some source defines)
//! would be dropped, and with them their `dependsOn` and `outputs`.
//!
//! Cargo (`experimentalCargoWorkspaces`, a `Cargo.toml` at the repository
//! root): every crate gets the tasks turbo registers, with turbo's commands
//! (turbo runs them at the repository root; here in the crate's directory,
//! where cargo finds the same workspace, and the crate's own manifest names the
//! crate whose tests `cargo test --package=<crate>` starts):
//!
//! | Task | Command |
//! |---|---|
//! | `build` | `cargo build --package=<crate> --locked` |
//! | `test` | `cargo test --package=<crate> --locked` |
//! | `check` | `cargo check --package=<crate> --locked` |
//! | `lint` | `cargo clippy --package=<crate> --locked` |
//! | `format` | `cargo fmt --package=<crate>` |
//! | `run`, `dev` (a crate with a binary) | `cargo run --package=<crate> --locked` |
//!
//! turbo takes the members from `cargo metadata`; here every crate of the
//! repository counts as one (a crate the workspace leaves out gets tasks turbo
//! does not have, which only adds to the task model). turbo gives `run` and
//! `dev` to a crate with exactly one binary; here to every crate with a binary.
//! The workspace's own tasks (`<workspace>#test`: `cargo test --workspace`) run
//! the same tests as the crates' tasks, and are not read: the trace has no
//! package for them besides the root's.
//!
//! uv (`experimentalPythonWorkspaces`, a `pyproject.toml` at the repository
//! root): `turbo trace` does not analyze Python, so only the tasks that start
//! tests are read, so that they are reported as tasks the selection does not
//! cover: `test` of a project that declares `pytest` (a dependency or a
//! dependency group of its own: `uv run --active --frozen --package <project>
//! pytest <dir>`), and of the workspace when the root project declares it (`uv
//! run --active --frozen --all-packages pytest`, a task of the root package).
//! The other tasks turbo derives for uv projects (`build`, `format`, `check`,
//! `lint` and the tool tasks below them) start no tests, and are not read.
//!
//! A package with a `package.json` is a JavaScript package, never a native one.

use std::path::Path;

use turborepo_trace_frameworks::fsx;

use crate::sources::{DiscoveredTask, SourceCx, TaskSource, taskfile::repo_rel, turbo::read_turbo};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&Native]
}

/// The tasks of turbo's native packages.
pub struct Native;

/// Which native package kinds the root `turbo.json` turns on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Flags {
    cargo: bool,
    python: bool,
}

fn flags(root: &Path) -> Flags {
    let Some((_, Some(v))) = read_turbo(root) else {
        return Flags::default();
    };
    let on =
        |k: &str| v.get("futureFlags").and_then(|f| f.get(k)) == Some(&serde_json::json!(true));
    Flags {
        cargo: on("experimentalCargoWorkspaces"),
        python: on("experimentalPythonWorkspaces"),
    }
}

fn read_toml(p: &Path) -> Option<toml::Table> {
    toml::from_str(&fsx::read_to_string(p).ok()?).ok()
}

/// The crate in `dir`, if it is a native package: its name and manifest.
fn crate_at(root: &Path, dir: &Path) -> Option<(String, toml::Table)> {
    if !fsx::is_file(&root.join("Cargo.toml")) {
        return None;
    }
    let m = read_toml(&dir.join("Cargo.toml"))?;
    let name = m.get("package")?.get("name")?.as_str()?.to_owned();
    Some((name, m))
}

/// The uv project in `dir`, if it is a native package: its name and manifest.
fn project_at(root: &Path, dir: &Path) -> Option<(String, toml::Table)> {
    if !fsx::is_file(&root.join("pyproject.toml")) {
        return None;
    }
    let m = read_toml(&dir.join("pyproject.toml"))?;
    let name = m.get("project")?.get("name")?.as_str()?.to_owned();
    Some((name, m))
}

/// turbo's name of the native package in `dir` (the crate's or the project's
/// name), when the root `turbo.json` turns its kind on and `dir` has no
/// `package.json`.
pub fn package_name(root: &Path, dir: &Path) -> Option<String> {
    if fsx::is_file(&dir.join("package.json")) {
        return None;
    }
    let f = flags(root);
    f.cargo
        .then(|| crate_at(root, dir))
        .flatten()
        .or_else(|| f.python.then(|| project_at(root, dir)).flatten())
        .map(|(name, _)| name)
}

/// Whether the crate in `dir` has a binary target (`src/main.rs`, `src/bin/`,
/// a `[[bin]]` table).
fn has_binary(dir: &Path, m: &toml::Table) -> bool {
    m.get("bin")
        .and_then(|b| b.as_array())
        .is_some_and(|b| !b.is_empty())
        || fsx::is_file(&dir.join("src/main.rs"))
        || fsx::is_dir(&dir.join("src/bin"))
}

/// The normalized names of the dependencies a pyproject declares itself: its
/// project's dependencies, `tool.uv.dev-dependencies` and every dependency
/// group (as turbo reads tool declarations).
fn declared(m: &toml::Table) -> Vec<String> {
    let strings = |v: Option<&toml::Value>| -> Vec<String> {
        v.and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|d| d.as_str().map(str::to_owned))
            .collect()
    };
    let mut all = strings(m.get("project").and_then(|p| p.get("dependencies")));
    all.extend(strings(
        m.get("tool")
            .and_then(|t| t.get("uv"))
            .and_then(|u| u.get("dev-dependencies")),
    ));
    if let Some(groups) = m.get("dependency-groups").and_then(|g| g.as_table()) {
        for g in groups.values() {
            all.extend(strings(Some(g)));
        }
    }
    all.iter()
        .filter_map(|d| {
            let d = d.trim_start();
            let end = d
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                .unwrap_or(d.len());
            (end > 0).then(|| d[..end].to_ascii_lowercase().replace(['_', '.'], "-"))
        })
        .collect()
}

impl TaskSource for Native {
    fn id(&self) -> &'static str {
        "native"
    }

    /// None: discovery lists no directory. What it reads (the root
    /// `turbo.json`, the package's manifest) and the paths it finds missing
    /// (`src/main.rs`, `src/bin`) are its observations, so a change to them
    /// discovers the package's tasks again; without the future flags it
    /// observes nothing beyond the root `turbo.json`, which the
    /// `turbo.json` source reads too.
    fn config_file_names(&self) -> &'static [&'static str] {
        &[]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let (root, dir) = (&cx.workspace.root, &cx.package.dir);
        if fsx::is_file(&dir.join("package.json")) && dir != root {
            return Vec::new();
        }
        let f = flags(root);
        if !(f.cargo || f.python) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut task = |name: &str, line: String, file: &Path, cwd: &Path| {
            let mut t = DiscoveredTask::new(cx.script_id(name), name, cwd);
            t.lines.push(line);
            t.files.push(file.to_path_buf());
            out.push(t);
        };
        let no_package_json = !fsx::is_file(&dir.join("package.json"));
        if f.cargo
            && no_package_json
            && let Some((name, m)) = crate_at(root, dir)
        {
            let manifest = dir.join("Cargo.toml");
            for (task_name, sub) in [
                ("build", "build"),
                ("test", "test"),
                ("check", "check"),
                ("lint", "clippy"),
            ] {
                task(
                    task_name,
                    format!("cargo {sub} --package={name} --locked"),
                    &manifest,
                    dir,
                );
            }
            task(
                "format",
                format!("cargo fmt --package={name}"),
                &manifest,
                dir,
            );
            if has_binary(dir, &m) {
                for task_name in ["run", "dev"] {
                    task(
                        task_name,
                        format!("cargo run --package={name} --locked"),
                        &manifest,
                        dir,
                    );
                }
            }
        }
        if f.python {
            let manifest = dir.join("pyproject.toml");
            let pytest = |m: &toml::Table| declared(m).iter().any(|d| d == "pytest");
            let mut member = false;
            if no_package_json
                && let Some((name, m)) = project_at(root, dir)
                && pytest(&m)
            {
                let rel = repo_rel(root, dir);
                task(
                    "test",
                    format!("uv run --active --frozen --package {name} pytest {rel}"),
                    &manifest,
                    root,
                );
                member = true;
            }
            if dir == root
                && !member
                && let Some(m) = read_toml(&manifest)
                && m.get("tool")
                    .and_then(|t| t.get("uv"))
                    .and_then(|u| u.get("workspace"))
                    .is_some()
                && pytest(&m)
            {
                task(
                    "test",
                    "uv run --active --frozen --all-packages pytest".to_owned(),
                    &manifest,
                    root,
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_names_are_normalized() {
        let m: toml::Table = toml::from_str(
            "[project]\nname = \"a\"\ndependencies = [\"PyTest>=8\", \
             \"requests\"]\n[dependency-groups]\ndev = [\"ruff\", { include-group = \"x\" }]\n",
        )
        .unwrap();
        assert_eq!(declared(&m), ["pytest", "requests", "ruff"]);
    }
}
