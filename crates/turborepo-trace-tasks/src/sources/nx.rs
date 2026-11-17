//! nx (`nx.json`, `project.json`, `package.json` `nx` key).
//!
//! Every target of a project is a task `nx:<project name>:<target>`. Targets
//! merge with `nx.json` `targetDefaults` (by target name and by executor; the
//! project wins).
//!
//! | Field | Becomes |
//! |---|---|
//! | `dependsOn` (`"build"`, `"^build"`, `{ target, projects, dependencies }`) | task dependencies on nx targets (and, for `^`, on the dependencies' package scripts) |
//! | `inputs` (filesets with `{projectRoot}`/`{workspaceRoot}`, `!` negations, named inputs from `namedInputs`, `^named`, `{ env }`, `{ runtime }`, `{ externalDependencies }`) | memberships, env names, toolchain inputs; `^named` covers the project's dependencies (its workspace package dependencies and `implicitDependencies`; every project when they are unknown) |
//! | `outputs` (`{options.outputPath}` etc.), else nx's default output directories | [`crate::spec::Output`]s |
//! | `nx:run-commands` (`command`, `commands`, `cwd`, `envFile`, `env`) | command lines through the registry |
//! | `nx:run-script` | a dependency on the package script |
//! | `@nx/js:tsc`, `@nx/vite:build`/`test`, `@nx/webpack:webpack`, `@nx/jest:jest`, `@nx/next:build`, `@nx/esbuild:esbuild`, `@nx/rollup:rollup`, `@nx/eslint:lint`, `@nx/playwright:playwright`, `@nx/js:swc` (and the `@nrwl/` names) | the equivalent command line (`tsc -p <tsConfig>`, `vite build --config <configFile>`, `jest --config <jestConfig>`, ...) recognized by the existing recognizers, plus `main` entries and `assets` |
//! | other executors | declared inputs (else the `default` named input, else the project directory) as memberships, plus an uncertainty |
//!
//! The [`NxCli`] recognizer turns `nx run web:build`, `nx build web`, `nx
//! run-many -t test` and `nx affected -t lint` into dependencies.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::add_glob,
    registry::{Recognition, RecognizeCx, Recognizer},
    sources::{
        DiscoveredTask, SourceCx, TaskSource, read_json, taskfile::repo_rel, turbo::parse_jsonc,
    },
    spec::{Command, EntryLang, Output, OutputKind, TaskId, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&Nx]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&NxCli]
}

/// The id of target `target` of project `project`.
pub fn target_id(project: &str, target: &str) -> TaskId {
    TaskId::other("nx", format!("{project}:{target}"))
}

/// One nx project.
#[derive(Clone, Debug)]
pub struct Project {
    /// Project name.
    pub name: String,
    /// Project root (absolute).
    pub dir: PathBuf,
    /// The defining file (`project.json` or `package.json`).
    pub file: PathBuf,
    /// Targets.
    pub targets: Map<String, Value>,
    /// Project-level named inputs.
    pub named_inputs: Map<String, Value>,
    /// `implicitDependencies` (project names; `!x` removals are ignored).
    pub implicit_deps: Vec<String>,
    /// `package.json` scripts (for `nx:run-script` inference).
    pub scripts: Vec<String>,
}

fn obj(v: Option<&Value>) -> Map<String, Value> {
    v.and_then(Value::as_object).cloned().unwrap_or_default()
}

fn strs(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Reads `nx.json` at `root`.
pub fn read_nx_json(root: &Path) -> Option<Value> {
    let s = fsx::read_to_string(root.join("nx.json")).ok()?;
    parse_jsonc(&s)
}

/// Loads the project rooted at `dir` (`project.json`, else `package.json` with
/// an `nx` key).
pub fn load_project(dir: &Path) -> Option<Project> {
    let pj = read_json(&dir.join("package.json"));
    let pkg_name = pj
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let scripts: Vec<String> = pj
        .as_ref()
        .and_then(|p| p.get("scripts"))
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let base = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into());
    let project_json = dir.join("project.json");
    if let Ok(s) = fsx::read_to_string(&project_json) {
        let j = parse_jsonc(&s).unwrap_or(Value::Null);
        // A package.json `nx` key adds to project.json.
        let mut targets = obj(pj
            .as_ref()
            .and_then(|p| p.get("nx"))
            .and_then(|n| n.get("targets")));
        targets.extend(obj(j.get("targets")));
        return Some(Project {
            name: j
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(pkg_name)
                .unwrap_or(base),
            dir: dir.to_path_buf(),
            file: project_json,
            targets,
            named_inputs: obj(j.get("namedInputs")),
            implicit_deps: strs(j.get("implicitDependencies"))
                .into_iter()
                .filter(|d| !d.starts_with('!'))
                .collect(),
            scripts,
        });
    }
    let nx = pj.as_ref()?.get("nx")?;
    Some(Project {
        name: nx
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(pkg_name)
            .unwrap_or(base),
        dir: dir.to_path_buf(),
        file: dir.join("package.json"),
        targets: obj(nx.get("targets")),
        named_inputs: obj(nx.get("namedInputs")),
        implicit_deps: strs(nx.get("implicitDependencies"))
            .into_iter()
            .filter(|d| !d.starts_with('!'))
            .collect(),
        scripts,
    })
}

