//! `turbo trace lint`: checks of first-party code that exit non-zero when a
//! rule is broken.
//!
//! A rule is a named function from the graph to findings ([`Rule`], registered
//! in [`RULES`]). The command runs the enabled rules and then, for all of them
//! alike:
//!
//! 1. drops findings in excluded files (`[lint] exclude`, `--exclude`);
//! 2. applies inline suppressions ([`suppress`]): `// turbo-trace-ignore
//!    <rule>: <reason>`;
//! 3. matches the rest against the committed baseline ([`baseline`]);
//! 4. prints the report ([`output`]) and fails on a new error-level finding, a
//!    stale baseline entry of an error-level rule, or an invalid suppression.
//!
//! # Adding a rule
//!
//! Write `fn my_rule(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>>` in a new file
//! of this directory and add one [`Rule`] to [`RULES`]. Nothing else changes:
//! the severity setting (`[lint.rules]`, `--rule`), excludes, suppressions, the
//! baseline and the three output formats work on [`Finding`]s.

mod baseline;
mod config;
mod cwdpaths;
mod output;
mod patterns;
mod suppress;
mod unconstrained;
mod undeclared;

use std::{
    cell::{Cell, OnceCell},
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
pub(crate) use patterns::above_root_note;
use turborepo_trace_graph::{
    AffectedOptions, BoundedSites, Engine, FloorOrder, FloorReport, LoadReach, NodeSet, Patched,
    Policy, floor::FloorAssumptions, glob::Glob,
};
use turborepo_trace_store::Graph;

use crate::{
    ExitCode,
    affected::{collect_entries, selection_options},
    graph::{GraphLoc, open},
};

/// The graph the rules read: the stored graph with the derived layer applied
/// (`crate::graph::derived`: per-test working directories, hints, cross-file
/// binding), as `affected` and `uncertainty` evaluate it. Node ids are those of
/// the stored graph.
pub type Derived<'g> = Patched<'g, Graph>;

/// How a rule's findings count.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The rule does not run.
    Off,
    /// Findings are reported and never fail the check.
    Warn,
    /// A new finding fails the check.
    Error,
}

impl Severity {
    /// `off`, `warn` or `error`.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Off => "off",
            Severity::Warn => "warn",
            Severity::Error => "error",
        }
    }

    /// Parses `off`, `warn` or `error`.
    pub fn parse(s: &str) -> Option<Severity> {
        match s {
            "off" => Some(Severity::Off),
            "warn" => Some(Severity::Warn),
            "error" => Some(Severity::Error),
            _ => None,
        }
    }
}

/// The tests a finding holds on the uncertainty floor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Cost {
    /// Tests that reach the finding's file: selected on every change while it
    /// is open.
    pub reached: usize,
    /// Of those, the tests that no other open file holds: fixing this file
    /// frees them.
    pub exclusive: usize,
    /// The tests are not on the floor: a bound from outside the site (the
    /// declared packages, `crate::loads`) selects them only for a change in
    /// a package the load can reach. `exclusive` then counts the tests that
    /// reach no other such site and no open one.
    pub bounded: bool,
}

/// One violation of a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The name of the rule that produced it ([`Rule::name`]).
    pub rule: &'static str,
    /// The file, relative to the repository root.
    pub file: String,
    /// 1-based line; 0 if unknown.
    pub line: u32,
    /// What the baseline matches, together with the rule and the file. It must
    /// stay the same when unrelated code changes: no line numbers, no
    /// counts, no costs, nothing that depends on other files. Two findings
    /// of one file may have the same key (the baseline counts them).
    pub key: String,
    /// What is wrong, in one line.
    pub message: String,
    /// The tests it holds on the floor, if the rule knows.
    pub cost: Option<Cost>,
    /// How to fix it: one or two lines.
    pub fix: String,
}

/// A lint rule.
pub struct Rule {
    /// The name used in `--rule`, `[lint.rules]`, suppression comments and the
    /// baseline: lowercase words joined by `-`.
    pub name: &'static str,
    /// The severity when neither the configuration nor the command line sets
    /// one.
    pub default: Severity,
    /// One line for the documentation and error messages.
    pub summary: &'static str,
    /// Produces the findings. Called only when the rule's severity is not
    /// `off`.
    pub run: fn(&Ctx<'_, '_>) -> Result<Vec<Finding>>,
}

