//! `Dockerfile`/`.dockerignore` and compose files: `COPY`/`ADD` sources, base
//! images as toolchain.
//!
//! See [`crate::sources`] for the [`TaskSource`]
//! contract and [`mod@crate::registry`] for the [`Recognizer`] contract.
//!
//! # Tasks
//!
//! | Task | What |
//! |---|---|
//! | `docker:<path>#<stage>` | one build stage (`FROM … AS stage`; unnamed stages by index) |
//! | `docker:<path>` | the image: the final stage |
//! | `docker:<compose>#<service>` | a compose service with a `build:` section (its context, `dockerfile`, `args`, `target`) |
//! | `docker:<compose>` | every service of a compose file |
//!
//! # What a stage depends on
//!
//! | Instruction | Becomes |
//! |---|---|
//! | `FROM image` | a `docker` toolchain entry (the reference string; a tag change also changes the Dockerfile, a config) |
//! | `FROM stage`, `COPY --from=stage`, `RUN --mount=from=stage` | a dep on that stage's task |
//! | `COPY`/`ADD` sources | inputs relative to the build context, filtered by `.dockerignore`; directories and globs become memberships (`COPY . .` is the whole context minus the ignore patterns) |
//! | `COPY --from=image`, `ADD <url>` | toolchain (`docker` image, `url`) |
//! | `RUN --mount=type=bind,source=x` | an input/membership of the context |
//! | `RUN` commands | the shell parser and the registry, with the image's `WORKDIR` mapped back to the repo directory it was copied from: task deps (`pnpm build`, `turbo run build --filter=x`, `make x`) and toolchain packages are kept. File paths in `RUN` refer to the image, whose repo files all come from `COPY`/`ADD` (already inputs), so unknown commands add nothing |
//! | `ARG`/`ENV` | substitution in `FROM`, `COPY`, `ADD`, `WORKDIR` (`ARG` defaults and compose `args`); an unknown variable in a source is an uncertainty with the whole context as a membership |
//!
//! The Dockerfile and both ignore-file locations (`<Dockerfile>.dockerignore`,
//! then `<context>/.dockerignore`; missing ones are probes) are configs.
//!
//! # Build context
//!
//! The compose file or `docker build` command decides the context. For a
//! standalone `docker:<path>` task it is inferred: the Dockerfile's directory,
//! unless an ancestor (up to the repo root) contains more of the `COPY` sources
//! (`docker build -f apps/web/Dockerfile .` from the root is common).
//! [`DockerBuild`] links a `docker build` whose context and build args match
//! the inferred ones to the `docker:` task, and inlines the analysis otherwise.

pub mod compose;
pub mod dockerfile;
pub mod ignore;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use self::{
    dockerfile::{Args as DArgs, BuildVars, Instruction, substitute},
    ignore::{Ignore, clean},
};
use crate::{
    recognizers::util::{Arg, Args, opt, opts, positionals},
    registry::{CommandReport, Recognition, RecognizeCx, Recognizer, registry},
    sources::{DiscoveredTask, SourceCx, TaskSource, Workspace},
    spec::{Command, OutputKind, TaskId, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&DockerSource]
}

/// Recognizers of this module (registered before the generic fallbacks).
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&DockerBuild]
}

/// How deep below a package root Dockerfiles and compose files are looked for.
const MAX_DEPTH: usize = 8;

/// The listing recorded for discovery: the file names [`is_dockerfile_name`]
/// and [`is_compose_name`] accept (a superset), at any depth.
const DISCOVERY_LISTING: &str = "**/{Dockerfile,dockerfile,Dockerfile.*,dockerfile.*,*.Dockerfile,\
                                 *.dockerfile,compose.yml,compose.yaml,compose.*.yml,compose.*.\
                                 yaml,docker-compose.yml,docker-compose.yaml,docker-compose.*.yml,\
                                 docker-compose.*.yaml,*.compose.yml,*.compose.yaml,*.\
                                 docker-compose.yml,*.docker-compose.yaml}";

/// Whether `name` is a Dockerfile name: `Dockerfile`, `Dockerfile.<x>`,
/// `<x>.Dockerfile`.
pub fn is_dockerfile_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "dockerfile" {
        return true;
    }
    let skip_ext = [
        ".dockerignore",
        ".md",
        ".txt",
        ".bak",
        ".orig",
        ".swp",
        ".json",
        ".yml",
        ".yaml",
        ".lock",
        ".sh",
        ".j2",
        ".tmpl",
        ".template",
    ];
    if skip_ext.iter().any(|e| lower.ends_with(e)) {
        return false;
    }
    lower.starts_with("dockerfile.") || (lower.ends_with(".dockerfile") && lower.len() > 11)
}

/// Whether `name` is a compose file name (`compose.yaml`,
/// `docker-compose.prod.yml`, …).
pub fn is_compose_name(name: &str) -> bool {
    let Some(stem) = name
        .strip_suffix(".yml")
        .or_else(|| name.strip_suffix(".yaml"))
    else {
        return false;
    };
    stem == "compose"
        || stem == "docker-compose"
        || stem.starts_with("compose.")
        || stem.starts_with("docker-compose.")
        || stem.ends_with(".compose")
        || stem.ends_with(".docker-compose")
}

/// Whether [`DockerSource`] discovers `file` (by name and location), so a
/// `docker:` task id for it resolves; otherwise recognizers inline the
/// analysis.
fn discoverable(repo_root: &Path, file: &Path, by_name: fn(&str) -> bool) -> bool {
    let Ok(rel) = file.strip_prefix(repo_root) else {
        return false;
    };
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let Some((name, dirs)) = comps.split_last() else {
        return false;
    };
    by_name(name)
        && dirs.len() <= MAX_DEPTH
        && dirs
            .iter()
            .all(|d| !fsx::skip_dir(d) && !matches!(d.as_str(), "dist" | "build" | "target"))
}

/// One build stage.
#[derive(Clone, Debug, Default)]
pub struct Stage {
    /// Position in the file.
    pub index: usize,
    /// `AS` name (lowercased), if any.
    pub alias: Option<String>,
    /// What it depends on (its own instructions only).
    pub spec: TaskSpec,
    /// Earlier stages it depends on.
    pub deps: BTreeSet<usize>,
}

impl Stage {
    /// The stage id used in task ids: the alias, else the index.
    pub fn id(&self) -> String {
        self.alias.clone().unwrap_or_else(|| self.index.to_string())
    }
}

