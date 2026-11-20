//! `why`: the chain of references that puts a file into an entry's trace.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use turborepo_trace_core::{
    Externals, PhaseFilter, Trace, Tracer, TracerBuilder, WhyVia, lexkit::LineIndex,
};

use crate::{ExitCode, langs};

/// `why`.
#[derive(Debug, Default)]
pub struct WhyCmd {
    /// The entry file.
    pub entry: PathBuf,
    /// The file whose inclusion to explain.
    pub file: PathBuf,
    /// Trace options.
    pub opts: TraceOpts,
}

/// Options shared by `trace` and `why`.
#[derive(Debug)]
pub struct TraceOpts {
    /// Output paths are relative to this directory [default: current
    /// directory].
    pub base: Option<PathBuf>,
    /// Working directory assumed for cwd-relative reads [default: current
    /// directory].
    pub cwd: Option<PathBuf>,
    /// What to do with third-party dependencies.
    pub externals: ExternalsArg,
    /// Resolution conditions, comma separated (JS `exports`).
    pub conditions: Vec<String>,
    /// Also resolve every other condition a package's `exports` defines and
    /// include all results (over-approximation for safety).
    pub all_conditions: bool,
    /// Which phases to include.
    pub phase: PhaseArg,
    /// Build target, shared by all languages: <os>/<arch>[/musl] in Node or Go
    /// names (linux/x64, linux/amd64, linux/arm64/musl, darwin/arm64) or a
    /// Rust target triple (x86_64-unknown-linux-gnu). JS uses it for native
    /// binaries and package special cases [default: host].
    pub target: Option<String>,
    /// Ignore files matching this glob (relative to the base). Repeatable.
    pub ignore: Vec<String>,
    /// Maximum files a glob or directory expands to.
    pub max_glob: usize,
    /// Include type-only references.
    pub include_types: bool,
    /// Emit file hashes (nft `fileHashes`).
    pub hashes: bool,
    /// Worker threads (0 = one per core).
    pub threads: usize,
    /// Use the on-disk extraction cache (.turbotrace/cache).
    pub cache: bool,
    /// Include files for every target, not only --target, as conditional edges:
    /// Go files excluded by build constraints, Rust items and dependencies
    /// behind target `cfg`.
    pub all_targets: bool,
    /// Language-specific options.
    pub lang: langs::LangArgs,
}

impl Default for TraceOpts {
    fn default() -> Self {
        TraceOpts {
            base: Default::default(),
            cwd: Default::default(),
            externals: ExternalsArg::Follow,
            conditions: Default::default(),
            all_conditions: Default::default(),
            phase: PhaseArg::All,
            target: Default::default(),
            ignore: Default::default(),
            max_glob: turborepo_trace_core::DEFAULT_MAX_GLOB,
            include_types: Default::default(),
            hashes: Default::default(),
            threads: 0,
            cache: Default::default(),
            all_targets: Default::default(),
            lang: Default::default(),
        }
    }
}

/// What a trace does with third-party dependencies.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExternalsArg {
    /// Traced into.
    Follow,
    /// Not traced.
    Stop,
    /// Not traced, and listed.
    List,
}

/// Which phases a trace includes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PhaseArg {
    /// Runtime references.
    Runtime,
    /// Build-time references.
    Build,
    /// Both.
    All,
}

impl TraceOpts {
    fn builder(&self) -> Result<TracerBuilder> {
        if self.cache {
            bail!("not yet implemented: --cache (on-disk extraction cache)");
        }
        let mut b = Tracer::builder()
            .externals(match self.externals {
                ExternalsArg::Follow => Externals::Follow,
                ExternalsArg::Stop => Externals::Stop,
                ExternalsArg::List => Externals::List,
            })
            .phase(match self.phase {
                PhaseArg::Runtime => PhaseFilter::Runtime,
                PhaseArg::Build => PhaseFilter::Build,
                PhaseArg::All => PhaseFilter::All,
            })
            .conditions(self.conditions.iter().cloned())
            .all_conditions(self.all_conditions)
            .max_glob(self.max_glob)
            .include_types(self.include_types)
            .hashes(self.hashes)
            .all_targets(self.all_targets)
            .threads(self.threads);
        if let Some(base) = &self.base {
            b = b.base(base);
        }
        if let Some(cwd) = &self.cwd {
            b = b.cwd(cwd);
        }
        if let Some(t) = &self.target {
            b = b.target(t);
        }
        for g in &self.ignore {
            b = b.ignore(g);
        }
        Ok(with_languages(b, &self.lang))
    }
}

fn with_languages(mut b: TracerBuilder, args: &langs::LangArgs) -> TracerBuilder {
    for l in langs::all(args) {
        b = b.language_arc(l);
    }
    b
}

fn display(trace: &Trace, p: &Path) -> String {
    match trace.relative(p) {
        Some(r) => r.display().to_string(),
        None => p.display().to_string(),
    }
}

pub(crate) fn cmd_why(cmd: WhyCmd, out: &mut dyn Write) -> Result<ExitCode> {
    let tracer = cmd.opts.builder()?.build()?;
    let trace = tracer.trace(std::slice::from_ref(&cmd.entry))?;
    let cwd = std::env::current_dir().context("cannot read current directory")?;
    let entry = turborepo_trace_core::fs::normalize(&cwd.join(&cmd.entry));
    let file = turborepo_trace_core::fs::normalize(&cwd.join(&cmd.file));
    let Some(chain) = trace.why(&entry, &file) else {
        bail!(
            "{} is not in the trace of {}",
            cmd.file.display(),
            cmd.entry.display()
        );
    };
    let mut lines: std::collections::HashMap<&Path, Option<LineIndex>> = Default::default();
    writeln!(out, "{}", display(&trace, &entry))?;
    for step in &chain {
        let how = match step.via {
            WhyVia::Symlink => "symlink target".to_string(),
            // File-level edges (the file's package scope, special cases keyed on the file)
            // have no source location.
            WhyVia::Edge(e) if e.span.is_empty() && e.span.start == 0 => {
                format!(
                    "{} \"{}\" of {}",
                    e.kind,
                    e.spec,
                    display(&trace, step.from)
                )
            }
            WhyVia::Edge(e) => {
                let idx = lines
                    .entry(step.from)
                    .or_insert_with(|| std::fs::read(step.from).ok().map(|s| LineIndex::new(&s)));
                let loc = idx
                    .as_ref()
                    .map(|i| {
                        let lc = i.line_col(e.span.start);
                        format!(":{}:{}", lc.line, lc.col)
                    })
                    .unwrap_or_default();
                format!(
                    "{} \"{}\" at {}{}",
                    e.kind,
                    e.spec,
                    display(&trace, step.from),
                    loc
                )
            }
        };
        writeln!(out, "  -> {}  ({how})", display(&trace, step.to))?;
    }
    if let Some(reason) = trace.file_id(&file).and_then(|id| trace.reason(id)) {
        let kinds: Vec<_> = reason.kinds.names().collect();
        let parents: Vec<String> = reason
            .parents
            .iter()
            .filter_map(|&p| trace.path(p))
            .map(|p| display(&trace, p))
            .collect();
        writeln!(out, "reasons: {}", kinds.join(", "))?;
        if !parents.is_empty() {
            writeln!(out, "parents: {}", parents.join(", "))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
