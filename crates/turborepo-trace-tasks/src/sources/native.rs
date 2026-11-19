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
//! Go (`experimentalGoWorkspaces`, a `go.work` at the repository root): every
//! module the `go.work` uses gets turbo's tasks, run in the module's directory
//! (turbo's name of the package is the module path's last element, with a
//! trailing major version kept: `api/v2`):
//!
//! | Task | Command |
//! |---|---|
//! | `build` | `go build <main package>`, else `go build ./...` |
//! | `test` | `go test ./...` |
//! | `lint` | `go vet ./...` |
//! | `format` | `go fmt ./...` |
//! | `dev` (a module with one main package) | `go run <main package>` |
//!
//! turbo asks `go list` for the main packages; here a package is a directory of
//! the module (not `testdata`, `vendor`, `.`- or `_`-prefixed, nor another
//! module) with a `.go` file that is no test, and it is a main package when one
//! of those files says `package main` (build constraints are not evaluated). A
//! module with exactly one main package is runnable: its build writes the
//! binary into the module's directory (`<dir>/<name>`, `<name>.exe`), which is
//! an output of the task. What the scan asked of the tree (the directories it
//! listed, the files it looked at) keys the package's tasks
//! ([`super::with_decided_by`]), and is no input of them; a `.go` file whose
//! package clause changes alone does not discover the tasks again.
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
    go: bool,
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
        go: on("experimentalGoWorkspaces"),
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

/// The directories (repository-relative) a `go.work` uses.
fn go_work_uses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let entry = if block {
            if line == ")" {
                block = false;
                continue;
            }
            line
        } else if let Some(rest) = line.strip_prefix("use") {
            let rest = rest.trim();
            if rest == "(" {
                block = true;
                continue;
            }
            if rest.is_empty() || !line[3..].starts_with([' ', '\t']) {
                continue;
            }
            rest
        } else {
            continue;
        };
        let entry = entry.trim_matches('"');
        if !entry.is_empty() {
            out.push(super::norm_rel(entry));
        }
    }
    out
}

/// The module in `dir`, if it is a native package: its module path.
fn module_at(root: &Path, dir: &Path) -> Option<String> {
    let work = fsx::read_to_string(root.join("go.work")).ok()?;
    let rel = repo_rel(root, dir);
    if !go_work_uses(&work).contains(&rel) {
        return None;
    }
    let m = fsx::read_to_string(dir.join("go.mod")).ok()?;
    m.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("module")?;
        let path = rest.trim().trim_matches('"');
        (rest.starts_with([' ', '\t']) && !path.is_empty()).then(|| path.to_owned())
    })
}

/// `vN` with N >= 2 and no leading zero (Go's major version element).
fn is_version_element(e: &str) -> bool {
    e.strip_prefix('v').is_some_and(|v| {
        !v.is_empty() && v != "1" && !v.starts_with('0') && v.bytes().all(|b| b.is_ascii_digit())
    })
}

/// turbo's name of a Go module's package: the last element of the module path,
/// with a trailing major version kept (`example.com/api/v2`: `api/v2`).
fn go_package_name(module: &str) -> String {
    let Some((prefix, last)) = module.rsplit_once('/') else {
        return module.to_owned();
    };
    if is_version_element(last) {
        module[prefix.rfind('/').map_or(0, |i| i + 1)..].to_owned()
    } else {
        last.to_owned()
    }
}

/// The name `go build` gives the binary of the package at `import`: its last
/// element, skipping a trailing major version.
fn binary_name(import: &str) -> String {
    let mut parts = import.rsplit('/');
    let last = parts.next().unwrap_or(import);
    if is_version_element(last) {
        parts.next().unwrap_or(last).to_owned()
    } else {
        last.to_owned()
    }
}

