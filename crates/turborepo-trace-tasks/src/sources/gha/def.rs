//! The *effective definition* of a workflow job: what decides how the job
//! executes, as one JSON value that can be compared between two versions of the
//! repository.
//!
//! A workflow file is not one input of all its jobs. An edit changes a job only
//! if it changes this value:
//!
//! | Part | In the definition |
//! |---|---|
//! | the job's own keys: `runs-on`, `container`, `services`, `env`, `defaults`, `strategy` (matrix), `steps` (every key except a step's `name`), `uses`/`with`/`secrets` (a reusable workflow call), `needs`, `permissions`, `environment`, `timeout-minutes`, `continue-on-error`, `outputs`, and any key not listed below | yes |
//! | the job's `name`, `if`, `concurrency` | no: display, whether the job runs, and scheduling |
//! | workflow-level `env`, `defaults`, `permissions`, and any key not listed below | yes, for every job of the file |
//! | workflow-level `name`, `run-name`, `concurrency`, and `on` (events, branches, `paths`, schedules) | no |
//! | `on.workflow_call.inputs`/`secrets` and `on.workflow_dispatch.inputs` (types and defaults are the values of `inputs.x`) | yes |
//! | a local action a step uses (`uses: ./path`): its `action.yml` `runs` and `inputs` (defaults), with the local actions its own steps use, recursively | yes (not its `name`, `description`, `author`, `branding`, or the descriptions of inputs and outputs) |
//! | `uses: org/action@ref`, `docker://image` | yes, as written: a version bump changes the job |
//! | comments, formatting, key order, YAML anchors | no (the parsed value is compared) |
//!
//! A step-level `if` stays: it decides which steps set up the job. When in
//! doubt a key counts. The jobs a job calls (a reusable workflow) or needs are
//! separate jobs with their own definitions; how changes travel between jobs is
//! decided by the caller (`crates/turborepo-trace/src/workflows.rs`).
//!
//! Files are read through a callback, so the same code computes the definitions
//! of the working tree and of a base revision.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::sources::taskfile::{parse_yaml, scalar};

/// Job keys that don't decide how the job executes.
const JOB_STEERING: &[&str] = &["name", "if", "concurrency"];
/// Workflow keys that don't decide how its jobs execute (`on` is filtered
/// separately). (`true`: `on` as YAML 1.1 parsers read it.)
const WORKFLOW_STEERING: &[&str] = &["name", "run-name", "concurrency", "jobs", "on", "true"];
/// Action metadata keys that don't decide what the action does.
const ACTION_STEERING: &[&str] = &["name", "description", "author", "branding"];
/// Nesting limit for local actions.
const MAX_DEPTH: usize = 8;

/// Whether `rel` (repo-relative) is a workflow file.
pub fn is_workflow_file(rel: &str) -> bool {
    rel.strip_prefix(".github/workflows/")
        .is_some_and(|n| !n.contains('/') && (n.ends_with(".yml") || n.ends_with(".yaml")))
}

/// Whether `rel` is named like a local action's metadata file.
pub fn is_action_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name == "action.yml" || name == "action.yaml"
}

/// The effective definition of one job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobDef {
    /// The comparable value (see the module docs).
    pub value: Value,
    /// The files it was computed from, repo-relative: the workflow file and the
    /// metadata files of the local actions it uses (also missing ones, as
    /// they were looked up).
    pub files: BTreeSet<String>,
}

/// Computes definitions, reading repo-relative files with `read` (`None`:
/// missing).
pub struct Defs<'a> {
    read: &'a dyn Fn(&str) -> Option<String>,
    docs: BTreeMap<String, Option<Value>>,
}

impl<'a> Defs<'a> {
    /// A new context over `read`.
    pub fn new(read: &'a dyn Fn(&str) -> Option<String>) -> Self {
        Defs {
            read,
            docs: BTreeMap::new(),
        }
    }

    fn doc(&mut self, rel: &str) -> Option<Value> {
        if let Some(d) = self.docs.get(rel) {
            return d.clone();
        }
        let d = (self.read)(rel)
            .and_then(|s| parse_yaml(&s))
            .filter(Value::is_object);
        self.docs.insert(rel.to_string(), d.clone());
        d
    }

    /// The definitions of the jobs of workflow `rel`, by job id. `None` when
    /// the file is missing or not a YAML mapping.
    pub fn workflow(&mut self, rel: &str) -> Option<BTreeMap<String, JobDef>> {
        let doc = self.doc(rel)?;
        let shared = workflow_part(&doc);
        let mut out = BTreeMap::new();
        if let Some(Value::Object(jobs)) = doc.get("jobs") {
            for (id, job) in jobs {
                let mut files = BTreeSet::new();
                files.insert(rel.to_string());
                let mut v = Map::new();
                v.insert("workflow".into(), shared.clone());
                v.insert("job".into(), self.job_part(job, &mut files));
                out.insert(
                    id.clone(),
                    JobDef {
                        value: Value::Object(v),
                        files,
                    },
                );
            }
        }
        Some(out)
    }

