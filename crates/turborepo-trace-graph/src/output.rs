//! Query output: `list`, `json`, `dot` and `tree`.

use std::io::{self, Write};

use serde::Serialize;

use crate::{
    change::{AffectedReport, Cause, EntryResult},
    engine::{Engine, QueryResult},
    set::NodeSet,
    view::{GraphView, NodeId},
};

/// Output format.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// Sorted paths, one per line.
    #[default]
    List,
    /// JSON: nodes (and optionally edges inside the result), or the `affected`
    /// report.
    Json,
    /// Graphviz: nodes and edges inside the result.
    Dot,
    /// Indented dependency tree over edges inside the result, with cycle
    /// markers.
    Tree,
}

impl Format {
    /// Parses `list`, `json`, `dot` or `tree`.
    pub fn parse(s: &str) -> Option<Format> {
        Some(match s {
            "list" => Format::List,
            "json" => Format::Json,
            "dot" => Format::Dot,
            "tree" => Format::Tree,
            _ => return None,
        })
    }
}

/// Output options.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputOptions {
    /// Format.
    pub format: Format,
    /// JSON: include the edges between result nodes.
    pub edges: bool,
}

/// Writes a query result.
///
/// For a top-level `affected`, `list` prints the affected entries, `json` the
/// full report ([`write_affected_json`]) and `tree` one explanation chain per
/// affected or unknown entry.
pub fn write<G: GraphView>(
    engine: &Engine<'_, G>,
    result: &QueryResult,
    opts: &OutputOptions,
    w: &mut dyn Write,
) -> io::Result<()> {
    let g = engine.graph();
    if let Some(report) = &result.affected {
        match opts.format {
            Format::Json => return write_affected_json(g, report, w),
            Format::Tree => return write_affected_explain(g, report, w),
            _ => {}
        }
    }
    match opts.format {
        Format::List => write_list(g, &result.nodes, w),
        Format::Json => write_json(engine, result, opts.edges, w),
        Format::Dot => write_dot(engine, &result.nodes, w),
        Format::Tree => write_tree(engine, &result.nodes, w),
    }
}

fn sorted<G: GraphView>(g: &G, set: &NodeSet) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = set.iter().collect();
    v.sort_unstable_by(|a, b| g.path(*a).cmp(g.path(*b)));
    v
}

/// Sorted paths, one per line.
pub fn write_list<G: GraphView>(g: &G, set: &NodeSet, w: &mut dyn Write) -> io::Result<()> {
    let mut out = io::BufWriter::new(w);
    for n in sorted(g, set) {
        writeln!(out, "{}", g.path(n))?;
    }
    out.flush()
}

#[derive(Serialize)]
struct JsonNode<'a> {
    path: &'a str,
    package: &'a str,
    version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    external: bool,
}

#[derive(Serialize)]
struct JsonEdge<'a> {
    from: &'a str,
    to: &'a str,
    kind: &'static str,
    phase: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    flags: Vec<&'static str>,
    #[serde(skip_serializing_if = "is_zero")]
    line: u32,
}

fn is_zero(x: &u32) -> bool {
    *x == 0
}

fn json_node<G: GraphView>(g: &G, n: NodeId) -> JsonNode<'_> {
    let info = g.package_info(g.package(n));
    JsonNode {
        path: g.path(n),
        package: info.name,
        version: info.version.as_str(),
        lang: g.lang(n),
        external: g.is_external(n),
    }
}

fn edges_within<'g, G: GraphView>(g: &'g G, set: &NodeSet, order: &[NodeId]) -> Vec<JsonEdge<'g>> {
    let mut out = Vec::new();
    for &n in order {
        let mut es: Vec<_> = g
            .out_edges(n)
            .filter_map(|e| e.to.filter(|t| set.contains(*t)).map(|t| (t, e)))
            .collect();
        es.sort_by(|a, b| g.path(a.0).cmp(g.path(b.0)));
        for (t, e) in es {
            out.push(JsonEdge {
                from: g.path(n),
                to: g.path(t),
                kind: e.kind().as_str(),
                phase: e.phase().as_str(),
                flags: e.flags().names().collect(),
                line: e.line,
            });
        }
    }
    out
}

