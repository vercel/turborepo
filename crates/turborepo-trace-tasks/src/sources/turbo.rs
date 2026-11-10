//! `turbo.json` / `turbo.jsonc`: the task graph and declared inputs.
//!
//! The root config's `tasks` (turbo 2) or `pipeline` (turbo 1) apply to every
//! package that has the script (`build`), to one package (`web#build`) or to
//! the root (`//#build`). A package-level `turbo.json` (`"extends": ["//"]`)
//! adds to them. Every turbo contribution
//! [augments](super::Contribution::Augments) the package script of the same
//! name, so tasks without a script are dropped (as turbo does), while `^`
//! references still reach through them (resolved transitively).
//!
//! | Field | Becomes |
//! |---|---|
//! | `dependsOn: ["^build", "lint", "web#gen"]` | [`TaskRef::Upstream`], [`TaskRef::Local`], [`TaskRef::Package`] (`$VAR`, turbo 1: env) |
//! | `inputs` (globs, `!negations`, `$TURBO_DEFAULT$`, `$TURBO_ROOT$`) | memberships of the package directory; `$TURBO_DEFAULT$` is the whole package |
//! | `outputs` (globs, `!negations`) | [`Output`]s, scoped to the package (below) |
//! | `env`, `passThroughEnv`, `globalEnv`, `globalPassThroughEnv` | env names |
//! | `persistent: true` | [`TaskSpec::persistent`]: its outputs don't link |
//! | `globalDependencies` | inputs of every task (relative to the repo root) |
//!
//! Declared inputs are never dropped: recognized inputs only add to them.
//! Package-level values are unioned with the root's rather than replacing them
//! (an over-approximation). Without declared `inputs`, nothing is added: the
//! recognized commands decide (turbo's default, the whole package, is what
//! unknown commands fall back to).
//!
//! Declared `outputs` link producers only within the declaring package, minus
//! nested workspace packages ([`scope_outputs`]). A file elsewhere is produced
//! by the tasks of the package that contains it; a glob reaching there
//! (`$TURBO_ROOT$/**/dist/**/*`, a task collecting its dependencies' `dist/`
//! for the cache) is kept as [`OutputKind::CacheOnly`], which doesn't link.
//! Otherwise such a task "produces" every `dist/` in the repository, and every
//! test that resolves through one depends on it.

use std::path::{Path, PathBuf};

use serde_json::Value;
use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::add_glob,
    registry::{Recognition, RecognizeCx, Recognizer},
    sources::{Contribution, DiscoveredTask, SourceCx, TaskSource, Workspace, npm::filter_ref},
    spec::{Command, Output, OutputKind, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&Turbo]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&TurboRun]
}

/// `turbo.json` files.
pub struct Turbo;

/// Reads `turbo.json` or `turbo.jsonc` in `dir`: the path and the parsed value.
pub fn read_turbo(dir: &Path) -> Option<(PathBuf, Option<Value>)> {
    for n in ["turbo.json", "turbo.jsonc"] {
        let p = dir.join(n);
        if fsx::is_file(&p) {
            let v = fsx::read_to_string(&p).ok().and_then(|s| parse_jsonc(&s));
            return Some((p, v));
        }
    }
    None
}

