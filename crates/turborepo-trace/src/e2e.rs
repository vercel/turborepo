//! `affected --tests playwright`: which end-to-end specs a change of
//! a Next.js app affects.
//!
//! ```sh
//! affected --tests playwright --since <base>
//! ```
//!
//! The selection is made from source ([`crate::e2e_static`]): the entry files
//! of every route are entries of the evaluation next to the specs, and a spec
//! runs when its own code changed or a route it visits is reached by the
//! change. That is the only way to select: what is selected is a function of
//! the two source trees.
//!
//! # Validation tooling: two builds and recorded visits
//!
//! The rest of this module is what the static answer is checked against (two
//! builds of the app and the visits a recorder saw), not a way to select. Its
//! flags are hidden from `--help`:
//!
//! ```sh
//! affected --tests playwright --since <recorded commit> \
//!     --next-fingerprint head.json --visits visits/ [--base-fingerprint base.json]
//! ```
//!
//! With a fingerprint of the build under test, Playwright specs are decided by
//! what a recorder saw them request. A spec is skipped only if all of this
//! holds:
//!
//! 1. its own import closure is unchanged (the ordinary `affected` verdict of
//!    the spec file, of its Playwright config and of the config's global
//!    setup), and none of its screenshots changed;
//! 2. it has a recording of every one of its tests against a build whose
//!    fingerprint is known, the recorded runs passed, and the spec file is the
//!    one that was recorded;
//! 3. everything it reached on that build is the same on the new build
//!    ([`turborepo_trace_frameworks::playwright::selection`]).
//!
//! A spec without a recording falls back to the visits read from its source
//! ([`turborepo_trace_frameworks::playwright::visits`]). That is only enough to
//! skip it when every navigation is a known URL that matches without ambiguity,
//! the routes it can reach from there (through links and calls, transitively)
//! are all known and unchanged, and nothing global, no route table entry and no
//! `public/` file differs. Otherwise it runs.
//!
//! A changed `.env*` file in or above the app directory or the Playwright
//! config's directory runs every spec: the server reads it when it starts, so
//! it is not part of the build output.
//!
//! A helper that reads a file relative to `process.cwd()` does not make its
//! specs uncertain: Playwright is started in the config's directory or in the
//! package directory above it, so [`E2e::entry_cwds`] declares those to the
//! per-test resolution of working-directory-relative references
//! (`turborepo_trace_store::Graph::cwd_patch`).
//!
//! `affected.rs` calls [`prepare`] (the specs become entries of the ordinary
//! evaluation) and [`E2e::finish`] (the output).

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::json;
use sha1::{Digest, Sha1};
use turborepo_trace_frameworks::{
    Framework,
    inputs::Target,
    nextjs::{
        LoadOptions, NextApp,
        build::{
            diff::{CssMode, FingerprintDiff, diff},
            fingerprint::Fingerprint,
            invokes::InvokeTarget,
        },
        is_next_app,
        jseval::Files,
        matcher::{Ambiguity, Outcome},
        select::GraphImports,
    },
    playwright::{
        Playwright, load_configs,
        recorded::{self, Loaded, RouteMapper, TestRuns, Visits},
        selection::{Comparison, Why},
        visits::{ExtractOptions, extract_visits},
    },
};
use turborepo_trace_graph::{
    AffectedReport, Cause, Change, ChangeSet, EntryResult, GraphView, Verdict,
};
use turborepo_trace_store::Graph;

use crate::{
    ExitCode,
    e2e_static::{self, Static},
};

/// The framework id.
pub const FRAMEWORK: &str = "playwright";

/// Options of `affected --tests playwright`.
#[derive(Debug, Default)]
pub struct E2eArgs {
    /// Validation tooling, not a way to select (hidden): it compares the
    /// selection from source with two builds and recorded visits. The
    /// fingerprint of the build under test (`next fingerprint`). Repeatable:
    /// one per app.
    pub next_fingerprint: Vec<PathBuf>,
    /// Validation tooling, not a way to select (hidden): a directory of
    /// recorded visits (written by the Playwright recorder). Repeatable.
    pub visits: Vec<PathBuf>,
    /// Validation tooling, not a way to select (hidden): the fingerprint of a
    /// build the recordings ran against. Repeatable. Fingerprint files
    /// inside a --visits directory are found without it.
    pub base_fingerprint: Vec<PathBuf>,
    /// With --tests playwright: only the specs of this Playwright config.
    /// Repeatable. Default: every `playwright.config.*` under the root.
    pub playwright_config: Vec<PathBuf>,
    /// With --tests playwright: how far a spec's visit is followed from the
    /// URLs it names. `links` (the default) follows redirects and links of the
    /// visited pages, and a target that is not known means the spec may
    /// visit any route: no assumption, and on a large app every spec of an
    /// app the change reaches. `calls` assumes a spec stays on the pages
    /// its URLs name and follows only what those pages request; `redirects`
    /// also follows redirects to one known URL; `named` follows
    /// nothing. These three can skip a spec that reaches a changed page by a
    /// click or a redirect.
    pub follow: Option<e2e_static::Follow>,
}

impl E2eArgs {
    fn any(&self) -> bool {
        self.oracle() || !self.playwright_config.is_empty() || self.follow.is_some()
    }

    /// Whether a flag of the validation tooling was given.
    fn oracle(&self) -> bool {
        !self.next_fingerprint.is_empty()
            || !self.visits.is_empty()
            || !self.base_fingerprint.is_empty()
    }
}

/// One spec to decide.
#[derive(Debug)]
struct Spec {
    /// Root-relative path.
    path: String,
    /// The entries that stand for its Playwright config (root-relative).
    config: Vec<String>,
    /// The directories Playwright is started in for this spec (root-relative,
    /// empty for the root): the config's directory and the package
    /// directory above it.
    cwds: Vec<String>,
    /// The directory of its Playwright config (root-relative).
    config_dir: String,
    /// The hosts of the config's base URLs: absolute URLs to them are the app's
    /// own.
    hosts: Vec<String>,
}

/// The loaded inputs of an end-to-end selection.
#[derive(Debug)]
pub struct E2e {
    root: PathBuf,
    /// The builds under test, one per app.
    heads: Vec<Fingerprint>,
    loaded: Loaded,
    /// Build id → fingerprint of that build.
    bases: BTreeMap<String, Fingerprint>,
    specs: Vec<Spec>,
    notes: Vec<String>,
    /// Recorded commits that git does not know here.
    unknown_commits: RefCell<BTreeSet<String>>,
    /// Without a build under test: the apps, to select from source
    /// (crate::e2e_static).
    stat: Option<Static>,
    /// `--follow`.
    follow: Option<e2e_static::Follow>,
}

