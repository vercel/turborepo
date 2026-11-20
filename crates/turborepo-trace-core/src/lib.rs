//! Core of `turbo trace` : a multi-language,
//! file-level dependency tracer.
//!
//! This crate holds everything that is language-independent:
//! - the data model ([`model`]): [`FileId`], [`Span`], [`RefKind`],
//!   [`Specifier`], [`RawRef`], [`Edge`], [`Target`], [`Uncertainty`] and
//!   reasons;
//! - the compiled-in plugin traits [`Language`] and [`Resolver`] ([`lang`]);
//! - shared infrastructure: the concurrent [`PathInterner`], the read-once
//!   [`DirCache`] ([`fs`]), `--ignore` globs ([`glob`]), graph-mode observation
//!   recording ([`record`]) and memoized lookups that replay their observations
//!   ([`memo`]);
//! - shared language building blocks: the target [`Platform`] ([`platform`]),
//!   package boundaries ([`boundary`]) and the API-table format ([`api`]);
//! - the parallel scheduler and the [`Tracer`] / [`Trace`] library API, plus
//!   graph mode ([`Tracer::analyze`]: per-file analysis without traversal, with
//!   recorded observations);
//! - output formats ([`output`]).
//!
//! ```no_run
//! # fn demo(lang: impl turborepo_trace_core::Language) -> Result<(), turborepo_trace_core::TraceError> {
//! use turborepo_trace_core::{Tracer, output};
//!
//! let tracer = Tracer::builder().base("/srv/app").language(lang).build()?;
//! let trace = tracer.trace(&["/srv/app/main.stub"])?;
//! for file in trace.files() {
//!     println!("{}", file.display());
//! }
//! output::write_nft(&trace, &output::OutputOptions::default(), &mut std::io::stdout())
//!     .expect("write");
//! # Ok(()) }
//! ```
//!
//! Platforms: macOS and Linux only. Paths are raw bytes and need not be UTF-8.

// Every public item is documented (the workspace lint, kept per crate here).
#![warn(missing_docs)]

// Unix only at run time: on other platforms the crate compiles (`os` has the
// fallbacks) and `turbo trace` refuses to run.

mod analyze;
pub mod api;
pub mod boundary;
#[doc(hidden)]
pub mod debug;
pub mod env;
pub mod eval;
pub mod fs;
pub mod glob;
pub mod intern;
pub mod lang;
pub mod lexkit;
pub mod located;
pub mod manifest;
pub mod memo;
pub mod model;
pub mod names;
mod options;
pub mod os;
pub mod output;
pub mod platform;
pub mod record;
pub mod resolve_path;
mod sched;
pub mod timing;
mod trace;

pub use analyze::{AnalyzeInput, AnalyzedEdge, CachedExtraction, FILE_HEAD_BYTES, FileAnalysis};
pub use boundary::PackageBoundary;
pub use fs::DirCache;
pub use intern::PathInterner;
pub use lang::{
    EnvRoot, ExtraEdge, ExtractCx, LangId, Language, LanguageSet, PackageIdentity, ProjectCx,
    RefSink, Resolution, ResolveCx, Resolver,
};
pub use memo::Memo;
pub use model::*;
pub use options::{DEFAULT_MAX_GLOB, Externals, TraceOptions};
pub use platform::Platform;
pub use record::Observations;
pub use sched::RefGroup;
pub use trace::{
    Extraction, Trace, TraceError, TraceStats, Tracer, TracerBuilder, WhyStep, WhyVia,
};
