//! Docker task source fixtures (`fixtures/tasks/docker/<case>`): every
//! `docker:` task (and every task whose commands the docker recognizer handled)
//! with resolved deps and spec, plus the recognition of each `RUN` command.
//! Paths are shown relative to the fixture root. Also a no-panic test of the
//! Dockerfile parser and analysis over random input.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeMap,
    fmt::Write,
    path::{Path, PathBuf},
};

use turborepo_trace_tasks::{
    Options, TaskSet, discover,
    sources::{
        Workspace,
        docker::{self, dockerfile},
    },
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/tasks/docker")
        .join(name)
        .canonicalize()
        .expect("fixture dir")
}

fn rel(s: String, root: &Path) -> String {
    s.replace(&format!("{}/", root.display()), "")
        .replace(&root.display().to_string(), ".")
}

fn render(set: &TaskSet, root: &Path) -> String {
    let mut out = String::new();
    for t in &set.tasks {
        let ours = t.id.kind() == "docker"
            || t.commands
                .iter()
                .any(|c| matches!(c.recognizer, Some("docker" | "make")));
        if !ours {
            continue;
        }
        let v = serde_json::json!({
            "sources": t.sources,
            "deps": t.deps,
            "commands": t.commands.iter().map(|c| format!("{} [{}] {:?}", c.text, c.recognizer.unwrap_or("-"), c.recognition)).collect::<Vec<_>>(),
            "spec": t.spec,
        });
        writeln!(
            out,
            "## {}\n{}\n",
            t.id,
            rel(serde_json::to_string_pretty(&v).unwrap(), root)
        )
        .unwrap();
    }
    out
}

fn runs(ws: &Workspace, root: &Path, dockerfile: &str) -> String {
    let a = docker::analyze(ws, &root.join(dockerfile), None, None, &BTreeMap::new());
    let mut out = format!(
        "## RUN commands of {dockerfile} (context {})\n",
        rel(a.context.display().to_string(), root)
    );
    for r in &a.runs {
        writeln!(
            out,
            "stage {}: {} [{}] {:?}",
            a.stages[r.stage].id(),
            r.report.text,
            r.report.recognizer.unwrap_or("-"),
            r.report.recognition
        )
        .unwrap();
    }
    out
}

/// Whether a membership of `spec` (or an input) names `needle` (a
/// context-relative source).
fn copies(spec: &turborepo_trace_tasks::spec::TaskSpec, needle: &str) -> bool {
    spec.inputs.iter().any(|p| p.ends_with(needle))
        || spec.memberships.iter().any(|m| m.pattern.contains(needle))
}

fn deps(set: &TaskSet, id: &str) -> Vec<String> {
    set.get(&id.parse().unwrap())
        .unwrap_or_else(|| panic!("missing task {id}"))
        .deps
        .iter()
        .map(|d| d.to_string())
        .collect()
}

#[test]
fn node_multistage() {
    let root = fixture("node-multistage");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    let s = render(&set, &root) + &runs(&ws, &root, "apps/web/Dockerfile");
    insta::assert_snapshot!("node_multistage", s);
    let df = "docker:apps/web/Dockerfile";
    assert_eq!(deps(&set, df), [format!("{df}#runner")]);
    assert_eq!(deps(&set, &format!("{df}#runner")), [format!("{df}#build")]);
    // RUN commands became task deps (turbo --filter, `pnpm build` in the mapped
    // WORKDIR).
    let build_deps = deps(&set, &format!("{df}#build"));
    assert!(
        build_deps.contains(&"task:apps/web#build".to_string()),
        "{build_deps:?}"
    );
    assert!(build_deps.contains(&format!("{df}#deps")));
    // The context was inferred as the repo root; `COPY . .` honours .dockerignore.
    let build = set.get(&format!("{df}#build").parse().unwrap()).unwrap();
    let m = build
        .spec
        .memberships
        .iter()
        .find(|m| m.reason == "COPY ." && m.pattern == "**")
        .expect("COPY . membership");
    assert_eq!(m.dir, root);
    assert!(m.exclude.iter().any(|e| e.contains("docs")));
    // `docker build -f apps/web/Dockerfile .` links to the task; another context
    // inlines.
    assert_eq!(deps(&set, "task:.#docker:web"), [df]);
    assert_eq!(
        deps(&set, "task:.#docker:web:build-stage"),
        [format!("{df}#build")]
    );
    let other = set
        .get(&"task:.#docker:web:other-context".parse().unwrap())
        .unwrap();
    assert!(other.deps.iter().all(|d| d.kind() != "docker"));
    assert!(copies(&other.spec, "pnpm-lock.yaml"));
    assert!(
        other
            .spec
            .memberships
            .iter()
            .all(|m| m.dir.ends_with("apps/web"))
    );
}

