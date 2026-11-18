#![cfg(unix)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
//! `turbo trace` reads `turbo.json` itself (`turborepo-trace-tasks`,
//! `sources/turbo.rs`) to know which task's outputs a test may read. Its
//! `dependsOn` and `outputs` must cover what turbo resolves for every task
//! (`Engine::task_definition`, as `turbo run --dry=json` prints it under
//! `resolvedTaskDefinition`), on every integration fixture turbo can plan:
//! equal where one definition of the root `turbo.json` applies, and a superset
//! where a package `turbo.json` or a root `<package>#<task>` adds to it (the
//! trace unions their values instead of taking the more specific one). The
//! trace's packages are those of its graph: the `package.json` workspaces and
//! the crates, Go modules and Python projects. turbo's native packages (Cargo
//! crates, Go modules, uv projects) have tasks turbo derives from the
//! toolchain; the trace reads those of crates and Go modules
//! (`sources/native.rs`), and an output turbo derives for one (the binary of
//! `cargo build` or `go build`) is covered when it lies in what the trace reads
//! the task's command to write. Of uv projects it reads only the tasks that
//! start tests (`turbo trace` does not analyze Python): their other tasks are
//! listed, not compared.

mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use common::{run_turbo_with_env, setup};
use turborepo_trace_tasks::{
    Options, TaskRef, analyze_package,
    sources::{SourceCx, TaskSource, Workspace, turbo::Turbo},
};

/// `p` with `.` and `..` resolved lexically.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// What a task declares: `dependsOn` as turbo spells it, and the absolute
/// output globs (`!` for an exclusion).
#[derive(Debug, Default, PartialEq, Eq)]
struct Declared {
    depends_on: BTreeSet<String>,
    outputs: BTreeSet<String>,
}

fn fixtures() -> Vec<String> {
    let dir = common::manifest_dir().join("../../turborepo-tests/integration/fixtures");
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            let p = e.path();
            p.join("turbo.json").is_file() || p.join("turbo.jsonc").is_file()
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

fn tasks_of(v: &serde_json::Value) -> Vec<String> {
    let tasks = v.get("tasks").or_else(|| v.get("pipeline"));
    tasks
        .and_then(|t| t.as_object())
        .map(|o| {
            o.keys()
                .map(|k| k.rsplit_once('#').map_or(k.as_str(), |(_, n)| n).to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn read_jsonc(p: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(p).ok()?;
    turborepo_trace_tasks::sources::turbo::parse_jsonc(&text)
}

/// The task names of every `turbo.json` of the workspace.
fn task_names(root: &Path, ws: &Workspace) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for p in &ws.packages {
        for f in ["turbo.json", "turbo.jsonc"] {
            if let Some(v) = read_jsonc(&p.dir.join(f)) {
                names.extend(tasks_of(&v));
            }
        }
    }
    for f in ["turbo.json", "turbo.jsonc"] {
        if let Some(v) = read_jsonc(&root.join(f)) {
            names.extend(tasks_of(&v));
        }
    }
    names
}

fn abs_glob(base: &Path, root: &Path, glob: &str) -> String {
    let (neg, glob) = match glob.strip_prefix('!') {
        Some(g) => ("!", g),
        None => ("", glob),
    };
    let p = match glob.strip_prefix("$TURBO_ROOT$/") {
        Some(r) => root.join(r),
        None => base.join(glob),
    };
    format!("{neg}{}", normalize(&p).to_string_lossy())
}

/// turbo's definitions: `(package directory, task)` of every task with a
/// command, from `turbo run <task> --dry=json` for each task name (one at a
/// time: a fixture may have a task that turbo refuses to plan, such as a root
/// script that runs `turbo` again). `None` when turbo plans none of them.
fn turbo_side(
    root: &Path,
    names: &BTreeSet<String>,
) -> Option<BTreeMap<(String, String), Declared>> {
    let mut out = BTreeMap::new();
    let mut planned = false;
    for name in names {
        let output = run_turbo_with_env(
            root,
            &["run", name, "--dry=json"],
            &[("TURBO_TELEMETRY_DISABLED", "1")],
        );
        if !output.status.success() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
            continue;
        };
        planned = true;
        for t in v["tasks"].as_array().into_iter().flatten() {
            if t["command"] == "<NONEXISTENT>" {
                continue;
            }
            let (Some(dir), Some(task)) = (t["directory"].as_str(), t["task"].as_str()) else {
                continue;
            };
            let dir = if dir.is_empty() { "." } else { dir };
            let base = if dir == "." {
                root.to_path_buf()
            } else {
                root.join(dir)
            };
            let def = &t["resolvedTaskDefinition"];
            let strings = |k: &str| -> Vec<String> {
                def[k]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            };
            out.insert(
                (dir.to_owned(), task.to_owned()),
                Declared {
                    depends_on: strings("dependsOn").into_iter().collect(),
                    outputs: strings("outputs")
                        .iter()
                        .map(|g| abs_glob(&base, root, g))
                        .collect(),
                },
            );
        }
    }
    planned.then_some(out)
}

/// The trace's packages: the `package.json` workspaces, and the directories
/// with a `Cargo.toml`, a `pyproject.toml` or a `go.mod` (the graph's crates,
/// Python projects and Go modules).
fn trace_workspace(root: &Path) -> Workspace {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if !matches!(
                    name.to_str(),
                    Some(".git" | "node_modules" | "target" | ".turbo")
                ) {
                    walk(root, &p, out);
                }
            } else if matches!(
                name.to_str(),
                Some("Cargo.toml" | "pyproject.toml" | "go.mod")
            ) {
                let rel = dir.strip_prefix(root).unwrap().to_string_lossy();
                out.push(if rel.is_empty() {
                    ".".into()
                } else {
                    rel.into_owned()
                });
            }
        }
    }
    let js = Workspace::discover(root);
    let mut rels: Vec<String> = js.packages.iter().map(|p| p.rel.clone()).collect();
    walk(root, root, &mut rels);
    Workspace::from_roots(root, rels)
}