/// Every project of the nx workspace at `root` (directories with
/// `project.json`, or a `package.json` with an `nx` key), up to 6 levels deep,
/// sorted by directory.
pub fn index(root: &Path) -> Vec<Project> {
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Project>) {
    if let Some(p) = load_project(dir) {
        out.push(p);
    }
    if depth >= 6 {
        return;
    }
    let Some(entries) = fsx::read_dir(dir) else {
        return;
    };
    for (p, is_dir) in entries {
        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_dir
            && !fsx::skip_dir(n)
            && !n.starts_with('.')
            && !matches!(n, "dist" | "build" | "target" | "tmp" | "coverage" | "out")
        {
            walk(&p, depth + 1, out);
        }
    }
}

/// The nx workspace of a source context: root, `nx.json` contents.
struct Ws<'a> {
    root: &'a Path,
    nx_json: Value,
    nx_file: PathBuf,
}

/// nx projects.
pub struct Nx;

impl TaskSource for Nx {
    fn id(&self) -> &'static str {
        "nx"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["nx.json", "project.json", "package.json"]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let root = &cx.workspace.root;
        let nx_json = read_nx_json(root);
        // Not an nx workspace and no project file here: nothing to find. Checked before
        // the walk below, which otherwise lists the whole package tree (the
        // repository for the root package) on every non-nx repository.
        if nx_json.is_none() && !fsx::is_file(&cx.package.dir.join("project.json")) {
            return Vec::new();
        }
        // Project files of this package (its own root and non-package subdirectories).
        let mut dirs: Vec<PathBuf> = cx
            .find_files(4, &|n| n == "project.json")
            .into_iter()
            .filter_map(|p| p.parent().map(Path::to_path_buf))
            .collect();
        if !dirs.contains(&cx.package.dir) {
            dirs.push(cx.package.dir.clone());
        }
        let ws = Ws {
            root,
            nx_json: nx_json.unwrap_or(Value::Null),
            nx_file: root.join("nx.json"),
        };
        let mut all: Option<Vec<Project>> = None;
        let mut out = Vec::new();
        for d in dirs {
            let Some(p) = load_project(&d) else { continue };
            // Projects inferred from package.json alone (no `nx` key) have no explicit
            // targets here: their scripts are npm tasks.
            if p.targets.is_empty() {
                continue;
            }
            let deps = dep_projects(cx, &p, &mut all, root);
            for (tname, def) in &p.targets {
                out.extend(target(&ws, cx, &p, &deps, tname, def));
            }
        }
        crate::sources::gha::script::sanitize_lines(&mut out);
        out
    }
}