/// One recognized `RUN` command (for coverage reports).
#[derive(Clone, Debug)]
pub struct RunReport {
    /// Stage index.
    pub stage: usize,
    /// The recognition.
    pub report: CommandReport,
}

/// A Dockerfile analyzed for one build context.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// The Dockerfile.
    pub dockerfile: PathBuf,
    /// The build context directory.
    pub context: PathBuf,
    /// Ignore-file locations (configs; the first existing one applies).
    pub ignore_files: Vec<PathBuf>,
    /// Stages in order.
    pub stages: Vec<Stage>,
    /// `RUN` command recognition.
    pub runs: Vec<RunReport>,
    /// The Dockerfile could not be read.
    pub missing: bool,
}

/// Per-stage evaluation state.
#[derive(Clone, Debug, Default)]
struct State {
    vars: BuildVars,
    workdir: String,
    /// Image directory ← repo directory, from `COPY`/`ADD`.
    mounts: Vec<(String, PathBuf)>,
    posix_shell: bool,
}

/// The local sources and `--from` of each `COPY`/`ADD`, with global `ARG`
/// defaults applied (for context inference).
fn local_sources(df: &dockerfile::Dockerfile) -> Vec<String> {
    let mut vars = BuildVars::new();
    let mut out = Vec::new();
    for ins in &df.instructions {
        match ins.keyword.as_str() {
            "ARG" => {
                for w in ins.args.words() {
                    if let Some((k, v)) = w.split_once('=') {
                        vars.insert(k.to_string(), Some(v.to_string()));
                    } else {
                        vars.entry(w.clone()).or_insert(None);
                    }
                }
            }
            "COPY" | "ADD" if ins.flag("from").is_none() => {
                let mut w = source_words(ins);
                w.pop();
                for s in w {
                    let (s, unk) = substitute(&s, &vars, df.escape);
                    if unk.is_empty() && !is_url(&s) {
                        let c = clean(&s);
                        if !c.is_empty() {
                            out.push(c);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The words of a `COPY`/`ADD` (sources and destination), without heredoc
/// markers.
fn source_words(ins: &Instruction) -> Vec<String> {
    let mut w = match &ins.args {
        DArgs::Json(v) => v.clone(),
        DArgs::Shell(s) => dockerfile::shell_words(s),
    };
    w.retain(|x| !x.starts_with("<<"));
    w
}

fn is_url(s: &str) -> bool {
    s.contains("://") || s.starts_with("git@")
}

/// Infers the build context of a standalone Dockerfile: the directory (from the
/// Dockerfile's own up to the repo root) where most `COPY` sources exist; ties
/// go to the nearest.
pub fn infer_context(ws: &Workspace, dockerfile: &Path, df: &dockerfile::Dockerfile) -> PathBuf {
    let own = dockerfile
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let srcs = local_sources(df);
    if srcs.is_empty() {
        return own;
    }
    // Scored by the first path segment of each source in the candidate's listing:
    // listing a directory is not a probe of the paths below it (which may be
    // other tasks' outputs).
    let firsts: BTreeSet<&str> = srcs
        .iter()
        .map(|s| s.split('/').next().unwrap_or(s))
        .collect();
    let narrow = if firsts.len() == 1 {
        firsts
            .iter()
            .next()
            .map(|s| s.to_string())
            .unwrap_or_default()
    } else {
        format!(
            "{{{}}}",
            firsts.iter().copied().collect::<Vec<_>>().join(",")
        )
    };
    let score = |d: &Path| {
        // Only the names checked are observed (a full listing would be shared by every
        // task of the package).
        let Some(entries) = turborepo_trace_core::record::pause_listings(|| fsx::read_dir(d))
        else {
            return 0;
        };
        turborepo_trace_core::record::listing(d, &narrow);
        let names: BTreeSet<String> = entries
            .iter()
            .filter_map(|(p, _)| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        srcs.iter()
            .filter(|s| {
                let first = s.split('/').next().unwrap_or(s);
                if crate::glob::is_glob(first) {
                    names.iter().any(|n| crate::glob::matches(first, n))
                } else {
                    names.contains(first)
                }
            })
            .count()
    };
    let mut best = (score(&own), own.clone());
    let mut d = own.parent();
    while let Some(x) = d {
        if !x.starts_with(&ws.root) {
            break;
        }
        let s = score(x);
        if s > best.0 {
            best = (s, x.to_path_buf());
        }
        if x == ws.root {
            break;
        }
        d = x.parent();
    }
    best.1
}

/// Analyzes the Dockerfile at `path` (or the inline text `inline`), with build
/// context `context` (inferred when `None`) and build args `args` (values
/// `None` are set from the environment: unknown).
pub fn analyze(
    ws: &Workspace,
    path: &Path,
    inline: Option<&str>,
    context: Option<&Path>,
    args: &BuildVars,
) -> Analysis {
    let text = match inline {
        Some(t) => Some(t.to_string()),
        None => fsx::read(path)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned()),
    };
    let missing = text.is_none();
    let df = dockerfile::parse(text.as_deref().unwrap_or(""));
    let ctx = match context {
        Some(c) => c.to_path_buf(),
        None => infer_context(ws, path, &df),
    };
    let mut a = Analysis {
        dockerfile: path.to_path_buf(),
        context: ctx.clone(),
        missing,
        ..Default::default()
    };
    let mut ignore_files = Vec::new();
    if inline.is_none() {
        let mut specific = path.as_os_str().to_owned();
        specific.push(".dockerignore");
        ignore_files.push(PathBuf::from(specific));
    }
    ignore_files.push(ctx.join(".dockerignore"));
    let ig = ignore_files
        .iter()
        .find_map(|f| fsx::read_to_string(f).ok())
        .map(|s| Ignore::parse(&s))
        .unwrap_or_default();
    a.ignore_files = ignore_files;
    let cx = Cx {
        ws,
        ctx: &ctx,
        ig: &ig,
        escape: df.escape,
    };
    let mut global = BuildVars::new();
    let mut states: Vec<State> = Vec::new();
    let mut cur: Option<(Stage, State)> = None;
    for ins in &df.instructions {
        if ins.keyword == "FROM" {
            if let Some((st, s)) = cur.take() {
                a.stages.push(st);
                states.push(s);
            }
            let index = a.stages.len();
            let mut stage = Stage {
                index,
                ..Default::default()
            };
            let words = ins.args.words();
            let image_raw = words.first().cloned().unwrap_or_default();
            if let Some(p) = words.iter().position(|w| w.eq_ignore_ascii_case("as"))
                && let Some(n) = words.get(p + 1)
            {
                stage.alias = Some(n.to_ascii_lowercase());
            }
            let mut gvars = global.clone();
            for (k, v) in args {
                if gvars.contains_key(k) {
                    gvars.insert(k.clone(), v.clone());
                }
            }
            let (image, unk) = substitute(&image_raw, &gvars, df.escape);
            let mut state = State {
                workdir: "/".into(),
                posix_shell: true,
                ..Default::default()
            };
            let lower = image.to_ascii_lowercase();
            if let Some(j) = a
                .stages
                .iter()
                .position(|s| s.alias.as_deref() == Some(lower.as_str()))
            {
                stage.deps.insert(j);
                state = states[j].clone();
            } else if !unk.is_empty() {
                // A computed base: maybe an earlier stage.
                stage.deps.extend(0..index);
                stage.spec.tool("docker", image.clone());
            } else if lower != "scratch" && !image.is_empty() {
                stage.spec.tool("docker", image.clone());
            }
            if image.is_empty() {
                stage.spec.uncertain(
                    UncertaintyKind::ParseIncomplete,
                    format!("FROM without an image (line {})", ins.line),
                );
            }
            cur = Some((stage, state));
            continue;
        }
        let Some((stage, state)) = cur.as_mut() else {
            // Before the first FROM: only global ARGs matter.
            if ins.keyword == "ARG" {
                for w in ins.args.words() {
                    let (k, v) = match w.split_once('=') {
                        Some((k, v)) => (k.to_string(), Some(v.to_string())),
                        None => (w.clone(), None),
                    };
                    let v = match args.get(&k) {
                        Some(x) => x.clone(),
                        None => v.map(|v| substitute(&v, &global, df.escape).0),
                    };
                    global.insert(k, v);
                }
            }
            continue;
        };
        cx.instruction(ins, stage, state, &global, args, &a.stages, &mut a.runs);
    }
    if let Some((st, s)) = cur.take() {
        a.stages.push(st);
        states.push(s);
    }
    a
}

/// What instruction evaluation needs.
struct Cx<'a> {
    ws: &'a Workspace,
    ctx: &'a Path,
    ig: &'a Ignore,
    escape: char,
}

impl Cx<'_> {
    #[allow(clippy::too_many_arguments)]
    fn instruction(
        &self,
        ins: &Instruction,
        stage: &mut Stage,
        state: &mut State,
        global: &BuildVars,
        args: &BuildVars,
        earlier: &[Stage],
        runs: &mut Vec<RunReport>,
    ) {
        match ins.keyword.as_str() {
            "ARG" => {
                for w in ins.args.words() {
                    let (k, default) = match w.split_once('=') {
                        Some((k, v)) => (k.to_string(), Some(v.to_string())),
                        None => (w.clone(), None),
                    };
                    let v = match args.get(&k) {
                        Some(x) => x.clone(),
                        None => match default {
                            Some(d) => Some(substitute(&d, &state.vars, self.escape).0),
                            None => global.get(&k).cloned().flatten(),
                        },
                    };
                    state.vars.insert(k, v);
                }
            }
            "ENV" => {
                if let DArgs::Shell(s) = &ins.args {
                    for (k, v) in dockerfile::env_pairs(s) {
                        let (v, unk) = substitute(&v, &state.vars, self.escape);
                        state
                            .vars
                            .insert(k, if unk.is_empty() { Some(v) } else { None });
                    }
                }
            }
            "WORKDIR" => {
                let w = match &ins.args {
                    DArgs::Shell(s) => s.trim().to_string(),
                    DArgs::Json(v) => v.first().cloned().unwrap_or_default(),
                };
                let (w, _) = substitute(&w, &state.vars, self.escape);
                state.workdir = image_join(&state.workdir, &w);
            }
            "SHELL" => {
                if let DArgs::Json(v) = &ins.args {
                    let sh = v
                        .first()
                        .map(|s| s.rsplit('/').next().unwrap_or(s).to_ascii_lowercase())
                        .unwrap_or_default();
                    state.posix_shell = matches!(
                        sh.as_str(),
                        "sh" | "bash" | "ash" | "dash" | "zsh" | "busybox"
                    );
                }
            }
            "COPY" | "ADD" => self.copy(ins, stage, state, earlier),
            "RUN" => self.run(ins, stage, state, earlier, runs),
            _ => {}
        }
    }

    /// `--from=x`: a stage dep or an image.
    fn stage_ref(&self, from: &str, stage: &mut Stage, state: &State, earlier: &[Stage]) {
        let (f, unk) = substitute(from, &state.vars, self.escape);
        let lower = f.to_ascii_lowercase();
        if let Some(j) = earlier
            .iter()
            .position(|s| s.alias.as_deref() == Some(lower.as_str()))
        {
            stage.deps.insert(j);
        } else if let Ok(n) = f.parse::<usize>()
            && n < earlier.len()
        {
            stage.deps.insert(n);
        } else if !unk.is_empty() {
            stage.deps.extend(0..earlier.len());
            stage.spec.tool("docker", f);
        } else {
            stage.spec.tool("docker", f);
        }
    }

    fn whole_context(&self, spec: &mut TaskSpec, reason: &str) {
        spec.memberships
            .extend(self.ig.memberships(self.ctx, None, &[], reason));
    }

    fn copy(&self, ins: &Instruction, stage: &mut Stage, state: &mut State, earlier: &[Stage]) {
        let mut words = source_words(ins);
        if let Some(from) = ins.flag("from") {
            self.stage_ref(from, stage, state, earlier);
            return;
        }
        let Some(dest_raw) = words.pop() else {
            return;
        };
        let (dest, _) = substitute(&dest_raw, &state.vars, self.escape);
        let dest_abs = image_join(&state.workdir, &dest);
        let excludes: Vec<String> = ins
            .flags_named("exclude")
            .into_iter()
            .map(|e| substitute(e, &state.vars, self.escape).0)
            .collect();
        let many = words.len() > 1;
        for raw in &words {
            let (s, unk) = substitute(raw, &state.vars, self.escape);
            if !unk.is_empty() {
                stage.spec.uncertain(
                    UncertaintyKind::ComputedSpecifier,
                    format!(
                        "{} source `{raw}` uses unknown {} (line {})",
                        ins.keyword,
                        unk.join(", "),
                        ins.line
                    ),
                );
                self.whole_context(&mut stage.spec, "computed COPY source");
                continue;
            }
            if is_url(&s) || (ins.keyword == "ADD" && s.ends_with(".git") && s.contains(':')) {
                stage.spec.tool("url", s);
                continue;
            }
            let c = clean(&s);
            let reason = format!("{} {}", ins.keyword, if c.is_empty() { "." } else { &c });
            if c.is_empty() {
                stage
                    .spec
                    .memberships
                    .extend(self.ig.memberships(self.ctx, None, &excludes, &reason));
                state
                    .mounts
                    .push((dest_abs.clone(), self.ctx.to_path_buf()));
                continue;
            }
            // `{c,c/**}` covers a file or a directory source without a filesystem check (a
            // probe during discovery would be shared by every task of the package).
            stage.spec.memberships.extend(self.ig.memberships(
                self.ctx,
                Some(&c),
                &excludes,
                &reason,
            ));
            let path = fsx::join(self.ctx, &c);
            let last = c.rsplit('/').next().unwrap_or(&c);
            if crate::glob::is_glob(&c) {
                if dest.ends_with('/') || many {
                    let (prefix, _) = crate::glob::split_static_prefix(&c);
                    state
                        .mounts
                        .push((dest_abs.clone(), fsx::join(self.ctx, prefix)));
                }
            } else if !last.contains('.') && !many {
                // Probably a directory: its contents go to `dest`.
                state.mounts.push((dest_abs.clone(), path));
            } else if dest.ends_with('/') || many {
                if let Some(parent) = path.parent() {
                    state.mounts.push((dest_abs.clone(), parent.to_path_buf()));
                }
            } else if let (Some(ip), Some(rp)) = (dest_abs.rsplit_once('/'), path.parent()) {
                // `COPY package.json /app/package.json`: the directory maps too.
                let ip = if ip.0.is_empty() { "/" } else { ip.0 };
                state.mounts.push((ip.to_string(), rp.to_path_buf()));
            }
        }
    }

    fn run(
        &self,
        ins: &Instruction,
        stage: &mut Stage,
        state: &State,
        earlier: &[Stage],
        runs: &mut Vec<RunReport>,
    ) {
        let mut mounts = state.mounts.clone();
        for m in ins.flags_named("mount") {
            let kv: BTreeMap<&str, &str> = m
                .split(',')
                .map(|p| p.split_once('=').unwrap_or((p, "")))
                .collect();
            let ty = kv.get("type").copied().unwrap_or("bind");
            if ty != "bind" {
                continue;
            }
            let target = kv
                .get("target")
                .or_else(|| kv.get("dst"))
                .or_else(|| kv.get("destination"))
                .copied()
                .unwrap_or("");
            if let Some(from) = kv.get("from") {
                self.stage_ref(from, stage, state, earlier);
                continue;
            }
            let src = kv
                .get("source")
                .or_else(|| kv.get("src"))
                .copied()
                .unwrap_or(".");
            let (src, unk) = substitute(src, &state.vars, self.escape);
            if !unk.is_empty() {
                stage.spec.uncertain(
                    UncertaintyKind::ComputedSpecifier,
                    format!("RUN --mount source `{src}` (line {})", ins.line),
                );
                self.whole_context(&mut stage.spec, "computed RUN --mount source");
                continue;
            }
            let c = clean(&src);
            let path = fsx::join(self.ctx, &c);
            let reason = format!("RUN --mount {}", if c.is_empty() { "." } else { &c });
            stage.spec.memberships.extend(self.ig.memberships(
                self.ctx,
                (!c.is_empty()).then_some(c.as_str()),
                &[],
                &reason,
            ));
            if !target.is_empty() {
                mounts.push((image_join(&state.workdir, target), path));
            }
        }
        if !state.posix_shell {
            return;
        }
        let script = match &ins.args {
            DArgs::Json(v) => v
                .iter()
                .map(|w| shell_quote(w))
                .collect::<Vec<_>>()
                .join(" "),
            DArgs::Shell(s) if s.trim().starts_with("<<") && !ins.heredocs.is_empty() => {
                let body = &ins.heredocs[0].body;
                if let Some(first) = body.lines().next()
                    && first.starts_with("#!")
                    && !["sh", "bash", "ash", "dash"]
                        .iter()
                        .any(|s| first.ends_with(s))
                {
                    return;
                }
                body.clone()
            }
            DArgs::Shell(s) => s.clone(),
        };
        let repo_cwd = map_image_path(&mounts, &state.workdir);
        let cwd = repo_cwd.clone().unwrap_or_else(|| self.ctx.to_path_buf());
        let env: BTreeMap<String, String> = state
            .vars
            .iter()
            .filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v)))
            .collect();
        let parsed = crate::shell::parse(&script, &cwd, &env);
        let Some(pkg) = self
            .ws
            .package_of(&cwd)
            .or_else(|| self.ws.packages.first())
        else {
            return;
        };
        let scripts: Vec<String> = crate::sources::read_json(&pkg.dir.join("package.json"))
            .and_then(|m| {
                m.get("scripts")
                    .and_then(|s| s.as_object())
                    .map(|o| o.keys().cloned().collect())
            })
            .unwrap_or_default();
        for cmd in &parsed.commands {
            let mut scratch = TaskSpec::default();
            let rcx = RecognizeCx {
                repo_root: &self.ws.root,
                package_root: &pkg.dir,
                env: &env,
                task_name: None,
                scripts: &scripts,
            };
            // Only task deps and toolchain names are kept, which no listing decides.
            let rep = turborepo_trace_core::record::pause_listings(|| {
                registry().recognize(cmd, &rcx, &mut scratch)
            });
            runs.push(RunReport {
                stage: stage.index,
                report: rep,
            });
            for d in scratch.task_deps {
                let target = match d.target {
                    TaskRef::Local { name } if repo_cwd.is_some() => TaskRef::Package {
                        package: pkg.rel.clone(),
                        name,
                    },
                    TaskRef::Upstream { name } if repo_cwd.is_some() => TaskRef::All { name },
                    TaskRef::Local { .. } | TaskRef::Upstream { .. } => continue,
                    t => t,
                };
                stage.spec.dep(target, format!("RUN: {}", d.reason));
            }
            stage.spec.toolchain.extend(scratch.toolchain);
        }
    }
}

/// `"word"` quoted for the shell parser.
fn shell_quote(w: &str) -> String {
    if !w.is_empty()
        && w.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:@%+,".contains(&b))
    {
        w.to_string()
    } else {
        format!("'{}'", w.replace('\'', "'\\''"))
    }
}

/// Joins an image path (POSIX) and normalizes it.
fn image_join(base: &str, p: &str) -> String {
    let full = if p.starts_with('/') {
        p.to_string()
    } else {
        format!("{}/{p}", base.trim_end_matches('/'))
    };
    let c = clean(&full);
    format!("/{c}")
}

/// The repo directory an image directory came from (the longest mapped prefix).
fn map_image_path(mounts: &[(String, PathBuf)], dir: &str) -> Option<PathBuf> {
    let dir = image_join("/", dir);
    let mut best: Option<(usize, PathBuf)> = None;
    for (img, repo) in mounts {
        let img = image_join("/", img);
        let rest = if img == "/" {
            Some(dir.trim_start_matches('/'))
        } else if dir == img {
            Some("")
        } else {
            dir.strip_prefix(&format!("{img}/"))
        };
        if let Some(rest) = rest
            && best.as_ref().is_none_or(|(l, _)| img.len() >= *l)
        {
            best = Some((
                img.len(),
                if rest.is_empty() {
                    repo.clone()
                } else {
                    fsx::join(repo, rest)
                },
            ));
        }
    }
    best.map(|(_, p)| p)
}

impl Analysis {
    /// The stage for `target` (a stage name or index), else the final stage.
    pub fn stage_index(&self, target: Option<&str>) -> Option<usize> {
        match target {
            None => self.stages.len().checked_sub(1),
            Some(t) => {
                let t = t.to_ascii_lowercase();
                self.stages
                    .iter()
                    .position(|s| s.alias.as_deref() == Some(t.as_str()))
                    .or_else(|| t.parse::<usize>().ok().filter(|&n| n < self.stages.len()))
            }
        }
    }

    /// Stage `idx` and the stages it depends on, transitively.
    pub fn closure(&self, idx: usize) -> Vec<usize> {
        let mut seen = BTreeSet::new();
        let mut todo = vec![idx];
        while let Some(i) = todo.pop() {
            if i < self.stages.len() && seen.insert(i) {
                todo.extend(self.stages[i].deps.iter().copied());
            }
        }
        seen.into_iter().collect()
    }

    /// The union of stage `idx`'s spec and its dependencies' (for builds whose
    /// context differs from the standalone task's), plus the Dockerfile and
    /// ignore files as configs.
    pub fn closure_spec(&self, idx: usize) -> TaskSpec {
        let mut spec = TaskSpec::default();
        for i in self.closure(idx) {
            spec.merge(self.stages[i].spec.clone());
        }
        self.add_configs(&mut spec);
        spec
    }

    /// The Dockerfile (unless inline) and ignore files as configs; the fallback
    /// membership when uncertain.
    pub fn add_configs(&self, spec: &mut TaskSpec) {
        spec.config(self.dockerfile.clone());
        for f in &self.ignore_files {
            spec.config(f.clone());
        }
        if !spec.uncertainties.is_empty() {
            spec.membership(
                self.context.clone(),
                "**",
                "Dockerfile not fully understood",
            );
        }
    }

    /// The tasks of this Dockerfile: one per stage and one for the file.
    pub fn tasks(&self, ws: &Workspace) -> Vec<DiscoveredTask> {
        let rel = ws.rel(&self.dockerfile);
        let dir = self
            .dockerfile
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let fname = self
            .dockerfile
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut out = Vec::new();
        for s in &self.stages {
            let id = s.id();
            let mut d = DiscoveredTask::new(
                TaskId::docker(rel.clone(), Some(id.clone())),
                id.clone(),
                dir.clone(),
            );
            d.files.push(self.dockerfile.clone());
            d.spec = s.spec.clone();
            for &j in &s.deps {
                d.spec.dep(
                    TaskRef::Id {
                        id: TaskId::docker(rel.clone(), Some(self.stages[j].id())),
                    },
                    "docker stage",
                );
            }
            self.add_configs(&mut d.spec);
            out.push(d);
        }
        let mut d = DiscoveredTask::new(TaskId::docker(rel.clone(), None), fname, dir);
        d.files.push(self.dockerfile.clone());
        match self.stages.last() {
            Some(last) => d.spec.dep(
                TaskRef::Id {
                    id: TaskId::docker(rel, Some(last.id())),
                },
                "final stage",
            ),
            None => d.spec.uncertain(
                UncertaintyKind::ParseIncomplete,
                "Dockerfile without a FROM instruction",
            ),
        }
        self.add_configs(&mut d.spec);
        out.push(d);
        out
    }
}

/// Dockerfiles and compose files.
pub struct DockerSource;

impl TaskSource for DockerSource {
    fn id(&self) -> &'static str {
        "docker"
    }

    fn ci_side_effects(&self) -> bool {
        // Images, not repository files.
        true
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &[
            "Dockerfile",
            ".dockerignore",
            "compose.yaml",
            "compose.yml",
            "docker-compose.yaml",
            "docker-compose.yml",
        ]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let ws = cx.workspace;
        let mut out = Vec::new();
        let files = crate::sources::make::find_quietly(
            cx,
            MAX_DEPTH,
            &|n| is_dockerfile_name(n) || is_compose_name(n),
            DISCOVERY_LISTING,
        );
        for f in files {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_compose_name(name) {
                out.extend(compose::tasks(ws, &f));
            } else {
                let a = analyze(ws, &f, None, None, &BuildVars::new());
                out.extend(a.tasks(ws));
            }
        }
        out
    }
}

/// `docker build`, `docker buildx build`, `docker image build`, `podman build`,
/// `buildah bud`, `docker compose build|up|run`, `docker-compose …`, `docker
/// run -v`.
pub struct DockerBuild;

const BUILD_VALUE_OPTS: &[&str] = &[
    "-f",
    "--file",
    "-t",
    "--tag",
    "--target",
    "--build-arg",
    "--platform",
    "--cache-from",
    "--cache-to",
    "-o",
    "--output",
    "--secret",
    "--ssh",
    "--label",
    "--network",
    "--progress",
    "--iidfile",
    "--metadata-file",
    "--build-context",
    "--add-host",
    "--shm-size",
    "--ulimit",
    "--builder",
    "--allow",
    "--attest",
    "--annotation",
    "--cgroup-parent",
    "--isolation",
    "-m",
    "--memory",
    "--memory-swap",
    "--cpu-shares",
    "--cpuset-cpus",
    "--cpu-period",
    "--cpu-quota",
    "--security-opt",
    "--call",
    "--policy",
    "--format",
    "--authfile",
    "--creds",
    "--layers",
    "--jobs",
    "--env",
    "-e",
];

/// Global options of the `docker` CLI that take a value.
const GLOBAL_VALUE_OPTS: &[&str] = &[
    "--context",
    "-c",
    "-H",
    "--host",
    "--config",
    "--log-level",
    "-l",
    "--tlscacert",
    "--tlscert",
    "--tlskey",
];

impl Recognizer for DockerBuild {
    fn id(&self) -> &'static str {
        "docker"
    }

    fn programs(&self) -> &'static [&'static str] {
        &[
            "docker",
            "podman",
            "nerdctl",
            "docker-compose",
            "podman-compose",
            "buildah",
        ]
    }

    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("docker").to_string();
        out.tool(
            "system",
            if prog == "docker-compose" {
                "docker"
            } else {
                prog.as_str()
            },
        );
        let words: Vec<&str> = cmd.args().collect();
        if prog == "docker-compose" || prog == "podman-compose" {
            return compose_command(cmd, 0, cx, out);
        }
        // Skip global options.
        let mut i = 0;
        while let Some(w) = words.get(i) {
            if GLOBAL_VALUE_OPTS.contains(w) {
                i += 2;
            } else if w.starts_with('-') {
                i += 1;
            } else {
                break;
            }
        }
        let sub = words.get(i).copied().unwrap_or("");
        let sub2 = words.get(i + 1).copied().unwrap_or("");
        match (prog.as_str(), sub, sub2) {
            ("buildah", "bud" | "build", _) | (_, "build", _) => build_command(cmd, i + 1, cx, out),
            (_, "buildx" | "builder" | "image", "build" | "b") => {
                build_command(cmd, i + 2, cx, out)
            }
            (_, "buildx", "bake") => {
                out.uncertain(UncertaintyKind::UnknownCommand, "docker buildx bake");
                cx.whole_package(out, "docker buildx bake");
                Recognition::Partial
            }
            (_, "compose", _) => compose_command(cmd, i + 1, cx, out),
            (_, "run" | "create", _) => run_command(cmd, i + 1, cx, out),
            _ => Recognition::Ignored,
        }
    }
}