/// What the trace reads a task's commands to write (absolute): the outputs of
/// its specification that are not declared in `turbo.json`.
type Written = BTreeSet<PathBuf>;

/// The trace's reading of the same tasks, and whether more than the root's
/// generic definition contributed to each (a package `turbo.json`, a root
/// `<package>#<task>`); and the tasks of the trace's task model, with what
/// their commands write.
#[allow(clippy::type_complexity)]
fn trace_side(
    ws: &Workspace,
) -> (
    BTreeMap<(String, String), (Declared, bool)>,
    BTreeMap<(String, String), Written>,
) {
    let mut out = BTreeMap::new();
    let mut tasks = BTreeMap::new();
    for pkg in &ws.packages {
        let cx = SourceCx {
            workspace: ws,
            package: pkg,
        };
        for t in analyze_package(ws, pkg, &Options::default()) {
            tasks.insert(
                (pkg.rel.clone(), t.name.clone()),
                t.spec.outputs.iter().map(|o| normalize(&o.path)).collect(),
            );
        }
        for t in Turbo.discover(&cx) {
            let own = t.files.len() > 1;
            let mut d = Declared::default();
            for dep in &t.spec.task_deps {
                if !dep.reason.starts_with("turbo dependsOn") {
                    continue;
                }
                d.depends_on.insert(match &dep.target {
                    TaskRef::Local { name } => name.clone(),
                    TaskRef::Upstream { name } => format!("^{name}"),
                    TaskRef::Package { package, name } if package == "." => format!("//#{name}"),
                    TaskRef::Package { package, name } => format!("{package}#{name}"),
                    other => format!("{other:?}"),
                });
            }
            for o in &t.spec.outputs {
                let glob = match &o.pattern {
                    Some(p) => o.path.join(p),
                    None => o.path.clone(),
                };
                d.outputs
                    .insert(normalize(&glob).to_string_lossy().into_owned());
                for x in &o.exclude {
                    d.outputs
                        .insert(format!("!{}", normalize(&o.path.join(x)).to_string_lossy()));
                }
            }
            // Contributions with one id are merged (the root's definition and the
            // package's, or the root's `<package>#<task>`), as the task graph merges
            // them: a union where turbo takes the more specific one.
            let key = (pkg.rel.clone(), t.name.clone());
            let several = out.contains_key(&key);
            let entry = out
                .entry(key)
                .or_insert_with(|| (Declared::default(), false));
            entry.0.depends_on.extend(d.depends_on);
            entry.0.outputs.extend(d.outputs);
            entry.1 |= own || several;
        }
    }
    (out, tasks)
}