/// The projects `p` depends on: workspace package dependencies plus implicit
/// dependencies; `None` when they cannot be known (not a JS package): every
/// project.
fn dep_projects(
    cx: &SourceCx<'_>,
    p: &Project,
    all: &mut Option<Vec<Project>>,
    root: &Path,
) -> Vec<Project> {
    let all = all.get_or_insert_with(|| index(root));
    let mut out: Vec<Project> = Vec::new();
    let pkg = cx.workspace.packages.iter().find(|k| k.dir == p.dir);
    match pkg {
        Some(pkg) => {
            for d in cx.workspace.transitive_deps(pkg) {
                if let Some(x) = all.iter().find(|x| x.dir == d.dir) {
                    out.push(x.clone());
                } else {
                    // A package without nx configuration: its package.json scripts are its
                    // targets; its files count for `^` inputs.
                    out.push(Project {
                        name: d.name.clone().unwrap_or_else(|| d.rel.clone()),
                        dir: d.dir.clone(),
                        file: d.dir.join("package.json"),
                        targets: Map::new(),
                        named_inputs: Map::new(),
                        implicit_deps: Vec::new(),
                        scripts: Vec::new(),
                    });
                }
            }
        }
        None => {
            out.extend(all.iter().filter(|x| x.dir != p.dir).cloned());
        }
    }
    for n in &p.implicit_deps {
        if n == "*" {
            out.extend(all.iter().filter(|x| x.dir != p.dir).cloned());
        } else if let Some(x) = all.iter().find(|x| &x.name == n) {
            out.push(x.clone());
        }
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out.dedup_by(|a, b| a.dir == b.dir);
    out
}

/// Merges `targetDefaults` for `name` / `executor` under the project's
/// definition.
fn merged_target(ws: &Ws<'_>, name: &str, def: &Value) -> Value {
    let defaults = obj(ws.nx_json.get("targetDefaults"));
    let mut m = Map::new();
    let executor = def.get("executor").and_then(Value::as_str);
    let mut layers: Vec<&Value> = Vec::new();
    if let Some(e) = executor
        && let Some(d) = defaults.get(e)
    {
        layers.push(d);
    }
    if let Some(d) = defaults.get(name) {
        layers.push(d);
    }
    layers.push(def);
    for l in layers {
        if let Some(o) = l.as_object() {
            for (k, v) in o {
                if k == "options"
                    && let (Some(Value::Object(prev)), Some(new)) =
                        (m.get_mut("options"), v.as_object())
                {
                    prev.extend(new.clone());
                    continue;
                }
                m.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(m)
}

/// `{projectRoot}`, `{projectName}`, `{workspaceRoot}` and `{options.x}` in
/// `s`; unknown tokens (`{args.x}`) become an unknown shell variable.
fn interpolate(
    s: &str,
    p: &Project,
    ws_root: &Path,
    options: &Map<String, Value>,
    abs_root: bool,
) -> (String, bool) {
    let prel = repo_rel(ws_root, &p.dir);
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut known = true;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let tok = &after[..j];
        let v = match tok {
            "projectRoot" => Some(prel.clone()),
            "projectName" => Some(p.name.clone()),
            "workspaceRoot" => Some(if abs_root {
                ws_root.to_string_lossy().into_owned()
            } else {
                String::new()
            }),
            t => t
                .strip_prefix("options.")
                .and_then(|k| options.get(k))
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        match v {
            Some(v) => out.push_str(&v),
            None if tok
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
                && !tok.is_empty() =>
            {
                known = false;
                out.push_str("${__NX_UNKNOWN}");
            }
            None => {
                out.push('{');
                out.push_str(tok);
                out.push('}');
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    (out, known)
}

/// A workspace-relative path value (`{workspaceRoot}/x`, `{projectRoot}/x`,
/// `apps/web/x`) made absolute.
fn ws_path(s: &str, p: &Project, ws_root: &Path, options: &Map<String, Value>) -> Option<PathBuf> {
    let (v, ok) = interpolate(s, p, ws_root, options, false);
    ok.then(|| fsx::join(ws_root, v.trim_start_matches('/')))
}

/// Quotes an absolute path for a command line.
fn q(p: &Path) -> String {
    let s = p.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Resolves one `inputs` entry into `spec`.
#[allow(clippy::too_many_arguments)]
fn add_input(
    ws: &Ws<'_>,
    p: &Project,
    deps: &[Project],
    input: &Value,
    spec: &mut TaskSpec,
    negations: &mut Vec<(PathBuf, String)>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    match input {
        Value::String(s) => {
            if let Some(n) = s.strip_prefix('^') {
                for d in deps {
                    add_named(ws, d, &[], n, spec, negations, depth + 1);
                }
            } else if s.contains('{') || s.contains('/') || s.contains('*') || s.starts_with('!') {
                fileset(ws, p, s, spec, negations);
            } else {
                add_named(ws, p, deps, s, spec, negations, depth + 1);
            }
        }
        Value::Object(o) => {
            if let Some(f) = o.get("fileset").and_then(Value::as_str) {
                fileset(ws, p, f, spec, negations);
            }
            if let Some(e) = o.get("env").and_then(Value::as_str) {
                spec.env.push(e.to_string());
            }
            if let Some(r) = o.get("runtime").and_then(Value::as_str) {
                spec.tool("system", r);
            }
            for e in strs(o.get("externalDependencies")) {
                spec.tool("npm", e);
            }
            if let Some(n) = o.get("input").and_then(Value::as_str) {
                let to_deps = o.get("dependencies").and_then(Value::as_bool) == Some(true)
                    || o.get("projects").and_then(Value::as_str) == Some("dependencies");
                if to_deps {
                    for d in deps {
                        add_named(ws, d, &[], n, spec, negations, depth + 1);
                    }
                } else {
                    add_named(ws, p, deps, n, spec, negations, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// Resolves named input `name` in project `p` (project `namedInputs`, else
/// `nx.json`). An undefined `default` is the whole project.
fn add_named(
    ws: &Ws<'_>,
    p: &Project,
    deps: &[Project],
    name: &str,
    spec: &mut TaskSpec,
    negations: &mut Vec<(PathBuf, String)>,
    depth: usize,
) {
    let global = obj(ws.nx_json.get("namedInputs"));
    let def = p
        .named_inputs
        .get(name)
        .or_else(|| global.get(name))
        .cloned();
    match def {
        Some(Value::Array(items)) => {
            for i in &items {
                add_input(ws, p, deps, i, spec, negations, depth + 1);
            }
        }
        _ => {
            // nx: an unknown named input (and a missing `default`) means every file of the
            // project.
            spec.membership(p.dir.clone(), "**", format!("nx input `{name}`"));
        }
    }
}

/// A fileset pattern.
fn fileset(
    ws: &Ws<'_>,
    p: &Project,
    s: &str,
    spec: &mut TaskSpec,
    negations: &mut Vec<(PathBuf, String)>,
) {
    let (neg, pat) = match s.strip_prefix('!') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (base, rest) = if let Some(r) = pat.strip_prefix("{projectRoot}") {
        (p.dir.clone(), r.trim_start_matches('/'))
    } else if let Some(r) = pat.strip_prefix("{workspaceRoot}") {
        (ws.root.to_path_buf(), r.trim_start_matches('/'))
    } else {
        (p.dir.clone(), pat)
    };
    let rest = if rest.is_empty() { "**" } else { rest };
    if neg {
        negations.push((base, rest.to_string()));
    } else {
        add_glob(&base, rest, "nx inputs", spec);
    }
}

/// The tasks of one target.
fn target(
    ws: &Ws<'_>,
    cx: &SourceCx<'_>,
    p: &Project,
    deps: &[Project],
    tname: &str,
    def: &Value,
) -> Vec<DiscoveredTask> {
    let def = merged_target(ws, tname, def);
    let options = obj(def.get("options"));
    let id = target_id(&p.name, tname);
    let mut base = DiscoveredTask::new(id.clone(), tname, ws.root.to_path_buf());
    base.files.push(p.file.clone());
    if fsx::is_file(&ws.nx_file) {
        base.files.push(ws.nx_file.clone());
    }
    let mut spec_v = TaskSpec::default();
    let mut env_v = std::collections::BTreeMap::new();
    let spec = &mut spec_v;
    // dependsOn.
    for d in def
        .get("dependsOn")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let (t, mode, list): (String, &str, Vec<String>) = match &d {
            Value::String(s) => match s.strip_prefix('^') {
                Some(n) => (n.to_string(), "deps", Vec::new()),
                None => (s.clone(), "self", Vec::new()),
            },
            Value::Object(o) => {
                let t = o
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if o.get("dependencies").and_then(Value::as_bool) == Some(true) {
                    (t, "deps", Vec::new())
                } else {
                    match o.get("projects") {
                        Some(Value::String(s)) if s == "dependencies" => (t, "deps", Vec::new()),
                        Some(Value::String(s)) if s == "self" => (t, "self", Vec::new()),
                        Some(v) => (t, "list", strs(Some(v))),
                        None => (t, "self", Vec::new()),
                    }
                }
            }
            _ => continue,
        };
        if t.is_empty() {
            continue;
        }
        match mode {
            "self" => spec.dep(
                TaskRef::Id {
                    id: target_id(&p.name, &t),
                },
                "nx dependsOn",
            ),
            "deps" => {
                for d in deps {
                    spec.dep(
                        TaskRef::Id {
                            id: target_id(&d.name, &t),
                        },
                        "nx dependsOn ^",
                    );
                }
                spec.dep(TaskRef::Upstream { name: t.clone() }, "nx dependsOn ^");
            }
            _ => {
                for n in list {
                    if n == "*" || crate::glob::is_glob(&n) {
                        for x in index(ws.root) {
                            if crate::glob::matches(&n, &x.name) {
                                spec.dep(
                                    TaskRef::Id {
                                        id: target_id(&x.name, &t),
                                    },
                                    "nx dependsOn",
                                );
                            }
                        }
                    } else {
                        spec.dep(
                            TaskRef::Id {
                                id: target_id(&n, &t),
                            },
                            "nx dependsOn",
                        );
                    }
                }
            }
        }
    }
    // Declared inputs.
    let mut negations = Vec::new();
    let declared = def.get("inputs").and_then(Value::as_array).cloned();
    if let Some(inputs) = &declared {
        for i in inputs {
            add_input(ws, p, deps, i, spec, &mut negations, 0);
        }
    }
    // The executor.
    let executor = def
        .get("executor")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| def.get("command").map(|_| "nx:run-commands".to_string()))
        .or_else(|| {
            p.scripts
                .iter()
                .any(|s| s == tname)
                .then(|| "nx:run-script".to_string())
        })
        .unwrap_or_else(|| "nx:noop".to_string());
    let exec_short = executor
        .strip_prefix("@nrwl/")
        .map(|r| format!("@nx/{r}"))
        .unwrap_or_else(|| executor.clone());
    // Outputs: declared, else nx's default output directories for executors that
    // build (not for scripts, tests and linters); plus `outputPath`-like
    // options.
    let builds = !matches!(
        exec_short.as_str(),
        "nx:run-script"
            | "nx:noop"
            | "@nx/jest:jest"
            | "@nx/vite:test"
            | "@nx/vitest:test"
            | "@nx/eslint:lint"
            | "@nx/linter:eslint"
            | "@nx/playwright:playwright"
    );
    let mut outs: Vec<String> = strs(def.get("outputs"));
    if def.get("outputs").is_none() && builds {
        outs.extend(
            [
                "{workspaceRoot}/dist/{projectRoot}",
                "{projectRoot}/dist",
                "{projectRoot}/build",
                "{projectRoot}/public",
            ]
            .map(str::to_string),
        );
    }
    for k in ["outputPath", "outputFile", "reportsDirectory"] {
        if let Some(v) = options.get(k).and_then(Value::as_str) {
            outs.push(v.to_string());
        }
    }
    for o in outs {
        let Some(abs) = ws_path(&o, p, ws.root, &options) else {
            continue;
        };
        let s = abs.to_string_lossy().into_owned();
        let (prefix, rest) = crate::glob::split_static_prefix(&s);
        spec.outputs.push(Output {
            path: fsx::normalize(Path::new(&format!("/{}", prefix.trim_start_matches('/')))),
            pattern: (!rest.is_empty()).then(|| rest.to_string()),
            exclude: Vec::new(),
            kind: OutputKind::Declared,
            part: None,
        });
    }
    let path_opt = |k: &str| -> Option<PathBuf> {
        options
            .get(k)
            .and_then(Value::as_str)
            .and_then(|v| ws_path(v, p, ws.root, &options))
    };
    let mut lines: Vec<(PathBuf, String)> = Vec::new();
    let pdir = p.dir.clone();
    let find_cfg = |names: &[&str]| fsx::first_existing(&pdir, names);
    match exec_short.as_str() {
        "nx:noop" => {}
        "nx:run-commands" => {
            let cwd = options
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|c| ws_path(c, p, ws.root, &options))
                .unwrap_or_else(|| ws.root.to_path_buf());
            let mut cmds: Vec<String> = Vec::new();
            if let Some(c) = def.get("command").and_then(Value::as_str) {
                cmds.push(c.to_string());
            }
            if let Some(c) = options.get("command").and_then(Value::as_str) {
                cmds.push(c.to_string());
            }
            for c in options
                .get("commands")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                match c {
                    Value::String(s) => cmds.push(s),
                    Value::Object(o) => {
                        if let Some(s) = o.get("command").and_then(Value::as_str) {
                            cmds.push(s.to_string());
                        }
                    }
                    _ => {}
                }
            }
            for c in cmds {
                lines.push((cwd.clone(), interpolate(&c, p, ws.root, &options, true).0));
            }
            if let Some(e) = path_opt("envFile") {
                spec.input(e);
            }
            if let Some(Value::Object(env)) = options.get("env") {
                for (k, v) in env {
                    if let Some(s) = v.as_str() {
                        env_v.insert(k.clone(), s.to_string());
                    }
                }
            }
        }
        "nx:run-script" => {
            let script = options
                .get("script")
                .and_then(Value::as_str)
                .unwrap_or(tname)
                .to_string();
            spec.dep(
                TaskRef::Package {
                    package: repo_rel(ws.root, &p.dir),
                    name: script,
                },
                "nx:run-script",
            );
        }
        "@nx/js:tsc" | "@nx/node:package" => {
            let ts =
                path_opt("tsConfig").or_else(|| find_cfg(&["tsconfig.lib.json", "tsconfig.json"]));
            if let Some(ts) = ts {
                lines.push((ws.root.to_path_buf(), format!("tsc -p {}", q(&ts))));
            }
        }
        "@nx/js:swc" => {
            if let Some(ts) = path_opt("tsConfig") {
                spec.config(ts);
            }
            spec.config(p.dir.join(".swcrc"));
            spec.membership(
                p.dir.clone(),
                "**/*.{ts,tsx,js,jsx,mts,cts,mjs,cjs}",
                "nx swc sources",
            );
            spec.tool("npm", "@swc/core");
        }
        "@nx/vite:build" | "@nx/vite:test" | "@nx/vitest:test" => {
            let cfg = path_opt("configFile").or_else(|| {
                find_cfg(&[
                    "vite.config.ts",
                    "vite.config.mts",
                    "vite.config.js",
                    "vite.config.mjs",
                    "vitest.config.ts",
                    "vitest.config.mts",
                ])
            });
            let prog = if exec_short.ends_with("build") {
                "vite build"
            } else {
                "vitest run"
            };
            let line = match cfg {
                Some(c) => format!("{prog} --config {}", q(&c)),
                None => prog.to_string(),
            };
            lines.push((p.dir.clone(), line));
        }
        "@nx/webpack:webpack" => {
            let line = match path_opt("webpackConfig")
                .or_else(|| find_cfg(&["webpack.config.js", "webpack.config.ts"]))
            {
                Some(c) => format!("webpack --config {}", q(&c)),
                None => "webpack".to_string(),
            };
            lines.push((p.dir.clone(), line));
            if let Some(ts) = path_opt("tsConfig") {
                spec.config(ts);
            }
            if let Some(i) = path_opt("index") {
                spec.input(i);
            }
        }
        "@nx/jest:jest" => {
            let line = match path_opt("jestConfig").or_else(|| {
                find_cfg(&[
                    "jest.config.ts",
                    "jest.config.js",
                    "jest.config.cjs",
                    "jest.config.mjs",
                ])
            }) {
                Some(c) => format!("jest --config {}", q(&c)),
                None => "jest".to_string(),
            };
            lines.push((p.dir.clone(), line));
        }
        "@nx/next:build" => {
            lines.push((p.dir.clone(), "next build".to_string()));
        }
        "@nx/esbuild:esbuild" => {
            let mut line = String::from("esbuild");
            if let Some(m) = path_opt("main") {
                line.push_str(&format!(" {}", q(&m)));
            }
            for e in strs(options.get("additionalEntryPoints")) {
                if let Some(m) = ws_path(&e, p, ws.root, &options) {
                    line.push_str(&format!(" {}", q(&m)));
                }
            }
            line.push_str(" --bundle");
            if let Some(ts) = path_opt("tsConfig") {
                line.push_str(&format!(" --tsconfig={}", q(&ts)));
            }
            if let Some(o) = path_opt("outputPath") {
                line.push_str(&format!(" --outdir={}", q(&o)));
            }
            lines.push((ws.root.to_path_buf(), line));
        }
        "@nx/rollup:rollup" => {
            let mut line = String::from("rollup");
            for c in strs(options.get("rollupConfig")) {
                if let Some(c) = ws_path(&c, p, ws.root, &options) {
                    line.push_str(&format!(" -c {}", q(&c)));
                }
            }
            lines.push((p.dir.clone(), line));
            if let Some(ts) = path_opt("tsConfig") {
                spec.config(ts);
            }
        }
        "@nx/eslint:lint" | "@nx/linter:eslint" => {
            let mut pats = strs(options.get("lintFilePatterns"));
            if pats.is_empty() {
                pats.push("{projectRoot}".into());
            }
            let mut line = String::from("eslint");
            for pt in pats {
                let (v, _) = interpolate(&pt, p, ws.root, &options, false);
                line.push(' ');
                line.push_str(&format!("'{}'", v.replace('\'', "'\\''")));
            }
            if let Some(c) = path_opt("eslintConfig") {
                line.push_str(&format!(" -c {}", q(&c)));
            }
            lines.push((ws.root.to_path_buf(), line));
        }
        "@nx/playwright:playwright" => {
            let line = match path_opt("config")
                .or_else(|| find_cfg(&["playwright.config.ts", "playwright.config.js"]))
            {
                Some(c) => format!("playwright test --config {}", q(&c)),
                None => "playwright test".to_string(),
            };
            lines.push((p.dir.clone(), line));
        }
        other => {
            spec.uncertain(
                UncertaintyKind::UnknownCommand,
                format!("nx executor `{other}`"),
            );
            if declared.is_none() {
                let global = obj(ws.nx_json.get("namedInputs"));
                if p.named_inputs.contains_key("default") || global.contains_key("default") {
                    add_named(ws, p, deps, "default", spec, &mut negations, 0);
                } else {
                    spec.membership(p.dir.clone(), "**", format!("nx executor `{other}`"));
                }
            }
        }
    }
    // Path-valued options common to the build executors.
    if exec_short.starts_with("@nx/") {
        if let Some(m) = path_opt("main") {
            let lang = EntryLang::of(&m);
            spec.entry(
                m,
                if lang == EntryLang::Auto {
                    EntryLang::Js
                } else {
                    lang
                },
            );
        }
        for a in options
            .get("assets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            match a {
                Value::String(s) => {
                    if let Some(abs) = ws_path(&s, p, ws.root, &options) {
                        let rel = repo_rel(ws.root, &abs);
                        add_glob(ws.root, &rel, "nx assets", spec);
                    }
                }
                Value::Object(o) => {
                    let input = o.get("input").and_then(Value::as_str).unwrap_or(".");
                    let glob = o.get("glob").and_then(Value::as_str).unwrap_or("**");
                    if let Some(abs) = ws_path(input, p, ws.root, &options) {
                        add_glob(&abs, glob, "nx assets", spec);
                    }
                }
                _ => {}
            }
        }
        // Configurations: their path options and file replacements.
        for (_, c) in obj(def.get("configurations")) {
            for k in [
                "tsConfig",
                "jestConfig",
                "configFile",
                "webpackConfig",
                "main",
            ] {
                if let Some(v) = c.get(k).and_then(Value::as_str)
                    && let Some(abs) = ws_path(v, p, ws.root, &options)
                {
                    spec.config(abs);
                }
            }
            for r in c
                .get("fileReplacements")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                for k in ["replace", "with"] {
                    if let Some(v) = r.get(k).and_then(Value::as_str)
                        && let Some(abs) = ws_path(v, p, ws.root, &options)
                    {
                        spec.input(abs);
                    }
                }
            }
        }
    }
    for (b, n) in negations {
        add_glob(&b, &format!("!{n}"), "nx inputs", spec);
    }
    let _ = cx;
    base.spec = spec_v;
    base.env = env_v;
    let mut out = vec![base];
    for (cwd, line) in lines {
        let mut t = DiscoveredTask::new(id.clone(), tname, cwd);
        t.files = out[0].files.clone();
        t.env = out[0].env.clone();
        t.lines.push(line);
        out.push(t);
    }
    out
}

/// `nx run p:t`, `nx run-many -t a -p x`, `nx affected -t a`, `nx <target>
/// [project]`.
pub struct NxCli;

impl Recognizer for NxCli {
    fn id(&self) -> &'static str {
        "nx"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["nx"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "nx");
        let words: Vec<&str> = cmd.args().take_while(|w| *w != "--").collect();
        let Some(&sub) = words.iter().find(|w| !w.starts_with('-')) else {
            return Recognition::Ignored;
        };
        let root = cmd
            .cwd
            .ancestors()
            .take_while(|d| d.starts_with(cx.repo_root))
            .find(|d| fsx::is_file(&d.join("nx.json")))
            .unwrap_or(cx.repo_root)
            .to_path_buf();
        // Option values: `-t a b`, `--targets=a,b`, `--target a`, `-p x y`,
        // `--projects=x`.
        let values = |names: &[&str]| -> Vec<String> {
            let mut out = Vec::new();
            let mut on = false;
            for w in &words {
                if let Some((k, v)) = w.split_once('=')
                    && names.contains(&k)
                {
                    out.extend(v.split(',').map(str::to_string));
                    on = false;
                    continue;
                }
                if names.contains(w) {
                    on = true;
                    continue;
                }
                if w.starts_with('-') {
                    on = false;
                    continue;
                }
                if on {
                    out.extend(w.split(',').map(str::to_string));
                }
            }
            out
        };
        let add = |out: &mut TaskSpec, project: &str, t: &str| {
            out.dep(
                TaskRef::Id {
                    id: target_id(project, t),
                },
                "nx",
            );
            out.dep(
                TaskRef::Package {
                    package: project.to_string(),
                    name: t.to_string(),
                },
                "nx",
            );
        };
        let add_all = |out: &mut TaskSpec, t: &str, filter: &[String]| {
            for x in index(&root) {
                if filter.is_empty() || filter.iter().any(|f| crate::glob::matches(f, &x.name)) {
                    out.dep(
                        TaskRef::Id {
                            id: target_id(&x.name, t),
                        },
                        "nx",
                    );
                }
            }
            if filter.is_empty() {
                out.dep(
                    TaskRef::All {
                        name: t.to_string(),
                    },
                    "nx",
                );
            } else {
                for f in filter {
                    out.dep(
                        TaskRef::Package {
                            package: f.clone(),
                            name: t.to_string(),
                        },
                        "nx",
                    );
                }
            }
        };
        match sub {
            "run" => {
                let Some(spec) = words.iter().skip_while(|w| **w != "run").nth(1) else {
                    return Recognition::Ignored;
                };
                let mut parts = spec.split(':');
                match (parts.next(), parts.next()) {
                    (Some(p), Some(t)) => add(out, p, t),
                    _ => {
                        out.uncertain(UncertaintyKind::UnknownCommand, format!("nx run {spec}"));
                        return Recognition::Partial;
                    }
                }
            }
            "run-many" | "affected" | "affected:build" | "affected:test" | "affected:lint" => {
                let mut targets = values(&["-t", "--target", "--targets"]);
                if let Some(t) = sub.strip_prefix("affected:") {
                    targets.push(t.to_string());
                }
                let projects = if sub == "run-many" {
                    values(&["-p", "--projects"])
                } else {
                    Vec::new()
                };
                for t in targets {
                    add_all(out, &t, &projects);
                }
            }
            "graph" | "dep-graph" | "format" | "format:check" | "format:write" | "reset"
            | "daemon" | "show" | "list" | "report" | "migrate" | "generate" | "g" | "init"
            | "add" | "connect" | "release" | "sync" | "sync:check" | "watch" | "repair"
            | "view-logs" | "print-affected" | "exec" | "import" | "login" | "logout"
            | "--version" | "help" => return Recognition::Ignored,
            t => {
                // `nx build web` / `nx web:build` / `nx build` (project of the cwd).
                if let Some((p, tg)) = t.split_once(':') {
                    add(out, p, tg);
                } else {
                    let pos: Vec<&&str> = words.iter().filter(|w| !w.starts_with('-')).collect();
                    match pos.get(1) {
                        Some(p) => add(out, p, t),
                        None => {
                            let proj = cmd
                                .cwd
                                .ancestors()
                                .take_while(|d| d.starts_with(&root))
                                .find_map(load_project);
                            match proj {
                                Some(p) => add(out, &p.name, t),
                                None => add_all(out, t, &[]),
                            }
                        }
                    }
                }
            }
        }
        Recognition::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation() {
        let p = Project {
            name: "web".into(),
            dir: PathBuf::from("/r/apps/web"),
            file: PathBuf::from("/r/apps/web/project.json"),
            targets: Map::new(),
            named_inputs: Map::new(),
            implicit_deps: Vec::new(),
            scripts: Vec::new(),
        };
        let mut o = Map::new();
        o.insert("outputPath".into(), Value::String("dist/x".into()));
        let r = Path::new("/r");
        assert_eq!(
            interpolate(
                "tsc -p {projectRoot}/tsconfig.json {args.x}",
                &p,
                r,
                &o,
                true
            ),
            (
                "tsc -p apps/web/tsconfig.json ${__NX_UNKNOWN}".into(),
                false
            )
        );
        assert_eq!(
            ws_path("{options.outputPath}", &p, r, &o),
            Some(PathBuf::from("/r/dist/x"))
        );
        assert_eq!(
            ws_path("{workspaceRoot}/dist/{projectRoot}", &p, r, &o),
            Some(PathBuf::from("/r/dist/apps/web"))
        );
        // JSON in a command is left alone.
        assert!(interpolate("echo '{\"a\": 1}'", &p, r, &o, true).1);
    }
}
