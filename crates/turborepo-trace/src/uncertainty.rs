//! The uncertainty floor report: the open uncertainty records and the tests
//! that each one selects on every change.
//!
//! `affected` selects every entry that reaches an open uncertainty, on every
//! change. This command reports that floor for a set of entries without a
//! change: its size, the classes of record behind it, and the sites to resolve
//! first (`turborepo_trace_graph::floor`).

use std::{io::Write, time::Instant};

use anyhow::{Context, Result, bail};
use turborepo_trace_graph::{
    Engine, FloorOrder, GraphView, NodeSet, Policy,
    floor::{AssumedSite, ClassPattern, FloorAssumptions},
};
use turborepo_trace_store::Graph;

use crate::{
    ExitCode,
    affected::{collect_entries, collect_tasks, selection_options},
    graph::{GraphLoc, open},
};

/// The format of `uncertainty`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReportFormat {
    /// Text.
    Text,
    /// JSON.
    Json,
}

/// How the sites to resolve first are ranked.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Order {
    /// The site that frees the most entries (those for which it is the last
    /// unresolved site).
    Freed,
    /// Every entry on the floor splits one point between the unresolved sites
    /// it reaches.
    Weighted,
}

/// `uncertainty`.
#[derive(Debug)]
pub struct UncertaintyCmd {
    /// Entry files or globs (relative to the current directory; globs match
    /// graph files).
    pub entries: Vec<String>,
    /// More entry files or globs. Repeatable.
    pub entries_opt: Vec<String>,
    /// Read entries from a file (`-`: stdin), one per line, relative to the
    /// graph root.
    pub entries_from: Vec<String>,
    /// Use the test files of a framework as entries: jest, vitest, pytest,
    /// gotest, cargo-test, or `all`. Repeatable.
    pub tests: Vec<String>,
    /// Use build tasks as entries (see `affected --tasks`). Repeatable.
    pub tasks: Vec<String>,
    /// With --tasks: also the tasks run by hand by their nature (see `affected
    /// --all-tasks`).
    pub all_tasks: bool,
    /// Count `eval`, `new Function` and other code evaluation (`eval_like`) as
    /// open, as `affected --strict-eval` does.
    pub strict_eval: bool,
    /// Count every uncertainty as open, as `affected --strict` does.
    pub strict: bool,
    /// Ignore type-only edges.
    pub runtime_only: bool,
    /// Number of sites in the greedy order.
    pub top: usize,
    /// How the order picks the next site. `freed`: the site that frees the most
    /// entries, those for which it is the last unresolved site. `weighted`:
    /// every entry on the floor splits one point between the unresolved
    /// sites it reaches, and the site with the most points goes first, so
    /// sites that only free entries together rank high too.
    pub order: Order,
    /// What if: report the floor as if the open records of this file, or of one
    /// line of it, were resolved. `FILE[:LINE]` as printed in the report
    /// (relative to the repository root). Repeatable.
    pub assume_resolved: Vec<String>,
    /// Read --assume-resolved values from a file (`-`: stdin), one per line.
    /// Empty lines and lines starting with `#` are skipped.
    pub assume_resolved_from: Vec<String>,
    /// What if: report the floor as if every open record of this class were
    /// resolved: `KIND[/EFFECT[/ORIGIN]]`, where `*` matches anything
    /// (`computed_specifier/read`, `*/*/param`). Repeatable.
    pub assume_class: Vec<String>,
    /// What if: report the floor as if this node were not reached: a file
    /// (relative to the repository root), a task id (`task:.#build`) or a
    /// glob over them; `PREFIX**` matches every node that starts with the
    /// prefix (`task:**`: every task). No entry reaches it or anything
    /// through it. Predicts the effect of removing a hub. Repeatable.
    pub assume_cut: Vec<String>,
    /// Debug: instead of the report, print the entries that reach one of the
    /// files listed in FILE (one path per line, relative to the repository
    /// root), along the edges the floor follows for each entry.
    pub reach_from: Option<String>,
    /// Count the entries that reach each load with an unknown target that the
    /// declared packages bound, and the floor without that bound (a walk
    /// per load).
    pub loads: bool,
    /// Instead of the report: the certain edges that break the rule "a file
    /// requires, imports or reads only files of the packages its package
    /// depends on" (the findings of the lint rule "no dependency outside
    /// the declared packages"; --top limits the lists).
    pub undeclared_dependencies: bool,
    /// Debug, with --reach-from: per listed file, the number of entries that
    /// reach it.
    pub reach_each: bool,
    /// Output format.
    pub format: ReportFormat,
    /// Where the graph lives.
    pub loc: GraphLoc,
    /// Use the stored graph as is, without refreshing it.
    pub no_refresh: bool,
    /// Print timings to stderr.
    pub stats: bool,
}

