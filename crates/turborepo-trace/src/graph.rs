//! The whole-repo graph: `graph build`, and opening it refreshed
//! for the commands.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use turborepo_trace_graph::{
    Engine, GraphView,
    output::{Format as QFormat, OutputOptions as QOutput},
};
use turborepo_trace_store::{Externals, Graph, GraphOptions, Manifest};

use crate::{ExitCode, langs};

/// Where the graph lives.
#[derive(Debug, Clone, Default)]
pub struct GraphLoc {
    /// Repository root [default: the root recorded in the graph, else the
    /// current directory].
    pub root: Option<PathBuf>,
    /// Graph directory [default: <root>/.turbo/trace].
    pub graph: Option<PathBuf>,
    /// Which directories are the JavaScript workspace members (default: what
    /// the repository declares; `turbo trace`: Turborepo's package graph).
    pub members: turborepo_trace_store::workspace::Members,
}

/// Build options.
#[derive(Debug, Clone)]
pub struct BuildOpts {
    /// Third-party packages: `stop` keeps one opaque node per name@version
    /// (default); `follow` traces into them.
    pub externals: GraphExternals,
    /// Resolution conditions, comma separated (JS `exports`).
    pub conditions: Vec<String>,
    /// Also resolve every other `exports` condition.
    pub all_conditions: bool,
    /// Build target (linux/x64, darwin/arm64, ...) [default: host].
    pub target: Option<String>,
    /// Maximum files a glob or directory expands to.
    pub max_glob: usize,
    /// Leave out type-only references (included by default: typecheck reads
    /// them).
    pub no_types: bool,
    /// Worker threads (0 = one per core).
    pub threads: usize,
    /// Include files for every target as conditional edges (Go, Rust).
    pub all_targets: bool,
    /// Language-specific options.
    pub lang: langs::LangArgs,
}

impl Default for BuildOpts {
    fn default() -> Self {
        BuildOpts {
            externals: GraphExternals::Stop,
            conditions: Default::default(),
            all_conditions: Default::default(),
            target: Default::default(),
            max_glob: turborepo_trace_core::DEFAULT_MAX_GLOB,
            no_types: Default::default(),
            threads: 0,
            all_targets: Default::default(),
            lang: Default::default(),
        }
    }
}

/// What the graph does with third-party packages.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GraphExternals {
    /// One opaque node per name@version.
    Stop,
    /// Traced into.
    Follow,
}

/// `graph build`.
#[derive(Debug, Default)]
pub struct BuildCmd {
    /// Where the graph lives.
    pub loc: GraphLoc,
    /// Build options.
    pub opts: BuildOpts,
    /// Print the report as JSON.
    pub json: bool,
}

/// The format of `graph export`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// One JSON document.
    Json,
    /// One JSON record per line.
    Ndjson,
    /// Graphviz.
    Dot,
}

/// `graph export`.
#[derive(Debug)]
pub struct ExportCmd {
    /// Where the graph lives.
    pub loc: GraphLoc,
    /// Output format.
    pub format: ExportFormat,
    /// Output file (stdout otherwise).
    pub output: Option<PathBuf>,
    /// Use the stored graph as is, without refreshing it.
    pub no_refresh: bool,
}

impl Default for ExportCmd {
    fn default() -> Self {
        ExportCmd {
            loc: Default::default(),
            format: ExportFormat::Json,
            output: Default::default(),
            no_refresh: Default::default(),
        }
    }
}

/// The format of a query's result.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QueryFormat {
    /// One path per line.
    List,
    /// JSON.
    Json,
    /// Graphviz.
    Dot,
    /// An indented tree.
    Tree,
}

/// Options shared by `query`, `deps` and `dependents`.
#[derive(Debug)]
pub struct QueryOpts {
    /// Where the graph lives.
    pub loc: GraphLoc,
    /// Output format.
    pub format: QueryFormat,
    /// JSON output: include the edges between result nodes.
    pub edges: bool,
    /// Use the stored graph as is, without refreshing it.
    pub no_refresh: bool,
    /// Print timings to stderr.
    pub stats: bool,
}