fn read_fingerprint(p: &Path) -> Result<Fingerprint> {
    let bytes = std::fs::read(p).with_context(|| format!("cannot read {}", p.display()))?;
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{}: not a build fingerprint (write one with `next fingerprint`)",
            p.display()
        )
    })
}

fn rel(root: &Path, p: &Path) -> Option<String> {
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let r = p.strip_prefix(root).ok()?;
    Some(r.to_string_lossy().replace('\\', "/"))
}

/// Finds the specs (and, for the validation tooling, loads the fingerprints and
/// recordings), if `--tests playwright` was given. `tests` loses the framework:
/// the specs are returned by [`E2e::entries`].
pub fn prepare(args: &E2eArgs, tests: &[String], root: &Path) -> Result<Option<E2e>> {
    if !tests.iter().any(|t| t == FRAMEWORK) {
        if args.oracle() {
            bail!(
                "--next-fingerprint, --visits and --base-fingerprint (validation tooling) need \
                 --tests {FRAMEWORK}"
            );
        }
        if args.any() {
            bail!("--playwright-config and --follow need --tests {FRAMEWORK}");
        }
        return Ok(None);
    }
    if tests.iter().any(|t| t != FRAMEWORK) {
        bail!(
            "--tests {FRAMEWORK} selects end-to-end specs through the routes of a Next.js app: \
             run it as its own command, without other --tests"
        );
    }
    // The selection is made from source (crate::e2e_static). Only the validation
    // tooling passes a build under test.
    let from_source = args.next_fingerprint.is_empty();
    if from_source && args.oracle() {
        bail!(
            "--visits and --base-fingerprint are validation tooling and do nothing without \
             --next-fingerprint <FILE>. --tests {FRAMEWORK} selects from source: leave them out"
        );
    }
    let heads: Vec<Fingerprint> = args
        .next_fingerprint
        .iter()
        .map(|p| read_fingerprint(p))
        .collect::<Result<_>>()?;
    let mut loaded = Loaded::default();
    let mut notes = Vec::new();
    for dir in &args.visits {
        let l = recorded::load_dir(dir).map_err(anyhow::Error::msg)?;
        if l.recordings.is_empty() {
            notes.push(format!("{}: no recordings", dir.display()));
        }
        loaded.recordings.extend(l.recordings);
        loaded.fingerprints.extend(l.fingerprints);
        loaded.problems.extend(l.problems);
    }
    let mut bases: BTreeMap<String, Fingerprint> = BTreeMap::new();
    for p in args.base_fingerprint.iter().chain(&loaded.fingerprints) {
        let fp = read_fingerprint(p)?;
        if heads.first().is_some_and(|h| fp.version != h.version) {
            notes.push(format!(
                "{}: fingerprint format {} (the build under test has {}): recompute it; \
                 recordings against it count as unknown",
                p.display(),
                fp.version,
                heads.first().map_or(0, |h| h.version)
            ));
            continue;
        }
        bases.entry(fp.build_id.clone()).or_insert(fp);
    }

    // The specs of every Playwright config under the root (or of the ones named).
    let only: Vec<PathBuf> = args
        .playwright_config
        .iter()
        .map(|p| std::fs::canonicalize(p).with_context(|| format!("cannot read {}", p.display())))
        .collect::<Result<_>>()?;
    let mut specs: BTreeMap<String, Spec> = BTreeMap::new();
    let configs: Vec<PathBuf> = Playwright
        .discover(root)
        .into_iter()
        .filter(|config| {
            only.is_empty()
                || only.contains(&std::fs::canonicalize(config).unwrap_or_else(|_| config.clone()))
        })
        .collect();
    // The configs are evaluated and their specs listed side by side.
    let evaluated = load_configs(&configs);
    for (config, (inputs, base_urls, found)) in configs.into_iter().zip(evaluated) {
        let hosts: Vec<String> = base_urls
            .0
            .iter()
            .filter_map(|(_, u)| u.as_deref().and_then(e2e_static::host_of))
            .collect();
        let config_dir = config
            .parent()
            .and_then(|d| rel(root, d))
            .unwrap_or_default();
        // Only what decides which files are specs: a base URL from the environment does
        // not. A value the environment decides in a file the config loads (a
        // shared factory) is reported on the field it reaches, if it reaches
        // one of these.
        let unread: BTreeSet<&str> = inputs
            .uncertainties
            .iter()
            .chain(inputs.projects.iter().flat_map(|p| &p.uncertainties))
            .filter_map(|u| match u.field.as_deref() {
                None if u.kind == turborepo_trace_core::UncertaintyKind::EnvDependent => None,
                None => Some("the config"),
                Some(f)
                    if ["testDir", "testMatch", "testIgnore", "projects"]
                        .iter()
                        .any(|k| f.starts_with(k)) =>
                {
                    Some(f)
                }
                Some(_) => None,
            })
            .collect();
        if !unread.is_empty() {
            notes.push(format!(
                "{}: {} could not be evaluated; a spec only it would find is {}",
                rel(root, &config).unwrap_or_default(),
                if from_source {
                    "not listed"
                } else {
                    "listed only if it has a recording"
                },
                unread.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        let mut entries: BTreeSet<String> = BTreeSet::new();
        entries.extend(rel(root, &config));
        for p in &inputs.projects {
            for f in p.global.config_files.iter().chain(&p.global.js_entries) {
                entries.extend(rel(root, f));
            }
            for e in &p.global.entries {
                if let Target::Path(f) = &e.target {
                    entries.extend(rel(root, f));
                }
            }
        }
        let entries: Vec<String> = entries.into_iter().collect();
        // `playwright test` runs in the config's directory, or in the package directory
        // with `--config <dir>/playwright.config.ts` (the root is always covered).
        let mut cwds: Vec<String> = Vec::new();
        if let Some(dir) = config.parent().and_then(|d| rel(root, d)) {
            let mut package = dir.as_str();
            while !package.is_empty() && !root.join(package).join("package.json").is_file() {
                package = package.rsplit_once('/').map_or("", |(d, _)| d);
            }
            if package != dir {
                cwds.push(package.to_string());
            }
            cwds.push(dir);
        }
        for s in found {
            if let Some(path) = rel(root, &s) {
                specs.entry(path.clone()).or_insert_with(|| Spec {
                    path,
                    config: entries.clone(),
                    cwds: cwds.clone(),
                    config_dir: config_dir.clone(),
                    hosts: hosts.clone(),
                });
            }
        }
    }
    // A spec with a recording was run by Playwright: it is a spec, whatever the
    // evaluation of the configs found.
    if only.is_empty() {
        let recorded: BTreeSet<&str> = loaded.recordings.iter().map(|r| r.spec.as_str()).collect();
        for path in recorded {
            let known = specs.keys().any(|k| {
                k == path || k.ends_with(&format!("/{path}")) || path.ends_with(&format!("/{k}"))
            });
            if !known && root.join(path).is_file() {
                specs.insert(
                    path.to_string(),
                    Spec {
                        path: path.to_string(),
                        config: Vec::new(),
                        cwds: Vec::new(),
                        config_dir: String::new(),
                        hosts: Vec::new(),
                    },
                );
            }
        }
    }
    if specs.is_empty() {
        notes.push(format!(
            "no Playwright specs under {} (no playwright.config.* found, or its testDir is empty)",
            root.display()
        ));
    }
    turborepo_trace_core::timing::mark("e2e: specs found");
    let stat = from_source.then(|| {
        let dirs: BTreeSet<String> = specs.values().map(|s| s.config_dir.clone()).collect();
        let st = Static::load(root, &dirs);
        turborepo_trace_core::timing::mark("e2e: apps loaded");
        if st.is_empty() && !specs.is_empty() {
            notes.push(format!(
                "no Next.js app under {}: a spec is selected by its own imports only",
                root.display()
            ));
        }
        notes.extend(st.notes.iter().cloned());
        st
    });
    if let Some(e) = stat
        .as_ref()
        .and_then(|st| st.url_api_errors().into_iter().next())
    {
        bail!("{e}");
    }
    Ok(Some(E2e {
        root: root.to_path_buf(),
        heads,
        loaded,
        bases,
        specs: specs.into_values().collect(),
        notes,
        unknown_commits: RefCell::new(BTreeSet::new()),
        stat,
        follow: args.follow,
    }))
}

/// How the change set selects an entry of a spec (the spec, its Playwright
/// setup).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Reached {
    /// A file on the chain to the entry changed (or its package).
    Changed,
    /// A file changed that code on the chain names, lists or looks for: the
    /// first file of the chain is not in the change.
    Named,
    /// By the uncertainty policy: no changed file is on the chain.
    Uncertain,
}

impl Reached {
    fn of(e: &EntryResult) -> Reached {
        match &e.cause {
            _ if e.uncertain.is_some() => Reached::Uncertain,
            Some(
                Cause::Named { .. }
                | Cause::Scope { .. }
                | Cause::Membership { .. }
                | Cause::Probe { .. },
            ) => Reached::Named,
            _ => Reached::Changed,
        }
    }

    /// The `cause` of a reason in JSON.
    fn name(self) -> &'static str {
        match self {
            Reached::Changed => "changed",
            Reached::Named => "named",
            Reached::Uncertain => "uncertain",
        }
    }
}

/// Why a spec runs.
#[derive(Debug)]
enum Reason {
    /// The spec's imports: the ordinary `affected` verdict.
    Imports { text: String, how: Reached },
    /// Its Playwright config or global setup.
    Config { text: String, how: Reached },
    /// A screenshot of the spec changed.
    Snapshot { path: String },
    /// An environment file the app or the test runner reads changed.
    Env { path: String },
    /// The build or the recording.
    Build(Why),
    /// From source: a route the spec reaches is reached by the change.
    Static(e2e_static::Why),
}

impl Reason {
    fn text(&self) -> String {
        match self {
            Reason::Imports { text, .. } | Reason::Config { text, .. } => text.clone(),
            Reason::Snapshot { path } => format!("its screenshot {path} changed"),
            Reason::Env { path } => format!(
                "the environment file {path} changed (the app reads it when it starts: it is not \
                 part of the build output)"
            ),
            Reason::Build(w) => w.to_string(),
            Reason::Static(w) => w.text(),
        }
    }

    fn value(&self) -> serde_json::Value {
        let mut v = match self {
            Reason::Imports { how, .. } => json!({
                "kind": if *how == Reached::Uncertain { "imports_uncertain" } else { "imports" },
                "cause": how.name(),
            }),
            Reason::Config { how, .. } => {
                json!({ "kind": "playwright_config", "cause": how.name() })
            }
            Reason::Snapshot { path } => json!({ "kind": "snapshot", "path": path }),
            Reason::Env { path } => json!({ "kind": "env_file", "path": path }),
            Reason::Build(w) => serde_json::to_value(w).unwrap_or_else(|_| json!({})),
            Reason::Static(w) => w.value(),
        };
        v["text"] = json!(self.text());
        v
    }

    /// The kind of the reason in `--format summary` and where its chain starts
    /// (`e2e_static::Verdict`): the JSON kind, and for the spec's own code and
    /// its Playwright setup whether a file on the chain changed, a file the
    /// code names, or none (the summary must not call a config "changed"
    /// that is in no diff).
    fn summary_kind(&self) -> (String, Option<String>) {
        let suffix = |how: &Reached| match how {
            Reached::Changed => "",
            Reached::Named => "_named",
            Reached::Uncertain => "_uncertain",
        };
        match self {
            Reason::Imports { how, .. } => (format!("imports{}", suffix(how)), None),
            Reason::Config { how, .. } => (format!("playwright_config{}", suffix(how)), None),
            _ => {
                let v = self.value();
                let start = v["changed"]
                    .as_str()
                    .or(v["site"].as_str())
                    .or(v["chain"][0].as_str());
                (
                    v["kind"].as_str().unwrap_or("").to_string(),
                    start.map(String::from),
                )
            }
        }
    }
}

/// What is known about a spec's recordings.
#[derive(Debug, Default)]
struct Recorded {
    tests: usize,
    runs: usize,
    builds: BTreeSet<String>,
    commits: BTreeSet<String>,
    routes: BTreeSet<String>,
    requests: usize,
    /// Without a recording: the routes read from the source.
    from_source: bool,
}

/// The verdict of one spec.
#[derive(Debug)]
struct Decision {
    spec: String,
    reasons: Vec<Reason>,
    recorded: Recorded,
}

/// What `affected` computed for the entries, as [`E2e::finish`] needs it.
pub struct Evaluated<'a> {
    /// The graph.
    pub graph: &'a Graph,
    /// The report over all entries.
    pub report: &'a AffectedReport,
    /// Entries that exist but are not in the graph.
    pub missing: &'a [String],
    /// The change set.
    pub changes: &'a [Change],
    /// `--explain` entries (root-relative).
    pub explain: &'a [String],
    /// `--format json`.
    pub json: bool,
    /// `--exit-code`.
    pub exit_code: bool,
    /// `--strict`.
    pub strict: bool,
    /// `--format summary`.
    pub summary: bool,
}