impl Default for UncertaintyCmd {
    fn default() -> Self {
        UncertaintyCmd {
            entries: Default::default(),
            entries_opt: Default::default(),
            entries_from: Default::default(),
            tests: Default::default(),
            tasks: Default::default(),
            all_tasks: Default::default(),
            strict_eval: Default::default(),
            strict: Default::default(),
            runtime_only: Default::default(),
            top: 25,
            order: Order::Freed,
            assume_resolved: Default::default(),
            assume_resolved_from: Default::default(),
            assume_class: Default::default(),
            assume_cut: Default::default(),
            reach_from: Default::default(),
            loads: Default::default(),
            undeclared_dependencies: Default::default(),
            reach_each: Default::default(),
            format: ReportFormat::Text,
            loc: Default::default(),
            no_refresh: Default::default(),
            stats: Default::default(),
        }
    }
}

/// The what-if assumptions of the command line.
fn assumptions(cmd: &UncertaintyCmd) -> Result<FloorAssumptions> {
    let mut assume = FloorAssumptions::default();
    for s in &cmd.assume_resolved {
        assume.sites.push(AssumedSite::parse(s));
    }
    for src in &cmd.assume_resolved_from {
        let text = if src == "-" {
            std::io::read_to_string(std::io::stdin())?
        } else {
            std::fs::read_to_string(src).with_context(|| format!("cannot read {src}"))?
        };
        for line in text.lines().map(str::trim) {
            if !line.is_empty() && !line.starts_with('#') {
                assume.sites.push(AssumedSite::parse(line));
            }
        }
    }
    for c in &cmd.assume_class {
        let p = ClassPattern::parse(c).map_err(|e| anyhow::anyhow!("--assume-class: {e}"))?;
        assume.classes.push(p);
    }
    assume.cut = cmd.assume_cut.clone();
    Ok(assume)
}