impl Default for QueryOpts {
    fn default() -> Self {
        QueryOpts {
            loc: Default::default(),
            format: QueryFormat::List,
            edges: Default::default(),
            no_refresh: Default::default(),
            stats: Default::default(),
        }
    }
}

/// `deps` / `dependents`.
#[derive(Debug, Default)]
pub struct FilesCmd {
    /// Files (relative to the current directory).
    pub files: Vec<PathBuf>,
    /// Maximum depth.
    pub depth: Option<u32>,
    /// Query options.
    pub opts: QueryOpts,
}

impl GraphLoc {
    /// `(root, graph dir)`, both absolute.
    pub(crate) fn resolve(&self) -> Result<(PathBuf, PathBuf)> {
        let cwd = std::env::current_dir().context("cannot read current directory")?;
        let abs = |p: &Path| turborepo_trace_core::fs::normalize(&cwd.join(p));
        match (&self.root, &self.graph) {
            (Some(r), Some(g)) => Ok((abs(r), abs(g))),
            (Some(r), None) => Ok((
                abs(r),
                abs(r).join(turborepo_trace_store::DEFAULT_GRAPH_PATH),
            )),
            (None, Some(g)) => {
                let g = abs(g);
                let root = Manifest::read(&g)
                    .map(|m| PathBuf::from(m.root))
                    .unwrap_or_else(|| cwd.clone());
                Ok((root, g))
            }
            (None, None) => Ok((
                cwd.clone(),
                cwd.join(turborepo_trace_store::DEFAULT_GRAPH_PATH),
            )),
        }
    }
}

impl BuildOpts {
    fn graph_options(&self, root: &Path, out: &Path) -> GraphOptions {
        GraphOptions {
            root: root.to_path_buf(),
            out: Some(out.to_path_buf()),
            externals: match self.externals {
                GraphExternals::Stop => Externals::Opaque,
                GraphExternals::Follow => Externals::Follow,
            },
            conditions: self.conditions.clone(),
            all_conditions: self.all_conditions,
            target: self.target.clone(),
            max_glob: self.max_glob,
            threads: self.threads,
            include_types: !self.no_types,
            all_targets: self.all_targets,
            members: Default::default(),
        }
    }
}

/// The options a stored graph was built with, for refreshes.
pub(crate) fn stored_options(root: &Path, out: &Path) -> GraphOptions {
    match Manifest::read(out) {
        Some(m) => graph_options(root, out, &m.options),
        None => GraphOptions {
            root: root.to_path_buf(),
            out: Some(out.to_path_buf()),
            ..Default::default()
        },
    }
}

/// The build options recorded in a manifest.
pub(crate) fn graph_options(
    root: &Path,
    out: &Path,
    m: &turborepo_trace_store::manifest::ManifestOptions,
) -> GraphOptions {
    GraphOptions {
        root: root.to_path_buf(),
        out: Some(out.to_path_buf()),
        externals: if m.externals == "follow" {
            Externals::Follow
        } else {
            Externals::Opaque
        },
        conditions: m.conditions.clone(),
        all_conditions: m.all_conditions,
        target: m.target.clone(),
        all_targets: m.all_targets,
        max_glob: m.max_glob,
        threads: 0,
        include_types: m.include_types,
        members: Default::default(),
    }
}

fn print_report(r: &turborepo_trace_store::BuildReport, out: &Path) {
    let t = r.timings_ms;
    eprintln!(
        "graph: {} files, {} first-party + {} third-party packages; {} shard(s) traced, {} \
         reused, {} written; {} file(s) analyzed ({} extraction(s) reused), {} kept from the \
         previous shards; store {} KiB",
        r.files,
        r.first_party,
        r.third_party,
        r.traced,
        r.reused,
        r.written,
        r.analyzed_files,
        r.reused_extractions,
        r.kept_files,
        turborepo_trace_store::store_size(out) / 1024
    );
    eprintln!(
        "graph: walk {} ms{}, validate {} ms ({} reused, {} checked), trace {} ms, assemble {} \
         ms, third-party {} ms, total {} ms",
        t[0],
        if r.walk_cached { " (cached)" } else { "" },
        t[1],
        r.tree_reused[0],
        r.tree_reused[1],
        t[2],
        t[3],
        t[4],
        t[5]
    );
}

