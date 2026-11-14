//! GitHub Actions workflows: jobs as tasks, `run:` steps as command lines,
//! `paths:` filters as declared inputs. Shell script files run by any task are
//! handled by [`script`].
//!
//! Every job of `.github/workflows/*.yml` is a task `gha:<workflow path>#<job
//! id>`.
//!
//! | Workflow part | Becomes |
//! |---|---|
//! | step `run:` (with `shell:`, `working-directory:`, `env:`, job/workflow `defaults.run`) | a command line in that directory (`${{ … }}` expressions are computed words, except `github.workspace`, static `env.X`, `inputs.x` given as literals and a few non-path values such as `github.sha`) |
//! | `shell: python`/`pwsh`/`cmd` | an uncertainty (not a POSIX shell) |
//! | `needs:` | task dependencies on the other jobs |
//! | `uses: ./path` (local composite action, `action.yml`) | its steps, inlined (with `with:` literals for `inputs.x`); JavaScript actions → `runs.main` entry; Docker actions → the Dockerfile task |
//! | `uses: ./.github/workflows/x.yml` (reusable workflow) | dependencies on its jobs |
//! | `uses: org/action@v`, `docker://img` | toolchain inputs (ecosystem `gha` / `docker`); `with:` values naming existing repo files are inputs (`node-version-file: .nvmrc`) |
//! | `hashFiles('…')` in any expression | memberships (cache keys name inputs) |
//! | `on.<event>.paths` / `paths-ignore` | memberships with reason `gha paths filter` / `gha paths-ignore` |
//!
//! Step-to-task references (`pnpm turbo run build`, `make test`, `docker
//! build`) come from the registry's recognizers. [`lint`] reports workflows
//! whose `paths` filter misses files their recognized tasks depend on.
//!
//! Two things are kept apart from a job's inputs. What the job *runs* is its
//! [`crate::spec::TaskSpec::runs`] ([`crate::runs`]): the tests its steps
//! start, and the jobs of a reusable workflow it calls (not the jobs it
//! `needs`). What *defines* the job is its effective definition ([`def`]): the
//! keys of the workflow file and of the local actions that decide how it
//! executes, compared field by field between two revisions.

pub mod def;
pub mod lint;
pub mod script;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde_json::Value;
use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::add_glob,
    registry::Recognizer,
    sources::{
        DiscoveredTask, SourceCx, TaskSource,
        taskfile::{parse_yaml, repo_rel, scalar},
    },
    spec::{EntryLang, TaskId, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&Workflows]
}

/// Recognizers of this module: shell script files (`bash x.sh`, `./x.sh`,
/// `source x`) and shell builtins (`local`, `read`, `shift`, ...).
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&script::ShellScript, &script::ShellBuiltin]
}

/// Reason of the memberships recording `on.*.paths` filters.
pub const PATHS_REASON: &str = "gha paths filter";
/// Reason of the memberships recording `on.*.paths-ignore` filters.
pub const PATHS_IGNORE_REASON: &str = "gha paths-ignore";

/// The marker for an unknown `${{ … }}` value (an unknown shell variable, so
/// the word is computed).
const UNKNOWN_EXPR: &str = "${__GHA_EXPR}";

/// `.github/workflows/*.yml`.
pub struct Workflows;

/// The id of job `job` in workflow `rel` (repo-relative path).
pub fn job_id(rel: &str, job: &str) -> TaskId {
    TaskId::other("gha", format!("{rel}#{job}"))
}

impl TaskSource for Workflows {
    fn id(&self) -> &'static str {
        "gha"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &[]
    }

    fn ci_side_effects(&self) -> bool {
        // A job's writes become a later commit, not files other jobs' tests read.
        true
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        if cx.package.rel != "." {
            return Vec::new();
        }
        let root = &cx.workspace.root;
        let dir = root.join(".github/workflows");
        let Some(entries) = fsx::read_dir(&dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .into_iter()
            .filter(|(p, is_dir)| {
                !is_dir && p.extension().is_some_and(|e| e == "yml" || e == "yaml")
            })
            .map(|(p, _)| p)
            .collect();
        files.sort();
        let mut out = Vec::new();
        for f in files {
            workflow(root, &f, &mut out);
        }
        script::sanitize_lines(&mut out);
        out
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(scalar).collect(),
        Some(x) => scalar(x).into_iter().collect(),
        None => Vec::new(),
    }
}