/// Every rule, in the order they run and are documented. Add new rules here.
pub const RULES: &[Rule] = &[
    Rule {
        name: "unconstrained-load",
        default: Severity::Error,
        summary: "a module load (import, require, worker, child process) whose target turbo trace \
                  cannot bound",
        run: unconstrained::loads,
    },
    Rule {
        name: "unconstrained-read",
        default: Severity::Off,
        summary: "a file read, directory listing or asset path whose target turbo trace cannot \
                  bound",
        run: unconstrained::reads,
    },
    Rule {
        name: "unresolved-reference",
        default: Severity::Off,
        summary: "a literal path or module name that resolves to nothing and that nothing stands \
                  for",
        run: unconstrained::unresolved_references,
    },
    Rule {
        name: "undeclared-dependency",
        default: Severity::Off,
        summary: "a file that requires, imports or reads a file of a package its own package does \
                  not declare",
        run: undeclared::undeclared_dependencies,
    },
    Rule {
        name: "test-pattern-above-root",
        default: Severity::Warn,
        summary: "a test pattern that matches through the directories the checkout lies below: \
                  which files are tests depends on the place",
        run: patterns::above_root,
    },
    Rule {
        name: "cwd-path-explained",
        default: Severity::Off,
        summary: "a path relative to the working directory that is not read from the repository \
                  root because another directory explains it",
        run: cwdpaths::explained,
    },
];

/// A suppression comment without a reason, without a rule, or naming an unknown
/// rule. Always an error; not in the baseline.
pub const INVALID_SUPPRESSION: &str = "invalid-suppression";
/// A suppression comment that matches no finding of an enabled rule. Always a
/// warning; not in the baseline.
pub const UNUSED_SUPPRESSION: &str = "unused-suppression";

/// The rule called `name`.
pub fn rule(name: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.name == name)
}

/// The known rule names, for error messages.
pub fn rule_names() -> String {
    RULES.iter().map(|r| r.name).collect::<Vec<_>>().join(", ")
}

/// What a rule can read. Rules that need none of the lazily computed parts do
/// not pay for them.
// Not every part has a rule that reads it yet: this is the interface for new rules.
#[allow(dead_code)]
pub struct Ctx<'a, 'g> {
    /// The repository root (absolute, canonical).
    pub root: &'a Path,
    /// The stored graph, without the derived layer.
    pub graph: &'g Graph,
    /// The graph `affected` evaluates ([`Derived`]). Use it for paths, edges
    /// and records.
    pub derived: &'g Derived<'g>,
    /// The query engine over [`Ctx::derived`], with the options of `affected`.
    pub engine: &'a Engine<'g, Derived<'g>>,
    /// The options `affected` and `uncertainty` evaluate records with.
    pub options: &'a AffectedOptions,
    /// The entries whose cost is reported: the test files of `--tests` /
    /// `[lint] tests`.
    pub entries: &'a NodeSet,
    /// Of the entries, those evaluated without type-only edges (tests whose
    /// runner transpiles file by file), as in `affected`.
    pub runtime_entries: &'a NodeSet,
    floor: OnceCell<FloorReport>,
    floor_time: Cell<Duration>,
    floor_open: OnceCell<FloorReport>,
    reach: OnceCell<LoadReach>,
    bounded: OnceCell<BoundedSites>,
}

impl Ctx<'_, '_> {
    /// The uncertainty floor of the entries: every first-party file with open
    /// records ([`FloorReport::sites`]) and the entries that reach it.
    /// Computed on first use and shared by the rules.
    pub fn floor(&self) -> &FloorReport {
        self.floor.get_or_init(|| {
            let t = Instant::now();
            let report = self.engine.floor_for_runners(
                self.entries,
                self.runtime_entries,
                self.options,
                FloorOrder::Freed,
                0,
                &FloorAssumptions::default(),
            );
            self.floor_time.set(t.elapsed());
            report
        })
    }