/// Builds or refreshes the graph, unless `no_refresh` (then it must exist), and
/// opens it.
pub(crate) fn open(loc: &GraphLoc, no_refresh: bool, stats: bool) -> Result<Graph> {
    Ok(open_report(loc, no_refresh, stats)?.0)
}

/// `g` with the facts derived from the composed graph applied:
/// working-directory-relative references resolved per test
/// (`turborepo_trace_store::Graph::cwd_patch`) and the repository's
/// hints (`turborepo_trace_store::Hints`: `<root>/turbotrace.toml`, or the file
/// named by `TURBO_TRACE_DEBUG_HINTS`, for example when replaying commits that
/// predate the hints file). `TURBO_TRACE_DEBUG_HINTS=` (empty) turns the hints
/// off, and `TURBO_TRACE_DEBUG_CWD=0` the whole derived layer, hints included
/// (A/B measurements, escape hatches). Values that other files supply are bound
/// to their suppliers (`crate::xfile`; `TURBO_TRACE_DEBUG_BIND=0` turns that
/// off).
pub(crate) fn derived(g: &Graph, stats: bool) -> Result<turborepo_trace_graph::Patched<'_, Graph>> {
    let t = Instant::now();
    let hints_path = match turborepo_trace_core::debug::var_os("HINTS") {
        Some(p) if p.is_empty() => None,
        Some(p) => Some(PathBuf::from(p)),
        None if !turborepo_trace_core::project::reads_turbotrace_toml() => None,
        None => Some(Path::new(g.root()).join(turborepo_trace_store::HINTS_FILE)),
    };
    let hints = match &hints_path {
        Some(p) => turborepo_trace_store::Hints::load(p).map_err(|e| anyhow::anyhow!("{e}"))?,
        None => None,
    };
    let cwd = turborepo_trace_core::debug::var_os("CWD").is_none_or(|v| v != "0");
    let (patch, warnings) = match (&hints, cwd) {
        (Some(h), true) if !h.is_empty() => g.hints_patch(h),
        (_, true) => (g.cwd_patch(), Vec::new()),
        (_, false) => (turborepo_trace_graph::Patch::new(), Vec::new()),
    };
    let (mut patch, warnings) = (patch, warnings);
    // Paths that travel through environment variables
    // (`turborepo_trace_store::compose::env`).
    let env = turborepo_trace_core::debug::var_os("ENV").is_none_or(|v| v != "0");
    let env_stats = env.then(|| {
        let decls = hints.as_ref().map_or(&[][..], |h| h.env.as_slice());
        // A cross-file evaluator for assignments a file cannot evaluate alone plugs in
        // here (`Graph::env_patch_with`); without one those assignments stay unknown.
        let (p, s, cached) = g.env_patch_cached(decls, &|_, _| None);
        patch.merge(p);
        (s, cached)
    });
    // Manual mocks that files other than tests register: an edge from the module to
    // the mock (`turborepo_trace_store::compose::mocks`). Not a switch: without
    // it nothing depends on those mocks.
    let (p, mock_edges, mock_cached) = g.mock_patch_cached();
    patch.merge(p);
    if stats && mock_edges > 0 {
        eprintln!(
            "mocks: {mock_edges} edge(s) from modules to the manual mocks that files other than \
             tests register{}",
            if mock_cached { " (cached)" } else { "" }
        );
    }
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    if let (true, Some((s, cached))) = (stats, &env_stats) {
        eprintln!(
            "env: {} use(s) of {} variable(s) with {} assignment(s), {} helper(s); {} resolved, \
             {} kept as environment reads, {} edge(s) added{}",
            s.uses,
            s.variables,
            s.assignments,
            s.helpers,
            s.resolved,
            s.kept,
            s.edges,
            if *cached { " (cached)" } else { "" }
        );
    }
    // Values that other files supply (cross-file binding): dependencies of the
    // suppliers.
    let patch = {
        let mut patch = patch;
        if cwd && let Some(bound) = crate::xfile::patch(g, &patch, stats) {
            patch.merge(bound);
        }
        patch
    };
    if stats {
        eprintln!(
            "derived: {} hint(s); {} edge(s) added, {} uncertainty record(s) resolved per test, \
             {} dropped by hints, {:.1} ms",
            hints.as_ref().map_or(0, |h| h.hints.len()),
            patch.edge_count(),
            patch.resolved_count(),
            patch.dropped_count(),
            t.elapsed().as_secs_f64() * 1e3
        );
    }
    Ok(turborepo_trace_graph::Patched::new(g, patch))
}

