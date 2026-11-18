//! `Taskfile.yml` (go-task): tasks with `cmds`, `deps`, `dir`,
//! `vars`/`env`, `includes`, and the declared `sources`/`generates`.
//!
//! Every task is `taskfile:<dir>:<name>` (`<dir>` is the root Taskfile's
//! directory, repo-relative; included tasks are named `<namespace>:<task>`).
//!
//! | Field | Becomes |
//! |---|---|
//! | `cmds` (strings, `cmd:`) | command lines (Go templates `{{.VAR}}` expanded from static `vars`; unknown values make the word computed) |
//! | `cmds` `task:`, `deps` | task dependencies ([`TaskRef::Id`]) |
//! | `sources` (globs, `exclude:`) | memberships of the task directory (declared inputs) |
//! | `generates` | [`Output`]s (generated-file linking) |
//! | `status`, `preconditions` | command lines too (they read files) |
//! | `dir` | the working directory |
//! | `env`, `dotenv` | environment for the commands; dotenv files are inputs |
//! | `includes` | the included Taskfiles' tasks, namespaced (or `flatten`ed) |
//! | `for` over a static list | one command per item; other loops are uncertain |
//!
//! `{{.CLI_ARGS}}` expands to nothing (a task run without extra arguments). The
//! [`TaskCli`] recognizer turns `task build` in other tasks into dependencies.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde_json::Value;
use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::{Args, add_glob, opt, positionals},
    registry::{Recognition, RecognizeCx, Recognizer},
    sources::{DiscoveredTask, SourceCx, TaskSource},
    spec::{Command, Output, OutputKind, TaskId, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&Taskfile]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&TaskCli]
}

/// File names go-task looks for, in order.
pub const TASKFILE_NAMES: &[&str] = &[
    "Taskfile.yml",
    "taskfile.yml",
    "Taskfile.yaml",
    "taskfile.yaml",
    "Taskfile.dist.yml",
    "taskfile.dist.yml",
    "Taskfile.dist.yaml",
    "taskfile.dist.yaml",
];

/// Parses YAML (merge keys applied) into a JSON value. Non-string keys become
/// strings. `None` when the document is not valid YAML.
pub fn parse_yaml(s: &str) -> Option<Value> {
    let mut v: serde_yaml_ng::Value = serde_yaml_ng::from_str(s).ok()?;
    let _ = v.apply_merge();
    Some(yaml_to_json(&v, 0))
}