/// JSON for a node-set result:
/// `{"nodes": [...], "edges": [...]?, "path": [...]?}`.
pub fn write_json<G: GraphView>(
    engine: &Engine<'_, G>,
    result: &QueryResult,
    edges: bool,
    w: &mut dyn Write,
) -> io::Result<()> {
    #[derive(Serialize)]
    struct Out<'a> {
        count: usize,
        nodes: Vec<JsonNode<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        edges: Option<Vec<JsonEdge<'a>>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<Vec<&'a str>>,
    }
    let g = engine.graph();
    let order = sorted(g, &result.nodes);
    let out = Out {
        count: order.len(),
        nodes: order.iter().map(|&n| json_node(g, n)).collect(),
        edges: edges.then(|| edges_within(g, &result.nodes, &order)),
        path: result
            .path
            .as_ref()
            .map(|p| p.iter().map(|&n| g.path(n)).collect()),
    };
    let mut w = io::BufWriter::new(w);
    serde_json::to_writer_pretty(&mut w, &out)?;
    writeln!(w)?;
    w.flush()
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JsonCause<'a> {
    Modified {
        path: &'a str,
    },
    Deleted {
        path: &'a str,
    },
    Probe {
        added: &'a str,
        probe: &'a str,
    },
    Membership {
        path: &'a str,
        change: &'static str,
        dir: &'a str,
        pattern: &'a str,
    },
    Package {
        package: &'a str,
        version: &'a str,
    },
    /// A change inside the scope of a covered uncertainty.
    Scope {
        path: &'a str,
        change: &'static str,
        dir: &'a str,
        pattern: &'a str,
    },
    /// A changed path named by code loaded with a read whose path is computed.
    Named {
        path: &'a str,
        by: &'a str,
        line: u32,
        site: &'a str,
        site_line: u32,
    },
}

fn json_cause<'a, G: GraphView>(g: &'a G, c: &'a Cause) -> JsonCause<'a> {
    match c {
        Cause::Modified { path } => JsonCause::Modified { path },
        Cause::Deleted { path } => JsonCause::Deleted { path },
        Cause::Probe { added, probe } => JsonCause::Probe { added, probe },
        Cause::Membership {
            path,
            kind,
            dir,
            pattern,
        } => JsonCause::Membership {
            path,
            change: kind.as_str(),
            dir,
            pattern,
        },
        Cause::Package { package } => {
            let info = g.package_info(*package);
            JsonCause::Package {
                package: info.name,
                version: info.version.as_str(),
            }
        }
        Cause::Scope {
            path,
            kind,
            dir,
            pattern,
        } => JsonCause::Scope {
            path,
            change: kind.as_str(),
            dir,
            pattern,
        },
        Cause::Named {
            path,
            by,
            line,
            site,
            site_line,
        } => JsonCause::Named {
            path,
            by,
            line: *line,
            site,
            site_line: *site_line,
        },
    }
}

#[derive(Serialize)]
struct JsonReason<'a> {
    kind: &'static str,
    #[serde(skip_serializing_if = "is_zero")]
    line: u32,
    detail: &'a str,
}

#[derive(Serialize)]
struct JsonUncertain<'a> {
    file: &'a str,
    reasons: Vec<JsonReason<'a>>,
    /// For a load bounded by the declared packages: why the change selects the
    /// entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    bound: Option<&'a str>,
}

#[derive(Serialize)]
struct JsonEntry<'a> {
    entry: &'a str,
    verdict: crate::change::Verdict,
    witness: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<JsonCause<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uncertain: Option<JsonUncertain<'a>>,
}