    fn job_part(&mut self, job: &Value, files: &mut BTreeSet<String>) -> Value {
        let Value::Object(job) = job else {
            return job.clone();
        };
        let mut o = Map::new();
        for (k, v) in job {
            if JOB_STEERING.contains(&k.as_str()) {
                continue;
            }
            if k == "steps" {
                o.insert(k.clone(), self.steps(v, files, 0));
            } else {
                o.insert(k.clone(), v.clone());
            }
        }
        Value::Object(o)
    }

    fn steps(&mut self, steps: &Value, files: &mut BTreeSet<String>, depth: usize) -> Value {
        let Value::Array(steps) = steps else {
            return steps.clone();
        };
        let mut out = Vec::with_capacity(steps.len());
        for step in steps {
            let Value::Object(step) = step else {
                out.push(step.clone());
                continue;
            };
            let mut o = Map::new();
            for (k, v) in step {
                if k != "name" {
                    o.insert(k.clone(), v.clone());
                }
            }
            if let Some(local) = step
                .get("uses")
                .and_then(scalar)
                .as_deref()
                .and_then(|u| u.strip_prefix("./"))
            {
                o.insert("(action)".into(), self.action(local, files, depth));
            }
            out.push(Value::Object(o));
        }
        Value::Array(out)
    }

    /// The definition of the local action in directory `dir` (repo-relative).
    fn action(&mut self, dir: &str, files: &mut BTreeSet<String>, depth: usize) -> Value {
        let dir = dir.trim_end_matches('/');
        let mut found = None;
        for n in ["action.yml", "action.yaml"] {
            let rel = if dir.is_empty() || dir == "." {
                n.to_string()
            } else {
                format!("{dir}/{n}")
            };
            files.insert(rel.clone());
            if let Some(d) = self.doc(&rel) {
                found = Some(d);
                break;
            }
        }
        let Some(Value::Object(doc)) = found else {
            return Value::String("(missing or unreadable action)".into());
        };
        if depth >= MAX_DEPTH {
            return Value::String("(action nested too deeply)".into());
        }
        let mut o = Map::new();
        for (k, v) in &doc {
            if ACTION_STEERING.contains(&k.as_str()) {
                continue;
            }
            match (k.as_str(), v) {
                ("inputs" | "outputs", Value::Object(decls)) => {
                    let mut d = Map::new();
                    for (name, decl) in decls {
                        let mut decl = decl.clone();
                        if let Value::Object(m) = &mut decl {
                            m.remove("description");
                            m.remove("deprecationMessage");
                        }
                        d.insert(name.clone(), decl);
                    }
                    o.insert(k.clone(), Value::Object(d));
                }
                ("runs", Value::Object(runs)) => {
                    let mut r = Map::new();
                    for (rk, rv) in runs {
                        if rk == "steps" {
                            r.insert(rk.clone(), self.steps(rv, files, depth + 1));
                        } else {
                            r.insert(rk.clone(), rv.clone());
                        }
                    }
                    o.insert(k.clone(), Value::Object(r));
                }
                _ => {
                    o.insert(k.clone(), v.clone());
                }
            }
        }
        Value::Object(o)
    }
}

/// The workflow-level part of every job's definition.
fn workflow_part(doc: &Value) -> Value {
    let mut o = Map::new();
    let Value::Object(doc) = doc else {
        return Value::Null;
    };
    for (k, v) in doc {
        if !WORKFLOW_STEERING.contains(&k.as_str()) {
            o.insert(k.clone(), v.clone());
        }
    }
    // Declared inputs and secrets: their types and defaults are values of
    // `inputs.x`.
    if let Some(Value::Object(on)) = doc.get("on").or_else(|| doc.get("true")) {
        let mut decl = Map::new();
        for ev in ["workflow_call", "workflow_dispatch"] {
            let mut e = Map::new();
            for part in ["inputs", "secrets"] {
                if let Some(Value::Object(items)) = on.get(ev).and_then(|x| x.get(part)) {
                    let mut d = Map::new();
                    for (name, item) in items {
                        let mut item = item.clone();
                        if let Value::Object(m) = &mut item {
                            m.remove("description");
                        }
                        d.insert(name.clone(), item);
                    }
                    e.insert(part.into(), Value::Object(d));
                }
            }
            if !e.is_empty() {
                decl.insert(ev.into(), Value::Object(e));
            }
        }
        if !decl.is_empty() {
            o.insert("(declared)".into(), Value::Object(decl));
        }
    }
    Value::Object(o)
}