/// The `paths` / `paths-ignore` filters of a workflow's `on:` (positive
/// patterns with `!` negations kept in order, and ignore patterns).
pub fn paths_filters(doc: &Value) -> (Vec<String>, Vec<String>) {
    let mut paths = Vec::new();
    let mut ignore = Vec::new();
    if let Some(Value::Object(on)) = doc.get("on") {
        for (_, ev) in on {
            paths.extend(str_list(ev.get("paths")));
            ignore.extend(str_list(ev.get("paths-ignore")));
        }
    }
    (paths, ignore)
}

/// The filters of each `on:` event that has `paths` or `paths-ignore`: (event,
/// paths, paths-ignore), patterns in order (`!` negations kept).
pub fn event_filters(doc: &Value) -> Vec<(String, Vec<String>, Vec<String>)> {
    let mut out = Vec::new();
    if let Some(Value::Object(on)) = doc.get("on") {
        for (name, ev) in on {
            let paths = str_list(ev.get("paths"));
            let ignore = str_list(ev.get("paths-ignore"));
            if !paths.is_empty() || !ignore.is_empty() {
                out.push((name.clone(), paths, ignore));
            }
        }
    }
    out
}

/// Static `env:` values (values with expressions are dropped).
fn static_env(v: Option<&Value>, env: &mut BTreeMap<String, String>) {
    if let Some(Value::Object(m)) = v {
        for (k, x) in m {
            match scalar(x) {
                Some(s) if !s.contains("${{") => {
                    env.insert(k.clone(), s);
                }
                _ => {
                    env.remove(k);
                }
            }
        }
    }
}

/// Defaults for `run:` steps.
#[derive(Clone, Default)]
struct RunDefaults {
    shell: Option<String>,
    wd: Option<String>,
}

fn run_defaults(v: &Value, base: &RunDefaults) -> RunDefaults {
    let r = v.get("defaults").and_then(|d| d.get("run"));
    RunDefaults {
        shell: r
            .and_then(|r| r.get("shell"))
            .and_then(scalar)
            .or_else(|| base.shell.clone()),
        wd: r
            .and_then(|r| r.get("working-directory"))
            .and_then(scalar)
            .or_else(|| base.wd.clone()),
    }
}

/// Expression context for `${{ … }}` substitution.
struct ExprCx<'a> {
    root: &'a Path,
    env: &'a BTreeMap<String, String>,
    inputs: &'a BTreeMap<String, String>,
    matrix: &'a BTreeMap<String, String>,
}

/// Replaces `${{ … }}` expressions: known values are substituted, the rest
/// become [`UNKNOWN_EXPR`].
fn subst_exprs(s: &str, cx: &ExprCx<'_>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("${{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 3..];
        let Some(j) = after.find("}}") else {
            out.push_str(&rest[i..]);
            return out;
        };
        let e = after[..j].trim();
        out.push_str(&eval_expr(e, cx).unwrap_or_else(|| UNKNOWN_EXPR.to_string()));
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    out
}

fn eval_expr(e: &str, cx: &ExprCx<'_>) -> Option<String> {
    if e == "github.workspace" {
        return Some(cx.root.to_string_lossy().into_owned());
    }
    if let Some(k) = e.strip_prefix("env.") {
        return cx.env.get(k).cloned();
    }
    if let Some(k) = e.strip_prefix("inputs.") {
        return cx.inputs.get(k).cloned();
    }
    if let Some(k) = e.strip_prefix("matrix.") {
        return cx.matrix.get(k).cloned();
    }
    // Values that never name repository files.
    const OPAQUE: &[&str] = &[
        "github.sha",
        "github.ref",
        "github.ref_name",
        "github.head_ref",
        "github.base_ref",
        "github.run_id",
        "github.run_number",
        "github.run_attempt",
        "github.actor",
        "github.triggering_actor",
        "github.repository",
        "github.repository_owner",
        "github.event_name",
        "github.job",
        "github.token",
        "github.server_url",
        "github.api_url",
        "github.event.pull_request.number",
        "github.event.pull_request.head.sha",
        "github.event.pull_request.base.sha",
        "github.event.number",
        "github.event.before",
        "github.event.after",
        "runner.os",
        "runner.arch",
        "job.status",
    ];
    if OPAQUE.contains(&e) || e.starts_with("secrets.") {
        return Some("GHA_VALUE".into());
    }
    None
}

/// Collects `hashFiles('a', 'b')` patterns from every string in `v`.
fn hash_files(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            let mut rest = s.as_str();
            while let Some(i) = rest.find("hashFiles(") {
                let after = &rest[i + "hashFiles(".len()..];
                let end = after.find(')').unwrap_or(after.len());
                for a in after[..end].split(',') {
                    let a = a.trim().trim_matches(['\'', '"']);
                    if !a.is_empty() {
                        out.push(a.to_string());
                    }
                }
                rest = &after[end..];
            }
        }
        Value::Array(a) => a.iter().for_each(|x| hash_files(x, out)),
        Value::Object(o) => o.values().for_each(|x| hash_files(x, out)),
        _ => {}
    }
}