fn json_entry<'a, G: GraphView>(g: &'a G, e: &'a EntryResult) -> JsonEntry<'a> {
    JsonEntry {
        entry: g.path(e.entry),
        verdict: e.verdict,
        witness: e.witness.iter().map(|&n| g.path(n)).collect(),
        cause: e.cause.as_ref().map(|c| json_cause(g, c)),
        uncertain: e.uncertain.as_ref().map(|u| JsonUncertain {
            file: g.path(u.node),
            reasons: u
                .reasons
                .iter()
                .map(|(k, line, d)| JsonReason {
                    kind: k.as_str(),
                    line: *line,
                    detail: d,
                })
                .collect(),
            bound: u.bound.as_deref(),
        }),
    }
}

/// One affected or unknown entry as a JSON value: `entry`, `verdict`, `witness`
/// (paths, seed first), `cause` and `uncertain`, as in the `results` of
/// [`write_affected_json`].
pub fn affected_entry_value<G: GraphView>(g: &G, e: &EntryResult) -> serde_json::Value {
    serde_json::to_value(json_entry(g, e)).unwrap_or(serde_json::Value::Null)
}

/// The `affected` report as JSON:
///
/// ```json
/// {"policy": "affected", "entries": 3, "affected": 1, "unknown": 0, "unaffected": 2,
///  "seeds": 1, "reached": 4,
///  "results": [{"entry": "t/a.test.ts", "verdict": "affected",
///               "witness": ["src/a.ts", "t/a.test.ts"],
///               "cause": {"type": "modified", "path": "src/a.ts"}}],
///  "unmatched": [{"path": "README.md", "kind": "modified"}]}
/// ```
pub fn write_affected_json<G: GraphView>(
    g: &G,
    report: &AffectedReport,
    w: &mut dyn Write,
) -> io::Result<()> {
    #[derive(Serialize)]
    struct Out<'a> {
        policy: crate::change::Policy,
        entries: usize,
        affected: usize,
        unknown: usize,
        unaffected: usize,
        seeds: usize,
        reached: usize,
        results: Vec<JsonEntry<'a>>,
        unmatched: &'a [crate::change::Change],
    }
    let affected = report
        .with_verdict(crate::change::Verdict::Affected)
        .count();
    let unknown = report.with_verdict(crate::change::Verdict::Unknown).count();
    let out = Out {
        policy: report.policy,
        entries: report.entry_count,
        affected,
        unknown,
        unaffected: report.entry_count - affected - unknown,
        seeds: report.seed_count,
        reached: report.reached_count,
        results: report.entries.iter().map(|e| json_entry(g, e)).collect(),
        unmatched: &report.unmatched,
    };
    let mut w = io::BufWriter::new(w);
    serde_json::to_writer_pretty(&mut w, &out)?;
    writeln!(w)?;
    w.flush()
}

/// Why an entry is affected, as the words `--explain` prints between the entry
/// and its chain: `modified`, `added a.ts (probed a) at`, `uncertain
/// (computed_specifier)`.
pub fn affected_why<G: GraphView>(g: &G, e: &EntryResult) -> String {
    let first = e.witness.first().map(|&n| g.path(n));
    match (&e.cause, &e.uncertain) {
        (Some(Cause::Modified { path }), _) if first != Some(path.as_str()) => {
            format!("modified {path} (read or probed) at")
        }
        (Some(Cause::Deleted { path }), _) if first != Some(path.as_str()) => {
            format!("deleted {path} (read or probed) at")
        }
        (Some(Cause::Modified { .. }), _) => "modified".to_owned(),
        (Some(Cause::Deleted { .. }), _) => "deleted".to_owned(),
        (Some(Cause::Probe { added, probe }), _) => {
            format!("added {added} (probed {probe}) at")
        }
        (
            Some(Cause::Membership {
                path,
                kind,
                dir,
                pattern,
            }),
            _,
        ) => {
            format!("{} {path} (matches {dir}/{pattern}) at", kind.as_str())
        }
        (Some(Cause::Package { package }), _) => {
            let info = g.package_info(*package);
            format!("package {}@{} changed at", info.name, info.version.as_str())
        }
        (
            Some(Cause::Scope {
                path,
                kind,
                dir,
                pattern,
            }),
            _,
        ) => {
            format!(
                "{} {path} (inside the scope {} of a covered uncertainty) at",
                kind.as_str(),
                join_scope(dir, pattern)
            )
        }
        (
            Some(Cause::Named {
                path,
                by,
                line,
                site,
                site_line,
            }),
            _,
        ) => {
            format!(
                "changed {path}, named by {by} in {}:{line} (loaded with the read at \
                 {site}:{site_line}, whose path is computed):",
                first.unwrap_or_default()
            )
        }
        (None, Some(u)) => {
            let kinds: Vec<_> = u.reasons.iter().map(|r| r.0.as_str()).collect();
            match &u.bound {
                // A load with an unknown target, bounded by the declared packages.
                Some(b) => format!("uncertain ({}; {b}) at", kinds.join(", ")),
                None => format!("uncertain ({})", kinds.join(", ")),
            }
        }
        (None, None) => String::new(),
    }
}