/// [`open`], also returning the refresh report (`None` with `no_refresh`).
pub(crate) fn open_report(
    loc: &GraphLoc,
    no_refresh: bool,
    stats: bool,
) -> Result<(Graph, Option<turborepo_trace_store::BuildReport>)> {
    let (root, out) = loc.resolve()?;
    let mut report = None;
    if !no_refresh {
        let t = Instant::now();
        let opts = GraphOptions {
            members: loc.members.clone(),
            ..stored_options(&root, &out)
        };
        // The language flags of the last `graph build` (their digest is part of the
        // options digest: different flags would rebuild every shard).
        let r = turborepo_trace_store::build(&opts, &langs::all(&langs::LangArgs::load(&out)))?;
        turborepo_trace_core::timing::mark("graph: build returned (its locals dropped)");
        if stats {
            print_report(&r, &out);
            eprintln!("refresh: {} ms", t.elapsed().as_millis());
        }
        report = Some(r);
    }
    let t = Instant::now();
    let g = Graph::open(&out)?;
    if stats {
        let s = g.load_stats();
        eprintln!(
            "load: {} ms ({} shards, {} KiB; read {} ms, nodes {} ms, link {} ms (tasks {} ms, \
             tables {} ms)); {} nodes, {} edges{}",
            t.elapsed().as_millis(),
            s.shards,
            s.bytes / 1024,
            s.read_us / 1000,
            s.nodes_us / 1000,
            s.link_us / 1000,
            s.tasks_us / 1000,
            s.tables_us / 1000,
            g.node_count(),
            g.edge_count(),
            if s.reused {
                "; composed parts reused (the change kept every reference)"
            } else {
                ""
            }
        );
    }
    Ok((g, report))
}

/// `graph build`.
pub(crate) fn cmd_build(b: BuildCmd, out: &mut dyn Write) -> Result<ExitCode> {
    let (root, dir) = b.loc.resolve()?;
    let r = turborepo_trace_store::build(
        &GraphOptions {
            members: b.loc.members.clone(),
            ..b.opts.graph_options(&root, &dir)
        },
        &langs::all(&b.opts.lang),
    )?;
    turborepo_trace_core::timing::mark("graph: build returned (its locals dropped)");
    b.opts
        .lang
        .save(&dir)
        .with_context(|| format!("writing {}", dir.join(langs::LangArgs::FILE).display()))?;
    if b.json {
        let v = serde_json::json!({
            "files": r.files,
            "first_party": r.first_party,
            "third_party": r.third_party,
            "traced": r.traced,
            "reused": r.reused,
            "written": r.written,
            // Tree parts (a shard's observed tree state) reused and re-checked, and
            // what a refresh wrote besides shards: all zero and false when nothing
            // changed.
            "tree_reused": r.tree_reused[0],
            "tree_checked": r.tree_reused[1],
            "walk_cached": r.walk_cached,
            "manifest_written": r.manifest_written,
            "obs_cache_written": r.obs_written,
            "analyzed_files": r.analyzed_files,
            "reused_extractions": r.reused_extractions,
            // Files of the traced shards that were not analyzed: their records
            // came from the previous shard (nothing they observed changed).
            "kept_files": r.kept_files,
            "timings_ms": {
                "walk": r.timings_ms[0], "validate": r.timings_ms[1],
                "trace": r.timings_ms[2], "assemble": r.timings_ms[3],
                "third_party": r.timings_ms[4], "total": r.timings_ms[5],
            },
            "store_bytes": turborepo_trace_store::store_size(&dir),
        });
        serde_json::to_writer_pretty(&mut *out, &v)?;
        writeln!(out)?;
    } else {
        print_report(&r, &dir);
    }
    Ok(ExitCode::SUCCESS)
}

