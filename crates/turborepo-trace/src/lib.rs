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
//! trace` calls. `commands` (hidden) has the full option set of each command
//! and `graph build`, for the crate's tests (`examples/harness.rs`).
//!
//! Everything here is synchronous and CPU-bound (rayon inside): async callers
//! use `spawn_blocking`.

// Every public item is documented.
#![warn(missing_docs)]

mod adapter;
mod affected;
mod aliasdiff;
mod api;
mod barrel;
mod configdiff;
mod confignames;
mod cycles;
mod dirchange;
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
mod why;
mod workflows;
mod xfile;

pub use adapter::Packages;
pub use api::{
    AffectedRequest, AffectedTest, AffectedTests, Changes, LintRequest, Reason, Repo,
    UncoveredTask, affected, explain, lint, prefetch_requests, supported, why,
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

/// The full option set of each command and `graph build`, for the crate's
/// tests (the test harness, `examples/harness.rs`, drives the library through
/// these). `turbo trace` uses the typed API at the crate root.
#[doc(hidden)]
pub mod commands {
    use std::io::Write;

    use crate::{Error, ExitCode};
    pub use crate::{
        affected::{AffectedCmd, AffectedFormat, OnUncertain},
        graph::{BuildCmd, BuildOpts, GraphExternals, GraphLoc},
        langs::LangArgs,
        lint::{LintCmd, LintFormat},
        selection::Mode,
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

    /// Runs `f` on the graph at `loc` (refreshed unless `no_refresh`) with its
    /// derived facts applied, as `affected` reads it, and the repository root:
    /// the tests observe the graph through it (the harness's `graph export`
    /// and `deps`).
    pub fn with_graph<R>(
        loc: &GraphLoc,
        no_refresh: bool,
        f: impl FnOnce(&turborepo_trace_graph::Patched<'_, turborepo_trace_store::Graph>, &str) -> R,
    ) -> Result<R, Error> {
        let opened = (|| -> anyhow::Result<R> {
            let g = crate::graph::open(loc, no_refresh, false)?;
            let root = g.root().to_owned();
            let d = crate::graph::derived(&g, false)?;
            Ok(f(&d, &root))
        })();
        turborepo_trace_store::wait_for_cache_writes();
        opened.map_err(Error::from)
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