/// The main packages of the module in `dir` (`.` or `./<rel>`), sorted.
fn main_packages(dir: &Path) -> Vec<String> {
    fn walk(module: &Path, d: &Path, out: &mut Vec<String>) {
        let Some(entries) = fsx::read_dir(d) else {
            return;
        };
        let mut main = false;
        for (p, is_dir) in &entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if *is_dir {
                if name.starts_with(['.', '_'])
                    || matches!(name, "testdata" | "vendor")
                    || fsx::is_file(&p.join("go.mod"))
                    // `./...` does not follow a symbolic link to a directory.
                    || fsx::is_link(p)
                {
                    continue;
                }
                walk(module, p, out);
            } else if !main
                && name.ends_with(".go")
                && !name.ends_with("_test.go")
                && !name.starts_with(['.', '_'])
                && fsx::read_to_string(p).is_ok_and(|src| package_clause(&src) == Some("main"))
            {
                main = true;
            }
        }
        if main {
            let rel = d
                .strip_prefix(module)
                .map(|r| r.to_string_lossy().replace('\\', "/"));
            out.push(match rel {
                Ok(r) if !r.is_empty() => format!("./{r}"),
                _ => ".".to_owned(),
            });
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The package name of a Go source file's `package` clause.
fn package_clause(src: &str) -> Option<&str> {
    let mut in_block = false;
    for line in src.lines() {
        let mut l = line.trim();
        if in_block {
            match l.find("*/") {
                Some(i) => {
                    in_block = false;
                    l = l[i + 2..].trim();
                }
                None => continue,
            }
        }
        if l.starts_with("/*") {
            match l.find("*/") {
                Some(i) => l = l[i + 2..].trim(),
                None => {
                    in_block = true;
                    continue;
                }
            }
        }
        if l.is_empty() || l.starts_with("//") {
            continue;
        }
        let rest = l.strip_prefix("package")?;
        return rest
            .trim_start()
            .split(|c: char| c.is_whitespace() || c == ';' || c == '/')
            .next()
            .filter(|n| !n.is_empty());
    }
    None
}

/// The tasks turbo derives for the Go module `module` in `dir`.
fn go_tasks(cx: &SourceCx<'_>, dir: &Path, module: &str) -> Vec<DiscoveredTask> {
    let manifest = dir.join("go.mod");
    let mains = main_packages(dir);
    let target = (mains.len() == 1).then(|| mains[0].clone());
    let task = |name: &str, line: String| {
        let mut t = DiscoveredTask::new(cx.script_id(name), name, dir);
        t.lines.push(line);
        t.files.push(manifest.clone());
        t
    };
    let mut build = task(
        "build",
        format!("go build {}", target.as_deref().unwrap_or("./...")),
    );
    if let Some(target) = &target {
        let import = match target.as_str() {
            "." => module.to_owned(),
            t => format!("{module}/{}", t.trim_start_matches("./")),
        };
        let name = binary_name(&import);
        for n in [name.clone(), format!("{name}.exe")] {
            build.spec.output(dir.join(n));
        }
    }
    let mut out = vec![
        build,
        task("test", "go test ./...".to_owned()),
        task("lint", "go vet ./...".to_owned()),
        task("format", "go fmt ./...".to_owned()),
    ];
    if let Some(target) = &target {
        out.push(task("dev", format!("go run {target}")));
    }
    out
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
        .or_else(|| {
            f.go.then(|| module_at(root, dir))
                .flatten()
                .map(|m| go_package_name(&m))
        })
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
        if !(f.cargo || f.python || f.go) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut go = Vec::new();
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
        if f.go
            && no_package_json
            && let Some(module) = module_at(root, dir)
        {
            // The scan's listings and reads key the tasks and are no input of them.
            let found = turborepo_trace_core::record::pause(|| {
                super::with_decided_by(|| go_tasks(cx, dir, &module))
            });
            go = found;
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
        out.extend(go);
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

    #[test]
    fn go_names_and_clauses() {
        assert_eq!(go_package_name("example.com/api/v2"), "api/v2");
        assert_eq!(go_package_name("example.com/api"), "api");
        assert_eq!(binary_name("example.com/api/v2"), "api");
        assert_eq!(
            package_clause("/* a\n b */\n// c\npackage main // x\n"),
            Some("main")
        );
        assert_eq!(package_clause("package lib;"), Some("lib"));
        assert_eq!(
            go_work_uses("go 1.22\nuse ./a\nuse (\n\t./b // c\n\t\"./c\"\n)\n"),
            ["a", "b", "c"]
        );
    }
}