/// `graph export`.
pub(crate) fn cmd_export(e: ExportCmd, out: &mut dyn Write) -> Result<ExitCode> {
    let g = open(&e.loc, e.no_refresh, false)?;
    let root = g.root().to_owned();
    let g = derived(&g, false)?;
    let mut file;
    let w: &mut dyn Write = match (&e.output, e.format) {
        (Some(p), _) => {
            file = std::fs::File::create(p)
                .with_context(|| format!("cannot create {}", p.display()))?;
            &mut file
        }
        (None, _) => &mut *out,
    };
    match e.format {
        ExportFormat::Json => turborepo_trace_graph::json::write_json(&g, Some(&root), false, w)?,
        ExportFormat::Ndjson => turborepo_trace_graph::json::write_ndjson(&g, Some(&root), w)?,
        ExportFormat::Dot => {
            let engine = Engine::new(&g);
            let mut all = engine.empty();
            for i in 0..g.node_count() {
                all.insert(turborepo_trace_graph::NodeId(i));
            }
            turborepo_trace_graph::output::write_dot(&engine, &all, w)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_query(expr: &str, opts: &QueryOpts, out: &mut dyn Write) -> Result<ExitCode> {
    let g = open(&opts.loc, opts.no_refresh, opts.stats)?;
    if opts.stats && expr.contains("affected") {
        let t = Instant::now();
        g.warm();
        eprintln!("probe/membership tables: {} ms", t.elapsed().as_millis());
    }
    let g = derived(&g, opts.stats)?;
    // The graph reflects the working tree (refreshed), so `affected(deleted(..),
    // ..)` must also match probes of deleted paths
    // (`AffectedOptions::after_change`).
    let engine = Engine::with_options(
        &g,
        turborepo_trace_graph::EngineOptions {
            affected: turborepo_trace_graph::AffectedOptions::default().after_change(true),
            ..Default::default()
        },
    );
    let t = Instant::now();
    let result = engine.query(expr).map_err(|e| anyhow::anyhow!("{e}"))?;
    if opts.stats {
        eprintln!(
            "query: {:.3} ms, {} node(s)",
            t.elapsed().as_secs_f64() * 1e3,
            result.nodes.len()
        );
    }
    let format = match opts.format {
        QueryFormat::List => QFormat::List,
        QueryFormat::Json => QFormat::Json,
        QueryFormat::Dot => QFormat::Dot,
        QueryFormat::Tree => QFormat::Tree,
    };
    turborepo_trace_graph::output::write(
        &engine,
        &result,
        &QOutput {
            format,
            edges: opts.edges,
        },
        out,
    )?;
    Ok(ExitCode::SUCCESS)
}

/// `deps` (`reverse = false`) and `dependents` (`reverse = true`): the closure
/// of the given files without the files themselves.
pub(crate) fn cmd_files(cmd: FilesCmd, reverse: bool, out: &mut dyn Write) -> Result<ExitCode> {
    let (root, _) = cmd.opts.loc.resolve()?;
    let cwd = std::env::current_dir().context("cannot read current directory")?;
    let mut lits = Vec::new();
    for f in &cmd.files {
        let p = turborepo_trace_core::fs::normalize(&cwd.join(f));
        let rel = p.strip_prefix(&root).with_context(|| {
            format!(
                "{} is outside the graph root {}",
                f.display(),
                root.display()
            )
        })?;
        lits.push(format!("{:?}", rel.to_string_lossy()));
    }
    let set = lits.join(" + ");
    let f = if reverse { "rdeps" } else { "deps" };
    let depth = cmd.depth.map(|d| format!(", {d}")).unwrap_or_default();
    let expr = format!("{f}({set}{depth}) - ({set})");
    run_query(&expr, &cmd.opts, out)
}