/// Static single-value matrix entries (`matrix: { node: [20] }` → `node=20`).
fn static_matrix(job: &Value) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(Value::Object(mx)) = job.get("strategy").and_then(|s| s.get("matrix")) {
        for (k, v) in mx {
            match v {
                Value::Array(a) if a.len() == 1 => {
                    if let Some(s) = scalar(&a[0]) {
                        m.insert(k.clone(), s);
                    }
                }
                x => {
                    if let Some(s) = scalar(x) {
                        m.insert(k.clone(), s);
                    }
                }
            }
        }
    }
    m
}

/// Adds every task of one workflow file to `out`.
fn workflow(root: &Path, file: &Path, out: &mut Vec<DiscoveredTask>) {
    let rel = repo_rel(root, file);
    let doc = fsx::read_to_string(file)
        .ok()
        .and_then(|s| parse_yaml(&s))
        .filter(Value::is_object);
    let Some(doc) = doc else {
        let mut t = DiscoveredTask::new(job_id(&rel, "*"), "*", root.to_path_buf());
        t.files.push(file.to_path_buf());
        t.spec.uncertain(
            UncertaintyKind::ParseIncomplete,
            format!("unreadable workflow {rel}"),
        );
        t.spec.runs_unknown(format!("unreadable workflow {rel}"));
        out.push(t);
        return;
    };
    // Declared inputs from the trigger filters.
    let mut filters = TaskSpec::default();
    let (paths, ignore) = paths_filters(&doc);
    for p in &paths {
        if p.starts_with('!') {
            add_glob(root, p, PATHS_REASON, &mut filters);
            continue;
        }
        let (prefix, rest) = crate::glob::split_static_prefix(p);
        if rest.is_empty() {
            // A literal path: a directory covers everything below it, a file only itself.
            let abs = fsx::join(root, prefix);
            if fsx::is_dir(&abs) {
                filters.membership(abs, "**", PATHS_REASON);
            } else {
                let (d, f) = prefix.rsplit_once('/').unwrap_or(("", prefix));
                filters.membership(fsx::join(root, d), f, PATHS_REASON);
            }
        } else {
            filters.membership(fsx::join(root, prefix), rest, PATHS_REASON);
        }
    }
    if !ignore.is_empty() {
        let mut m = crate::spec::Membership::new(root.to_path_buf(), "**", PATHS_IGNORE_REASON);
        m.exclude = ignore.clone();
        filters.memberships.push(m);
    }
    let mut wf_env = BTreeMap::new();
    static_env(doc.get("env"), &mut wf_env);
    let wf_defaults = run_defaults(&doc, &RunDefaults::default());
    let Some(Value::Object(jobs)) = doc.get("jobs") else {
        return;
    };
    for (job_name, job) in jobs {
        let id = job_id(&rel, job_name);
        let mut base = DiscoveredTask::new(id.clone(), job_name.clone(), root.to_path_buf());
        base.files.push(file.to_path_buf());
        base.spec.merge(filters.clone());
        for n in str_list(job.get("needs")) {
            base.spec.dep(
                TaskRef::Id {
                    id: job_id(&rel, &n),
                },
                "gha needs",
            );
        }
        let mut globs = Vec::new();
        hash_files(job, &mut globs);
        for g in globs {
            add_glob(root, &g, "gha hashFiles", &mut base.spec);
        }
        if let Some(u) = job.get("uses").and_then(scalar) {
            reusable_workflow(root, &u, &mut base);
        }
        let mut env = wf_env.clone();
        static_env(job.get("env"), &mut env);
        let defaults = run_defaults(job, &wf_defaults);
        let matrix = static_matrix(job);
        let mut steps_out = Vec::new();
        let mut cx = StepsCx {
            root,
            id: &id,
            name: job_name,
            matrix: &matrix,
            files: vec![file.to_path_buf()],
            depth: 0,
        };
        if let Some(Value::Array(steps)) = job.get("steps") {
            cx.steps(
                steps,
                &env,
                &defaults,
                &BTreeMap::new(),
                &mut base,
                &mut steps_out,
            );
        }
        base.files = cx.files.clone();
        for s in &mut steps_out {
            s.files = cx.files.clone();
        }
        out.push(base);
        out.extend(steps_out);
    }
}