/// How the specifier of an edge starts that stands for one reading of a path
/// which leaves the repository by the way it is written (an absolute path, a
/// climb above the root): the repository seen from another place. The graph
/// store writes it (the core crate's `output::ELSEWHERE_PREFIX`, the same
/// text); the rest is the reading in one sentence.
pub const ELSEWHERE_PREFIX: &str = "elsewhere ";

/// How the detail of the record ends that a read below a temporary directory
/// keeps (the core crate's `output::TEMP_READ_NOTE`, the same text, written by
/// the graph store).
pub const TEMP_READ_NOTE: &str = "is below a temporary directory: what is there is made at run \
                                  time, not a file of the repository";

/// How the detail of the record ends that a read keeps whose path has a
/// computed directory directly below a place the repository may be mounted at
/// (the core crate's `output::MOUNT_READ_NOTE`, the same text, written by the
/// graph store).
pub const MOUNT_READ_NOTE: &str = "is below a directory that is computed directly below a place \
                                   the repository may be mounted at: not read as a path of the \
                                   repository";

/// The assumption about a computed directory directly below a mount point,
/// stated for an entry when one of its `notes` is such a read
/// ([`MOUNT_READ_NOTE`]) that the policy ignores: one line of `--explain`.
pub fn write_mount_assumption(
    notes: &[crate::change::UncertaintyNote],
    w: &mut dyn Write,
) -> io::Result<()> {
    let Some(n) = notes.iter().find(|n| {
        n.kind == crate::vocab::UncertaintyKind::GeneratedFile
            && n.disposition == crate::change::Disposition::Ignored
            && n.detail.ends_with(MOUNT_READ_NOTE)
    }) else {
        return Ok(());
    };
    let path = n.detail.strip_suffix(MOUNT_READ_NOTE).unwrap_or("").trim();
    writeln!(
        w,
        "  assumed: a directory that is computed directly below a place the repository may be \
         mounted at ({path}) names no directory of the repository (--strict does not assume it)"
    )
}

/// The assumption about paths below a temporary directory, stated for an entry
/// when one of its `notes` is such a read ([`TEMP_READ_NOTE`]) that the policy
/// ignores: one line of `--explain`.
pub fn write_temp_assumption(
    notes: &[crate::change::UncertaintyNote],
    w: &mut dyn Write,
) -> io::Result<()> {
    let Some(n) = notes.iter().find(|n| {
        n.kind == crate::vocab::UncertaintyKind::GeneratedFile
            && n.disposition == crate::change::Disposition::Ignored
            && n.detail.ends_with(TEMP_READ_NOTE)
    }) else {
        return Ok(());
    };
    let path = n.detail.strip_suffix(TEMP_READ_NOTE).unwrap_or("").trim();
    writeln!(
        w,
        "  assumed: a path below a temporary directory that the code writes out ({path}) names no \
         file of the repository: what is there is made at run time (--strict does not assume it)"
    )
}