/// Parses `--build-arg K=V` / `--build-arg K` into build vars.
fn build_args(vals: &[&str], env: &BTreeMap<String, String>) -> BuildVars {
    let mut out = BuildVars::new();
    for v in vals {
        match v.split_once('=') {
            Some((k, v)) => out.insert(k.to_string(), Some(v.to_string())),
            None => out.insert(v.to_string(), env.get(*v).cloned()),
        };
    }
    out
}

fn build_command(
    cmd: &Command,
    skip: usize,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Recognition {
    let args: Vec<Arg<'_>> = Args::new(cmd, skip, BUILD_VALUE_OPTS).collect_all();
    let pos = positionals(&args);
    let Some(ctx_arg) = pos.first().copied() else {
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            "docker build without a context",
        );
        cx.whole_package(out, "docker build without a context");
        return Recognition::Partial;
    };
    if ctx_arg == "-" || is_url(ctx_arg) {
        if is_url(ctx_arg) {
            out.tool("url", ctx_arg);
            return Recognition::Full;
        }
        out.uncertain(UncertaintyKind::UnknownCommand, "docker build from stdin");
        cx.whole_package(out, "docker build from stdin");
        return Recognition::Partial;
    }
    let ctx = cmd.path(ctx_arg);
    let file = match opt(&args, &["-f", "--file"]) {
        Some("-") => {
            out.uncertain(UncertaintyKind::UnknownCommand, "docker build -f - (stdin)");
            out.membership(ctx, "**", "docker build context");
            return Recognition::Partial;
        }
        Some(f) => cmd.path(f),
        None => ctx.join("Dockerfile"),
    };
    let target = opt(&args, &["--target"]);
    let bargs = build_args(&opts(&args, &["--build-arg"]), cx.env);
    for o in opts(&args, &["-o", "--output"]) {
        // `type=local,dest=out` or a plain path.
        let dest = o
            .split(',')
            .find_map(|p| p.strip_prefix("dest=").or_else(|| p.strip_prefix("dst=")))
            .or_else(|| (!o.contains('=') && o != "-").then_some(o));
        // `-o <dir>` exports the build result as real files that tests may load
        // (`require('../build/artifacts/out.js')`), so they link like any build
        // artifact. Inside CI tasks (`gha:`, `docker:` sources) every output is
        // reclassified as a side effect anyway (`analyze_package`).
        if let Some(d) = dest {
            out.output_of_kind(cmd.path(d), OutputKind::Artifact);
        }
    }
    for f in opts(&args, &["--iidfile", "--metadata-file"]) {
        out.output_of_kind(cmd.path(f), OutputKind::CiSideEffect);
    }
    for bc in opts(&args, &["--build-context"]) {
        if let Some((_, v)) = bc.split_once('=')
            && !v.contains("://")
            && !v.starts_with("docker-image:")
            && !v.starts_with("target:")
        {
            out.membership(cmd.path(v), "**", "docker --build-context");
        }
    }
    if !fsx::is_file(&file) {
        out.input(file.clone());
        out.uncertain(
            UncertaintyKind::UnresolvedSpecifier,
            format!(
                "missing Dockerfile {}",
                crate::sources::make::rel_of(cx.repo_root, &file)
            ),
        );
        out.membership(ctx, "**", "docker build context");
        return Recognition::Partial;
    }
    let ws = workspace_for(cx);
    let inferred = analyze(&ws, &file, None, None, &BuildVars::new());
    // The standalone `docker:` task describes this build only for the same context
    // and no build args.
    let same = inferred.context == fsx::normalize(&ctx) && bargs.is_empty();
    let a = if same {
        inferred
    } else {
        analyze(&ws, &file, None, Some(&ctx), &bargs)
    };
    let Some(idx) = a.stage_index(target) else {
        out.config(file.clone());
        out.uncertain(
            UncertaintyKind::UnresolvedSpecifier,
            format!("docker build --target {} not found", target.unwrap_or("")),
        );
        out.memberships.extend(Ignore::default().memberships(
            &a.context,
            None,
            &[],
            "docker build context",
        ));
        return Recognition::Partial;
    };
    if same && discoverable(cx.repo_root, &file, is_dockerfile_name) {
        let rel = crate::sources::make::rel_of(cx.repo_root, &file);
        let id = match target {
            Some(_) => TaskId::docker(rel, Some(a.stages[idx].id())),
            None => TaskId::docker(rel, None),
        };
        out.dep(TaskRef::Id { id }, "docker build");
        out.config(file);
        Recognition::Full
    } else {
        // A different context or build args: the standalone task does not describe this
        // build, so its analysis is inlined.
        let spec = a.closure_spec(idx);
        let uncertain = !spec.uncertainties.is_empty();
        out.merge(spec);
        if uncertain {
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// The workspace for analysis inside a recognizer (`RUN` deps resolve by
/// package directory): the JS workspace packages plus the current package.
///
/// Discovery re-reads every manifest, so inside a shared dir cache the result
/// is memoized per cache and package root (one docker build after another in a
/// large repo), and what discovery observed is replayed into the caller's
/// recording frame on every use.
fn workspace_for(cx: &RecognizeCx<'_>) -> std::sync::Arc<Workspace> {
    use std::sync::{Arc, OnceLock};

    use crate::recognizers::bins::RecMemo;
    static D: OnceLock<RecMemo<PathBuf, Arc<Workspace>>> = OnceLock::new();
    static M: OnceLock<RecMemo<(PathBuf, PathBuf), Arc<Workspace>>> = OnceLock::new();
    let discovered = D
        .get_or_init(RecMemo::new)
        .get_or(cx.repo_root.to_path_buf(), || {
            Arc::new(Workspace::discover(cx.repo_root))
        });
    let rel =
        crate::sources::norm_rel(&crate::sources::make::rel_of(cx.repo_root, cx.package_root));
    if discovered.packages.iter().any(|p| p.rel == rel) {
        // Adding a package that is already there rebuilds the same workspace from the
        // same manifests (whose reads discovery recorded).
        return discovered;
    }
    M.get_or_init(RecMemo::new).get_or(
        (cx.repo_root.to_path_buf(), cx.package_root.to_path_buf()),
        || {
            let mut rels: Vec<String> = discovered.packages.iter().map(|p| p.rel.clone()).collect();
            rels.push(rel);
            Arc::new(Workspace::from_roots(cx.repo_root, rels))
        },
    )
}

fn compose_command(
    cmd: &Command,
    skip: usize,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Recognition {
    let args: Vec<Arg<'_>> = Args::new(
        cmd,
        skip,
        &[
            "-f",
            "--file",
            "-p",
            "--project-name",
            "--project-directory",
            "--profile",
            "--env-file",
            "--ansi",
            "--parallel",
            "--progress",
        ],
    )
    .collect_all();
    // Global options come before the subcommand.
    let pos = positionals(&args);
    let Some(sub) = pos.first().copied() else {
        return Recognition::Ignored;
    };
    let builds = match sub {
        "build" => true,
        // `up --no-build` still starts the services, from the images an earlier build made.
        "up" => true,
        "run" | "create" => !args.iter().any(|a| matches!(a, Arg::Opt("--no-build", _))),
        _ => false,
    };
    if !builds {
        return Recognition::Ignored;
    }
    let starts = sub == "up"
        && !args.iter().any(|a| {
            matches!(
                a,
                Arg::Opt(
                    "--exit-code-from"
                        | "--abort-on-container-exit"
                        | "--abort-on-container-failure",
                    _
                )
            )
        });
    let dir = match opt(&args, &["--project-directory"]) {
        Some(d) => cmd.path(d),
        None => cmd.cwd.clone(),
    };
    for e in opts(&args, &["--env-file"]) {
        out.config(cmd.path(e));
    }
    // Without `-f`, `COMPOSE_FILE` names the files (a `VAR=x` prefix, or the
    // environment of the task: a CI job's `env:`), separated by
    // `COMPOSE_PATH_SEPARATOR` (`:` by default).
    let env_of = |k: &str| -> Option<String> {
        cmd.assigns
            .iter()
            .rev()
            .find(|(n, w)| n == k && !w.dynamic)
            .map(|(_, w)| w.text.clone())
            .or_else(|| cx.env.get(k).cloned())
            .filter(|v| !v.is_empty() && !v.contains('$'))
    };
    let from_env: Vec<PathBuf> = env_of("COMPOSE_FILE")
        .map(|v| {
            let sep = env_of("COMPOSE_PATH_SEPARATOR").unwrap_or_else(|| ":".into());
            v.split(sep.as_str())
                .filter(|p| !p.is_empty())
                .map(|p| fsx::join(&dir, p))
                .collect()
        })
        .unwrap_or_default();
    let files: Vec<PathBuf> = {
        let f = opts(&args, &["-f", "--file"]);
        if f.is_empty() && !from_env.is_empty() {
            from_env
        } else if f.is_empty() {
            match fsx::first_existing(
                &dir,
                &[
                    "compose.yaml",
                    "compose.yml",
                    "docker-compose.yaml",
                    "docker-compose.yml",
                ],
            ) {
                Some(p) => vec![p],
                None => Vec::new(),
            }
        } else {
            f.iter().map(|x| cmd.path(x)).collect()
        }
    };
    if files.is_empty() {
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            format!("docker compose {sub}: no compose file"),
        );
        cx.whole_package(out, "docker compose without a compose file");
        return Recognition::Partial;
    }
    // Services named after the subcommand (for build: every positional).
    let services: Vec<&str> = if sub == "build" {
        pos[1..].to_vec()
    } else {
        pos.get(1).copied().into_iter().collect()
    };
    let ws = workspace_for(cx);
    let mut partial = false;
    for f in &files {
        let rel = crate::sources::make::rel_of(cx.repo_root, f);
        out.config(f.clone());
        let tasks = compose::tasks(&ws, f);
        if !discoverable(cx.repo_root, f, is_compose_name) {
            // No `docker:` task for this file: inline the services' analysis.
            for t in tasks {
                let wanted = match &t.id {
                    Some(TaskId::Docker { stage: Some(s), .. }) => {
                        services.is_empty() || services.contains(&s.as_str())
                    }
                    _ => false,
                };
                if wanted {
                    let mut spec = t.spec;
                    spec.task_deps.retain(|d| {
                        !matches!(
                            &d.target,
                            TaskRef::Id {
                                id: TaskId::Docker { .. }
                            }
                        )
                    });
                    if !spec.uncertainties.is_empty() {
                        partial = true;
                    }
                    out.merge(spec);
                }
            }
            continue;
        }
        let have: BTreeSet<TaskId> = tasks.iter().filter_map(|t| t.id.clone()).collect();
        // `up` leaves the project running for the commands after it (the tests a job or
        // script runs next talk to it), unless it waits for a container to exit: then
        // the containers are the run itself.
        let file_id = TaskId::docker(rel.clone(), None);
        if starts && have.contains(&file_id) {
            out.starts_service(TaskRef::Id { id: file_id });
        }
        if services.is_empty() || files.len() > 1 {
            // Several files merge: link the whole files.
            let id = TaskId::docker(rel.clone(), None);
            if have.contains(&id) {
                out.dep(TaskRef::Id { id }, format!("docker compose {sub}"));
            }
            if files.len() > 1 && !services.is_empty() {
                partial = true;
            }
            continue;
        }
        for s in &services {
            let id = TaskId::docker(rel.clone(), Some(s.to_string()));
            if have.contains(&id) {
                out.dep(TaskRef::Id { id }, format!("docker compose {sub}"));
            }
        }
    }
    if partial {
        Recognition::Partial
    } else {
        Recognition::Full
    }
}

/// `docker run`: bind mounts of repo paths are read.
fn run_command(
    cmd: &Command,
    skip: usize,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Recognition {
    let words: Vec<&str> = cmd.args().skip(skip).collect();
    let mut mounted = false;
    let mut partial = false;
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        i += 1;
        let val = if w == "-v" || w == "--volume" || w == "--mount" {
            let v = words.get(i).copied();
            i += 1;
            v
        } else if let Some(v) = w
            .strip_prefix("--volume=")
            .or_else(|| w.strip_prefix("--mount="))
        {
            Some(v)
        } else if !w.starts_with('-') {
            // The image: the rest is the container command.
            break;
        } else {
            None
        };
        let Some(v) = val else { continue };
        let host = if v.contains("type=") {
            v.split(',')
                .find_map(|p| p.strip_prefix("source=").or_else(|| p.strip_prefix("src=")))
        } else {
            v.split(':').next()
        };
        let Some(host) = host else { continue };
        if !(host.starts_with('.') || host.starts_with('/') || host.contains('$')) {
            // A named volume.
            continue;
        }
        mounted = true;
        if host.contains('$') || host.contains('`') {
            partial = true;
            out.uncertain(
                UncertaintyKind::ComputedSpecifier,
                format!("docker run -v {host}"),
            );
            cx.whole_package(out, "docker run bind mount");
            continue;
        }
        let p = cmd.path(host);
        if p.starts_with(cx.repo_root) {
            out.membership(p, "**", "docker run bind mount");
        }
    }
    match (mounted, partial) {
        (_, true) => Recognition::Partial,
        (true, false) => Recognition::Full,
        (false, _) => Recognition::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for n in [
            "Dockerfile",
            "Dockerfile.dev",
            "api.Dockerfile",
            "web.dockerfile",
            "dockerfile",
        ] {
            assert!(is_dockerfile_name(n), "{n}");
        }
        for n in [
            "Dockerfile.dockerignore",
            "Dockerfile.md",
            ".dockerfile",
            "Dockerfiles",
        ] {
            assert!(!is_dockerfile_name(n), "{n}");
        }
        for n in [
            "compose.yaml",
            "docker-compose.yml",
            "docker-compose.prod.yml",
            "compose.override.yaml",
        ] {
            assert!(is_compose_name(n), "{n}");
        }
        for n in ["test.docker-compose.yaml", "local.compose.yml"] {
            assert!(is_compose_name(n), "{n}");
        }
        for n in ["composer.json", "compose.json", "my-compose.yml"] {
            assert!(!is_compose_name(n), "{n}");
        }
    }

    #[test]
    fn image_paths() {
        assert_eq!(image_join("/app", "src"), "/app/src");
        assert_eq!(image_join("/app", "/x/../y/"), "/y");
        let m = vec![
            ("/app".to_string(), PathBuf::from("/repo")),
            ("/app/apps/web".to_string(), PathBuf::from("/repo/apps/web")),
        ];
        assert_eq!(
            map_image_path(&m, "/app/apps/web/src"),
            Some(PathBuf::from("/repo/apps/web/src"))
        );
        assert_eq!(
            map_image_path(&m, "/app/packages"),
            Some(PathBuf::from("/repo/packages"))
        );
        assert_eq!(map_image_path(&m, "/usr"), None);
    }

    #[test]
    fn stages_and_sources() {
        let ws = Workspace::from_roots(Path::new("/nonexistent-docker"), Vec::<String>::new());
        let src = "ARG NODE=20\nFROM node:${NODE}-alpine AS base\nWORKDIR /app\nFROM base AS deps\n\
                   COPY package.json pnpm-lock.yaml ./\nRUN pnpm install --frozen-lockfile\n\
                   FROM base AS build\nCOPY --from=deps /app/node_modules ./node_modules\nCOPY . .\n\
                   ARG APP\nCOPY apps/${APP} ./x\nRUN pnpm build\nFROM nginx:1.27\nCOPY --from=build /app/dist /usr/share/nginx/html\n\
                   ADD https://example.com/x.tgz /tmp/\n";
        let a = analyze(
            &ws,
            Path::new("/nonexistent-docker/Dockerfile"),
            Some(src),
            Some(Path::new("/nonexistent-docker")),
            &BuildVars::new(),
        );
        assert_eq!(a.stages.len(), 4);
        assert_eq!(a.stages[0].spec.toolchain[0].name, "node:20-alpine");
        assert_eq!(a.stages[1].deps, BTreeSet::from([0]));
        assert_eq!(a.stages[1].spec.memberships.len(), 2);
        assert_eq!(a.stages[2].deps, BTreeSet::from([0, 1]));
        // `COPY apps/${APP}` with an unknown ARG.
        assert!(!a.stages[2].spec.uncertainties.is_empty());
        assert_eq!(a.stages[3].id(), "3");
        assert!(
            a.stages[3]
                .spec
                .toolchain
                .iter()
                .any(|t| t.ecosystem == "url")
        );
        assert_eq!(a.closure(3), vec![0, 1, 2, 3]);
        assert!(
            a.runs
                .iter()
                .any(|r| r.report.program.as_deref() == Some("pnpm"))
        );
    }
}