/// `jobs.<id>.uses`: a reusable workflow.
fn reusable_workflow(root: &Path, uses: &str, t: &mut DiscoveredTask) {
    if let Some(local) = uses.strip_prefix("./") {
        let p = root.join(local);
        let rel = repo_rel(root, &p);
        t.files.push(p.clone());
        match fsx::read_to_string(&p).ok().and_then(|s| parse_yaml(&s)) {
            Some(doc) => {
                if let Some(Value::Object(jobs)) = doc.get("jobs") {
                    for j in jobs.keys() {
                        t.spec.dep(
                            TaskRef::Id {
                                id: job_id(&rel, j),
                            },
                            "gha reusable workflow",
                        );
                        // The caller runs what the called jobs run (`needs:` does not).
                        t.spec.runs_task(TaskRef::Id {
                            id: job_id(&rel, j),
                        });
                    }
                }
            }
            None => {
                t.spec.uncertain(
                    UncertaintyKind::ParseIncomplete,
                    format!("unreadable reusable workflow {rel}"),
                );
                t.spec
                    .runs_unknown(format!("unreadable reusable workflow {rel}"));
            }
        }
    } else {
        t.spec.tool("gha", uses);
    }
}

struct StepsCx<'a> {
    root: &'a Path,
    id: &'a TaskId,
    name: &'a str,
    matrix: &'a BTreeMap<String, String>,
    files: Vec<PathBuf>,
    depth: usize,
}