fn yaml_to_json(v: &serde_yaml_ng::Value, depth: usize) -> Value {
    use serde_yaml_ng::Value as Y;
    if depth > 128 {
        return Value::Null;
    }
    match v {
        Y::Null => Value::Null,
        Y::Bool(b) => Value::Bool(*b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                serde_json::Number::from_f64(n.as_f64().unwrap_or(0.0))
                    .map_or(Value::String(n.to_string()), Value::Number)
            }
        }
        Y::String(s) => Value::String(s.clone()),
        Y::Sequence(a) => Value::Array(a.iter().map(|x| yaml_to_json(x, depth + 1)).collect()),
        Y::Mapping(m) => {
            let mut o = serde_json::Map::new();
            for (k, x) in m {
                let key = match k {
                    Y::String(s) => s.clone(),
                    Y::Bool(b) => b.to_string(),
                    Y::Number(n) => n.to_string(),
                    Y::Null => "null".to_string(),
                    other => serde_yaml_ng::to_string(other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                o.insert(key, yaml_to_json(x, depth + 1));
            }
            Value::Object(o)
        }
        Y::Tagged(t) => yaml_to_json(&t.value, depth + 1),
    }
}

/// `p` relative to `root`, `/`-separated (`.` for the root itself).
pub fn repo_rel(root: &Path, p: &Path) -> String {
    match p.strip_prefix(root) {
        Ok(r) if r.as_os_str().is_empty() => ".".into(),
        Ok(r) => r.to_string_lossy().replace('\\', "/"),
        Err(_) => p.to_string_lossy().into_owned(),
    }
}

/// A scalar as a string (`1` → `"1"`, `true` → `"true"`).
pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The marker inserted for a value that is not known statically: an unknown
/// shell variable, so the shell parser marks the word computed.
pub const UNKNOWN: &str = "${__TASKFILE_UNKNOWN}";

/// Variables for template expansion: `None` for values known to exist but not
/// statically (`sh:` variables).
pub type Vars = BTreeMap<String, Option<String>>;

/// Expands Go-template actions (`{{.VAR}}`, `{{.VAR | default "x"}}`,
/// `{{default "x" .VAR}}`) in `s`. Anything else becomes [`UNKNOWN`]. Returns
/// the text and whether everything was known.
pub fn expand_template(s: &str, vars: &Vars) -> (String, bool) {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut known = true;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("}}") else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let inner = after[..j].trim().trim_matches('-').trim();
        match eval_action(inner, vars) {
            Some(v) => out.push_str(&v),
            None => {
                known = false;
                out.push_str(UNKNOWN);
            }
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    (out, known)
}

fn eval_action(inner: &str, vars: &Vars) -> Option<String> {
    let lookup = |name: &str| -> Option<Option<String>> {
        let n = name.trim_start_matches('.');
        vars.get(n).cloned()
    };
    let unquote = |s: &str| -> Option<String> {
        let s = s.trim();
        (s.len() >= 2
            && (s.starts_with('"') && s.ends_with('"') || s.starts_with('`') && s.ends_with('`')))
        .then(|| s[1..s.len() - 1].to_string())
    };
    if inner == "exeExt" {
        return Some(String::new());
    }
    let (head, pipe) = match inner.split_once('|') {
        Some((h, p)) => (h.trim(), Some(p.trim())),
        None => (inner, None),
    };
    if let Some(name) = head.strip_prefix('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        let v = lookup(name);
        return match (v, pipe) {
            (Some(Some(v)), None) => Some(v),
            (Some(Some(v)), Some(p)) if p.starts_with("default") && !v.is_empty() => Some(v),
            (Some(None), _) => None,
            (_, Some(p)) => p.strip_prefix("default").and_then(unquote),
            (None, None) => None,
        };
    }
    if let Some(rest) = head.strip_prefix("default ") {
        // `default "x" .VAR`
        let rest = rest.trim();
        let end = rest.get(1..)?.find(['"', '`'])?;
        let (d, var) = (&rest[..end + 2], rest[end + 2..].trim());
        let d = unquote(d)?;
        return match lookup(var) {
            Some(Some(v)) if !v.is_empty() => Some(v),
            Some(None) => None,
            _ => Some(d),
        };
    }
    None
}

/// Evaluates a `vars:` map on top of `base` (later entries see earlier ones).
fn eval_vars(map: Option<&Value>, base: &Vars) -> Vars {
    let mut vars = base.clone();
    let Some(Value::Object(m)) = map else {
        return vars;
    };
    for (k, v) in m {
        let val = match v {
            Value::Object(o) if o.contains_key("sh") => None,
            Value::Object(o) => match o.get("ref").and_then(Value::as_str) {
                Some(r) => vars
                    .get(r.trim_start_matches('.').trim())
                    .cloned()
                    .flatten(),
                None => None,
            },
            Value::Array(a) => {
                let items: Option<Vec<String>> = a.iter().map(scalar).collect();
                items.map(|i| i.join(" "))
            }
            x => scalar(x).and_then(|s| {
                let (e, ok) = expand_template(&s, &vars);
                ok.then_some(e)
            }),
        };
        vars.insert(k.clone(), val);
    }
    vars
}

/// Static environment from an `env:` map (values with unknown parts are
/// dropped).
fn eval_env(map: Option<&Value>, vars: &Vars, env: &mut BTreeMap<String, String>) {
    if let Some(Value::Object(m)) = map {
        for (k, v) in m {
            if let Some(s) = scalar(v) {
                let (e, ok) = expand_template(&s, vars);
                if ok {
                    env.insert(k.clone(), e);
                } else {
                    env.remove(k);
                }
            } else {
                env.remove(k);
            }
        }
    }
}

/// The first Taskfile in `dir`.
pub fn find_taskfile(dir: &Path) -> Option<PathBuf> {
    fsx::first_existing(dir, TASKFILE_NAMES)
}

/// `Taskfile.yml`.
pub struct Taskfile;

impl TaskSource for Taskfile {
    fn id(&self) -> &'static str {
        "taskfile"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        TASKFILE_NAMES
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let Some(file) = find_taskfile(&cx.package.dir) else {
            return Vec::new();
        };
        let root_dir = cx.package.dir.clone();
        let mut l = Loader {
            root_dir: root_dir.clone(),
            rel_root: repo_rel(&cx.workspace.root, &root_dir),
            out: Vec::new(),
        };
        let mut base = Vars::new();
        base.insert(
            "ROOT_DIR".into(),
            Some(root_dir.to_string_lossy().into_owned()),
        );
        base.insert("USER_WORKING_DIR".into(), None);
        base.insert("CLI_ARGS".into(), Some(String::new()));
        base.insert("CLI_FORCE".into(), Some("false".into()));
        l.load(&file, "", &root_dir, &base, &BTreeMap::new(), &[], 0);
        crate::sources::gha::script::sanitize_lines(&mut l.out);
        l.out
    }
}

struct Loader {
    root_dir: PathBuf,
    rel_root: String,
    out: Vec<DiscoveredTask>,
}

impl Loader {
    fn id(&self, name: &str) -> TaskId {
        TaskId::other("taskfile", format!("{}:{name}", self.rel_root))
    }

    /// The full name of a `task:` reference made from namespace `ns`.
    fn ref_name(ns: &str, name: &str) -> String {
        if let Some(n) = name.strip_prefix(':') {
            n.to_string()
        } else if ns.is_empty() {
            name.to_string()
        } else {
            format!("{ns}:{name}")
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn load(
        &mut self,
        file: &Path,
        ns: &str,
        default_dir: &Path,
        inherited: &Vars,
        inherited_env: &BTreeMap<String, String>,
        chain: &[PathBuf],
        depth: usize,
    ) {
        let mut files: Vec<PathBuf> = chain.to_vec();
        files.push(file.to_path_buf());
        let tf_dir = file.parent().unwrap_or(&self.root_dir).to_path_buf();
        let parsed = fsx::read_to_string(file).ok().and_then(|s| parse_yaml(&s));
        let Some(doc) = parsed.filter(Value::is_object) else {
            let name = if ns.is_empty() {
                "*".to_string()
            } else {
                format!("{ns}:*")
            };
            let mut t = DiscoveredTask::new(self.id(&name), name, default_dir.to_path_buf());
            t.files = files;
            t.spec.uncertain(
                UncertaintyKind::ParseIncomplete,
                format!("unreadable {}", file.display()),
            );
            self.out.push(t);
            return;
        };
        let mut vars = inherited.clone();
        vars.insert(
            "TASKFILE_DIR".into(),
            Some(tf_dir.to_string_lossy().into_owned()),
        );
        vars.insert("TASKFILE".into(), Some(file.to_string_lossy().into_owned()));
        let vars = eval_vars(doc.get("vars"), &vars);
        let mut env = inherited_env.clone();
        eval_env(doc.get("env"), &vars, &mut env);
        let dotenv: Vec<PathBuf> = str_list(doc.get("dotenv"))
            .iter()
            .map(|d| fsx::join(&tf_dir, &expand_template(d, &vars).0))
            .collect();
        // Includes.
        if let Some(Value::Object(incs)) = doc.get("includes") {
            for (name, spec) in incs {
                let (path, dir, flatten, inc_vars) = match spec {
                    Value::String(s) => (s.clone(), None, false, None),
                    Value::Object(o) => (
                        o.get("taskfile").and_then(scalar).unwrap_or_default(),
                        o.get("dir").and_then(scalar),
                        o.get("flatten").and_then(Value::as_bool).unwrap_or(false),
                        o.get("vars"),
                    ),
                    _ => continue,
                };
                let (path, ok) = expand_template(&path, &vars);
                if !ok || path.is_empty() || depth >= 8 {
                    continue;
                }
                let mut p = fsx::join(&tf_dir, &path);
                if fsx::is_dir(&p) {
                    match find_taskfile(&p) {
                        Some(f) => p = f,
                        None => continue,
                    }
                }
                if files.contains(&p) {
                    continue;
                }
                let sub_dir = match dir {
                    Some(d) => fsx::join(&tf_dir, &expand_template(&d, &vars).0),
                    None => default_dir.to_path_buf(),
                };
                let sub_ns = if flatten {
                    ns.to_string()
                } else if ns.is_empty() {
                    name.clone()
                } else {
                    format!("{ns}:{name}")
                };
                let sub_vars = eval_vars(inc_vars, &vars);
                if !fsx::is_file(&p) {
                    continue; // `optional: true`, or a missing include (task fails).
                }
                self.load(&p, &sub_ns, &sub_dir, &sub_vars, &env, &files, depth + 1);
            }
        }
        let Some(Value::Object(tasks)) = doc.get("tasks") else {
            return;
        };
        for (name, def) in tasks {
            let full = if ns.is_empty() {
                name.clone()
            } else {
                format!("{ns}:{name}")
            };
            let def = match def {
                Value::String(_) | Value::Array(_) => {
                    let mut o = serde_json::Map::new();
                    o.insert("cmds".into(), def.clone());
                    Value::Object(o)
                }
                Value::Object(_) => def.clone(),
                _ => continue,
            };
            let mut tvars = vars.clone();
            tvars.insert("TASK".into(), Some(full.clone()));
            tvars.insert("ALIAS".into(), Some(full.clone()));
            let tvars = eval_vars(def.get("vars"), &tvars);
            let mut spec = TaskSpec::default();
            let cwd = match def.get("dir").and_then(scalar) {
                Some(d) => {
                    let (d, ok) = expand_template(&d, &tvars);
                    if !ok {
                        spec.uncertain(
                            UncertaintyKind::CwdDependent,
                            format!("computed `dir` of task `{full}`"),
                        );
                    }
                    fsx::join(&tf_dir, &d)
                }
                None => default_dir.to_path_buf(),
            };
            let mut tvars = tvars;
            tvars.insert("TASK_DIR".into(), Some(cwd.to_string_lossy().into_owned()));
            let mut tenv = env.clone();
            eval_env(def.get("env"), &tvars, &mut tenv);
            for d in &dotenv {
                spec.input(d.clone());
            }
            for d in str_list(def.get("dotenv")) {
                spec.input(fsx::join(&cwd, &expand_template(&d, &tvars).0));
            }
            // Declared inputs and outputs.
            let mut excludes = Vec::new();
            for s in list(def.get("sources")) {
                match s {
                    Value::Object(o) => {
                        if let Some(e) = o.get("exclude").and_then(scalar) {
                            excludes.push(expand_template(&e, &tvars).0);
                        }
                    }
                    x => {
                        if let Some(g) = scalar(&x) {
                            let (g, ok) = expand_template(&g, &tvars);
                            if ok {
                                add_glob(&cwd, &g, "taskfile sources", &mut spec);
                            } else {
                                spec.uncertain(
                                    UncertaintyKind::PartialSpecifier,
                                    format!("computed `sources` entry `{g}`"),
                                );
                            }
                        }
                    }
                }
            }
            for e in excludes {
                add_glob(&cwd, &format!("!{e}"), "taskfile sources", &mut spec);
            }
            for g in list(def.get("generates")) {
                let Some(g) = scalar(&g) else { continue };
                let (g, ok) = expand_template(&g, &tvars);
                if !ok {
                    continue;
                }
                let abs = fsx::join(&cwd, &g).to_string_lossy().into_owned();
                let (prefix, rest) = crate::glob::split_static_prefix(&abs);
                spec.outputs.push(Output {
                    path: fsx::normalize(Path::new(&format!(
                        "/{}",
                        prefix.trim_start_matches('/')
                    ))),
                    pattern: (!rest.is_empty()).then(|| rest.to_string()),
                    exclude: Vec::new(),
                    kind: OutputKind::Declared,
                    part: None,
                });
            }
            for d in list(def.get("deps")) {
                let (n, for_) = match &d {
                    Value::Object(o) => (o.get("task").and_then(scalar), o.get("for")),
                    x => (scalar(x), None),
                };
                if for_.is_some() {
                    spec.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!("`for` in deps of task `{full}`"),
                    );
                }
                if let Some(n) = n {
                    let (n, ok) = expand_template(&n, &tvars);
                    if ok {
                        spec.dep(
                            TaskRef::Id {
                                id: self.id(&Self::ref_name(ns, &n)),
                            },
                            "taskfile deps",
                        );
                    } else {
                        spec.uncertain(
                            UncertaintyKind::UnknownCommand,
                            format!("computed dependency `{n}` of task `{full}`"),
                        );
                    }
                }
            }
            let mut lines: Vec<String> = Vec::new();
            for c in list(def.get("cmds")) {
                self.cmd(&c, ns, &full, &tvars, &mut lines, &mut spec);
            }
            for key in ["status", "preconditions"] {
                for c in list(def.get(key)) {
                    let s = match &c {
                        Value::Object(o) => o.get("sh").and_then(scalar),
                        x => scalar(x),
                    };
                    if let Some(s) = s {
                        lines.push(expand_template(&s, &tvars).0);
                    }
                }
            }
            for a in str_list(def.get("aliases")) {
                let alias = if ns.is_empty() {
                    a.clone()
                } else {
                    format!("{ns}:{a}")
                };
                let mut t = DiscoveredTask::new(self.id(&alias), alias, cwd.clone());
                t.files = files.clone();
                t.spec
                    .dep(TaskRef::Id { id: self.id(&full) }, "taskfile alias");
                self.out.push(t);
            }
            let mut t = DiscoveredTask::new(self.id(&full), full.clone(), cwd);
            t.lines = lines;
            t.env = tenv;
            t.spec = spec;
            t.files = files.clone();
            self.out.push(t);
        }
    }

    /// One `cmds` entry.
    fn cmd(
        &self,
        c: &Value,
        ns: &str,
        task: &str,
        vars: &Vars,
        lines: &mut Vec<String>,
        spec: &mut TaskSpec,
    ) {
        match c {
            Value::Object(o) => {
                if let Some(d) = o.get("defer") {
                    self.cmd(d, ns, task, vars, lines, spec);
                    return;
                }
                let items: Option<Vec<String>> = match o.get("for") {
                    None => None,
                    Some(Value::Array(a)) => Some(a.iter().filter_map(scalar).collect()),
                    Some(Value::Object(f)) if f.get("var").is_some() => {
                        let v = f.get("var").and_then(scalar).unwrap_or_default();
                        match vars.get(&v).cloned().flatten() {
                            Some(val) => Some(val.split_whitespace().map(str::to_string).collect()),
                            None => {
                                spec.uncertain(
                                    UncertaintyKind::UnknownCommand,
                                    format!("`for` over computed `{v}` in task `{task}`"),
                                );
                                return;
                            }
                        }
                    }
                    Some(_) => {
                        spec.uncertain(
                            UncertaintyKind::UnknownCommand,
                            format!("`for` loop in task `{task}`"),
                        );
                        return;
                    }
                };
                let runs: Vec<Vars> = match items {
                    None => vec![vars.clone()],
                    Some(items) => items
                        .into_iter()
                        .map(|it| {
                            let mut v = vars.clone();
                            v.insert("ITEM".into(), Some(it));
                            v
                        })
                        .collect(),
                };
                for v in runs {
                    if let Some(s) = o.get("cmd").and_then(scalar) {
                        lines.push(expand_template(&s, &v).0);
                    }
                    if let Some(n) = o.get("task").and_then(scalar) {
                        let (n, ok) = expand_template(&n, &v);
                        if ok {
                            spec.dep(
                                TaskRef::Id {
                                    id: self.id(&Self::ref_name(ns, &n)),
                                },
                                "taskfile task call",
                            );
                        } else {
                            spec.uncertain(
                                UncertaintyKind::UnknownCommand,
                                format!("computed task call `{n}` in task `{task}`"),
                            );
                        }
                    }
                }
            }
            x => {
                if let Some(s) = scalar(x) {
                    lines.push(expand_template(&s, vars).0);
                }
            }
        }
    }
}

fn list(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(x) => vec![x.clone()],
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    list(v).iter().filter_map(scalar).collect()
}

/// `task build test [-d dir] [-t file] [-- args]` (go-task's CLI): task
/// dependencies.
pub struct TaskCli;

impl Recognizer for TaskCli {
    fn id(&self) -> &'static str {
        "go-task"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["task", "go-task"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "go-task");
        let args = Args::new(
            cmd,
            0,
            &[
                "-d",
                "--dir",
                "-t",
                "--taskfile",
                "-o",
                "--output",
                "-c",
                "--color",
                "-C",
                "--concurrency",
                "--interval",
                "--sort",
            ],
        )
        .collect_all();
        let file = match (
            opt(&args, &["-t", "--taskfile"]),
            opt(&args, &["-d", "--dir"]),
        ) {
            (Some(t), _) => {
                let p = cmd.path(t);
                if fsx::is_dir(&p) {
                    find_taskfile(&p)
                } else {
                    Some(p)
                }
            }
            (None, Some(d)) => find_taskfile(&cmd.path(d)),
            (None, None) => {
                let mut found = None;
                for d in cmd.cwd.ancestors() {
                    if let Some(f) = find_taskfile(d) {
                        found = Some(f);
                        break;
                    }
                    if d == cx.repo_root {
                        break;
                    }
                }
                found
            }
        };
        let Some(file) = file else {
            out.uncertain(UncertaintyKind::UnknownCommand, "task without a Taskfile");
            cx.whole_package(out, "task without a Taskfile");
            return Recognition::Partial;
        };
        out.config(file.clone());
        let dir = repo_rel(cx.repo_root, file.parent().unwrap_or(cx.repo_root));
        // Positionals up to `--` (the rest are CLI_ARGS); `VAR=x` positionals are
        // variables.
        let after_dash = cmd
            .args()
            .position(|w| w == "--")
            .map_or(0, |p| cmd.args().count() - p - 1);
        let pos = positionals(&args);
        let names: Vec<&str> = pos[..pos.len() - after_dash.min(pos.len())]
            .iter()
            .copied()
            .filter(|p| !p.contains('='))
            .collect();
        let names = if names.is_empty() {
            vec!["default"]
        } else {
            names
        };
        for n in names {
            out.dep(
                TaskRef::Id {
                    id: TaskId::other("taskfile", format!("{dir}:{n}")),
                },
                "task",
            );
        }
        Recognition::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(kv: &[(&str, Option<&str>)]) -> Vars {
        kv.iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
            .collect()
    }

    #[test]
    fn templates() {
        let v = vars(&[("A", Some("x")), ("B", None), ("E", Some(""))]);
        assert_eq!(expand_template("a {{.A}} b", &v), ("a x b".into(), true));
        assert_eq!(expand_template("{{ .A }}", &v), ("x".into(), true));
        assert!(!expand_template("{{.B}}", &v).1);
        assert_eq!(
            expand_template("{{.Z | default \"d\"}}", &v),
            ("d".into(), true)
        );
        assert_eq!(
            expand_template("{{.E | default \"d\"}}", &v),
            ("d".into(), true)
        );
        assert_eq!(
            expand_template("{{default \"d\" .Z}}", &v),
            ("d".into(), true)
        );
        assert!(!expand_template("{{OS}}", &v).1);
        assert_eq!(expand_template("{{unterminated", &v).0, "{{unterminated");
    }

    #[test]
    fn yaml() {
        let v = parse_yaml("a: &x {b: 1}\nc:\n  <<: *x\n  d: on\n1: y\n").unwrap();
        assert_eq!(v["c"]["b"], 1);
        assert_eq!(v["c"]["d"], "on");
        assert_eq!(v["1"], "y");
        assert!(parse_yaml("a: [").is_none());
    }

    #[test]
    fn parsers_never_panic() {
        let seeds = [
            "{{",
            "}}",
            "{{.",
            "{{default \"",
            "{{.A | default",
            "{{default `x",
        ];
        let v = Vars::new();
        let mut x: u64 = 0x2545f4914f6cdd1d;
        for s in seeds {
            expand_template(s, &v);
        }
        for _ in 0..3000 {
            let mut s = String::new();
            for _ in 0..24 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let a = b"{}. |\"`defaultA:-[]\n &*<";
                s.push(a[(x % a.len() as u64) as usize] as char);
            }
            expand_template(&s, &v);
            parse_yaml(&s);
        }
    }
}