    /// [`Ctx::floor`] as it is without the bound of the declared packages
    /// ([`AffectedOptions::loads_declared`], `crate::loads`): the module loads
    /// with an unknown target are sites of this report, with the tests that
    /// reach them. The same report as [`Ctx::floor`] where that bound does
    /// not apply.
    pub fn floor_without_bounds(&self) -> &FloorReport {
        if !self.options.loads_declared {
            return self.floor();
        }
        self.floor_open.get_or_init(|| {
            let t = Instant::now();
            let report = self.engine.floor_for_runners(
                self.entries,
                self.runtime_entries,
                &self.options.clone().loads_declared(false),
                FloorOrder::Freed,
                0,
                &FloorAssumptions::default(),
            );
            self.floor_time.set(self.floor_time.get() + t.elapsed());
            report
        })
    }

    /// What every first-party package declares, as what one load from it can
    /// reach (`crate::loads::reach`, read from the manifests).
    pub fn reach(&self) -> &LoadReach {
        self.reach
            .get_or_init(|| crate::loads::reach(self.derived, self.root).0)
    }

    /// The module loads that the declared packages bound, with the packages of
    /// the tests that reach each one.
    pub fn bounded_loads(&self) -> &BoundedSites {
        self.bounded.get_or_init(|| {
            self.engine.bounded_sites(
                self.reach(),
                self.entries,
                Some(self.runtime_entries),
                self.options,
            )
        })
    }
}

/// What became of a finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not in the baseline: fails the check when its severity is `error`.
    New,
    /// Accepted by a baseline entry.
    Baselined,
    /// Silenced by a suppression comment, with its reason.
    Suppressed(String),
}

impl Status {
    fn as_str(&self) -> &'static str {
        match self {
            Status::New => "new",
            Status::Baselined => "baselined",
            Status::Suppressed(_) => "suppressed",
        }
    }
}

/// A finding with its severity and status.
#[derive(Clone, Debug)]
pub struct Item {
    /// The finding.
    pub finding: Finding,
    /// The severity of its rule.
    pub severity: Severity,
    /// New, baselined or suppressed.
    pub status: Status,
}

/// A baseline entry (or part of its count) without a finding.
#[derive(Clone, Debug)]
pub struct Stale {
    /// The entry; `count` is the number of findings that are gone.
    pub entry: baseline::Entry,
    /// The severity of its rule (`error` for a rule this binary does not know).
    pub severity: Severity,
}

/// Everything the output formats print.
pub struct Report {
    /// Every finding: by file, line, rule and key.
    pub items: Vec<Item>,
    /// Baseline entries without a finding.
    pub stale: Vec<Stale>,
    /// The baseline file as given (for messages); `None` with `--no-baseline`.
    pub baseline: Option<String>,
    /// `(tests on the floor, tests)`, when a rule computed the floor.
    pub floor: Option<(usize, usize)>,
}

impl Report {
    fn count(&self, status: &str) -> usize {
        self.items
            .iter()
            .filter(|i| i.status.as_str() == status)
            .count()
    }

    /// Whether the check fails: a new error-level finding (invalid suppressions
    /// are such findings) or a stale entry of an error-level rule.
    pub fn failed(&self) -> bool {
        self.items
            .iter()
            .any(|i| i.status == Status::New && i.severity == Severity::Error)
            || self.stale.iter().any(|s| s.severity == Severity::Error)
    }
}

/// The output of `lint`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LintFormat {
    /// Text.
    Text,
    /// JSON.
    Json,
    /// GitHub Actions annotations.
    Github,
}