/// The ids of the jobs whose definition differs between `old` and `new` (jobs
/// only in `new` included; jobs only in `old` no longer exist).
pub fn changed_jobs(
    old: Option<&BTreeMap<String, JobDef>>,
    new: &BTreeMap<String, JobDef>,
) -> Vec<String> {
    new.iter()
        .filter(|(id, d)| old.and_then(|o| o.get(*id)).map(|o| &o.value) != Some(&d.value))
        .map(|(id, _)| id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs(files: &[(&str, &str)]) -> BTreeMap<String, JobDef> {
        let read = |p: &str| {
            files
                .iter()
                .find(|(n, _)| *n == p)
                .map(|(_, s)| s.to_string())
        };
        Defs::new(&read)
            .workflow(".github/workflows/ci.yml")
            .unwrap()
    }

    const BASE: &str = "name: CI\non:\n  pull_request:\n    paths: ['src/**']\nenv:\n  A: \
                        1\njobs:\n  test:\n    name: Test\n    runs-on: ubuntu-latest\n    \
                        steps:\n      - uses: actions/checkout@v4\n      - name: Setup\n        \
                        uses: ./.github/actions/setup\n      - run: pnpm test\n  deploy:\n    \
                        runs-on: ubuntu-latest\n    steps:\n      - run: ./deploy.sh\n";
    const ACTION: &str = "name: setup\ndescription: d\nruns:\n  using: composite\n  steps:\n    - \
                          uses: actions/setup-node@v4\n      with:\n        node-version: 20\n";

    fn changed(new_wf: &str, new_action: &str) -> Vec<String> {
        let wf = ".github/workflows/ci.yml";
        let ac = ".github/actions/setup/action.yml";
        let old = defs(&[(wf, BASE), (ac, ACTION)]);
        let new = defs(&[(wf, new_wf), (ac, new_action)]);
        changed_jobs(Some(&old), &new)
    }

    #[test]
    fn steering_edits_change_no_job() {
        // Comments, formatting, names, triggers, path filters, concurrency, job `if`.
        let edited = BASE
            .replace("name: CI", "# a comment\nname:   Continuous\nrun-name: x")
            .replace(
                "paths: ['src/**']",
                "paths: ['lib/**', 'src/**']\n  push: {}",
            )
            .replace(
                "    name: Test\n",
                "    name: Unit tests\n    if: github.actor != 'bot'\n    concurrency: g\n",
            )
            .replace("      - name: Setup\n", "      - name: Set up node\n");
        assert_ne!(edited, BASE);
        assert_eq!(changed(&edited, ACTION), Vec::<String>::new());
        assert_eq!(
            changed(
                BASE,
                &ACTION.replace("description: d", "description: other")
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn execution_edits_change_their_jobs() {
        assert_eq!(
            changed(&BASE.replace("./deploy.sh", "./deploy.sh --prod"), ACTION),
            ["deploy"]
        );
        // The runner image of one job.
        let edited = BASE.replacen("runs-on: ubuntu-latest", "runs-on: ubuntu-24.04", 1);
        assert_eq!(changed(&edited, ACTION), ["test"]);
        // Workflow-level env reaches every job.
        assert_eq!(
            changed(&BASE.replace("A: 1", "A: 2"), ACTION),
            ["deploy", "test"]
        );
        // A local action reaches the jobs that use it.
        assert_eq!(
            changed(
                BASE,
                &ACTION.replace("node-version: 20", "node-version: 22")
            ),
            ["test"]
        );
        // A pinned third-party action.
        assert_eq!(
            changed(&BASE.replace("checkout@v4", "checkout@v5"), ACTION),
            ["test"]
        );
        // A new job.
        let added = format!("{BASE}  lint:\n    runs-on: x\n    steps: []\n");
        assert_eq!(changed(&added, ACTION), ["lint"]);
    }

    #[test]
    fn nested_actions_and_declared_inputs() {
        let wf = ".github/workflows/ci.yml";
        let a = ".github/actions/setup/action.yml";
        let b = ".github/actions/inner/action.yml";
        let outer = "runs:\n  using: composite\n  steps:\n    - uses: ./.github/actions/inner\n";
        let inner = "runs:\n  using: composite\n  steps:\n    - run: echo 1\n      shell: bash\n";
        let old = defs(&[(wf, BASE), (a, outer), (b, inner)]);
        let new = defs(&[
            (wf, BASE),
            (a, outer),
            (b, &inner.replace("echo 1", "echo 2")),
        ]);
        assert_eq!(changed_jobs(Some(&old), &new), ["test"]);
        assert!(old["test"].files.contains(b));
        assert!(!old["deploy"].files.contains(a));

        let call =
            "on:\n  workflow_call:\n    inputs:\n      node:\n        description: v\n        \
             default: '20'\njobs:\n  t:\n    runs-on: x\n    steps: []\n";
        let old = defs(&[(wf, call)]);
        let same = defs(&[(wf, &call.replace("description: v", "description: w"))]);
        assert!(changed_jobs(Some(&old), &same).is_empty());
        let new = defs(&[(wf, &call.replace("'20'", "'22'"))]);
        assert_eq!(changed_jobs(Some(&old), &new), ["t"]);
    }

    #[test]
    fn file_kinds() {
        assert!(is_workflow_file(".github/workflows/ci.yml"));
        assert!(!is_workflow_file(".github/workflows/sub/ci.yml"));
        assert!(!is_workflow_file(".github/workflows/notes.md"));
        assert!(is_action_file(".github/actions/setup/action.yaml"));
        assert!(!is_action_file(".github/actions/setup/main.yml"));
    }
}