/// The steps of a witness chain that go through a path which leaves the
/// repository by the way it is written, each in one sentence: `src/a.test.js
/// refers to /app/data/x.json, an absolute path: it names data/x.json if the
/// repository is mounted at /app`.
pub fn elsewhere_steps<G: GraphView>(g: &G, witness: &[NodeId]) -> Vec<String> {
    let mut out = Vec::new();
    for step in witness.windows(2) {
        let (target, reader) = (step[0], step[1]);
        let reading = g
            .out_edges(reader)
            .filter(|e| e.to == Some(target))
            .find_map(|e| e.specifier?.strip_prefix(ELSEWHERE_PREFIX));
        if let Some(reading) = reading {
            out.push(format!("{} refers to {reading}", g.path(reader)));
        }
    }
    out
}

/// One readable line per affected or unknown entry, such as
/// `affected test/b.test.ts: modified src/a.ts → src/b.ts → test/b.test.ts`. A
/// step through a path that leaves the repository by the way it is written says
/// so ([`elsewhere_steps`]).
pub fn write_affected_explain<G: GraphView>(
    g: &G,
    report: &AffectedReport,
    w: &mut dyn Write,
) -> io::Result<()> {
    let mut w = io::BufWriter::new(w);
    for e in &report.entries {
        let chain: Vec<&str> = e.witness.iter().map(|&n| g.path(n)).collect();
        let why = affected_why(g, e);
        let verdict = match e.verdict {
            crate::change::Verdict::Affected => "affected",
            crate::change::Verdict::Unknown => "unknown",
            crate::change::Verdict::Unaffected => "unaffected",
        };
        let steps = elsewhere_steps(g, &e.witness);
        let steps = match steps.is_empty() {
            true => String::new(),
            false => format!(" ({})", steps.join("; ")),
        };
        writeln!(
            w,
            "{verdict} {}: {why} {}{steps}",
            g.path(e.entry),
            chain.join(" → ")
        )?;
    }
    w.flush()
}

/// `dir/pattern` (`pattern` alone for the root).
pub fn join_scope(dir: &str, pattern: &str) -> String {
    if dir.is_empty() {
        pattern.to_owned()
    } else {
        format!("{dir}/{pattern}")
    }
}

/// Readable lines for [`crate::Engine::uncertainty_notes`], indented by two
/// spaces: `covered src/a.ts:3 pattern_too_broad (scope data/**): <detail>` and
/// `ignored by default src/b.ts:9 eval_like (--strict counts it):
/// <detail>`. At most `max` lines, then a count of the rest.
pub fn write_uncertainty_notes<G: GraphView>(
    g: &G,
    notes: &[crate::change::UncertaintyNote],
    max: usize,
    w: &mut dyn Write,
) -> io::Result<()> {
    for n in notes.iter().take(max) {
        let at = if n.line > 0 {
            format!("{}:{}", g.path(n.node), n.line)
        } else {
            g.path(n.node).to_owned()
        };
        match (&n.scope, n.disposition) {
            (Some((dir, pattern)), _) => writeln!(
                w,
                "  covered {at} {} (scope {}): {}",
                n.kind.as_str(),
                join_scope(dir, pattern),
                n.detail
            )?,
            // A Python record rests on an assumption about the environment: say it.
            (None, crate::change::Disposition::Probed) if g.lang(n.node) == Some("python") => {
                writeln!(
                    w,
                    "  covered {at} {} (by the paths its resolution probed, the places a module \
                     of that name could appear in the repository, and the dependency declarations \
                     above the file, from which its environment is assumed to be installed; \
                     --strict counts it): {}",
                    n.kind.as_str(),
                    n.detail
                )?
            }
            (None, crate::change::Disposition::Probed) => writeln!(
                w,
                "  covered {at} {} (by the paths its resolution probed): {}",
                n.kind.as_str(),
                n.detail
            )?,
            (None, crate::change::Disposition::Named) => writeln!(
                w,
                "  covered {at} {} (bounded by the path-like strings of the code loaded with it): \
                 {}",
                n.kind.as_str(),
                n.detail
            )?,
            (None, crate::change::Disposition::Declared) => writeln!(
                w,
                "  covered {at} {} (a load bounded by the declared packages: it selects this \
                 entry when the change affects a package the load can reach): {}",
                n.kind.as_str(),
                n.detail
            )?,
            (None, _) => writeln!(
                w,
                "  ignored by default {at} {} (--strict counts it): {}",
                n.kind.as_str(),
                n.detail
            )?,
        }
    }
    if notes.len() > max {
        writeln!(
            w,
            "  … and {} more uncertaintie(s) that don't select on every change",
            notes.len() - max
        )?;
    }
    Ok(())
}