/// Parses JSON with comments and trailing commas.
pub fn parse_jsonc(s: &str) -> Option<Value> {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    while i < c.len() {
        let ch = c[i];
        if in_str {
            out.push(ch);
            if ch == '\\' && i + 1 < c.len() {
                out.push(c[i + 1]);
                i += 2;
                continue;
            }
            if ch == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match ch {
            '"' => {
                in_str = true;
                out.push('"');
                i += 1;
            }
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            ',' => {
                // Drop a trailing comma.
                let mut j = i + 1;
                while j < c.len() && c[j].is_whitespace() {
                    j += 1;
                }
                if !matches!(c.get(j), Some('}' | ']')) {
                    out.push(',');
                }
                i += 1;
            }
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }
    serde_json::from_str(&out).ok()
}

/// The task table of a turbo config (`tasks`, else `pipeline`).
fn tasks_of(v: &Value) -> Option<&serde_json::Map<String, Value>> {
    v.get("tasks")
        .or_else(|| v.get("pipeline"))
        .and_then(|t| t.as_object())
}

// ---------------------------------------------------------------------------------------
// Sections: what of a turbo config one task depends on

/// File names of turbo configs.
pub const CONFIG_NAMES: &[&str] = &["turbo.json", "turbo.jsonc"];

/// Suffix of the section holding everything but the task table
/// (`globalDependencies`, `globalEnv`, `extends`, ...): an input of every task
/// the config governs.
pub const GLOBAL_SECTION: &str = "#global";

/// Whether `path` (any form, `/`-separated) names a turbo config.
pub fn is_config(path: &str) -> bool {
    CONFIG_NAMES.contains(&path.rsplit('/').next().unwrap_or(path))
}

/// The suffix of the section of a turbo config that defines the tasks named
/// `name`: the entries `name`, `<package>#name` and `//#name` of the task
/// table.
///
/// A task does not depend on its turbo config as a whole. Editing the `lint`
/// entry of the root `turbo.json` changes nothing for `build` tasks, so the
/// graph store records, per task, the pseudo-paths `<config>#global` and
/// `<config>#tasks/<name>` as inputs instead of the file (the file's contents
/// still key the package's shard). A change of the config is translated into
/// changes of the sections that differ ([`changed_sections`]).
pub fn task_section(name: &str) -> String {
    format!("#tasks/{}", name.replace('%', "%25").replace('/', "%2F"))
}

/// Splits a section pseudo-path (`turbo.json#tasks/build`) into the config path
/// and the section suffix. `None` for any other path.
pub fn split_section(path: &str) -> Option<(&str, &str)> {
    path.match_indices('#')
        .map(|(i, _)| path.split_at(i))
        .find(|(file, section)| {
            is_config(file) && (*section == GLOBAL_SECTION || section.starts_with("#tasks/"))
        })
}

/// The task name of a task table key (`build`, `web#build`, `//#build`).
fn key_name(key: &str) -> &str {
    key.rsplit_once('#').map_or(key, |(_, n)| n)
}

/// The config without its task table (the global section's value).
fn global_value(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("tasks");
        o.remove("pipeline");
    }
    v
}

/// The task table entries of the tasks named `name`, by key (sorted).
fn task_values<'a>(v: &'a Value, name: &str) -> Vec<(&'a String, &'a Value)> {
    let mut out: Vec<(&String, &Value)> = tasks_of(v)
        .into_iter()
        .flatten()
        .filter(|(k, _)| key_name(k) == name)
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// The section suffixes of a turbo config that differ between two versions of
/// its text (`None`: the file does not exist on that side, or the version is
/// not known). Formatting and comments don't count. Without both versions, or
/// when one does not parse, only [`GLOBAL_SECTION`] is returned: every task the
/// config governs depends on it.
pub fn changed_sections(old: Option<&str>, new: Option<&str>) -> Vec<String> {
    let parsed = |s: Option<&str>| s.and_then(parse_jsonc);
    let (Some(old), Some(new)) = (parsed(old), parsed(new)) else {
        return vec![GLOBAL_SECTION.to_string()];
    };
    let mut out = Vec::new();
    if global_value(&old) != global_value(&new) {
        out.push(GLOBAL_SECTION.to_string());
    }
    let mut names: Vec<&str> = [&old, &new]
        .into_iter()
        .flat_map(tasks_of)
        .flat_map(|t| t.keys())
        .map(|k| key_name(k))
        .collect();
    names.sort_unstable();
    names.dedup();
    for n in names {
        if task_values(&old, n) != task_values(&new, n) {
            out.push(task_section(n));
        }
    }
    out
}

/// A canonical text of one section of a turbo config (for fingerprints): it
/// changes exactly when [`changed_sections`] reports the section. `None` when
/// the config does not parse.
pub fn section_text(config: &str, section: &str) -> Option<String> {
    let v = parse_jsonc(config)?;
    if section == GLOBAL_SECTION {
        return Some(global_value(&v).to_string());
    }
    let names: Vec<&str> = tasks_of(&v)
        .into_iter()
        .flat_map(|t| t.keys())
        .map(|k| key_name(k))
        .collect();
    let name = names.into_iter().find(|n| task_section(n) == section);
    Some(match name {
        Some(n) => serde_json::to_string(&task_values(&v, n)).unwrap_or_default(),
        None => String::new(),
    })
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Adds one task definition (`{ dependsOn, inputs, outputs, env, … }`) to
/// `spec`. `nested`: the directories of the workspace packages strictly inside
/// `pkg_dir`.
fn apply_def(
    def: &Value,
    pkg_dir: &Path,
    repo_root: &Path,
    nested: &[PathBuf],
    spec: &mut TaskSpec,
) {
    for d in str_list(def.get("dependsOn")) {
        if let Some(n) = d.strip_prefix('^') {
            spec.dep(
                TaskRef::Upstream {
                    name: n.to_string(),
                },
                "turbo dependsOn ^",
            );
        } else if let Some(v) = d.strip_prefix('$') {
            spec.env.push(v.to_string());
        } else if let Some((p, n)) = d.split_once('#') {
            let package = if p == "//" {
                ".".to_string()
            } else {
                p.to_string()
            };
            spec.dep(
                TaskRef::Package {
                    package,
                    name: n.to_string(),
                },
                "turbo dependsOn",
            );
        } else {
            spec.dep(TaskRef::Local { name: d }, "turbo dependsOn");
        }
    }
    if def.get("inputs").is_some() {
        let inputs = str_list(def.get("inputs"));
        let (neg, pos): (Vec<&String>, Vec<&String>) =
            inputs.iter().partition(|s| s.starts_with('!'));
        let mut declared = TaskSpec::default();
        for i in pos {
            if i == "$TURBO_DEFAULT$" {
                declared.membership(pkg_dir.to_path_buf(), "**", "turbo inputs $TURBO_DEFAULT$");
            } else if let Some(r) = i.strip_prefix("$TURBO_ROOT$/") {
                add_glob(repo_root, r, "turbo inputs", &mut declared);
            } else {
                add_glob(pkg_dir, i, "turbo inputs", &mut declared);
            }
        }
        for n in neg {
            let n = n.trim_start_matches('!');
            match n.strip_prefix("$TURBO_ROOT$/") {
                Some(r) => add_glob(repo_root, &format!("!{r}"), "turbo inputs", &mut declared),
                None => add_glob(pkg_dir, &format!("!{n}"), "turbo inputs", &mut declared),
            }
        }
        for m in &mut declared.memberships {
            m.reason = "turbo inputs".into();
            m.data = true;
        }
        // The files turbo hashes for the task: read, whatever the commands run or
        // compile (their recognizers add that).
        declared.data_inputs = std::mem::take(&mut declared.inputs);
        spec.merge(declared);
    }
    let outs = str_list(def.get("outputs"));
    let (neg, pos): (Vec<&String>, Vec<&String>) = outs.iter().partition(|s| s.starts_with('!'));
    let mut outputs: Vec<Output> = Vec::new();
    for o in pos {
        let o = o
            .strip_prefix("$TURBO_ROOT$/")
            .map_or_else(|| pkg_dir.join(o), |r| repo_root.join(r));
        let o = o.to_string_lossy().into_owned();
        let (prefix, rest) = crate::glob::split_static_prefix(&o);
        outputs.push(Output {
            path: fsx::normalize(Path::new(&format!("/{}", prefix.trim_start_matches('/')))),
            pattern: (!rest.is_empty()).then(|| rest.to_string()),
            exclude: Vec::new(),
            kind: OutputKind::Declared,
            part: None,
        });
    }
    for n in neg {
        let abs = pkg_dir.join(n.trim_start_matches('!'));
        for o in &mut outputs {
            if let Ok(r) = abs.strip_prefix(&o.path) {
                o.exclude.push(r.to_string_lossy().into_owned());
            }
        }
    }
    spec.outputs.extend(scope_outputs(outputs, pkg_dir, nested));
    if def.get("persistent").and_then(Value::as_bool) == Some(true) {
        spec.persistent = true;
    }
    spec.env.extend(str_list(def.get("env")));
    spec.env.extend(str_list(def.get("passThroughEnv")));
}

/// The part of glob `pattern` (relative to a directory `D`) below `D/<segs…>`,
/// relative to that subdirectory: `None` when no match can lie at or below it.
/// Exact for literal and single-segment wildcard segments; a `**` keeps the
/// rest (a superset).
fn narrow_pattern(pattern: Option<&str>, segs: &[&str]) -> Option<Option<String>> {
    let Some(pattern) = pattern else {
        // The whole directory `D` is the output.
        return Some(None);
    };
    let parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
    let mut j = 0;
    for s in segs {
        match parts.get(j) {
            // Matches end above the subdirectory.
            None => return None,
            Some(&"**") => return Some(Some(parts[j..].join("/"))),
            Some(p) if crate::glob::matches(p, s) => j += 1,
            Some(_) => return None,
        }
    }
    if j == parts.len() {
        // The subdirectory itself matches: all of it is the output.
        Some(None)
    } else {
        Some(Some(parts[j..].join("/")))
    }
}

/// Scope declared outputs to the declaring package (`pkg_dir`, minus the
/// `nested` package directories inside it): the in-package part stays
/// [`OutputKind::Declared`] (it links producers); when an output reaches beyond
/// it, the original is kept as [`OutputKind::CacheOnly`] (recorded, not
/// linked).
pub fn scope_outputs(outputs: Vec<Output>, pkg_dir: &Path, nested: &[PathBuf]) -> Vec<Output> {
    let mut out = Vec::new();
    for o in outputs {
        // The in-package part: (base, pattern, excludes relative to base).
        let part: Option<(PathBuf, Option<String>, Vec<String>)> = if o.path.starts_with(pkg_dir) {
            Some((o.path.clone(), o.pattern.clone(), o.exclude.clone()))
        } else if let Ok(rel) = pkg_dir.strip_prefix(&o.path) {
            let segs: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
            // Excludes relative to the outer directory are dropped (a superset).
            narrow_pattern(o.pattern.as_deref(), &segs).map(|p| (pkg_dir.to_path_buf(), p, vec![]))
        } else {
            None
        };
        let part = part.filter(|(base, _, _)| !nested.iter().any(|n| base.starts_with(n)));
        let scoped = part.map(|(base, pattern, mut exclude)| {
            for n in nested {
                if let Ok(r) = n.strip_prefix(&base) {
                    exclude.push(format!("{}/**", r.to_string_lossy().replace('\\', "/")));
                }
            }
            Output {
                path: base,
                pattern,
                exclude,
                kind: o.kind,
                part: None,
            }
        });
        match scoped {
            Some(s) if s == o => out.push(o),
            Some(s) => {
                out.push(Output {
                    kind: OutputKind::CacheOnly,
                    ..o
                });
                out.push(s);
            }
            None => out.push(Output {
                kind: OutputKind::CacheOnly,
                ..o
            }),
        }
    }
    out
}

/// Adds root-level `globalDependencies`/`globalEnv` to `spec`.
fn apply_globals(root: &Value, repo_root: &Path, spec: &mut TaskSpec) {
    // Hash inputs of every task, like `inputs`: read, not run or compiled.
    let mut declared = TaskSpec::default();
    for g in str_list(root.get("globalDependencies")) {
        add_glob(repo_root, &g, "turbo globalDependencies", &mut declared);
    }
    for m in &mut declared.memberships {
        m.data = true;
    }
    declared.data_inputs = std::mem::take(&mut declared.inputs);
    spec.merge(declared);
    spec.env.extend(str_list(root.get("globalEnv")));
    spec.env.extend(str_list(root.get("globalPassThroughEnv")));
}

impl TaskSource for Turbo {
    fn id(&self) -> &'static str {
        "turbo"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["turbo.json", "turbo.jsonc"]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let ws: &Workspace = cx.workspace;
        let Some((root_file, root_val)) = read_turbo(&ws.root) else {
            return Vec::new();
        };
        let mut files = vec![root_file.clone()];
        let Some(root_val) = root_val else {
            return unreadable(cx, &root_file);
        };
        let pkg = cx.package;
        let is_root = pkg.rel == ".";
        // Definitions that apply: (task name, definition).
        let mut defs: Vec<(String, Value)> = Vec::new();
        if let Some(tasks) = tasks_of(&root_val) {
            for (k, v) in tasks {
                match k.split_once('#') {
                    None => defs.push((k.clone(), v.clone())),
                    Some(("//", n)) if is_root => defs.push((n.to_string(), v.clone())),
                    Some((p, n))
                        if !is_root && (pkg.name.as_deref() == Some(p) || pkg.rel == p) =>
                    {
                        defs.push((n.to_string(), v.clone()))
                    }
                    _ => {}
                }
            }
        }
        if !is_root && let Some((pf, pv)) = read_turbo(&pkg.dir) {
            files.push(pf.clone());
            match pv {
                Some(pv) => {
                    // `extends` other packages' configs (besides `//`).
                    for e in str_list(pv.get("extends")) {
                        if e == "//" {
                            continue;
                        }
                        if let Some(other) = ws.package(&e)
                            && let Some((of, Some(ov))) = read_turbo(&other.dir)
                        {
                            files.push(of);
                            if let Some(tasks) = tasks_of(&ov) {
                                for (k, v) in tasks {
                                    defs.push((k.clone(), v.clone()));
                                }
                            }
                        }
                    }
                    if let Some(tasks) = tasks_of(&pv) {
                        for (k, v) in tasks {
                            let n = k.rsplit_once('#').map_or(k.as_str(), |(_, n)| n);
                            defs.push((n.to_string(), v.clone()));
                        }
                    }
                }
                None => return unreadable(cx, &pf),
            }
        }
        let nested: Vec<PathBuf> = ws
            .packages
            .iter()
            .filter(|p| p.dir != pkg.dir && p.dir.starts_with(&pkg.dir))
            .map(|p| p.dir.clone())
            .collect();
        let mut out: Vec<DiscoveredTask> = Vec::new();
        for (name, def) in defs {
            let mut t = DiscoveredTask::new(cx.script_id(&name), name.clone(), pkg.dir.clone());
            t.mode = Contribution::Augments;
            t.files = files.clone();
            apply_def(&def, &pkg.dir, &ws.root, &nested, &mut t.spec);
            apply_globals(&root_val, &ws.root, &mut t.spec);
            out.push(t);
        }
        out
    }
}

/// An unreadable turbo config: every script of the package gets an uncertainty
/// and the whole package as a membership (the config may declare anything).
fn unreadable(cx: &SourceCx<'_>, file: &Path) -> Vec<DiscoveredTask> {
    let scripts = crate::sources::read_json(&cx.package.dir.join("package.json"))
        .and_then(|m| {
            m.get("scripts")
                .and_then(|s| s.as_object())
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
        })
        .unwrap_or_default();
    scripts
        .into_iter()
        .map(|name| {
            let mut t = DiscoveredTask::new(cx.script_id(&name), name, cx.package.dir.clone());
            t.mode = Contribution::Augments;
            t.files.push(file.to_path_buf());
            t.spec.uncertain(
                UncertaintyKind::ParseIncomplete,
                format!("unreadable {}", file.display()),
            );
            t.spec
                .membership(cx.package.dir.clone(), "**", "unreadable turbo config");
            t
        })
        .collect()
}

/// `turbo run a b [--filter x]`, `turbo a` (turbo 2).
pub struct TurboRun;

impl Recognizer for TurboRun {
    fn id(&self) -> &'static str {
        "turbo"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["turbo"]
    }
    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "turbo");
        let args = crate::recognizers::util::Args::new(
            cmd,
            0,
            &[
                "--filter",
                "-F",
                "--concurrency",
                "--cache-dir",
                "--output-logs",
                "--log-order",
                "--graph",
                "--env-mode",
                "--cache",
                "--log-prefix",
                "--profile",
                "--summarize",
                "--ui",
                "--affected-base",
                "--scope",
                "--since",
                "--team",
                "--token",
                "--api",
                "--global-deps",
            ],
        )
        .collect_all();
        let mut pos = crate::recognizers::util::positionals(&args);
        match pos.first() {
            Some(&"run") => {
                pos.remove(0);
            }
            Some(
                &("prune" | "daemon" | "login" | "logout" | "link" | "unlink" | "gen" | "generate"
                | "telemetry" | "bin" | "info" | "ls" | "query" | "watch" | "boundaries"
                | "scan" | "devtools"),
            ) => {
                return Recognition::Ignored;
            }
            _ => {}
        }
        let mut filters: Vec<&str> =
            crate::recognizers::util::opts(&args, &["--filter", "-F", "--scope"]);
        if crate::recognizers::util::flag(&args, &["--affected"])
            || crate::recognizers::util::opt(&args, &["--since"]).is_some()
        {
            filters.clear();
        }
        for t in pos {
            if let Some((p, n)) = t.split_once('#') {
                let package = if p == "//" {
                    ".".to_string()
                } else {
                    p.to_string()
                };
                out.dep(
                    TaskRef::Package {
                        package,
                        name: n.to_string(),
                    },
                    "turbo run",
                );
            } else if filters.is_empty() {
                out.dep(
                    TaskRef::All {
                        name: t.to_string(),
                    },
                    "turbo run",
                );
            } else {
                for f in &filters {
                    out.dep(filter_ref(f, t), "turbo run");
                }
            }
        }
        Recognition::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(path: &str, pattern: Option<&str>) -> Output {
        Output {
            path: PathBuf::from(path),
            pattern: pattern.map(str::to_string),
            exclude: vec![],
            kind: OutputKind::Declared,
            part: None,
        }
    }

    fn show(v: &[Output]) -> Vec<String> {
        v.iter()
            .map(|o| {
                format!(
                    "{} {}{}{} {}",
                    o.kind.as_str(),
                    o.path.display(),
                    o.pattern
                        .as_deref()
                        .map(|p| format!(" {p}"))
                        .unwrap_or_default(),
                    if o.exclude.is_empty() {
                        String::new()
                    } else {
                        format!(" !{}", o.exclude.join(","))
                    },
                    o.links_producer()
                )
            })
            .collect()
    }

    /// A turbo config edit concerns the tasks whose entries changed, or every
    /// task when anything outside the task table changed (or a side is
    /// missing or unreadable).
    #[test]
    fn config_edits_are_per_section() {
        let base = r#"{
          // comment
          "globalEnv": ["CI"],
          "tasks": {
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "web#build": { "outputs": [".next/**"] },
            "lint": {},
            "//#format": {},
          }
        }"#;
        let edit = |from: &str, to: &str| base.replace(from, to);
        let diff = |new: &str| changed_sections(Some(base), Some(new));
        // Formatting and comments: nothing.
        assert!(diff(&edit("// comment", "/* other */\n")).is_empty());
        assert!(diff(base).is_empty());
        // One entry: that task name, whichever key form names it.
        assert_eq!(
            diff(&edit(r#""lint": {}"#, r#""lint": {"cache": false}"#)),
            ["#tasks/lint"]
        );
        assert_eq!(diff(&edit(".next/**", ".next/cache/**")), ["#tasks/build"]);
        assert_eq!(diff(&edit(r#""//#format": {},"#, "")), ["#tasks/format"]);
        // An added entry.
        assert_eq!(
            diff(&edit(
                r#""lint": {},"#,
                r#""lint": {}, "e2e:ci": {"cache": false},"#
            )),
            ["#tasks/e2e:ci"]
        );
        // Anything outside the task table: every task.
        assert_eq!(
            diff(&edit(r#"["CI"]"#, r#"["CI", "TZ"]"#)),
            [GLOBAL_SECTION]
        );
        // No base, an unreadable side, an added or deleted config: every task.
        assert_eq!(changed_sections(None, Some(base)), [GLOBAL_SECTION]);
        assert_eq!(changed_sections(Some(base), None), [GLOBAL_SECTION]);
        assert_eq!(changed_sections(Some(base), Some("{")), [GLOBAL_SECTION]);
        // turbo 1 `pipeline`.
        assert_eq!(
            changed_sections(
                Some(r#"{"pipeline": {"test": {}}}"#),
                Some(r#"{"pipeline": {"test": {"cache": false}}}"#)
            ),
            ["#tasks/test"]
        );

        assert_eq!(
            split_section("apps/web/turbo.json#tasks/build"),
            Some(("apps/web/turbo.json", "#tasks/build"))
        );
        assert_eq!(
            split_section("turbo.jsonc#global"),
            Some(("turbo.jsonc", "#global"))
        );
        assert_eq!(split_section("turbo.jsonc"), None);
        assert_eq!(split_section("docs/turbo.json#anchor"), None);
        assert_eq!(split_section("src/a.ts#tasks/build"), None);
        assert_eq!(task_section("a/b"), "#tasks/a%2Fb");

        // The section text changes exactly with the section.
        let text = |c: &str, s: &str| section_text(c, s).unwrap();
        let lint = edit(r#""lint": {}"#, r#""lint": {"cache": false}"#);
        assert_ne!(text(base, "#tasks/lint"), text(&lint, "#tasks/lint"));
        assert_eq!(text(base, "#tasks/build"), text(&lint, "#tasks/build"));
        assert_eq!(text(base, GLOBAL_SECTION), text(&lint, GLOBAL_SECTION));
        assert_eq!(text(base, "#tasks/none"), "");
        assert_eq!(section_text("{", GLOBAL_SECTION), None);
    }

    #[test]
    fn narrows_patterns_to_a_subdirectory() {
        let n = |p: Option<&str>, s: &[&str]| narrow_pattern(p, s);
        assert_eq!(
            n(Some("**/dist/**/*"), &["t", "s"]),
            Some(Some("**/dist/**/*".into()))
        );
        assert_eq!(
            n(Some("*/s/dist/**"), &["t", "s"]),
            Some(Some("dist/**".into()))
        );
        assert_eq!(n(Some("apps/*/dist/x.json"), &["packages", "a"]), None);
        assert_eq!(n(Some("t/*"), &["t", "s"]), Some(None));
        assert_eq!(n(Some("t"), &["t", "s"]), None);
        assert_eq!(n(None, &["t"]), Some(None));
        assert_eq!(n(Some("t/**"), &["t", "s"]), Some(Some("**".into())));
    }

    #[test]
    fn declared_outputs_link_only_inside_their_package() {
        // A test server's build: `$TURBO_ROOT$/**/dist/**/*` collects every
        // dependency's dist/ for the cache. Only its own dist/ links.
        let v = scope_outputs(
            vec![o("/r", Some("**/dist/**/*"))],
            Path::new("/r/t/s"),
            &[],
        );
        assert_eq!(
            show(&v),
            [
                "cache_only /r **/dist/**/* false",
                "declared /r/t/s **/dist/**/* true"
            ]
        );
        // A root task `//#cache-tsbuildinfo-files`: outputs in other packages only.
        let nested = [PathBuf::from("/r/apps/a"), PathBuf::from("/r/apps/b")];
        let v = scope_outputs(
            vec![
                o("/r/apps", Some("*/dist/tsbuildinfo.json")),
                o("/r/apps/a/dist", None),
            ],
            Path::new("/r"),
            &nested,
        );
        assert_eq!(
            show(&v),
            [
                "cache_only /r/apps */dist/tsbuildinfo.json false",
                "declared /r/apps */dist/tsbuildinfo.json !a/**,b/** true",
                "cache_only /r/apps/a/dist false",
            ]
        );
        // Ordinary outputs are unchanged.
        let v = scope_outputs(vec![o("/r/p/dist", Some("**"))], Path::new("/r/p"), &[]);
        assert_eq!(show(&v), ["declared /r/p/dist ** true"]);
        // A sibling package's directory: not produced here.
        let v = scope_outputs(vec![o("/r/q/gen", None)], Path::new("/r/p"), &[]);
        assert_eq!(show(&v), ["cache_only /r/q/gen false"]);
    }

    #[test]
    fn jsonc() {
        let v = parse_jsonc("{ // c\n \"a\": [1, 2,], /* x */ \"b\": \"//not\", }").unwrap();
        assert_eq!(v["a"][1], 2);
        assert_eq!(v["b"], "//not");
        assert!(parse_jsonc("{\"é\": \"ü\"}").is_some());
    }
}
