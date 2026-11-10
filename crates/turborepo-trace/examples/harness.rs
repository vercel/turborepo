//! The test harness: the library behind turbotrace's own command line
//! (`affected`, `why`, `lint`, and what the tests observe the graph through:
//! `graph build|export`, `deps`, `dependents`, `uncertainty`), so the command
//! tests run it as a process with their arguments, working directory and
//! environment. The option types are turbotrace's command-line types (generated
//! from them), converted into the library's. Not shipped: a dev-only example.
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
        /// verdict and a witness chain per entry, or `summary`: how
        /// much is selected and why, from which changed files, through
        /// which hubs, into which packages, what keeps the uncertainty floor
        /// and what was not selected, as a text report.
        /// `selection` is the document a CI pipeline reads (`docs/ci.md`): per
        /// package, the test files turbotrace would run and what the
        /// caller must run under --mode.
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
        #[usage(flatten)]
        pub viz: crate::viz::VizArgs,
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
        /// Print the unlikely-path audit instead of the list: the selected
        /// entries with no path of ordinary imports from a changed
        /// file, their plausibility signals, and the sources
        /// that put them in the selection, ranked by the entries each one
        /// triggers.
        #[usage(long, conflicts = "explain")]
        pub audit: bool,
        /// With --audit: the number of sources to rank.
        #[usage(
            long,
            value_name = "N",
            default_value_t = 15,
            default = "15",
            requires = "audit"
        )]
        pub top: usize,
        /// With --audit --format json: the entries to list with their verdicts:
        /// all, selected, flagged (with a signal) or none.
        #[usage(long, value_name = "WHICH", default = "all", requires = "audit")]
        pub audit_entries: String,
        #[usage(flatten)]
        pub e2e: crate::e2e::E2eArgs,
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
                viz: c.viz.into(),
                explain: c.explain,
                exit_code: c.exit_code,
                loc: c.loc.into(),
                steering: c.steering,
                no_steering: c.no_steering,
                no_workflows: c.no_workflows,
                no_refresh: c.no_refresh,
                stats: c.stats,
                audit: c.audit,
                top: c.top,
                audit_entries: c.audit_entries,
                e2e: c.e2e.into(),
                change_source: None,
            }
        }
    }

    /// The output of `affected`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum AffectedFormat {
        List,
        Json,
        /// The impact report as text (crate::viz).
        Summary,
        /// The selection document for a CI pipeline (crate::selection,
        /// `docs/ci.md`).
        Selection,
    }

    impl From<AffectedFormat> for turborepo_trace::commands::AffectedFormat {
        fn from(c: AffectedFormat) -> Self {
            match c {
                AffectedFormat::List => Self::List,
                AffectedFormat::Json => Self::Json,
                AffectedFormat::Summary => Self::Summary,
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
pub mod e2e {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// Options of `affected --tests playwright`.
    #[derive(Debug, Default, Args)]
    pub struct E2eArgs {
        /// Validation tooling, not a way to select (hidden): the oracle scripts
        /// compare the selection from source with two builds and
        /// recorded visits. The fingerprint of the build under test
        /// (`turbotrace next fingerprint`). Repeatable: one per app.
        #[usage(long, value_name = "FILE", hide = true)]
        pub next_fingerprint: Vec<PathBuf>,
        /// Validation tooling, not a way to select (hidden): a directory of
        /// recorded visits (written by the Playwright recorder).
        /// Repeatable.
        #[usage(long, value_name = "DIR", hide = true)]
        pub visits: Vec<PathBuf>,
        /// Validation tooling, not a way to select (hidden): the fingerprint of
        /// a build the recordings ran against. Repeatable. Fingerprint
        /// files inside a --visits directory are found without it.
        #[usage(long, value_name = "FILE", hide = true)]
        pub base_fingerprint: Vec<PathBuf>,
        /// With --tests playwright: only the specs of this Playwright config.
        /// Repeatable. Default: every `playwright.config.*` under the
        /// root.
        #[usage(long, value_name = "FILE")]
        pub playwright_config: Vec<PathBuf>,
        /// With --tests playwright: how far a spec's visit is followed from the
        /// URLs it names. `links` (the default) follows redirects and links of
        /// the visited pages, and a target that is not known means the
        /// spec may visit any route: no assumption, and on a large app
        /// every spec of an app the change reaches. `calls`
        /// assumes a spec stays on the pages its URLs name and follows only
        /// what those pages request; `redirects` also follows redirects
        /// to one known URL; `named` follows nothing. These three can
        /// skip a spec that reaches a changed page by a click or a
        /// redirect.
        #[usage(long, value_enum, value_name = "RULE")]
        pub follow: Option<e2e_static::Follow>,
    }

    impl From<E2eArgs> for turborepo_trace::commands::E2eArgs {
        fn from(c: E2eArgs) -> Self {
            Self {
                next_fingerprint: c.next_fingerprint,
                visits: c.visits,
                base_fingerprint: c.base_fingerprint,
                playwright_config: c.playwright_config,
                follow: c.follow.map(Into::into),
            }
        }
    }
}
pub mod e2e_static {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// How far a visit is followed from the routes a spec names.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
    pub enum Follow {
        /// Only the routes the spec's URLs match.
        Named,
        /// Plus the routes those routes request at run time.
        Calls,
        /// Plus where their code redirects to, when the target is one known
        /// URL.
        Redirects,
        /// Plus every redirect target and the pages they can navigate to.
        Links,
    }

    impl From<Follow> for turborepo_trace::commands::Follow {
        fn from(c: Follow) -> Self {
            match c {
                Follow::Named => Self::Named,
                Follow::Calls => Self::Calls,
                Follow::Redirects => Self::Redirects,
                Follow::Links => Self::Links,
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

    /// `graph export`.
    #[derive(Debug, Args)]
    pub struct ExportCmd {
        #[usage(flatten)]
        pub loc: GraphLoc,
        /// Output format.
        #[usage(long, value_enum, default = "json")]
        pub format: ExportFormat,
        /// Output file (stdout otherwise).
        #[usage(long, short = 'o', value_name = "FILE")]
        pub output: Option<PathBuf>,
        /// Use the stored graph as is, without refreshing it.
        #[usage(long)]
        pub no_refresh: bool,
    }

    impl From<ExportCmd> for turborepo_trace::commands::ExportCmd {
        fn from(c: ExportCmd) -> Self {
            Self {
                loc: c.loc.into(),
                format: c.format.into(),
                output: c.output,
                no_refresh: c.no_refresh,
            }
        }
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum ExportFormat {
        Json,
        Ndjson,
        Dot,
    }

    impl From<ExportFormat> for turborepo_trace::commands::ExportFormat {
        fn from(c: ExportFormat) -> Self {
            match c {
                ExportFormat::Json => Self::Json,
                ExportFormat::Ndjson => Self::Ndjson,
                ExportFormat::Dot => Self::Dot,
            }
        }
    }

    /// `deps` / `dependents`.
    #[derive(Debug, Args)]
    pub struct FilesCmd {
        /// Files (relative to the current directory).
        #[usage(required = true)]
        pub files: Vec<PathBuf>,
        /// Maximum depth.
        #[usage(long, value_name = "N")]
        pub depth: Option<u32>,
        #[usage(flatten)]
        pub opts: QueryOpts,
    }

    impl From<FilesCmd> for turborepo_trace::commands::FilesCmd {
        fn from(c: FilesCmd) -> Self {
            Self {
                files: c.files,
                depth: c.depth,
                opts: c.opts.into(),
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

    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum QueryFormat {
        List,
        Json,
        Dot,
        Tree,
    }

    impl From<QueryFormat> for turborepo_trace::commands::QueryFormat {
        fn from(c: QueryFormat) -> Self {
            match c {
                QueryFormat::List => Self::List,
                QueryFormat::Json => Self::Json,
                QueryFormat::Dot => Self::Dot,
                QueryFormat::Tree => Self::Tree,
            }
        }
    }

    /// Options shared by `query`, `deps` and `dependents`.
    #[derive(Debug, Args)]
    pub struct QueryOpts {
        #[usage(flatten)]
        pub loc: GraphLoc,
        /// Output format.
        #[usage(long, value_enum, default = "list")]
        pub format: QueryFormat,
        /// JSON output: include the edges between result nodes.
        #[usage(long)]
        pub edges: bool,
        /// Use the stored graph as is, without refreshing it.
        #[usage(long)]
        pub no_refresh: bool,
        /// Print timings to stderr.
        #[usage(long)]
        pub stats: bool,
    }

    impl From<QueryOpts> for turborepo_trace::commands::QueryOpts {
        fn from(c: QueryOpts) -> Self {
            Self {
                loc: c.loc.into(),
                format: c.format.into(),
                edges: c.edges,
                no_refresh: c.no_refresh,
                stats: c.stats,
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
pub mod uncertainty {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum Order {
        Freed,
        Weighted,
    }

    impl From<Order> for turborepo_trace::commands::Order {
        fn from(c: Order) -> Self {
            match c {
                Order::Freed => Self::Freed,
                Order::Weighted => Self::Weighted,
            }
        }
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
    pub enum ReportFormat {
        Text,
        Json,
    }

    impl From<ReportFormat> for turborepo_trace::commands::ReportFormat {
        fn from(c: ReportFormat) -> Self {
            match c {
                ReportFormat::Text => Self::Text,
                ReportFormat::Json => Self::Json,
            }
        }
    }

    /// `uncertainty`.
    #[derive(Debug, Args)]
    pub struct UncertaintyCmd {
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
        /// gotest, cargo-test, or `all`. Repeatable.
        #[usage(long, value_name = "FRAMEWORK")]
        pub tests: Vec<String>,
        /// Use build tasks as entries (see `affected --tasks`). Repeatable.
        #[usage(long, value_name = "GLOB")]
        pub tasks: Vec<String>,
        /// With --tasks: also the tasks run by hand by their nature (see
        /// `affected --all-tasks`).
        #[usage(long, requires = "tasks")]
        pub all_tasks: bool,
        /// Count `eval`, `new Function` and other code evaluation (`eval_like`)
        /// as open, as `affected --strict-eval` does.
        #[usage(long)]
        pub strict_eval: bool,
        /// Count every uncertainty as open, as `affected --strict` does.
        #[usage(long)]
        pub strict: bool,
        /// Ignore type-only edges.
        #[usage(long)]
        pub runtime_only: bool,
        /// Number of sites in the greedy order.
        #[usage(long, value_name = "N", default_value_t = 25, default = "25")]
        pub top: usize,
        /// How the order picks the next site. `freed`: the site that frees the
        /// most entries, those for which it is the last unresolved
        /// site. `weighted`: every entry on the floor splits one point
        /// between the unresolved sites it reaches, and the site with the
        /// most points goes first, so sites that only free entries together
        /// rank high too.
        #[usage(long, value_enum, default = "freed")]
        pub order: Order,
        /// What if: report the floor as if the open records of this file, or of
        /// one line of it, were resolved. `FILE[:LINE]` as printed in
        /// the report (relative to the repository root). Repeatable.
        #[usage(long, value_name = "FILE[:LINE]")]
        pub assume_resolved: Vec<String>,
        /// Read --assume-resolved values from a file (`-`: stdin), one per
        /// line. Empty lines and lines starting with `#` are skipped.
        #[usage(long, value_name = "FILE|-")]
        pub assume_resolved_from: Vec<String>,
        /// What if: report the floor as if every open record of this class were
        /// resolved: `KIND[/EFFECT[/ORIGIN]]`, where `*` matches
        /// anything (`computed_specifier/read`, `*/*/param`).
        /// Repeatable.
        #[usage(long, value_name = "CLASS")]
        pub assume_class: Vec<String>,
        /// What if: report the floor as if this node were not reached: a file
        /// (relative to the repository root), a task id
        /// (`task:.#build`) or a glob over them; `PREFIX**` matches
        /// every node that starts with the prefix (`task:**`: every task). No
        /// entry reaches it or anything through it. Predicts the effect
        /// of removing a hub. Repeatable.
        #[usage(long, value_name = "NODE")]
        pub assume_cut: Vec<String>,
        /// Debug: instead of the report, print the entries that reach one of
        /// the files listed in FILE (one path per line, relative to the
        /// repository root), along the edges the floor follows for each
        /// entry.
        #[usage(long, value_name = "FILE", hide = true)]
        pub reach_from: Option<String>,
        /// Count the entries that reach each load with an unknown target that
        /// the declared packages bound, and the floor without that
        /// bound (a walk per load).
        #[usage(long)]
        pub loads: bool,
        /// Instead of the report: the certain edges that break the rule "a file
        /// requires, imports or reads only files of the packages its
        /// package depends on" (the findings of the lint rule "no
        /// dependency outside the declared packages"; --top limits the
        /// lists).
        #[usage(long, hide = true)]
        pub undeclared_dependencies: bool,
        /// Debug, with --reach-from: per listed file, the number of entries
        /// that reach it.
        #[usage(long, hide = true)]
        pub reach_each: bool,
        /// Output format.
        #[usage(long, value_enum, default = "text")]
        pub format: ReportFormat,
        #[usage(flatten)]
        pub loc: GraphLoc,
        /// Use the stored graph as is, without refreshing it.
        #[usage(long)]
        pub no_refresh: bool,
        /// Print timings to stderr.
        #[usage(long)]
        pub stats: bool,
    }

    impl From<UncertaintyCmd> for turborepo_trace::commands::UncertaintyCmd {
        fn from(c: UncertaintyCmd) -> Self {
            Self {
                entries: c.entries,
                entries_opt: c.entries_opt,
                entries_from: c.entries_from,
                tests: c.tests,
                tasks: c.tasks,
                all_tasks: c.all_tasks,
                strict_eval: c.strict_eval,
                strict: c.strict,
                runtime_only: c.runtime_only,
                top: c.top,
                order: c.order.into(),
                assume_resolved: c.assume_resolved,
                assume_resolved_from: c.assume_resolved_from,
                assume_class: c.assume_class,
                assume_cut: c.assume_cut,
                reach_from: c.reach_from,
                loads: c.loads,
                undeclared_dependencies: c.undeclared_dependencies,
                reach_each: c.reach_each,
                format: c.format.into(),
                loc: c.loc.into(),
                no_refresh: c.no_refresh,
                stats: c.stats,
            }
        }
    }
}
pub mod viz {
    #![allow(unused_imports)]
    use std::path::PathBuf;

    use super::*;
    /// The flags of the report.
    #[derive(Debug, Args)]
    pub struct VizArgs {
        /// Also write the impact report: one self-contained HTML file (no
        /// network requests) that shows which changed files select
        /// which entries, through which hubs, and what
        /// was not selected. Without FILE it goes to `impact.html` in the graph
        /// directory. The path is printed on stderr. Write the value as
        /// `--html=FILE`.
        #[usage(long, value_name = "FILE", require_equals = true)]
        pub html: Option<Option<PathBuf>>,
        /// Open the HTML report in the browser (implies --html). Ignored
        /// without a terminal and in CI.
        #[usage(long)]
        pub open: bool,
    }

    impl From<VizArgs> for turborepo_trace::commands::VizArgs {
        fn from(c: VizArgs) -> Self {
            Self {
                html: c.html,
                open: c.open,
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
    /// Files the given files transitively depend on.
    Deps(graph::FilesCmd),
    /// Files that transitively depend on the given files.
    Dependents(graph::FilesCmd),
    /// The uncertainty floor.
    Uncertainty(uncertainty::UncertaintyCmd),
}

#[derive(Debug, Subcommand)]
enum GraphCmd {
    /// Build or refresh the graph.
    Build(graph::BuildCmd),
    /// Export the composed graph.
    Export(graph::ExportCmd),
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
        } => commands::graph_export(c.into(), &mut out),
        Command::Deps(c) => commands::files(c.into(), false, &mut out),
        Command::Dependents(c) => commands::files(c.into(), true, &mut out),
        Command::Uncertainty(c) => commands::uncertainty(c.into(), &mut out),
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