/// The entries that `affected` evaluates without type-only edges: jest and
/// vitest tests whose runner transpiles file by file
/// (`turborepo_trace_graph::barrel::modes`). None with
/// `TURBO_TRACE_DEBUG_TYPE_EDGES=0`, as for `affected`.
pub(crate) fn runtime_entries(g: &Graph, root: &std::path::Path, entries: &NodeSet) -> NodeSet {
    use turborepo_trace_graph::barrel::{Runner, entry_runner};
    let mut set = NodeSet::new(g.node_count());
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

pub(crate) fn cmd_uncertainty(cmd: UncertaintyCmd, out: &mut dyn Write) -> Result<ExitCode> {
    let t0 = Instant::now();
    let (root, _) = cmd.loc.resolve()?;
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    if cmd.entries.is_empty()
        && cmd.entries_opt.is_empty()
        && cmd.entries_from.is_empty()
        && cmd.tests.is_empty()
        && cmd.tasks.is_empty()
    {
        bail!(
            "no entries: pass files or globs, --entries-from <file>, --tests <framework> or \
             --tasks <glob>"
        );
    }
    turborepo_trace_core::timing::mark("uncertainty: start");
    let mut g = open(&cmd.loc, cmd.no_refresh, cmd.stats)?;
    // Under --strict a working-directory path keeps its reading from the repository
    // root.
    g.set_cwd_root_always(cmd.strict);
    let t_graph = t0.elapsed();
    turborepo_trace_core::timing::mark("uncertainty: refresh + load");

    // The same options as `affected`; the policy doesn't change the report.
    let mut opts = selection_options(&root, cmd.runtime_only, Policy::Affected);
    opts.affected = opts
        .affected
        .strict_eval(cmd.strict_eval)
        .strict(cmd.strict);
    let aopts = opts.affected.clone();
    // The floor of the graph `affected` evaluates: per-test working directories and
    // hints applied (same node ids as `g`).
    let dg = crate::graph::derived(&g, cmd.stats)?;
    turborepo_trace_core::timing::mark("uncertainty: derived layer");
    let engine = Engine::with_options(&dg, opts);

    if cmd.undeclared_dependencies {
        let (reach, _) = crate::loads::reach(&dg, &root);
        let found = engine.undeclared_dependencies(&reach);
        crate::loads::write_undeclared(&dg, &found, cmd.top, out)?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut specs = cmd.entries.clone();
    specs.extend(cmd.entries_opt.iter().cloned());
    let mut entries = collect_entries(&g, &root, &specs, &cmd.entries_from, &cmd.tests)?;
    for n in collect_tasks(&g, &cmd.tasks, cmd.all_tasks)?.iter() {
        entries.set.insert(n);
    }
    let t_entries = t0.elapsed() - t_graph;
    turborepo_trace_core::timing::mark("uncertainty: entries");

    let t = Instant::now();
    let order = match cmd.order {
        Order::Freed => FloorOrder::Freed,
        Order::Weighted => FloorOrder::Weighted,
    };
    let assume = assumptions(&cmd)?;
    // Tests whose runner transpiles file by file don't follow type-only edges, as
    // in `affected`.
    let runtime = runtime_entries(&g, &root, &entries.set);
    turborepo_trace_core::timing::mark("uncertainty: runner of each entry");
    if let Some(list) = &cmd.reach_from {
        let text = std::fs::read_to_string(list).with_context(|| format!("cannot read {list}"))?;
        let mut targets = NodeSet::new(dg.node_count());
        let mut missing = 0;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            match dg.node_by_path(line) {
                Some(n) => {
                    targets.insert(n);
                }
                None => missing += 1,
            }
        }
        let all = engine.options().traversal;
        let rt = all.without_flags(turborepo_trace_graph::RefFlags::TYPE_ONLY);
        let reach_of = |targets: &NodeSet, filter: turborepo_trace_graph::EdgeFilter| {
            let rev = engine.reverse();
            let mut seen = targets.clone();
            let mut queue: Vec<_> = targets.iter().collect();
            while let Some(n) = queue.pop() {
                let (ts, attrs) = rev.neighbors(n);
                for (&t, &a) in ts.iter().zip(attrs) {
                    let t = turborepo_trace_graph::NodeId(t);
                    if (filter.is_all() || filter.accepts(a)) && seen.insert(t) {
                        queue.push(t);
                    }
                }
            }
            seen
        };
        if cmd.reach_each {
            // Per target: the number of entries that reach it.
            let mut rows: Vec<(usize, String)> = Vec::new();
            for t in targets.iter() {
                let mut one = NodeSet::new(dg.node_count());
                one.insert(t);
                let (with_types, without) = (reach_of(&one, all), reach_of(&one, rt));
                let n = entries
                    .set
                    .iter()
                    .filter(|&e| {
                        if runtime.contains(e) {
                            without.contains(e)
                        } else {
                            with_types.contains(e)
                        }
                    })
                    .count();
                rows.push((n, dg.path(t).to_string()));
            }
            rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            for (n, p) in rows {
                writeln!(out, "{n}\t{p}")?;
            }
            return Ok(ExitCode::SUCCESS);
        }
        let reach = |filter: turborepo_trace_graph::EdgeFilter| {
            let rev = engine.reverse();
            let mut seen = targets.clone();
            let mut queue: Vec<_> = targets.iter().collect();
            while let Some(n) = queue.pop() {
                let (ts, attrs) = rev.neighbors(n);
                for (&t, &a) in ts.iter().zip(attrs) {
                    let t = turborepo_trace_graph::NodeId(t);
                    if (filter.is_all() || filter.accepts(a)) && seen.insert(t) {
                        queue.push(t);
                    }
                }
            }
            seen
        };
        let (with_types, without) = (reach(all), reach(rt));
        let mut n = 0;
        for e in entries.set.iter() {
            let set = if runtime.contains(e) {
                &without
            } else {
                &with_types
            };
            if set.contains(e) {
                n += 1;
                writeln!(out, "{}", dg.path(e))?;
            }
        }
        eprintln!(
            "reach: {} target file(s) ({missing} not in the graph), {n} of {} entries reach one",
            targets.len(),
            entries.set.len()
        );
        return Ok(ExitCode::SUCCESS);
    }
    let report = engine.floor_for_runners(&entries.set, &runtime, &aopts, order, cmd.top, &assume);
    turborepo_trace_core::timing::mark("uncertainty: floor");
    for u in report.assumed.iter().flat_map(|a| &a.unmatched) {
        eprintln!("warning: no open record matches the assumption {u}");
    }
    let t_floor = t.elapsed();
    // Loads with an unknown target are not on the floor: the declared packages
    // bound them (crate::loads). The report says for which packages' changes
    // each one selects.
    let bounded = (report.records.declared > 0).then(|| {
        if cmd.loads {
            let (reach, _) = crate::loads::reach(&dg, &root);
            engine.bounded_sites(&reach, &entries.set, Some(&runtime), &aopts)
        } else {
            engine.bounded_records(&aopts)
        }
    });
    if cmd.stats {
        eprintln!(
            "uncertainty: {} entr(ies) ({} without type-only edges), {} open site(s), {} reached; \
             graph {} ms, entries {} ms, floor {:.1} ms, total {} ms",
            report.entries,
            report.runtime_entries,
            report.sites.len(),
            report.sites_reached(),
            t_graph.as_millis(),
            t_entries.as_millis(),
            t_floor.as_secs_f64() * 1e3,
            t0.elapsed().as_millis()
        );
    }

    match cmd.format {
        ReportFormat::Text => {
            report.write_text(&dg, out)?;
            if let Some(sites) = &bounded {
                crate::loads::write_bounded(
                    &dg,
                    sites,
                    report.floor,
                    aopts.loads_step,
                    cmd.top,
                    out,
                )?;
            }
            if !entries.missing.is_empty() {
                writeln!(
                    out,
                    "{} more entr(ies) are not in the graph (not traced source files): `affected` \
                     always selects them",
                    entries.missing.len()
                )?;
            }
        }
        ReportFormat::Json => {
            let mut v = report.to_value(&dg);
            v["options"] = serde_json::json!({
                "strict": cmd.strict,
                "strict_eval": cmd.strict_eval || cmd.strict,
                "runtime_only": cmd.runtime_only,
                "order": order,
                "assume_resolved": assume.sites.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "assume_class": assume.classes.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "assume_cut": assume.cut,
            });
            if let Some(sites) = &bounded {
                v["bounded_loads"] = crate::loads::bounded_value(&dg, sites);
            }
            v["not_in_graph"] = serde_json::json!(entries.missing);
            serde_json::to_writer_pretty(&mut *out, &v)?;
            writeln!(out)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