fn entry_text(g: &Graph, e: &EntryResult, subject: &str) -> (String, bool) {
    let chain: Vec<&str> = e.witness.iter().map(|&n| g.path(n)).collect();
    if let Some(u) = &e.uncertain {
        let site = u
            .reasons
            .first()
            .map(|(kind, line, detail)| format!(" ({}:{line}: {kind:?} {detail})", g.path(u.node)))
            .unwrap_or_default();
        return (
            match &u.bound {
                // A load that the declared packages bound (`crate::loads`): this change
                // selects it, not any change.
                Some(b) => {
                    format!("{subject} reaches a module load whose target is not known{site}; {b}")
                }
                None => format!(
                    "{subject} reaches code whose dependencies turbo trace cannot \
                     determine{site}, so any change may affect it"
                ),
            },
            true,
        );
    }
    let first = chain.first().copied().unwrap_or("");
    let via = if chain.len() > 2 {
        format!(" (through {})", chain[1..chain.len() - 1].join(" → "))
    } else {
        String::new()
    };
    // What changed when it is not the first file of the chain itself: a path that
    // the file names, lists or looks for.
    let indirect = match &e.cause {
        Some(Cause::Named {
            path,
            by,
            site,
            site_line,
            ..
        }) => Some(format!(
            "{path} changed, which {by} in {first} names ({site}:{site_line} reads a path it \
             computes)"
        )),
        Some(Cause::Scope {
            path, dir, pattern, ..
        }) => Some(format!(
            "{path} changed, inside {dir}/{pattern}, where {first} reads a path it computes"
        )),
        Some(Cause::Membership {
            path,
            kind,
            dir,
            pattern,
        }) => Some(format!(
            "{path} was {} in {dir}/{pattern}, which {first} lists",
            kind.as_str()
        )),
        Some(Cause::Probe { added, .. }) => {
            Some(format!("{added} was added, where {first} looks for a file"))
        }
        _ => None,
    };
    let text = match indirect {
        Some(what) if chain.len() <= 1 => what,
        Some(what) => format!("{what}; {subject} loads {first}{via}"),
        None if chain.len() <= 1 => format!("{subject} changed"),
        None => format!("{subject} imports {first}, which changed{via}"),
    };
    (text, false)
}