#[test]
fn compose() {
    let root = fixture("compose");
    let ws = Workspace::discover(&root);
    let set = discover(&ws, &Options::default());
    let s = render(&set, &root);
    insta::assert_snapshot!("compose", s);
    assert_eq!(
        deps(&set, "docker:compose.yaml"),
        ["docker:compose.yaml#api", "docker:compose.yaml#worker"]
    );
    let api = set
        .get(&"docker:compose.yaml#api".parse().unwrap())
        .unwrap();
    // `args` from the merge key and the service: APP=api; target runtime includes
    // build.
    assert!(copies(&api.spec, "api/package.json"));
    assert!(api.spec.toolchain.iter().any(|t| t.name == "node:22-slim"));
    assert!(
        api.spec.uncertainties.is_empty(),
        "{:?}",
        api.spec.uncertainties
    );
    let worker = set
        .get(&"docker:compose.yaml#worker".parse().unwrap())
        .unwrap();
    assert!(copies(&worker.spec, "requirements.txt"));
    // The worker's own Dockerfile task (context: its directory) exists too.
    assert!(
        set.get(&"docker:worker/Dockerfile".parse().unwrap())
            .is_some()
    );
}

/// `RUN` command recognition over a real repository's Dockerfiles:
/// `TURBO_TRACE_DEBUG_DOCKER_COVERAGE=<repo> cargo test -p
/// turborepo-trace-tasks --test docker -- --ignored --nocapture`.
#[test]
#[ignore]
fn run_coverage() {
    let Ok(root) = std::env::var("TURBO_TRACE_DEBUG_DOCKER_COVERAGE") else {
        return;
    };
    let root = PathBuf::from(root).canonicalize().expect("repo");
    let ws = Workspace::discover(&root);
    let mut files = Vec::new();
    for p in &ws.packages {
        let cx = turborepo_trace_tasks::sources::SourceCx {
            workspace: &ws,
            package: p,
        };
        files.extend(cx.find_files(8, &docker::is_dockerfile_name));
    }
    let mut by_rec: BTreeMap<String, usize> = BTreeMap::new();
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    let mut uncertain_stages = 0;
    let mut stages = 0;
    let mut contexts: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        let a = docker::analyze(&ws, f, None, None, &BTreeMap::new());
        let own = f.parent().unwrap();
        *contexts
            .entry(
                if a.context == own {
                    "dockerfile dir"
                } else {
                    "ancestor"
                }
                .to_string(),
            )
            .or_default() += 1;
        stages += a.stages.len();
        uncertain_stages += a
            .stages
            .iter()
            .filter(|s| !s.spec.uncertainties.is_empty())
            .count();
        for r in &a.runs {
            *by_rec
                .entry(format!("{:?}", r.report.recognition))
                .or_default() += 1;
            if r.report.recognition == turborepo_trace_tasks::registry::Recognition::Unknown {
                *unknown
                    .entry(r.report.program.clone().unwrap_or_default())
                    .or_default() += 1;
            }
        }
    }
    let mut top: Vec<(String, usize)> = unknown.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(15);
    println!(
        "{} Dockerfiles, {stages} stages ({uncertain_stages} uncertain), contexts \
         {contexts:?}\nRUN commands: {by_rec:?}\ntop unknown RUN programs: {top:?}",
        files.len()
    );
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const TOKENS: &[&str] = &[
    "FROM ",
    "AS ",
    "COPY ",
    "ADD ",
    "RUN ",
    "ARG ",
    "ENV ",
    "WORKDIR ",
    "SHELL ",
    "ONBUILD ",
    "--from=",
    "--mount=type=bind,source=",
    "--chown=",
    "--exclude=",
    "[\"",
    "\"]",
    "\", \"",
    "<<",
    "<<-",
    "EOF",
    "\n",
    "\\\n",
    "`\n",
    "# escape=`\n",
    "$",
    "${",
    "}",
    ":-",
    ":+",
    ".",
    "./",
    "/",
    "*",
    "**",
    "?",
    "[",
    "a",
    "src",
    "node:20",
    "0",
    "1",
    " ",
    "&&",
    "pnpm build",
    "make x",
    "http://x/y",
    "=",
];

#[test]
fn parser_never_panics() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let ws = Workspace::from_roots(Path::new("/nonexistent-fuzz"), Vec::<String>::new());
    let path = Path::new("/nonexistent-fuzz/Dockerfile");
    for round in 0..3000 {
        let mut s = String::new();
        for _ in 0..rng.below(60) {
            if round % 3 == 0 {
                let b: Vec<u8> = (0..rng.below(8)).map(|_| rng.next() as u8).collect();
                s.push_str(&String::from_utf8_lossy(&b));
            } else {
                s.push_str(TOKENS[rng.below(TOKENS.len())]);
            }
        }
        let _ = dockerfile::parse(&s);
        let a = docker::analyze(
            &ws,
            path,
            Some(&s),
            Some(Path::new("/nonexistent-fuzz")),
            &BTreeMap::new(),
        );
        if let Some(i) = a.stage_index(None) {
            let _ = a.closure_spec(i);
        }
        let _ = a.tasks(&ws);
        let _ =
            docker::ignore::Ignore::parse(&s).memberships(Path::new("/c"), Some("src"), &[], "x");
    }
}
