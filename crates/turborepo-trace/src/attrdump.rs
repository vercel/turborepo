//! Measurement tooling (not a product feature): with
//! `TURBO_TRACE_DEBUG_ATTR_DUMP=<dir>`, `affected` writes what its selection
//! was computed from, as tab-separated tables for offline analysis:
//!
//! | File | Rows |
//! |---|---|
//! | `nodes.tsv` | id, path, language, external, package id, package name, package root |
//! | `edges.tsv` | source, target, kind, phase, flag bits, line, specifier (the derived view) |
//! | `unc.tsv` | node, kind, effect, origin, line, disposition, resolved per entry |
//! | `pass-<i>.json` | one evaluation pass ([`turborepo_trace_graph::attr::PassRecord`]) |
//! | `layer-<name>.tsv` | entry, first witness node, cause kind, cause detail, chain length |
//! | `named.tsv` | kind, label, changed paths, example, naming files, entries |
//! | `meta.json` | the changes and what was done with them |
//!
//! Layers: `engine` (the passes, after barrel and erase pruning), `steering`,
//! `workflow`, `services` (each evaluated alone), and `final` (what `affected`
//! prints). The dump holds paths and specifiers of the repository: it is as
//! private as the repository.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Engine, GraphView, NodeId,
    attr::{PassRecord, cause_fields},
    names::NamedGroup,
};

/// The dump directory, when the dump is on.
pub(crate) fn dir() -> Option<PathBuf> {
    let d = turborepo_trace_core::debug::var_os("ATTR_DUMP").filter(|d| !d.is_empty())?;
    let d = PathBuf::from(d);
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn put(dir: &Path, name: &str, text: &str) -> Result<()> {
    std::fs::write(dir.join(name), text).with_context(|| format!("writing {name}"))
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// `nodes.tsv`, `edges.tsv` and `unc.tsv` of the view the engine evaluates.
pub(crate) fn graph<V: GraphView>(
    dir: &Path,
    engine: &Engine<'_, V>,
    opts: &AffectedOptions,
) -> Result<()> {
    let g = engine.graph();
    let (mut nodes, mut edges, mut unc) = (String::new(), String::new(), String::new());
    for i in 0..g.node_count() {
        let n = NodeId(i);
        let p = g.package(n);
        let info = g.package_info(p);
        let _ = writeln!(
            nodes,
            "{i}\t{}\t{}\t{}\t{}\t{}@{}\t{}",
            clean(g.path(n)),
            g.lang(n).unwrap_or(""),
            u8::from(g.is_external(n)),
            p.0,
            info.name,
            info.version.as_str(),
            info.root,
        );
        for e in g.out_edges(n) {
            let Some(to) = e.to else { continue };
            let _ = writeln!(
                edges,
                "{i}\t{}\t{}\t{}\t{}\t{}\t{}",
                to.0,
                e.kind(),
                e.phase(),
                e.flags().bits(),
                e.line,
                clean(e.specifier.unwrap_or("")),
            );
        }
        if g.is_external(n) {
            continue;
        }
        for u in g.uncertainties(n) {
            let d = engine.disposition_of(n, &u, opts);
            let _ = writeln!(
                unc,
                "{i}\t{}\t{}\t{}\t{}\t{}\t{}",
                u.kind,
                u.effect,
                u.origin,
                u.line,
                serde_json::to_string(&d)
                    .unwrap_or_default()
                    .trim_matches('"'),
                u8::from(g.is_entry_resolved(n, &u)),
            );
        }
    }
    put(dir, "nodes.tsv", &nodes)?;
    put(dir, "edges.tsv", &edges)?;
    put(dir, "unc.tsv", &unc)
}

/// `pass-<i>.json`.
pub(crate) fn passes(dir: &Path, passes: &[PassRecord]) -> Result<()> {
    for (i, p) in passes.iter().enumerate() {
        let v = serde_json::json!({
            "runtime_only": p.runtime_only,
            "data_excluded": p.data_excluded,
            "floor": p.floor,
            "entries": p.entries,
            "seeds": p.seeds,
            "deferred": p.deferred,
            "open": p.open,
            "per_entry": p.per_entry,
            "exempt": p.exempt,
        });
        put(dir, &format!("pass-{i}.json"), &v.to_string())?;
    }
    Ok(())
}

/// A report with no entries, to evaluate one layer alone.
pub(crate) fn empty(like: &AffectedReport) -> AffectedReport {
    AffectedReport {
        policy: like.policy,
        entry_count: like.entry_count,
        entries: Vec::new(),
        seed_count: 0,
        reached_count: 0,
        unmatched: Vec::new(),
    }
}

/// `layer-<name>.tsv`: the entries of `report`.
pub(crate) fn layer<V: GraphView>(
    dir: &Path,
    name: &str,
    g: &V,
    report: &AffectedReport,
) -> Result<()> {
    let mut out = String::new();
    for e in &report.entries {
        let first = e.witness.first().copied().unwrap_or(e.entry);
        let (kind, detail) = match (&e.cause, &e.uncertain) {
            (Some(c), _) => cause_fields(g, first, c),
            (None, Some(u)) => ("floor", g.path(u.node).to_owned()),
            (None, None) => ("none", String::new()),
        };
        let _ = writeln!(
            out,
            "{}\t{}\t{kind}\t{}\t{}",
            e.entry.0,
            first.0,
            clean(&detail),
            e.witness.len()
        );
    }
    put(dir, &format!("layer-{name}.tsv"), &out)
}

/// `named.tsv` and `runtime-entries.txt`.
pub(crate) fn named(dir: &Path, groups: &[NamedGroup], runtime: &[u32]) -> Result<()> {
    let ids = |v: &[u32]| v.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let mut out = String::new();
    for g in groups {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}",
            g.kind,
            clean(&g.label),
            g.paths,
            clean(&g.example),
            ids(&g.files),
            ids(&g.entries)
        );
    }
    put(dir, "named.tsv", &out)?;
    put(dir, "runtime-entries.txt", &ids(runtime))
}

/// `meta.json`.
pub(crate) fn meta(dir: &Path, v: &serde_json::Value) -> Result<()> {
    put(dir, "meta.json", &v.to_string())
}
