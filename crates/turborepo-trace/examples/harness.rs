//! The test harness: a command line over the library (`affected`, `why`,
//! `lint` with their full option sets, and `graph build`), so the command tests
//! run it as a process with their arguments, working directory and
//! environment. The option types are converted into the library's. Not
//! shipped: a dev-only example.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

pub use graph::GraphLoc;
use usage::{Args, Cli as Parser, Subcommands as Subcommand, ValueEnum};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ExternalsArg {
    Follow,
    Stop,
    List,
}

impl From<ExternalsArg> for turborepo_trace::commands::ExternalsArg {
    fn from(c: ExternalsArg) -> Self {
        match c {
            ExternalsArg::Follow => Self::Follow,
            ExternalsArg::Stop => Self::Stop,
            ExternalsArg::List => Self::List,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PhaseArg {
    Runtime,
    Build,
    All,
}

impl From<PhaseArg> for turborepo_trace::commands::PhaseArg {
    fn from(c: PhaseArg) -> Self {
        match c {
            PhaseArg::Runtime => Self::Runtime,
            PhaseArg::Build => Self::Build,
            PhaseArg::All => Self::All,
        }
    }
}

/// Options shared by `trace` and `why`.
#[derive(Debug, Args)]
pub struct TraceOpts {
    /// Output paths are relative to this directory [default: current
    /// directory].
    #[usage(long, value_name = "DIR")]
    pub base: Option<PathBuf>,
    /// Working directory assumed for cwd-relative reads [default: current
    /// directory].
    #[usage(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    /// What to do with third-party dependencies.
    #[usage(long, value_enum, default = "follow")]
    pub externals: ExternalsArg,
    /// Resolution conditions, comma separated (JS `exports`).
    #[usage(long, delimiter = ',', value_name = "COND")]
    pub conditions: Vec<String>,
    /// Also resolve every other condition a package's `exports` defines and
    /// include all results (over-approximation for safety).
    #[usage(long)]
    pub all_conditions: bool,
    /// Which phases to include.
    #[usage(long, value_enum, default = "all")]
    pub phase: PhaseArg,
    /// Build target, shared by all languages: <os>/<arch>[/musl] in Node or Go
    /// names (linux/x64, linux/amd64, linux/arm64/musl, darwin/arm64) or a
    /// Rust target triple (x86_64-unknown-linux-gnu). JS uses it for native
    /// binaries and package special cases [default: host].
    #[usage(long, value_name = "OS/ARCH")]
    pub target: Option<String>,
    /// Ignore files matching this glob (relative to the base). Repeatable.
    #[usage(long, value_name = "GLOB")]
    pub ignore: Vec<String>,
    /// Maximum files a glob or directory expands to.
    #[usage(long, value_name = "N", default_value_t = turborepo_trace_core::DEFAULT_MAX_GLOB, default = "1000")]
    pub max_glob: usize,
    /// Include type-only references.
    #[usage(long)]
    pub include_types: bool,
    /// Emit file hashes (nft `fileHashes`).
    #[usage(long)]
    pub hashes: bool,
    /// Worker threads (0 = one per core).
    #[usage(long, value_name = "N", default_value_t = 0, default = "0")]
    pub threads: usize,
    /// Use the on-disk extraction cache (.turbotrace/cache).
    #[usage(long)]
    pub cache: bool,
    /// Include files for every target, not only --target, as conditional edges:
    /// Go files excluded by build constraints, Rust items and dependencies
    /// behind target `cfg`.
    #[usage(long)]
    pub all_targets: bool,
    #[usage(flatten)]
    pub lang: langs::LangArgs,
}

impl From<TraceOpts> for turborepo_trace::commands::TraceOpts {
    fn from(c: TraceOpts) -> Self {
        Self {
            base: c.base,
            cwd: c.cwd,
            externals: c.externals.into(),
            conditions: c.conditions,
            all_conditions: c.all_conditions,
            phase: c.phase.into(),
            target: c.target,
            ignore: c.ignore,
            max_glob: c.max_glob,
            include_types: c.include_types,
            hashes: c.hashes,
            threads: c.threads,
            cache: c.cache,
            all_targets: c.all_targets,
            lang: c.lang.into(),
        }
    }
}

#[derive(Debug, Args)]
pub struct WhyCmd {
    /// The entry file.
    pub entry: PathBuf,
    /// The file whose inclusion to explain.
    pub file: PathBuf,
    #[usage(flatten)]
    pub opts: TraceOpts,
}

impl From<WhyCmd> for turborepo_trace::commands::WhyCmd {
    fn from(c: WhyCmd) -> Self {
        Self {
            entry: c.entry,
            file: c.file,
            opts: c.opts.into(),
        }
    }
}

pub mod affected {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// `affected`.
    #[derive(Debug, Args)]
    pub struct AffectedCmd {
        /// Entry files or globs (relative to the current directory; globs match
        /// graph files).
        #[usage(value_name = "ENTRIES")]
        pub entries: Vec<String>,
        /// More entry files or globs. Repeatable.
        #[usage(long = "entries", value_name = "GLOB|FILE")]
        pub entries_opt: Vec<String>,
        /// Read entries from a file (`-`: stdin), one per line, relative to the
        /// graph root.
        #[usage(long, value_name = "FILE|-")]
        pub entries_from: Vec<String>,
        /// Use the test files of a framework as entries: jest, vitest, pytest,
        /// gotest, cargo-test, or `all`. Repeatable. `playwright`
        /// selects the end-to-end specs of Next.js apps from source
        /// (the routes the change reaches and the routes a spec visits)
        /// and is its own command.
        #[usage(long, value_name = "FRAMEWORK")]
        pub tests: Vec<String>,
        /// Use build tasks as entries: a glob over task ids without the kind
        /// prefix (`apps/*#build`, `*#test`), or over full ids
        /// (`task:apps/*#build`, `make:*`). Repeatable. Task ids can
        /// also be passed as entries (`task:apps/web#build`).
        #[usage(long, value_name = "GLOB")]
        pub tasks: Vec<String>,
        /// With --tasks: also the tasks a wildcard leaves out because they are
        /// run by hand by their nature: dev servers, watch modes and
        /// the fix variants of checks (a `dev`, `start`, `serve`,
        /// `preview`, `watch` or `fix` segment in the name), unless a CI job
        /// or an install script runs them. A task named exactly is always an
        /// entry.
        #[usage(long, requires = "tasks")]
        pub all_tasks: bool,
        /// Changed paths (relative to the current directory): modified if the
        /// path exists, deleted otherwise. Repeatable.
        #[usage(long, value_name = "PATH", num_args = 1..)]
        pub changed: Vec<String>,
        /// Read changes from a file (`-`: stdin): newline- or NUL-separated
        /// paths relative to the graph root, or `git diff
        /// --name-status` lines (`A`/`M`/`D`/`R`...).
        #[usage(long, value_name = "FILE|-")]
        pub changed_from: Vec<String>,
        /// The revision the branch is compared with, usually the default branch
        /// (`origin/main`). The base is the merge base of <REV> and HEAD: the
        /// latest commit of <REV> that the branch contains. Without
        /// other change input, the changes are the difference between
        /// that commit's tree and the working tree, plus untracked files:
        /// the branch's own changes, not what <REV> gained since, and not what
        /// a merge of <REV> into the branch brought in. The base is
        /// also the source of old lockfiles, manifests and configs. It
        /// is printed on stderr. When it can't be told (a shallow
        /// checkout that lacks the history between the two), the command fails;
        /// with --format selection the document then says that
        /// everything runs.
        #[usage(long, value_name = "REV")]
        pub since: Option<String>,
        /// Compare with exactly the --since revision, without the merge-base
        /// step: for a caller that has resolved the base itself, such
        /// as a CI system that hands over the base commit of a pull
        /// request (the environment variable TURBO_TRACE_DEBUG_EXACT_BASE=1
        /// does the same).
        #[usage(long, requires = "since")]
        pub exact_base: bool,
        /// Debug: print to stderr the N files whose names select the most
        /// entries (one reachability pass per naming file: slow).
        #[usage(long, value_name = "N", hide = true)]
        pub names_report: Option<usize>,
        /// Verdict for entries that reach first-party uncertainty.
        #[usage(long, value_enum, default = "affected")]
        pub on_uncertain: OnUncertain,
        /// Let `eval`, `new Function` and other code evaluation (`eval_like`)
        /// trigger --on-uncertain. By default it is reported but
        /// ignored.
        #[usage(long)]
        pub strict_eval: bool,
        /// Let every uncertainty trigger --on-uncertain, including the ones
        /// that are covered or ignored by default: JS/TS imports that
        /// don't resolve (covered by the paths their resolution
        /// probed), committed generated files, file reads whose path comes from
        /// the environment or the command line, and code evaluation
        /// (--strict-eval).
        #[usage(long)]
        pub strict: bool,
        /// Ignore type-only edges (pure test execution; typecheck reads them).
        #[usage(long)]
        pub runtime_only: bool,
        /// Output format: the affected entries one per line, JSON with a
        /// verdict and a witness chain per entry, or `selection`: the
        /// document a CI pipeline reads: per package, the test files the
        /// selection would run and what the caller must run under --mode.
        #[usage(long, value_enum, default = "list")]
        pub format: AffectedFormat,
        /// With --format selection: the switch. `shadow` (the default): the
        /// caller must run the full set, and the selection is recorded
        /// next to it. `enforce`: the caller runs the selection.
        /// Without the flag the environment variable TURBO_TRACE_DEBUG_MODE
        /// decides.
        #[usage(long, value_enum, value_name = "MODE")]
        pub mode: Option<crate::selection::Mode>,
        /// With --format selection: a changed path that matches makes the
        /// caller run the full set in every mode, because it steers the
        /// selection itself (a glob relative to the repository root;
        /// repeatable). `turbotrace.toml` always counts.
        #[usage(long, value_name = "GLOB")]
        pub run_all_if_changed: Vec<String>,
        /// With --format selection: files that look like tests (`*.test.*`,
        /// `*.spec.*`) and belong to a runner that was not asked for,
        /// such as end-to-end specs. They are not listed as unknown
        /// tests (a glob relative to the repository root; repeatable).
        #[usage(long, value_name = "GLOB")]
        pub not_tests: Vec<String>,
        /// With --format selection: packages that run all their tests or none,
        /// never a part, because they check something over the whole
        /// run, such as a coverage threshold (a glob over package
        /// directories relative to the repository root; repeatable).
        #[usage(long, value_name = "GLOB")]
        pub full_package: Vec<String>,
        /// With --format selection: exit with an error when the selection can't
        /// be computed. By default the document then says that the full
        /// set runs and why, and the exit code is 0, so that a pipeline
        /// never stops because of the selection step.
        #[usage(long)]
        pub no_fail_open: bool,
        /// Print a readable reason for these entries instead of the list (they
        /// are checked as entries too). Repeatable.
        #[usage(long, value_name = "ENTRY")]
        pub explain: Vec<String>,
        /// Exit 0 if nothing is affected, 1 if an entry is affected, 2 if one
        /// is unknown.
        #[usage(long)]
        pub exit_code: bool,
        #[usage(flatten)]
        pub loc: GraphLoc,
        /// More repo steering files, as a glob relative to the graph root.
        /// Repeatable. Steering files (by default `.github/**`,
        /// `CODEOWNERS`, `.vercel.approvers`, `.gitattributes`,
        /// `.git-blame-ignore-revs`, `.editorconfig`, `.mergify.yml`,
        /// `renovate.json`, `.renovaterc*`) can't change what an
        /// application does: a changed one affects only the
        /// entries that reference it explicitly.
        #[usage(long, value_name = "GLOB")]
        pub steering: Vec<String>,
        /// Treat repo steering files like any other changed file.
        #[usage(long)]
        pub no_steering: bool,
        /// Don't derive changed CI jobs and the tests they run from the changes
        /// (README "CI workflow changes"): an A/B switch for measuring
        /// what that rule selects.
        #[usage(long)]
        pub no_workflows: bool,
        /// Use the stored graph as is, without refreshing it.
        #[usage(long)]
        pub no_refresh: bool,
        /// Print timings to stderr.
        #[usage(long)]
        pub stats: bool,
    }

    impl From<AffectedCmd> for turborepo_trace::commands::AffectedCmd {
        fn from(c: AffectedCmd) -> Self {
            Self {
                entries: c.entries,
                entries_opt: c.entries_opt,
                entries_from: c.entries_from,
                tests: c.tests,
                tasks: c.tasks,
                all_tasks: c.all_tasks,
                changed: c.changed,
                changed_from: c.changed_from,
                since: c.since,
                exact_base: c.exact_base,
                names_report: c.names_report,
                on_uncertain: c.on_uncertain.into(),
                strict_eval: c.strict_eval,
                strict: c.strict,
                runtime_only: c.runtime_only,
                format: c.format.into(),
                mode: c.mode.map(Into::into),
                run_all_if_changed: c.run_all_if_changed,
                not_tests: c.not_tests,
                full_package: c.full_package,
                no_fail_open: c.no_fail_open,
                explain: c.explain,
                exit_code: c.exit_code,
                loc: c.loc.into(),
                steering: c.steering,
                no_steering: c.no_steering,
                no_workflows: c.no_workflows,
                no_refresh: c.no_refresh,
                stats: c.stats,
                change_source: None,
            }
        }
    }

    /// The output of `affected`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum AffectedFormat {
        List,
        Json,
        /// The selection document for a CI pipeline (crate::selection,
        /// `docs/ci.md`).
        Selection,
    }

    impl From<AffectedFormat> for turborepo_trace::commands::AffectedFormat {
        fn from(c: AffectedFormat) -> Self {
            match c {
                AffectedFormat::List => Self::List,
                AffectedFormat::Json => Self::Json,
                AffectedFormat::Selection => Self::Selection,
            }
        }
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum OnUncertain {
        Affected,
        Unknown,
        Ignore,
    }

    impl From<OnUncertain> for turborepo_trace::commands::OnUncertain {
        fn from(c: OnUncertain) -> Self {
            match c {
                OnUncertain::Affected => Self::Affected,
                OnUncertain::Unknown => Self::Unknown,
                OnUncertain::Ignore => Self::Ignore,
            }
        }
    }
}
pub mod graph {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// `graph build`.
    #[derive(Debug, Args)]
    pub struct BuildCmd {
        #[usage(flatten)]
        pub loc: GraphLoc,
        #[usage(flatten)]
        pub opts: BuildOpts,
        /// Print the report as JSON.
        #[usage(long)]
        pub json: bool,
    }

    impl From<BuildCmd> for turborepo_trace::commands::BuildCmd {
        fn from(c: BuildCmd) -> Self {
            Self {
                loc: c.loc.into(),
                opts: c.opts.into(),
                json: c.json,
            }
        }
    }

    /// Build options.
    #[derive(Debug, Args, Clone)]
    pub struct BuildOpts {
        /// Third-party packages: `stop` keeps one opaque node per name@version
        /// (default); `follow` traces into them.
        #[usage(long, value_enum, default = "stop")]
        pub externals: GraphExternals,
        /// Resolution conditions, comma separated (JS `exports`).
        #[usage(long, delimiter = ',', value_name = "COND")]
        pub conditions: Vec<String>,
        /// Also resolve every other `exports` condition.
        #[usage(long)]
        pub all_conditions: bool,
        /// Build target (linux/x64, darwin/arm64, ...) [default: host].
        #[usage(long, value_name = "OS/ARCH")]
        pub target: Option<String>,
        /// Maximum files a glob or directory expands to.
        #[usage(long, value_name = "N", default_value_t = turborepo_trace_core::DEFAULT_MAX_GLOB, default = "1000")]
        pub max_glob: usize,
        /// Leave out type-only references (included by default: typecheck reads
        /// them).
        #[usage(long)]
        pub no_types: bool,
        /// Worker threads (0 = one per core).
        #[usage(long, value_name = "N", default_value_t = 0, default = "0")]
        pub threads: usize,
        /// Include files for every target as conditional edges (Go, Rust).
        #[usage(long)]
        pub all_targets: bool,
        #[usage(flatten)]
        pub lang: langs::LangArgs,
    }

    impl From<BuildOpts> for turborepo_trace::commands::BuildOpts {
        fn from(c: BuildOpts) -> Self {
            Self {
                externals: c.externals.into(),
                conditions: c.conditions,
                all_conditions: c.all_conditions,
                target: c.target,
                max_glob: c.max_glob,
                no_types: c.no_types,
                threads: c.threads,
                all_targets: c.all_targets,
                lang: c.lang.into(),
            }
        }
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum GraphExternals {
        Stop,
        Follow,
    }

    impl From<GraphExternals> for turborepo_trace::commands::GraphExternals {
        fn from(c: GraphExternals) -> Self {
            match c {
                GraphExternals::Stop => Self::Stop,
                GraphExternals::Follow => Self::Follow,
            }
        }
    }

    /// Where the graph lives.
    #[derive(Debug, Args, Clone)]
    pub struct GraphLoc {
        /// Repository root [default: the root recorded in the graph, else the
        /// current directory].
        #[usage(long, value_name = "DIR")]
        pub root: Option<PathBuf>,
        /// Graph directory [default: <root>/.turbo/trace].
        #[usage(long, alias = "out", value_name = "DIR")]
        pub graph: Option<PathBuf>,
    }

    impl From<GraphLoc> for turborepo_trace::commands::GraphLoc {
        fn from(c: GraphLoc) -> Self {
            Self {
                root: c.root,
                graph: c.graph,
                members: Default::default(),
            }
        }
    }
}
pub mod langs {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// Language-specific options, shared by `trace`, `why`, `extract` and
    /// `graph build`.
    ///
    /// `graph build` records them (paths made absolute) in `<graph
    /// dir>/lang-args.json`, so the refresh before `graph query`/`export`
    /// uses the same languages as the build.
    #[derive(Debug, Args, Clone, Default, PartialEq)]
    pub struct LangArgs {
        /// Go build tags, comma separated (`go build -tags`).
        #[usage(long, delimiter = ',', value_name = "TAG", help_heading = "Go")]
        pub tags: Vec<String>,
        /// Cargo features of the workspace members, comma separated (`name` or
        /// `pkg/name`).
        #[usage(long, delimiter = ',', value_name = "FEATURE", help_heading = "Rust")]
        pub features: Vec<String>,
        /// Enable every feature of the workspace members.
        #[usage(long, help_heading = "Rust")]
        pub all_features: bool,
        /// Do not enable the members' `default` features.
        #[usage(long, help_heading = "Rust")]
        pub no_default_features: bool,
        /// Extra Python module search roots (like `PYTHONPATH`). Repeatable.
        #[usage(long, value_name = "DIR", help_heading = "Python")]
        pub python_path: Vec<PathBuf>,
        /// The Python virtual environment [default: $VIRTUAL_ENV, else .venv/
        /// or venv/ in the base].
        #[usage(long, value_name = "DIR", help_heading = "Python")]
        pub venv: Option<PathBuf>,
    }

    impl From<LangArgs> for turborepo_trace::commands::LangArgs {
        fn from(c: LangArgs) -> Self {
            Self {
                tags: c.tags,
                features: c.features,
                all_features: c.all_features,
                no_default_features: c.no_default_features,
                python_path: c.python_path,
                venv: c.venv,
            }
        }
    }
}
pub mod lint {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// `lint`.
    #[derive(Debug, Args)]
    pub struct LintCmd {
        /// The test files whose cost is reported for each finding: jest,
        /// vitest, pytest, gotest, cargo-test, or `all`. Repeatable
        /// [default: `tests` of `[lint]` in turbotrace.toml, else all].
        #[usage(long, value_name = "FRAMEWORK")]
        pub tests: Vec<String>,
        /// Set the severity of a rule: `<rule>=off|warn|error`. Overrides
        /// `[lint.rules]` in turbotrace.toml. Repeatable.
        #[usage(long = "rule", value_name = "RULE=SEVERITY")]
        pub rules: Vec<String>,
        /// Skip findings in files matching this glob (relative to the
        /// repository root), in addition to `exclude` of `[lint]`.
        /// Repeatable.
        #[usage(long, value_name = "GLOB")]
        pub exclude: Vec<String>,
        /// The baseline file [default: `baseline` of `[lint]`, else
        /// turbotrace-lint-baseline.json in the repository root].
        #[usage(long, value_name = "FILE", conflicts = "no_baseline")]
        pub baseline: Option<PathBuf>,
        /// Ignore the baseline: every finding is new.
        #[usage(long)]
        pub no_baseline: bool,
        /// Write the baseline from the current findings and exit with 0.
        #[usage(long, conflicts = "no_baseline")]
        pub update_baseline: bool,
        /// Output format. `github`: workflow commands that annotate the pull
        /// request.
        #[usage(long, value_enum, default = "text")]
        pub format: LintFormat,
        /// Print the rules with the severity each has in this repository, and
        /// exit.
        #[usage(long)]
        pub list_rules: bool,
        #[usage(flatten)]
        pub loc: GraphLoc,
        /// Use the stored graph as is, without refreshing it.
        #[usage(long)]
        pub no_refresh: bool,
        /// Print timings to stderr.
        #[usage(long)]
        pub stats: bool,
    }

    impl From<LintCmd> for turborepo_trace::commands::LintCmd {
        fn from(c: LintCmd) -> Self {
            Self {
                tests: c.tests,
                rules: c.rules,
                exclude: c.exclude,
                baseline: c.baseline,
                no_baseline: c.no_baseline,
                update_baseline: c.update_baseline,
                format: c.format.into(),
                list_rules: c.list_rules,
                loc: c.loc.into(),
                no_refresh: c.no_refresh,
                stats: c.stats,
            }
        }
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum LintFormat {
        Text,
        Json,
        Github,
    }

    impl From<LintFormat> for turborepo_trace::commands::LintFormat {
        fn from(c: LintFormat) -> Self {
            match c {
                LintFormat::Text => Self::Text,
                LintFormat::Json => Self::Json,
                LintFormat::Github => Self::Github,
            }
        }
    }
}
pub mod selection {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// The switch: what `run` carries.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum Mode {
        /// The full set runs; the selection is computed and recorded next to
        /// it.
        Shadow,
        /// The selection runs.
        Enforce,
    }

    impl From<Mode> for turborepo_trace::commands::Mode {
        fn from(c: Mode) -> Self {
            match c {
                Mode::Shadow => Self::Shadow,
                Mode::Enforce => Self::Enforce,
            }
        }
    }
}

/// The command line.
#[derive(Debug, Parser)]
#[usage(name = "turbotrace", about = "turborepo-trace test harness")]
struct Cli {
    #[usage(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Which entries a change affects.
    Affected(affected::AffectedCmd),
    /// Why an entry's trace contains a file.
    Why(WhyCmd),
    /// The lints.
    Lint(lint::LintCmd),
    /// The whole-repo graph.
    Graph {
        #[usage(subcommand)]
        command: GraphCmd,
    },
    /// Files the given files transitively depend on, one per line.
    Deps(DepsCmd),
    /// The uncertainty floor, as `lint` and `affected` report it.
    Floor(FloorCmd),
}

#[derive(Debug, Subcommand)]
enum GraphCmd {
    /// Build or refresh the graph.
    Build(graph::BuildCmd),
    /// Export the composed graph (JSON or NDJSON).
    Export(ExportCmd),
}

/// `graph export`: the composed graph, as the library reads it.
#[derive(Debug, Args)]
struct ExportCmd {
    #[usage(flatten)]
    loc: graph::GraphLoc,
    /// Output format.
    #[usage(long, value_enum, default = "json")]
    format: ExportFormat,
    /// Use the stored graph as is, without refreshing it.
    #[usage(long)]
    no_refresh: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ExportFormat {
    /// One JSON document.
    Json,
    /// One JSON record per line.
    Ndjson,
}

/// `deps`: the closure of the given files, without the files themselves.
#[derive(Debug, Args)]
struct DepsCmd {
    /// Files (relative to the current directory).
    #[usage(required = true)]
    files: Vec<PathBuf>,
    #[usage(flatten)]
    loc: graph::GraphLoc,
}

type Out<'a> = std::io::BufWriter<std::io::StdoutLock<'a>>;

fn export(
    c: ExportCmd,
    out: &mut Out<'_>,
) -> Result<turborepo_trace::ExitCode, turborepo_trace::Error> {
    let loc = c.loc.into();
    let format = c.format;
    turborepo_trace::commands::with_graph(&loc, c.no_refresh, |g, root| match format {
        ExportFormat::Json => turborepo_trace_graph::json::write_json(g, Some(root), false, out),
        ExportFormat::Ndjson => turborepo_trace_graph::json::write_ndjson(g, Some(root), out),
    })?
    .map_err(|e| turborepo_trace::Error::Failed(e.to_string()))?;
    Ok(turborepo_trace::ExitCode::SUCCESS)
}

fn deps(
    c: DepsCmd,
    out: &mut Out<'_>,
) -> Result<turborepo_trace::ExitCode, turborepo_trace::Error> {
    use turborepo_trace_graph::{AffectedOptions, Engine, EngineOptions, output};
    let loc: turborepo_trace::commands::GraphLoc = c.loc.into();
    let cwd = std::env::current_dir().map_err(turborepo_trace::Error::Io)?;
    turborepo_trace::commands::with_graph(&loc, false, |g, root| {
        let root = std::path::Path::new(root);
        let mut lits = Vec::new();
        for f in &c.files {
            let p = turborepo_trace_core::fs::normalize(&cwd.join(f));
            let rel = p.strip_prefix(root).map_err(|_| {
                format!(
                    "{} is outside the graph root {}",
                    f.display(),
                    root.display()
                )
            })?;
            lits.push(format!("{:?}", rel.to_string_lossy()));
        }
        let set = lits.join(" + ");
        // The graph reflects the working tree, as for `affected`.
        let engine = Engine::with_options(
            g,
            EngineOptions {
                affected: AffectedOptions::default().after_change(true),
                ..Default::default()
            },
        );
        let result = engine
            .query(&format!("deps({set}) - ({set})"))
            .map_err(|e| e.to_string())?;
        output::write(
            &engine,
            &result,
            &output::OutputOptions {
                format: output::Format::List,
                edges: false,
            },
            out,
        )
        .map_err(|e| e.to_string())
    })?
    .map_err(turborepo_trace::Error::Failed)?;
    Ok(turborepo_trace::ExitCode::SUCCESS)
}

/// `floor`: the uncertainty floor through the library's commands: its size
/// and the sites that hold it from `lint` (no baseline), the tests on it from
/// `affected` with a change of nothing (selected on every change), and the
/// uncertainty records by disposition from `affected --format json`.
#[derive(Debug, Args)]
struct FloorCmd {
    /// The test files: jest, vitest, gotest, cargo-test or `all`. Repeatable.
    #[usage(long, value_name = "FRAMEWORK")]
    tests: Vec<String>,
    #[usage(flatten)]
    loc: graph::GraphLoc,
    /// Use the stored graph as is, without refreshing it.
    #[usage(long)]
    no_refresh: bool,
    /// Output JSON.
    #[usage(long)]
    json: bool,
}

fn floor(
    c: FloorCmd,
    out: &mut Out<'_>,
) -> Result<turborepo_trace::ExitCode, turborepo_trace::Error> {
    use std::io::Write;

    use turborepo_trace::commands::{self, AffectedCmd, AffectedFormat, LintCmd, LintFormat};
    let failed = |e: serde_json::Error| turborepo_trace::Error::Failed(e.to_string());
    let loc: commands::GraphLoc = c.loc.into();
    let mut lint = Vec::new();
    commands::lint(
        LintCmd {
            tests: c.tests.clone(),
            no_baseline: true,
            format: LintFormat::Json,
            loc: loc.clone(),
            no_refresh: c.no_refresh,
            ..Default::default()
        },
        &mut lint,
    )?;
    let lint: serde_json::Value = serde_json::from_slice(&lint).map_err(failed)?;
    // A change of nothing (a path relative to the working directory, inside the
    // repository).
    let nothing = "no/such/file-of-the-floor";
    let run = |format| {
        let mut buf = Vec::new();
        commands::affected(
            AffectedCmd {
                tests: c.tests.clone(),
                changed: vec![nothing.to_owned()],
                format,
                loc: loc.clone(),
                no_refresh: true,
                ..Default::default()
            },
            &mut buf,
        )
        .map(|_| buf)
    };
    let list = String::from_utf8_lossy(&run(AffectedFormat::List)?).into_owned();
    let json: serde_json::Value =
        serde_json::from_slice(&run(AffectedFormat::Json)?).map_err(failed)?;
    let findings: Vec<&serde_json::Value> =
        lint["findings"].as_array().into_iter().flatten().collect();
    let mut sites: Vec<&str> = findings.iter().filter_map(|f| f["file"].as_str()).collect();
    sites.dedup();
    let entries: Vec<&str> = list.lines().collect();
    let v = serde_json::json!({
        "floor": lint["summary"]["floor"],
        "entries": lint["summary"]["tests"],
        "sites": sites,
        "floor_entries": entries,
        "records": json["uncertainty"],
    });
    let w = |e: std::io::Error| turborepo_trace::Error::Io(e);
    if c.json {
        serde_json::to_writer_pretty(&mut *out, &v).map_err(failed)?;
        writeln!(out).map_err(w)?;
    } else {
        writeln!(
            out,
            "uncertainty floor: {} of {} entries",
            v["floor"], v["entries"]
        )
        .map_err(w)?;
        for f in &findings {
            writeln!(
                out,
                "  site {}:{} {}: {}",
                f["file"].as_str().unwrap_or_default(),
                f["line"],
                f["rule"].as_str().unwrap_or_default(),
                f["message"].as_str().unwrap_or_default()
            )
            .map_err(w)?;
        }
        for e in &entries {
            writeln!(out, "  entry {e}").map_err(w)?;
        }
    }
    Ok(turborepo_trace::ExitCode::SUCCESS)
}

fn main() -> std::process::ExitCode {
    use std::io::Write;

    use turborepo_trace::commands;
    let cli = Cli::parse();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let r = match cli.command {
        Command::Affected(c) => commands::affected(c.into(), &mut out),
        Command::Why(c) => commands::why(c.into(), &mut out),
        Command::Lint(c) => commands::lint(c.into(), &mut out),
        Command::Graph {
            command: GraphCmd::Build(c),
        } => commands::graph_build(c.into(), &mut out),
        Command::Graph {
            command: GraphCmd::Export(c),
        } => export(c, &mut out),
        Command::Deps(c) => deps(c, &mut out),
        Command::Floor(c) => floor(c, &mut out),
    };
    let flushed = out.flush();
    match (r, flushed) {
        (Ok(code), Ok(())) => std::process::ExitCode::from(code.0),
        (Err(e), _) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
        (Ok(_), Err(e)) => {
            eprintln!("error: cannot write output: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