/// The package assumption of [`crate::names`], stated for `entry` when one of
/// its `notes` is a read bounded by names and the assumption is on: one line of
/// `--explain` that a user can check against the repository.
pub fn write_names_assumption<G: GraphView>(
    g: &G,
    entry: NodeId,
    notes: &[crate::change::UncertaintyNote],
    opts: &crate::change::AffectedOptions,
    w: &mut dyn Write,
) -> io::Result<()> {
    let named = |n: &crate::change::UncertaintyNote| {
        n.scope.is_none() && n.disposition == crate::change::Disposition::Named
    };
    if !opts.named || !opts.named_in_package || !notes.iter().any(named) {
        return Ok(());
    }
    let root = g.package_info(g.package(entry)).root.trim_matches('/');
    let root = crate::names::Packages::of(g).outermost(root);
    let package = if root.is_empty() || root == "." {
        "the root package: the files outside every other package".to_owned()
    } else {
        format!("{root}/")
    };
    writeln!(
        w,
        "  assumed: a read with a computed path reads only files of the package of this entry \
         ({package}) or of the package of the file that holds the read, unless the code says \
         where the path goes (--strict does not assume it)"
    )
}

/// The assumption of [`crate::loads`], stated for `entry` when one of its
/// `notes` is a load bounded by the declared packages: one line of `--explain`
/// that a user can check against the repository's manifests.
pub fn write_loads_assumption<G: GraphView>(
    g: &G,
    entry: NodeId,
    notes: &[crate::change::UncertaintyNote],
    opts: &crate::change::AffectedOptions,
    w: &mut dyn Write,
) -> io::Result<()> {
    let bounded = |n: &crate::change::UncertaintyNote| {
        n.scope.is_none() && n.disposition == crate::change::Disposition::Declared
    };
    if !opts.loads_declared || !notes.iter().any(bounded) {
        return Ok(());
    }
    let root = g.package_info(g.package(entry)).root.trim_matches('/');
    let package = if root.is_empty() || root == "." {
        "the root package".to_owned()
    } else {
        format!("{root}/")
    };
    if opts.loads_step {
        writeln!(
            w,
            "  assumed: a file requires, imports and reads only files of its own package and of \
             the packages that package declares, so a load with a computed target loads a file of \
             the package of the loading file or of this entry ({package}), or of a package one of \
             them declares. The module system guarantees it for a specifier; for a computed path \
             or a mapper pattern it is an assumption (--strict does not make it)"
        )
    } else {
        writeln!(
            w,
            "  assumed: this entry requires, imports and reads only files of the packages its \
             package ({package}) depends on, whatever declares them; a load with a computed \
             target is held to that (--strict does not assume it)"
        )
    }
}