/// `lint`.
#[derive(Debug)]
pub struct LintCmd {
    /// The test files whose cost is reported for each finding: jest, vitest,
    /// pytest, gotest, cargo-test, or `all`. Repeatable [default: `tests`
    /// of `[lint]` in turbotrace.toml, else all].
    pub tests: Vec<String>,
    /// Set the severity of a rule: `<rule>=off|warn|error`. Overrides
    /// `[lint.rules]` in turbotrace.toml. Repeatable.
    pub rules: Vec<String>,
    /// Skip findings in files matching this glob (relative to the repository
    /// root), in addition to `exclude` of `[lint]`. Repeatable.
    pub exclude: Vec<String>,
    /// The baseline file [default: `baseline` of `[lint]`, else
    /// turbotrace-lint-baseline.json in the repository root].
    pub baseline: Option<PathBuf>,
    /// Ignore the baseline: every finding is new.
    pub no_baseline: bool,
    /// Write the baseline from the current findings and exit with 0.
    pub update_baseline: bool,
    /// Output format. `github`: workflow commands that annotate the pull
    /// request.
    pub format: LintFormat,
    /// Print the rules with the severity each has in this repository, and exit.
    pub list_rules: bool,
    /// Where the graph lives.
    pub loc: GraphLoc,
    /// Use the stored graph as is, without refreshing it.
    pub no_refresh: bool,
    /// Print timings to stderr.
    pub stats: bool,
}

impl Default for LintCmd {
    fn default() -> Self {
        LintCmd {
            tests: Default::default(),
            rules: Default::default(),
            exclude: Default::default(),
            baseline: Default::default(),
            no_baseline: Default::default(),
            update_baseline: Default::default(),
            format: LintFormat::Text,
            list_rules: Default::default(),
            loc: Default::default(),
            no_refresh: Default::default(),
            stats: Default::default(),
        }
    }
}

/// The settings of one run: defaults, then `[lint]`, then the command line.
struct Settings {
    /// Severity per rule name.
    severity: BTreeMap<&'static str, Severity>,
    tests: Vec<String>,
    exclude: Vec<Glob>,
    /// Absolute path and the name to print; `None` with `--no-baseline`.
    baseline: Option<(PathBuf, String)>,
}

/// `<rule>=<severity>` of `--rule` or one `[lint.rules]` entry.
fn set_severity(
    severity: &mut BTreeMap<&'static str, Severity>,
    name: &str,
    value: &str,
    source: &str,
) -> Result<()> {
    let Some(r) = rule(name) else {
        if name == INVALID_SUPPRESSION || name == UNUSED_SUPPRESSION {
            bail!("{source}: `{name}` is not a rule that can be configured");
        }
        bail!(
            "{source}: unknown rule `{name}`: the rules are {}",
            rule_names()
        );
    };
    let Some(s) = Severity::parse(value) else {
        bail!("{source}: `{value}` for rule `{name}`: expected off, warn or error");
    };
    severity.insert(r.name, s);
    Ok(())
}

fn settings(cmd: &LintCmd, root: &Path) -> Result<Settings> {
    let conf = config::load(root)?;
    let mut severity: BTreeMap<&'static str, Severity> =
        RULES.iter().map(|r| (r.name, r.default)).collect();
    for (name, value) in &conf.rules {
        set_severity(&mut severity, name, value, &conf.source)?;
    }
    for arg in &cmd.rules {
        let Some((name, value)) = arg.split_once('=') else {
            bail!("--rule {arg}: expected <rule>=off|warn|error");
        };
        set_severity(&mut severity, name.trim(), value.trim(), "--rule")?;
    }
    let tests = if !cmd.tests.is_empty() {
        cmd.tests.clone()
    } else {
        conf.tests.clone().unwrap_or_else(|| vec!["all".to_owned()])
    };
    let mut exclude = Vec::new();
    for (g, source) in (conf.exclude.iter().map(|g| (g, conf.source.as_str())))
        .chain(cmd.exclude.iter().map(|g| (g, "--exclude")))
    {
        let g = g.trim_start_matches("./");
        exclude.push(Glob::new(g).map_err(|e| anyhow!("{source}: exclude `{g}`: {e}"))?);
    }
    let baseline = if cmd.no_baseline {
        None
    } else if let Some(p) = &cmd.baseline {
        // Like every path on the command line: relative to the current directory.
        let cwd = std::env::current_dir().context("cannot read current directory")?;
        Some((cwd.join(p), p.display().to_string()))
    } else {
        // In the configuration: relative to the repository root.
        let name = conf
            .baseline
            .clone()
            .unwrap_or_else(|| baseline::DEFAULT_FILE.to_owned());
        Some((root.join(&name), name))
    };
    Ok(Settings {
        severity,
        tests,
        exclude,
        baseline,
    })
}

