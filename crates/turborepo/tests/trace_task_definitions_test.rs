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
//! trace's workspace is that of `package.json` workspaces: the tasks of Cargo
//! and uv packages (turbo's native packages) are listed, not compared.

mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use common::{run_turbo_with_env, setup};
use turborepo_trace_tasks::{
    TaskRef,
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

/// The trace's reading of the same tasks, and whether more than the root's
/// generic definition contributed to each (a package `turbo.json`, a root
/// `<package>#<task>`).
fn trace_side(ws: &Workspace) -> BTreeMap<(String, String), (Declared, bool)> {
    let mut out = BTreeMap::new();
    for pkg in &ws.packages {
        let cx = SourceCx {
            workspace: ws,
            package: pkg,
        };
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
    out
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
        let ws = Workspace::discover(&root);
        let names = task_names(&root, &ws);
        if names.is_empty() {
            skipped.push(format!("{name}: no tasks"));
            continue;
        }
        let Some(theirs) = turbo_side(&root, &names) else {
            skipped.push(format!("{name}: turbo cannot plan it"));
            continue;
        };
        let ours = trace_side(&ws);
        compared += 1;
        for (key, t) in &theirs {
            if !ws.packages.iter().any(|p| p.rel == key.0) {
                native.insert(format!("{name} {}", key.0));
                continue;
            }
            tasks += 1;
            let Some((o, own)) = ours.get(key) else {
                wrong.push(format!(
                    "{name} {key:?}: not read by the trace (turbo: {t:?})"
                ));
                continue;
            };
            let covered = t.depends_on.is_subset(&o.depends_on) && t.outputs.is_subset(&o.outputs);
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
    eprintln!("not compared, packages outside the trace's workspace: {native:?}");
    assert!(compared >= 35, "too few fixtures compared: {compared}");
    assert!(
        wrong.is_empty(),
        "definitions differ:\n{}",
        wrong.join("\n")
    );
}
