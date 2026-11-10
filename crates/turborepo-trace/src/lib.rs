//! The command logic of `turbo trace`: which tests a change affects, why a
//! test's trace contains a file, and the lints of the code the selection cannot
//! see through.
//!
//! The engine (the `turborepo-trace-*` crates) builds a file-level dependency
//! graph of the repository, stored per package and refreshed against the tree,
//! and evaluates a change against it. This crate holds what the commands do
//! with it. It is the only trace crate that depends on turbo's crates: the
//! workspace members come from turbo's
//! [`PackageGraph`](turborepo_repository::package_graph::PackageGraph) and the
//! changed files from turbo's [`SCM`](turborepo_scm::SCM) ([`Repo`]).
//!
//! The typed API ([`affected`], [`explain`], [`why`], [`lint`]) is what `turbo
//! trace` calls. `commands` (hidden) has the full option set of each command,
//! as the standalone command line has them, and the read-only inspection
//! commands its tests observe the graph through (`graph export`, `deps`,
//! `uncertainty`).
//!
//! Everything here is synchronous and CPU-bound (rayon inside): async callers
//! use `spawn_blocking`.

// Every public item is documented.
#![warn(missing_docs)]

mod adapter;
mod affected;
mod aliasdiff;
mod api;
mod attrdump;
mod audit;
mod barrel;
mod configdiff;
mod confignames;
mod cycles;
mod dirchange;
mod e2e;
mod e2e_cache;
mod e2e_static;
mod erase;
mod error;
mod exportsdiff;
mod gonames;
mod gosumdiff;
mod graph;
mod jsblank;
mod langs;
mod lint;
mod loadcode;
mod loads;
mod lockdiff;
mod names;
mod nextconfigdiff;
mod selection;
mod services;
mod steering;
mod task_entries;
pub mod transport;
mod tsconfigdiff;
mod tsgovern;
mod uncertainty;
mod viz;
mod why;
mod workflows;
mod xfile;

pub use adapter::Packages;
pub use api::{
    AffectedRequest, AffectedTest, AffectedTests, Changes, LintRequest, Reason, Repo,
    UncoveredTask, affected, explain, lint, supported, why,
};
pub use error::Error;

/// The exit code of a command: `0` success; `affected --exit-code` and `lint`
/// give `1` and `2` their meanings.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ExitCode(pub u8);

impl ExitCode {
    /// `0`.
    pub const SUCCESS: ExitCode = ExitCode(0);
    /// `1`.
    pub const FAILURE: ExitCode = ExitCode(1);
}

impl From<u8> for ExitCode {
    fn from(code: u8) -> Self {
        ExitCode(code)
    }
}

/// The full option set of each command, as the standalone command line has it
/// (the test harness, `examples/harness.rs`, and the standalone tooling drive
/// the engine through these). `turbo trace` uses the typed API at the crate
/// root.
#[doc(hidden)]
pub mod commands {
    use std::io::Write;

    use crate::{Error, ExitCode};
    pub use crate::{
        affected::{AffectedCmd, AffectedFormat, OnUncertain},
        e2e::E2eArgs,
        e2e_static::Follow,
        graph::{
            BuildCmd, BuildOpts, ExportCmd, ExportFormat, FilesCmd, GraphExternals, GraphLoc,
            QueryFormat, QueryOpts,
        },
        langs::LangArgs,
        lint::{LintCmd, LintFormat},
        selection::Mode,
        uncertainty::{Order, ReportFormat, UncertaintyCmd},
        viz::VizArgs,
        why::{ExternalsArg, PhaseArg, TraceOpts, WhyCmd},
    };

    /// `affected`.
    pub fn affected(cmd: AffectedCmd, out: &mut dyn Write) -> Result<ExitCode, Error> {
        finish(crate::affected::cmd_affected(cmd, out))
    }

    /// `why`.
    pub fn why(cmd: WhyCmd, out: &mut dyn Write) -> Result<ExitCode, Error> {
        finish(crate::why::cmd_why(cmd, out))
    }

    /// `lint`.
    pub fn lint(cmd: LintCmd, out: &mut dyn Write) -> Result<ExitCode, Error> {
        finish(crate::lint::cmd_lint(cmd, out))
    }

    /// `graph build`.
    pub fn graph_build(cmd: BuildCmd, out: &mut dyn Write) -> Result<ExitCode, Error> {
        finish(crate::graph::cmd_build(cmd, out))
    }

    /// `graph export`.
    pub fn graph_export(cmd: ExportCmd, out: &mut dyn Write) -> Result<ExitCode, Error> {
        finish(crate::graph::cmd_export(cmd, out))
    }

    /// `deps` (`reverse = false`) and `dependents` (`reverse = true`).
    pub fn files(cmd: FilesCmd, reverse: bool, out: &mut dyn Write) -> Result<ExitCode, Error> {
        finish(crate::graph::cmd_files(cmd, reverse, out))
    }

    /// `uncertainty`: the uncertainty floor report.
    pub fn uncertainty(cmd: UncertaintyCmd, out: &mut dyn Write) -> Result<ExitCode, Error> {
        finish(crate::uncertainty::cmd_uncertainty(cmd, out))
    }

    /// Cache writes a graph build left in the background finish before a
    /// command returns (the process may exit right after), and
    /// `TURBO_TRACE_DEBUG_TIMING` reports its spans.
    pub(crate) fn finish(r: anyhow::Result<ExitCode>) -> Result<ExitCode, Error> {
        turborepo_trace_core::timing::mark("command done (results dropped)");
        turborepo_trace_store::wait_for_cache_writes();
        turborepo_trace_core::timing::mark("background cache writes done");
        turborepo_trace_core::timing::report_spans();
        r.map_err(Error::from)
    }
}