// Invariant: `Baseline::matches` answers once per key, and the keys are the
// open findings in this order.
#[allow(clippy::expect_used)]
pub fn cmd_lint(cmd: LintCmd, out: &mut dyn Write) -> Result<ExitCode> {
    let t0 = Instant::now();
    let (root, _) = cmd.loc.resolve()?;
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    // Before the graph: a wrong rule name or severity fails at once.
    let set = settings(&cmd, &root)?;
    if cmd.list_rules {
        for r in RULES {
            writeln!(
                out,
                "{:<22} {:<6} {}",
                r.name,
                set.severity[r.name].as_str(),
                r.summary
            )?;
        }
        return Ok(ExitCode::SUCCESS);
    }

    let g = open(&cmd.loc, cmd.no_refresh, cmd.stats)?;
    let t_graph = t0.elapsed();
    // The same evaluation as `uncertainty` (and `affected` without its strict
    // flags).
    let opts = selection_options(&root, false, Policy::Affected);
    let aopts = opts.affected.clone();
    let t = Instant::now();
    let dg = crate::graph::derived(&g, cmd.stats)?;
    let t_derived = t.elapsed();
    let engine = Engine::with_options(&dg, opts);
    let t = Instant::now();
    let entries = collect_entries(&g, &root, &[], &[], &set.tests)?;
    let runtime = runtime_entries(&g, &root, &entries.set);
    let t_entries = t.elapsed();

    let ctx = Ctx {
        root: &root,
        graph: &g,
        derived: &dg,
        engine: &engine,
        options: &aopts,
        entries: &entries.set,
        runtime_entries: &runtime,
        floor: OnceCell::new(),
        floor_time: Cell::new(Duration::ZERO),
        floor_open: OnceCell::new(),
        reach: OnceCell::new(),
        bounded: OnceCell::new(),
    };
    let t = Instant::now();
    let mut findings: Vec<Finding> = Vec::new();
    let mut ran = 0;
    for r in RULES {
        if set.severity[r.name] == Severity::Off {
            continue;
        }
        ran += 1;
        let found = (r.run)(&ctx).with_context(|| format!("rule {}", r.name))?;
        // The severity, the baseline and the suppressions go by this name.
        if let Some(f) = found.iter().find(|f| f.rule != r.name) {
            bail!("rule {} returned a finding named {}", r.name, f.rule);
        }
        findings.extend(found);
    }
    let t_rules = t.elapsed();
    findings.retain(|f| !set.exclude.iter().any(|g| g.matches(&f.file)));
    findings
        .sort_by(|a, b| (&a.file, a.line, a.rule, &a.key).cmp(&(&b.file, b.line, b.rule, &b.key)));

    // Suppression comments, read from the files that have findings.
    let t = Instant::now();
    let enabled = |name: &str| rule(name).map(|r| set.severity[r.name] != Severity::Off);
    let sup = suppress::apply(&root, &findings, &enabled);
    let t_sources = t.elapsed();

    let mut items: Vec<Item> = Vec::with_capacity(findings.len() + sup.meta.len());
    for (f, reason) in findings.into_iter().zip(sup.reasons) {
        items.push(Item {
            severity: set.severity[f.rule],
            status: reason.map_or(Status::New, Status::Suppressed),
            finding: f,
        });
    }
    let open_keys = |items: &[Item]| -> Vec<baseline::Key> {
        items
            .iter()
            .filter(|i| i.status == Status::New)
            .map(|i| baseline::Key::of(&i.finding))
            .collect()
    };

    if cmd.update_baseline {
        let Some((path, name)) = set.baseline.as_ref() else {
            bail!("--update-baseline cannot be combined with --no-baseline");
        };
        let b = baseline::Baseline::from_keys(open_keys(&items));
        b.write(path)?;
        let suppressed = items.len() - b.total();
        writeln!(
            out,
            "{name}: {} finding(s) written as {} entr(ies){}",
            b.total(),
            b.entries.len(),
            if suppressed > 0 {
                format!("; {suppressed} suppressed finding(s) are not in the baseline")
            } else {
                String::new()
            }
        )?;
        for m in &sup.meta {
            eprintln!("warning: {}:{}: {}: {}", m.file, m.line, m.rule, m.message);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut stale = Vec::new();
    if let Some((path, _)) = &set.baseline {
        let b = baseline::Baseline::load(path)?;
        // Entries of rules that did not run and of excluded files are not judged: the
        // check did not look there.
        let in_scope = |e: &baseline::Entry| {
            rule(&e.rule).is_none_or(|r| set.severity[r.name] != Severity::Off)
                && !set.exclude.iter().any(|g| g.matches(&e.file))
        };
        let m = b.matches(&open_keys(&items), &in_scope);
        let mut hit = m.baselined.into_iter();
        for i in items.iter_mut().filter(|i| i.status == Status::New) {
            if hit.next().expect("one answer per open finding") {
                i.status = Status::Baselined;
            }
        }
        stale = (m.stale.into_iter())
            .map(|entry| Stale {
                severity: rule(&entry.rule).map_or(Severity::Error, |r| set.severity[r.name]),
                entry,
            })
            .collect();
    }
    // Problems with the suppression comments themselves: never suppressed or
    // baselined.
    for f in sup.meta {
        items.push(Item {
            severity: if f.rule == INVALID_SUPPRESSION {
                Severity::Error
            } else {
                Severity::Warn
            },
            status: Status::New,
            finding: f,
        });
    }
    items.sort_by(|a, b| {
        let (a, b) = (&a.finding, &b.finding);
        (&a.file, a.line, a.rule, &a.key).cmp(&(&b.file, b.line, b.rule, &b.key))
    });

    let report = Report {
        items,
        stale,
        baseline: set.baseline.as_ref().map(|(_, name)| name.clone()),
        floor: ctx.floor.get().map(|f| (f.floor, f.entries)),
    };
    if cmd.stats {
        eprintln!(
            "lint: {} finding(s) from {ran} rule(s), {} entr(ies); graph {} ms, derived {:.1} ms, \
             entries {:.1} ms, rules {:.1} ms (floor {:.1} ms), sources {:.1} ms ({} file(s) \
             read), total {} ms",
            report.items.len(),
            entries.set.len(),
            t_graph.as_millis(),
            t_derived.as_secs_f64() * 1e3,
            t_entries.as_secs_f64() * 1e3,
            t_rules.as_secs_f64() * 1e3,
            ctx.floor_time.get().as_secs_f64() * 1e3,
            t_sources.as_secs_f64() * 1e3,
            sup.files_read,
            t0.elapsed().as_millis()
        );
    }
    match cmd.format {
        LintFormat::Text => output::text(&report, out)?,
        LintFormat::Json => output::json(&report, out)?,
        LintFormat::Github => output::github(&report, out)?,
    }
    Ok(if report.failed() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// The entries that `affected` evaluates without type-only edges: jest and
/// vitest tests whose runner transpiles file by file
/// (`turborepo_trace_graph::barrel::modes`). None with
/// `TURBO_TRACE_DEBUG_TYPE_EDGES=0`, as for `affected`.
fn runtime_entries(
    g: &turborepo_trace_store::Graph,
    root: &std::path::Path,
    entries: &turborepo_trace_graph::NodeSet,
) -> turborepo_trace_graph::NodeSet {
    use turborepo_trace_graph::{
        GraphView,
        barrel::{Runner, entry_runner},
    };
    let mut set = turborepo_trace_graph::NodeSet::new(g.node_count());
    if turborepo_trace_core::debug::var_os("TYPE_EDGES").is_some_and(|v| v == "0") {
        return set;
    }
    let disk = crate::barrel::DiskSource::new(root);
    for n in entries.iter() {
        if entry_runner(g, &disk, n) == Runner::Isolated {
            set.insert(n);
        }
    }
    set
}