/// The id git gives a file with this content (SHA-1 object format).
fn git_blob_id(content: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", content.len()).as_bytes());
    h.update(content);
    format!("{:x}", h.finalize())
}

/// What the source fallback compares: one base build, the build under test, the
/// app.
struct Fallback<'a> {
    base: &'a Fingerprint,
    head: &'a Fingerprint,
    app: NextApp,
}

impl E2e {
    /// The source fallback applies with one build under test and one base build
    /// of its app.
    fn fallback(&self) -> Option<Fallback<'_>> {
        let [head] = self.heads.as_slice() else {
            return None;
        };
        let mut bases = self.bases.values().filter(|b| self.head_for(b).is_some());
        let base = bases.next()?;
        if bases.next().is_some() {
            return None;
        }
        // The app directory is relative to the repository root, which the graph root
        // is.
        let dir = self.root.join(head.app.as_deref()?);
        // The rules as the build evaluated them, when the build output is there.
        let manifest = dir.join(".next/routes-manifest.json");
        is_next_app(&dir).then(|| Fallback {
            base,
            head,
            app: NextApp::load_with(
                &dir,
                &LoadOptions {
                    routes_manifest: manifest.is_file().then_some(manifest),
                    ..LoadOptions::default()
                },
            ),
        })
    }

    /// For a spec without a recording: the routes its source says it can reach,
    /// if that is all it can reach and none of it differs. `Err`: why not
    /// (`None`: nothing to add to "no recording").
    fn read_source(
        &self,
        spec: &Spec,
        f: &Fallback<'_>,
        files: &Files<'_>,
        strict_css: bool,
    ) -> std::result::Result<BTreeSet<String>, Option<String>> {
        let sv = extract_visits(
            &self.root.join(&spec.path),
            files,
            &ExtractOptions::default(),
        );
        if !sv.complete() {
            return Err(sv.unresolved.first().map(|u| {
                format!(
                    "its source does not say where it navigates ({}:{}: {})",
                    rel(&self.root, &u.file).unwrap_or_default(),
                    u.line,
                    u.text.chars().take(60).collect::<String>()
                )
            }));
        }
        let m = f.app.matcher();
        let mut reach: BTreeSet<String> = BTreeSet::new();
        for v in &sv.visits {
            let r = m.match_pat(&v.url, v.soft);
            let at = format!(
                "{}:{}",
                rel(&self.root, &v.file).unwrap_or_default(),
                v.line
            );
            if !r.ambiguities.is_empty() || r.outcome != Outcome::Routes {
                let why = match (r.ambiguities.first(), &r.outcome) {
                    (Some(Ambiguity::Middleware), _) => {
                        "the middleware may rewrite or redirect it".to_string()
                    }
                    (Some(Ambiguity::RulesIncomplete), _) => {
                        "not every redirect and rewrite of the config could be read".to_string()
                    }
                    (Some(Ambiguity::ConditionalRule { phase, source }), _) => {
                        format!("the {phase} rule {source} has conditions")
                    }
                    (Some(Ambiguity::ParamMayEqualStatic { .. } | Ambiguity::OpenPattern), _) => {
                        "the URL has unknown parts".to_string()
                    }
                    (Some(other), _) => format!("{other:?}"),
                    (None, outcome) => format!("it does not end at a route ({outcome:?})"),
                };
                return Err(Some(format!(
                    "which route {} reaches ({at}) is not certain from the source: {why}",
                    r.input
                )));
            }
            for id in &r.candidates {
                match f.app.table.route(id) {
                    Some(route) if f.base.routes.contains_key(&route.pattern) => {
                        reach.insert(route.pattern.clone());
                    }
                    _ => {
                        return Err(Some(format!(
                            "the route of {} ({at}) is not a route of the recorded build",
                            r.input
                        )));
                    }
                }
            }
        }
        // Everything those routes link to or call: a click can go there.
        let mut stack: Vec<String> = reach.iter().cloned().collect();
        while let Some(p) = stack.pop() {
            let Some(r) = f.base.routes.get(&p) else {
                continue;
            };
            for inv in &r.invokes {
                match &inv.target {
                    InvokeTarget::Route(t) => {
                        if f.base.routes.contains_key(t) && reach.insert(t.clone()) {
                            stack.push(t.clone());
                        }
                    }
                    InvokeTarget::Unknown(_) | InvokeTarget::Procedure { .. } => {
                        return Err(Some(format!(
                            "read from the source it visits {p}, which makes a request whose \
                             target is not known"
                        )));
                    }
                    InvokeTarget::Action(_) | InvokeTarget::External(_) => {}
                }
            }
        }
        let mode = if strict_css {
            CssMode::Strict
        } else {
            CssMode::Utilities
        };
        let d = diff(f.base, f.head, mode).map_err(|_| None)?;
        let listed = || reach.iter().cloned().collect::<Vec<_>>().join(", ");
        if !d.global.is_empty() || !d.public.is_empty() || !d.added.is_empty() {
            return Err(Some(format!(
                "read from the source it visits {}, but {} changed, and without a recording the \
                 paths it requests are not known",
                listed(),
                d.global
                    .first()
                    .cloned()
                    .or_else(|| d.public.first().map(|p| format!("public/{p}")))
                    .unwrap_or_else(|| "the route table".to_string())
            )));
        }
        if let Some(p) = reach.iter().find(|p| {
            d.changed.contains_key(*p) || d.affected.contains_key(*p) || d.removed.contains(*p)
        }) {
            return Err(Some(format!(
                "read from the source it visits {}, and {p} changed",
                listed()
            )));
        }
        Ok(reach)
    }

    /// The build under test of the app `base` is a build of.
    fn head_for(&self, base: &Fingerprint) -> Option<&Fingerprint> {
        self.heads.iter().find(|h| match (&h.app, &base.app) {
            (Some(a), Some(b)) => a == b,
            // A fingerprint that does not know its app pairs with the only one given.
            _ => self.heads.len() == 1,
        })
    }

    /// The entries to evaluate with the import graph: the specs and their
    /// configs, as absolute paths.
    pub fn entries(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        for s in &self.specs {
            out.insert(s.path.clone());
            out.extend(s.config.iter().cloned());
        }
        let mut out: Vec<String> = out
            .into_iter()
            .map(|p| self.root.join(p).to_string_lossy().into_owned())
            .collect();
        // From source: the entry files of every route are entries too.
        if let Some(st) = &self.stat {
            out.extend(st.entries());
        }
        out
    }

    /// The working directories of the specs, for the per-test resolution of
    /// references relative to `process.cwd()`.
    pub fn entry_cwds(&self, g: &Graph) -> Vec<(turborepo_trace_graph::NodeId, String)> {
        let mut out = Vec::new();
        for s in &self.specs {
            // The config and its global setup are loaded by the same process as the spec.
            for p in std::iter::once(&s.path).chain(&s.config) {
                if let Some(n) = g.node_by_path(p) {
                    out.extend(s.cwds.iter().map(|c| (n, c.clone())));
                }
            }
        }
        if let Some(st) = &self.stat {
            out.extend(st.entry_cwds(g));
        }
        out.sort_by(|a, b| (a.0.0, &a.1).cmp(&(b.0.0, &b.1)));
        out.dedup();
        out
    }

    /// Whether the selection is made from source: always, except under the
    /// validation tooling (a build fingerprint was given).
    pub fn is_from_source(&self) -> bool {
        self.stat.is_some()
    }

    /// The entries that are compiled file by file with the types erased: the
    /// specs and their Playwright setup (Babel), and from source the route
    /// files (SWC). For them a change of types or comments is no change, a
    /// type-only import is no edge, and a barrel carries a change only to
    /// the names an importer uses (`turborepo_trace_graph::barrel`).
    pub fn isolated(&self, g: &Graph) -> turborepo_trace_graph::NodeSet {
        let mut set = turborepo_trace_graph::NodeSet::new(g.node_count());
        for e in self.entries() {
            let rel = Path::new(&e)
                .strip_prefix(&self.root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or(e);
            if let Some(n) = g.node_by_path(&rel) {
                set.insert(n);
            }
        }
        set
    }

    /// The commit the recordings were made at, when they agree on one.
    pub fn recorded_commit(&self) -> Option<String> {
        let commits: BTreeSet<&str> = self
            .loaded
            .recordings
            .iter()
            .filter_map(|r| r.commit.as_deref())
            .collect();
        match commits.len() {
            1 => commits.into_iter().next().map(str::to_string),
            _ => None,
        }
    }

    /// The files that differ between the commits the recordings were made at
    /// and the base revision `since`: a recording made before the base does
    /// not know about them, so they count as changes too. Nothing when the
    /// recordings were made at `since`.
    pub fn changes_before(&self, root: &Path, since: Option<&str>) -> Vec<Change> {
        let Some(since) = since else {
            return Vec::new();
        };
        let commits: BTreeSet<&str> = self
            .loaded
            .recordings
            .iter()
            .filter_map(|r| r.commit.as_deref())
            .filter(|c| *c != since)
            .collect();
        let mut out = Vec::new();
        for c in commits {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args([
                    "diff",
                    "--name-status",
                    "--no-renames",
                    "--relative",
                    c,
                    since,
                    "--",
                ])
                .output();
            match o {
                Ok(o) if o.status.success() => {
                    for line in String::from_utf8_lossy(&o.stdout).lines() {
                        if let Ok(cs) = ChangeSet::from_name_status(line) {
                            out.extend(cs.files);
                        }
                    }
                }
                _ => {
                    self.unknown_commits.borrow_mut().insert(c.to_string());
                }
            }
        }
        out
    }

    fn decide(&self, ev: &Evaluated<'_>) -> (Vec<Decision>, Option<e2e_static::Decided>) {
        let g = ev.graph;
        let by_path: BTreeMap<&str, &EntryResult> = ev
            .report
            .entries
            .iter()
            .filter(|e| e.verdict != Verdict::Unaffected && !e2e_static::whatif_skips(e))
            .map(|e| (g.path(e.entry), e))
            .collect();
        let missing: BTreeSet<&str> = ev.missing.iter().map(String::as_str).collect();
        let by_spec = recorded::by_spec(&self.loaded.recordings);
        // Recordings name specs relative to the repository root; the graph root may be
        // below it.
        let lookup = |spec: &str| -> Option<&Vec<TestRuns<'_>>> {
            by_spec.get(spec).or_else(|| {
                let mut hits = by_spec.iter().filter(|(k, _)| {
                    k.ends_with(&format!("/{spec}")) || spec.ends_with(&format!("/{k}"))
                });
                let first = hits.next()?;
                hits.next().is_none().then_some(first.1)
            })
        };
        let mappers: BTreeMap<&str, RouteMapper<'_>> = self
            .bases
            .iter()
            .map(|(b, fp)| (b.as_str(), RouteMapper::new(fp)))
            .collect();
        // One diff per base build and stylesheet mode, against the build under test of
        // the same app.
        let mut diffs: BTreeMap<(&str, bool), FingerprintDiff> = BTreeMap::new();
        for (b, fp) in &self.bases {
            let Some(head) = self.head_for(fp) else {
                continue;
            };
            for strict in [false, true] {
                let mode = if strict {
                    CssMode::Strict
                } else {
                    CssMode::Utilities
                };
                if let Ok(d) = diff(fp, head, mode) {
                    diffs.insert((b.as_str(), strict), d);
                }
            }
        }
        // Without a recording: the source (only with one app and one base build).
        let fallback = self.fallback();
        let imports = fallback
            .is_some()
            .then(|| GraphImports::from_graph(g, &self.root));
        let files = imports.as_ref().map(|i| Files::new(i));
        // Environment files are read at start, by the app and by the Playwright config.
        let app_dirs: Vec<&str> = self.heads.iter().filter_map(|h| h.app.as_deref()).collect();
        let env_changes: Vec<&Change> = ev
            .changes
            .iter()
            .filter(|c| {
                let name = c.path.rsplit('/').next().unwrap_or(&c.path);
                name == ".env" || name.starts_with(".env.")
            })
            .collect();
        // From source: what the change reaches, and what each spec reaches.
        let decided = self.stat.as_ref().map(|st| {
            let specs: Vec<(String, String, Vec<String>)> = self
                .specs
                .iter()
                .map(|s| (s.path.clone(), s.config_dir.clone(), s.hosts.clone()))
                .collect();
            let force = turborepo_trace_core::debug::var_os("E2E_FOLLOW_ALWAYS").is_some();
            st.decide(
                g,
                ev.report,
                ev.changes,
                &specs,
                &e2e_static::Options {
                    strict: ev.strict,
                    follow: self.follow,
                    force_follow: force,
                },
            )
        });
        let comparisons: BTreeMap<(&str, bool), Comparison<'_>> = diffs
            .iter()
            .filter_map(|((build, strict), d)| {
                let base = self.bases.get(*build)?;
                let head = self.head_for(base)?;
                Some(((*build, *strict), Comparison::new(base, head, d)))
            })
            .collect();
        let mut out = Vec::new();
        for spec in &self.specs {
            let mut reasons: Vec<Reason> = Vec::new();
            let config_dir = spec.cwds.last().map_or("", String::as_str);
            if let Some(c) = env_changes.iter().find(|c| {
                let dir = c.path.rsplit_once('/').map_or("", |(d, _)| d);
                let above =
                    |of: &str| dir.is_empty() || of == dir || of.starts_with(&format!("{dir}/"));
                above(config_dir)
                    || app_dirs.iter().any(|a| above(a))
                    || self
                        .stat
                        .as_ref()
                        .is_some_and(|st| st.app_dirs_of(&spec.config_dir).iter().any(|a| above(a)))
            }) {
                reasons.push(Reason::Env {
                    path: c.path.clone(),
                });
            }
            // 1. The spec's own code.
            if let Some(e) = by_path.get(spec.path.as_str()) {
                let (text, _) = entry_text(g, e, "the spec");
                reasons.push(Reason::Imports {
                    text,
                    how: Reached::of(e),
                });
            } else if missing.contains(spec.path.as_str()) {
                reasons.push(Reason::Imports {
                    text: "the spec is not in the graph (not a traced source file)".to_string(),
                    how: Reached::Uncertain,
                });
            }
            for c in &spec.config {
                if let Some(e) = by_path.get(c.as_str()) {
                    let (text, _) = entry_text(g, e, &format!("its Playwright setup {c}"));
                    reasons.push(Reason::Config {
                        text,
                        how: Reached::of(e),
                    });
                    break;
                }
            }
            let shots = format!("{}-snapshots/", spec.path);
            if let Some(c) = ev.changes.iter().find(|c| c.path.starts_with(&shots)) {
                reasons.push(Reason::Snapshot {
                    path: c.path.clone(),
                });
            }
            // A spec that compares screenshots sees every stylesheet change.
            let strict_css = self.root.join(shots.trim_end_matches('/')).is_dir();

            // 2. From source, or the recordings.
            let mut recorded = Recorded::default();
            let mut why: Vec<Why> = Vec::new();
            if let (Some(st), Some(d)) = (&self.stat, &decided) {
                if let Some(s) = d.specs.get(&spec.path)
                    && let Some(w) = st.why(d, s, d.rule)
                {
                    reasons.push(Reason::Static(w));
                }
                out.push(Decision {
                    spec: spec.path.clone(),
                    reasons,
                    recorded,
                });
                continue;
            }
            match lookup(&spec.path) {
                None => {
                    let read = match (&fallback, &files) {
                        (Some(f), Some(files)) => self.read_source(spec, f, files, strict_css),
                        _ => Err(None),
                    };
                    match read {
                        Ok(routes) => {
                            recorded.routes = routes;
                            recorded.from_source = true;
                        }
                        Err(detail) => why.push(Why::NoRecording { detail }),
                    }
                }
                Some(tests) => {
                    let current = std::fs::read(self.root.join(&spec.path))
                        .ok()
                        .map(|b| git_blob_id(&b));
                    let mut groups: BTreeMap<&str, Visits> = BTreeMap::new();
                    for t in tests {
                        recorded.tests += 1;
                        recorded.runs += t.runs.len();
                        recorded
                            .commits
                            .extend(t.runs.iter().filter_map(|r| r.commit.clone()));
                        if let Some(c) = t
                            .runs
                            .iter()
                            .filter_map(|r| r.commit.as_ref())
                            .find(|c| self.unknown_commits.borrow().contains(*c))
                        {
                            let w = Why::UnknownCommit { commit: c.clone() };
                            if !why.contains(&w) {
                                why.push(w);
                            }
                        }
                        if !t.usable() {
                            let status = t
                                .runs
                                .last()
                                .and_then(|r| r.status.clone())
                                .unwrap_or_default();
                            why.push(Why::RecordedRunFailed {
                                test: t.name(),
                                status,
                            });
                            continue;
                        }
                        // Only SHA-1 blob ids are compared (a SHA-256 repository is not).
                        if t.runs.iter().any(|r| {
                            r.spec_hash.as_ref().is_some_and(|h| h.len() == 40)
                                && r.spec_hash != current
                        }) && !why.contains(&Why::SpecChanged)
                        {
                            why.push(Why::SpecChanged);
                        }
                        let Some(build) = t.build else {
                            why.push(Why::NoBuildId { test: t.name() });
                            continue;
                        };
                        recorded.builds.insert(build.to_string());
                        let Some(mapper) = mappers.get(build) else {
                            let w = Why::UnknownBuild {
                                build: build.to_string(),
                            };
                            if !why.contains(&w) {
                                why.push(w);
                            }
                            continue;
                        };
                        let v = groups.entry(build).or_default();
                        for r in &t.runs {
                            v.add(mapper, r);
                        }
                    }
                    // 3. What it reached, on the build under test.
                    for (build, visits) in &groups {
                        recorded.routes.extend(visits.routes.keys().cloned());
                        recorded.requests += visits.requests;
                        let (Some(base), Some(mapper)) =
                            (self.bases.get(*build), mappers.get(build))
                        else {
                            continue;
                        };
                        if self.head_for(base).is_none() {
                            why.push(Why::NoBuildUnderTest {
                                app: base.app.clone().unwrap_or_else(|| "?".to_string()),
                            });
                            continue;
                        }
                        let Some(c) = comparisons.get(&(*build, strict_css)) else {
                            why.push(Why::BadRecording {
                                detail: format!(
                                    "the fingerprint of build {build} was computed with other \
                                     options than the one under test"
                                ),
                            });
                            continue;
                        };
                        for w in c.decide(visits, mapper) {
                            if !why.contains(&w) {
                                why.push(w);
                            }
                        }
                    }
                }
            }
            reasons.extend(why.into_iter().map(Reason::Build));
            out.push(Decision {
                spec: spec.path.clone(),
                reasons,
                recorded,
            });
        }
        (out, decided)
    }

    /// Prints the selection.
    pub fn finish(&self, ev: &Evaluated<'_>, out: &mut dyn Write) -> Result<ExitCode> {
        let (decisions, decided) = self.decide(ev);
        let selected = decisions.iter().filter(|d| !d.reasons.is_empty()).count();
        if let (Some(st), Some(d)) = (&self.stat, &decided) {
            return self.finish_static(st, d, &decisions, ev, out);
        }
        let mut notes = self.notes.clone();
        for (f, why) in &self.loaded.problems {
            notes.push(format!("{}: {why}", f.display()));
        }
        // Recordings of specs that are not specs of any config here.
        let known: BTreeSet<&str> = self.specs.iter().map(|s| s.path.as_str()).collect();
        let stray: BTreeSet<&str> = self
            .loaded
            .recordings
            .iter()
            .map(|r| r.spec.as_str())
            .filter(|s| {
                !known.iter().any(|k| {
                    k == s || k.ends_with(&format!("/{s}")) || s.ends_with(&format!("/{k}"))
                })
            })
            .collect();
        if !stray.is_empty() {
            notes.push(format!(
                "{} recorded spec(s) are not specs of a Playwright config under the root (for \
                 example {}): they are not selected or skipped here",
                stray.len(),
                stray.iter().next().copied().unwrap_or("")
            ));
        }

        if !ev.explain.is_empty() {
            for x in ev.explain {
                match decisions.iter().find(|d| d.spec == *x) {
                    Some(d) => self.explain(d, out)?,
                    None => writeln!(out, "{x}: not a Playwright spec of a config under the root")?,
                }
            }
            for n in &notes {
                writeln!(out, "  note: {n}")?;
            }
        } else if ev.json {
            let results: Vec<serde_json::Value> = decisions
                .iter()
                .map(|d| {
                    json!({
                        "spec": d.spec,
                        "run": !d.reasons.is_empty(),
                        "reasons": d.reasons.iter().map(Reason::value).collect::<Vec<_>>(),
                        "recorded": {
                            "from_source": d.recorded.from_source,
                            "tests": d.recorded.tests,
                            "runs": d.recorded.runs,
                            "requests": d.recorded.requests,
                            "builds": d.recorded.builds,
                            "commits": d.recorded.commits,
                            "routes": d.recorded.routes,
                        },
                    })
                })
                .collect();
            let builds: Vec<serde_json::Value> = self
                .bases
                .iter()
                .filter_map(|(b, fp)| {
                    let d = diff(fp, self.head_for(fp)?, CssMode::Utilities).ok()?;
                    Some(json!({
                        "build": b,
                        "routes": fp.routes.len(),
                        "changed": d.changed,
                        "affected": d.affected,
                        "added": d.added,
                        "removed": d.removed,
                        "global": d.global,
                        "public": d.public,
                        "css_only_utilities": d.css_only_utilities,
                    }))
                })
                .collect();
            let v = json!({
                "framework": FRAMEWORK,
                "specs": decisions.len(),
                "selected": selected,
                "skipped": decisions.len() - selected,
                "builds": self.heads.iter().map(|h| h.build_id.as_str()).collect::<Vec<_>>(),
                "recorded_builds": builds,
                "recordings": self.loaded.recordings.len(),
                "results": results,
                "notes": notes,
            });
            serde_json::to_writer_pretty(&mut *out, &v)?;
            writeln!(out)?;
        } else {
            for d in decisions.iter().filter(|d| !d.reasons.is_empty()) {
                writeln!(out, "{}", d.spec)?;
            }
            eprintln!(
                "{FRAMEWORK}: {selected} of {} spec(s) selected ({} recording(s), {} recorded \
                 build(s) with a fingerprint)",
                decisions.len(),
                self.loaded.recordings.len(),
                self.bases.len()
            );
            for n in &notes {
                eprintln!("note: {n}");
            }
        }
        if ev.exit_code {
            out.flush()?;
            return Ok(ExitCode::from(u8::from(selected > 0)));
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Prints a selection made from source (crate::e2e_static).
    fn finish_static(
        &self,
        st: &Static,
        d: &e2e_static::Decided,
        decisions: &[Decision],
        ev: &Evaluated<'_>,
        out: &mut dyn Write,
    ) -> Result<ExitCode> {
        let selected = decisions.iter().filter(|x| !x.reasons.is_empty()).count();
        if !ev.explain.is_empty() {
            for x in ev.explain {
                let Some(dec) = decisions.iter().find(|dd| dd.spec == *x) else {
                    writeln!(out, "{x}: not a Playwright spec of a config under the root")?;
                    continue;
                };
                let reach = d
                    .specs
                    .get(&dec.spec)
                    .map(|s| st.explain_reach(d, s))
                    .unwrap_or_default();
                if dec.reasons.is_empty() {
                    writeln!(out, "skip {}", dec.spec)?;
                    writeln!(out, "  its imports and its Playwright setup did not change")?;
                    for l in &reach {
                        writeln!(out, "  {l}")?;
                    }
                    writeln!(out, "  the change reaches none of them:")?;
                    for l in st.summary(d) {
                        writeln!(out, "    {l}")?;
                    }
                } else {
                    writeln!(out, "run {}", dec.spec)?;
                    for r in &dec.reasons {
                        writeln!(out, "  {}", r.text())?;
                    }
                    for l in &reach {
                        writeln!(out, "  {l}")?;
                    }
                }
            }
            for n in &self.notes {
                writeln!(out, "  note: {n}")?;
            }
        } else if ev.summary {
            let verdicts: Vec<e2e_static::Verdict> = decisions
                .iter()
                .map(|dec| {
                    (
                        dec.spec.clone(),
                        dec.reasons.first().map(Reason::summary_kind),
                    )
                })
                .collect();
            out.write_all(st.summary_text(d, &verdicts).as_bytes())?;
            for n in &self.notes {
                writeln!(out, "  note: {n}")?;
            }
        } else if ev.json {
            let results: Vec<serde_json::Value> = decisions
                .iter()
                .map(|dec| {
                    json!({
                        "spec": dec.spec,
                        "run": !dec.reasons.is_empty(),
                        "reasons": dec.reasons.iter().map(Reason::value).collect::<Vec<_>>(),
                        "static": d.specs.get(&dec.spec).map(|s| st.spec_value(d, s)),
                    })
                })
                .collect();
            let timings: serde_json::Map<String, serde_json::Value> = d
                .timings
                .iter()
                .map(|(k, ms)| ((*k).to_string(), json!(ms)))
                .collect();
            let v = json!({
                "framework": FRAMEWORK,
                "mode": "source",
                "rule": d.rule.name(),
                "specs": decisions.len(),
                "selected": selected,
                "skipped": decisions.len() - selected,
                "apps": st.apps_value(d),
                "results": results,
                "notes": self.notes,
                "timings_ms": timings,
            });
            serde_json::to_writer_pretty(&mut *out, &v)?;
            writeln!(out)?;
        } else {
            for dec in decisions.iter().filter(|x| !x.reasons.is_empty()) {
                writeln!(out, "{}", dec.spec)?;
            }
            eprintln!(
                "{FRAMEWORK}: {selected} of {} spec(s) selected from source (no build, no \
                 recording)",
                decisions.len()
            );
            for l in st.summary(d) {
                eprintln!("  {l}");
            }
            // How much of the answer is per spec: a spec that may visit any route is
            // selected with any route of its app.
            let any = d
                .specs
                .values()
                .filter(|s| s.reach.get(&d.rule).is_some_and(|r| !r.open.is_empty()))
                .count();
            if any > 0 {
                eprintln!(
                    "  {any} of {} spec(s) may visit any route of their app (--format summary \
                     says why)",
                    decisions.len()
                );
            }
            for n in &self.notes {
                eprintln!("note: {n}");
            }
        }
        if ev.exit_code {
            out.flush()?;
            return Ok(ExitCode::from(u8::from(selected > 0)));
        }
        Ok(ExitCode::SUCCESS)
    }

    fn explain(&self, d: &Decision, out: &mut dyn Write) -> Result<()> {
        let r = &d.recorded;
        let recorded = if r.from_source {
            format!(
                "no recording; read from its source it can reach {} route(s): {}",
                r.routes.len(),
                r.routes.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        } else if r.tests == 0 {
            String::new()
        } else {
            let mut routes: Vec<&str> = r.routes.iter().map(String::as_str).collect();
            let more = routes.len().saturating_sub(12);
            routes.truncate(12);
            format!(
                "recorded: {} test(s), {} run(s) against build {}{}; {} request(s) reaching {} \
                 route(s): {}{}",
                r.tests,
                r.runs,
                r.builds.iter().cloned().collect::<Vec<_>>().join(", "),
                r.commits
                    .iter()
                    .next()
                    .map(|c| format!(" (commit {})", &c[..c.len().min(10)]))
                    .unwrap_or_default(),
                r.requests,
                r.routes.len(),
                routes.join(", "),
                if more > 0 {
                    format!(", and {more} more")
                } else {
                    String::new()
                }
            )
        };
        if d.reasons.is_empty() {
            writeln!(out, "skip {}", d.spec)?;
            writeln!(out, "  its imports and its Playwright setup did not change")?;
            writeln!(out, "  {recorded}")?;
            writeln!(
                out,
                "  none of these routes, the routing rules that match its requests, the public \
                 files it requested or the middleware in front of them differ in build {}",
                self.heads
                    .iter()
                    .map(|h| h.build_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        } else {
            writeln!(out, "run {}", d.spec)?;
            for reason in &d.reasons {
                writeln!(out, "  {}", reason.text())?;
            }
            if !recorded.is_empty() {
                writeln!(out, "  {recorded}")?;
            }
        }
        Ok(())
    }
}