impl StepsCx<'_> {
    /// Adds the contributions of `steps` (each `run:` step is its own
    /// contribution, with its own directory and environment).
    fn steps(
        &mut self,
        steps: &[Value],
        env: &BTreeMap<String, String>,
        defaults: &RunDefaults,
        inputs: &BTreeMap<String, String>,
        base: &mut DiscoveredTask,
        out: &mut Vec<DiscoveredTask>,
    ) {
        for step in steps {
            let mut senv = env.clone();
            static_env(step.get("env"), &mut senv);
            let ecx = ExprCx {
                root: self.root,
                env: &senv,
                inputs,
                matrix: self.matrix,
            };
            if let Some(run) = step.get("run").and_then(scalar) {
                let shell = step
                    .get("shell")
                    .and_then(scalar)
                    .or_else(|| defaults.shell.clone())
                    .unwrap_or_else(|| "bash".into());
                let wd = step
                    .get("working-directory")
                    .and_then(scalar)
                    .or_else(|| defaults.wd.clone());
                let mut t =
                    DiscoveredTask::new(self.id.clone(), self.name, self.root.to_path_buf());
                if let Some(wd) = wd {
                    let wd = subst_exprs(&wd, &ecx);
                    if wd.contains(UNKNOWN_EXPR) {
                        t.spec.uncertain(
                            UncertaintyKind::CwdDependent,
                            format!("computed working-directory `{wd}`"),
                        );
                    } else {
                        t.cwd = fsx::join(self.root, &wd);
                    }
                }
                let prog = shell.split_whitespace().next().unwrap_or("bash");
                let prog = prog.rsplit('/').next().unwrap_or(prog);
                if matches!(prog, "bash" | "sh" | "zsh" | "dash" | "ksh") {
                    t.lines.push(subst_exprs(&run, &ecx));
                } else {
                    t.spec.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!("run step with shell `{shell}`"),
                    );
                    crate::runs::scan_text(
                        &run,
                        &format!("a `{shell}` step"),
                        self.root,
                        &mut t.spec,
                    );
                }
                t.env = senv.clone();
                out.push(t);
            }
            if let Some(uses) = step.get("uses").and_then(scalar) {
                let mut with = BTreeMap::new();
                if let Some(Value::Object(w)) = step.get("with") {
                    for (k, v) in w {
                        if let Some(s) = scalar(v) {
                            if !uses.starts_with("./") {
                                // A third-party action given a command (`command: pnpm test`
                                // of a retry action) runs it.
                                crate::runs::scan_text(
                                    &s,
                                    &format!("`with.{k}` of {uses}"),
                                    self.root,
                                    &mut base.spec,
                                );
                            }
                            let s = subst_exprs(&s, &ecx);
                            if !s.contains(UNKNOWN_EXPR) {
                                with.insert(k.clone(), s);
                            }
                        }
                    }
                }
                self.uses(&uses, &with, &senv, base, out);
            }
        }
    }

    /// A step's `uses:`.
    fn uses(
        &mut self,
        uses: &str,
        with: &BTreeMap<String, String>,
        env: &BTreeMap<String, String>,
        base: &mut DiscoveredTask,
        out: &mut Vec<DiscoveredTask>,
    ) {
        if let Some(img) = uses.strip_prefix("docker://") {
            base.spec.tool("docker", img);
            return;
        }
        let Some(local) = uses.strip_prefix("./") else {
            base.spec.tool("gha", uses);
            // `with:` values naming repository files (`node-version-file: .nvmrc`,
            // `cache-dependency-path: pnpm-lock.yaml`).
            for v in with.values() {
                for line in v.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.contains("://") || line.contains('*') {
                        continue;
                    }
                    let p = fsx::join(self.root, line);
                    if p.starts_with(self.root) && p != self.root && fsx::is_file(&p) {
                        base.spec.defines(p.clone());
                        base.spec.input(p);
                    }
                }
            }
            return;
        };
        // A local action.
        let dir = fsx::join(self.root, local);
        let Some(file) = fsx::first_existing(&dir, &["action.yml", "action.yaml"]) else {
            base.spec.input(dir.join("action.yml"));
            base.spec.uncertain(
                UncertaintyKind::UnresolvedSpecifier,
                format!("local action `{uses}` without action.yml"),
            );
            return;
        };
        if !self.files.contains(&file) {
            self.files.push(file.clone());
        }
        let Some(doc) = fsx::read_to_string(&file).ok().and_then(|s| parse_yaml(&s)) else {
            base.spec.uncertain(
                UncertaintyKind::ParseIncomplete,
                format!("unreadable action {}", repo_rel(self.root, &file)),
            );
            return;
        };
        let runs = doc.get("runs").cloned().unwrap_or(Value::Null);
        let using = runs.get("using").and_then(scalar).unwrap_or_default();
        // Inputs: `with:` literals, else the declared defaults.
        let mut inputs = BTreeMap::new();
        if let Some(Value::Object(decl)) = doc.get("inputs") {
            for (k, d) in decl {
                if let Some(v) = with.get(k) {
                    inputs.insert(k.clone(), v.clone());
                } else if let Some(def) = d.get("default").and_then(scalar)
                    && !def.contains("${{")
                {
                    inputs.insert(k.clone(), def);
                }
            }
        }
        match using.as_str() {
            "composite" => {
                if self.depth >= 8 {
                    base.spec.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!("local action `{uses}` nested too deeply"),
                    );
                    return;
                }
                if let Some(Value::Array(steps)) = runs.get("steps") {
                    self.depth += 1;
                    let steps = steps.clone();
                    self.steps(&steps, env, &RunDefaults::default(), &inputs, base, out);
                    self.depth -= 1;
                }
            }
            u if u.starts_with("node") => {
                for k in ["pre", "main", "post"] {
                    if let Some(m) = runs.get(k).and_then(scalar) {
                        base.spec.defines(fsx::join(&dir, &m));
                        base.spec.entry(fsx::join(&dir, &m), EntryLang::Js);
                    }
                }
            }
            "docker" => {
                let image = runs.get("image").and_then(scalar).unwrap_or_default();
                if let Some(img) = image.strip_prefix("docker://") {
                    base.spec.tool("docker", img);
                } else if !image.is_empty() {
                    let df = fsx::join(&dir, &image);
                    base.spec.defines(df.clone());
                    base.spec.input(df.clone());
                    base.spec.dep(
                        TaskRef::Id {
                            id: TaskId::docker(repo_rel(self.root, &df), None),
                        },
                        "gha docker action",
                    );
                }
            }
            other => base.spec.uncertain(
                UncertaintyKind::UnknownCommand,
                format!("local action `{uses}` using `{other}`"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        let env: BTreeMap<String, String> = [("A".to_string(), "x".to_string())].into();
        let none = BTreeMap::new();
        let cx = ExprCx {
            root: Path::new("/r"),
            env: &env,
            inputs: &none,
            matrix: &none,
        };
        assert_eq!(subst_exprs("cd ${{ github.workspace }}/a", &cx), "cd /r/a");
        assert_eq!(subst_exprs("echo ${{env.A}}", &cx), "echo x");
        assert_eq!(
            subst_exprs("echo ${{ steps.x.outputs.y }}", &cx),
            format!("echo {UNKNOWN_EXPR}")
        );
        assert_eq!(subst_exprs("${{ unterminated", &cx), "${{ unterminated");
    }

    #[test]
    fn hash_files_patterns() {
        let v: Value = serde_json::json!({"key": "${{ runner.os }}-${{ hashFiles('**/pnpm-lock.yaml', 'a.txt') }}"});
        let mut out = Vec::new();
        hash_files(&v, &mut out);
        assert_eq!(out, ["**/pnpm-lock.yaml", "a.txt"]);
    }
}