#[test]
fn depends_on_and_outputs_cover_turbos_task_definitions() {
    let mut compared = 0;
    let mut tasks = 0;
    let mut skipped = Vec::new();
    let mut wrong = Vec::new();
    let mut native = BTreeSet::new();
    for name in fixtures() {
        let dir = tempfile::tempdir().unwrap();
        setup::copy_fixture(&name, dir.path()).unwrap();
        setup::setup_git(dir.path()).unwrap();
        // A workspace turbo can resolve: the fixture's package manager, else npm.
        let manifest = std::fs::read_to_string(dir.path().join("package.json")).unwrap_or_default();
        if !manifest.is_empty() && !manifest.contains("\"packageManager\"") {
            setup::setup_package_manager(dir.path(), "npm@10.5.0").unwrap();
        }
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let ws = trace_workspace(&root);
        let names = task_names(&root, &ws);
        if names.is_empty() {
            skipped.push(format!("{name}: no tasks"));
            continue;
        }
        let Some(theirs) = turbo_side(&root, &names) else {
            skipped.push(format!("{name}: turbo cannot plan it"));
            continue;
        };
        let (ours, model) = trace_side(&ws);
        compared += 1;
        for (key, t) in &theirs {
            // A task of a uv project, or of the uv workspace (at the root, without a
            // script of that name).
            let dir = root.join(&key.0);
            let script = read_jsonc(&dir.join("package.json"))
                .is_some_and(|m| m["scripts"].get(&key.1).is_some());
            if dir.join("pyproject.toml").is_file() && !script {
                native.insert(format!("{name} {}#{}", key.0, key.1));
                continue;
            }
            tasks += 1;
            // A native package's task is in the trace's task model (its `turbo.json`
            // definition alone only augments a task some source defines).
            let none = Written::new();
            let written = match model.get(key) {
                Some(w) => Some(w),
                None if dir.join("package.json").is_file() => Some(&none),
                None => None,
            };
            let (Some((o, own)), Some(written)) = (ours.get(key), written) else {
                wrong.push(format!(
                    "{name} {key:?}: not read by the trace (turbo: {t:?})"
                ));
                continue;
            };
            // An output turbo derives (not in `turbo.json`) lies in what the task's
            // command writes.
            let output_covered = |g: &String| {
                o.outputs.contains(g) || written.iter().any(|w| Path::new(g).starts_with(w))
            };
            let covered =
                t.depends_on.is_subset(&o.depends_on) && t.outputs.iter().all(output_covered);
            // Outputs the trace adds for nested packages are exclusions (`!<nested>/**`).
            let extra_outputs: BTreeSet<&String> = o
                .outputs
                .difference(&t.outputs)
                .filter(|g| !g.starts_with('!'))
                .collect();
            let equal = t.depends_on == o.depends_on && extra_outputs.is_empty();
            if !covered || (!own && !equal) {
                wrong.push(format!(
                    "{name} {key:?}: turbo {t:?}, trace {o:?} (several definitions: {own})"
                ));
            }
        }
    }
    eprintln!(
        "compared {compared} fixtures, {tasks} tasks; skipped {}:",
        skipped.len()
    );
    for s in &skipped {
        eprintln!("  {s}");
    }
    eprintln!("not compared, tasks of uv projects: {native:?}");
    assert!(compared >= 35, "too few fixtures compared: {compared}");
    assert!(
        wrong.is_empty(),
        "definitions differ:\n{}",
        wrong.join("\n")
    );
}