fn dot_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Graphviz digraph of the result nodes and the edges between them (one edge
/// per distinct `(from, to, kind)`).
pub fn write_dot<G: GraphView>(
    engine: &Engine<'_, G>,
    set: &NodeSet,
    w: &mut dyn Write,
) -> io::Result<()> {
    let g = engine.graph();
    let mut w = io::BufWriter::new(w);
    writeln!(w, "digraph deps {{")?;
    writeln!(w, "  rankdir=LR;")?;
    writeln!(w, "  node [shape=box, fontname=\"monospace\"];")?;
    let order = sorted(g, set);
    for &n in &order {
        let style = if g.is_external(n) {
            ", style=dashed"
        } else if g.has_uncertainty(n) {
            ", color=orange"
        } else {
            ""
        };
        writeln!(w, "  {} [label={}{style}];", n.0, dot_quote(g.path(n)))?;
    }
    for &n in &order {
        let mut es: Vec<(u32, &'static str, bool)> = g
            .out_edges(n)
            .filter_map(|e| {
                e.to.filter(|t| set.contains(*t)).map(|t| {
                    (
                        t.0,
                        e.kind().as_str(),
                        e.flags().contains(crate::RefFlags::TYPE_ONLY),
                    )
                })
            })
            .collect();
        es.sort_unstable();
        es.dedup();
        for (t, kind, type_only) in es {
            let style = if type_only { ", style=dotted" } else { "" };
            writeln!(w, "  {} -> {} [label={kind}{style}];", n.0, t)?;
        }
    }
    writeln!(w, "}}")?;
    w.flush()
}

/// An indented dependency tree over the edges inside the result.
///
/// Roots are result nodes with no predecessor inside the result (then, for
/// cycles with no such entry point, the smallest remaining path). Children are
/// sorted by path. A node already printed is shown once more with `(*)` and not
/// expanded; a back edge to a node on the current branch is marked `(cycle)`.
pub fn write_tree<G: GraphView>(
    engine: &Engine<'_, G>,
    set: &NodeSet,
    w: &mut dyn Write,
) -> io::Result<()> {
    let g = engine.graph();
    let fwd = engine.forward();
    let rev = engine.reverse();
    let filter = engine.options().traversal;
    let order = sorted(g, set);
    let children = |n: NodeId| -> Vec<NodeId> {
        let (ts, attrs) = fwd.neighbors(n);
        let mut c: Vec<NodeId> = ts
            .iter()
            .zip(attrs)
            .filter(|(t, a)| set.contains(NodeId(**t)) && filter.accepts(**a))
            .map(|(t, _)| NodeId(*t))
            .collect();
        c.sort_unstable_by(|a, b| g.path(*a).cmp(g.path(*b)));
        c.dedup();
        c
    };
    let mut roots: Vec<NodeId> = order
        .iter()
        .copied()
        .filter(|&n| {
            let (ps, attrs) = rev.neighbors(n);
            !ps.iter()
                .zip(attrs)
                .any(|(p, a)| *p != n.0 && set.contains(NodeId(*p)) && filter.accepts(*a))
        })
        .collect();
    let mut w = io::BufWriter::new(w);
    let mut printed = NodeSet::new(g.node_count());
    let mut on_stack = NodeSet::new(g.node_count());
    let mut extra = order.iter();
    let mut ri = 0;
    loop {
        let root = if ri < roots.len() {
            ri += 1;
            roots[ri - 1]
        } else {
            match extra.by_ref().find(|n| !printed.contains(**n)) {
                Some(&n) => {
                    roots.push(n);
                    ri += 1;
                    n
                }
                None => break,
            }
        };
        if printed.contains(root) {
            continue;
        }
        writeln!(w, "{}", g.path(root))?;
        printed.insert(root);
        on_stack.insert(root);
        // Stack frames: node, its children, next child index, line prefix.
        let mut stack: Vec<(NodeId, Vec<NodeId>, usize, String)> =
            vec![(root, children(root), 0, String::new())];
        while let Some(top) = stack.last_mut() {
            let (node, kids, idx, prefix) = top;
            if *idx >= kids.len() {
                on_stack.remove(*node);
                stack.pop();
                continue;
            }
            let child = kids[*idx];
            *idx += 1;
            let last = *idx == kids.len();
            let branch = if last { "└── " } else { "├── " };
            let child_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
            if on_stack.contains(child) {
                writeln!(w, "{prefix}{branch}{} (cycle)", g.path(child))?;
            } else if printed.contains(child) {
                writeln!(w, "{prefix}{branch}{} (*)", g.path(child))?;
            } else {
                writeln!(w, "{prefix}{branch}{}", g.path(child))?;
                printed.insert(child);
                on_stack.insert(child);
                let k = children(child);
                stack.push((child, k, 0, child_prefix));
            }
        }
    }
    w.flush()
}
