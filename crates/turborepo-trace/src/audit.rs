//! `affected --audit`: the unlikely-path audit of a selection
//! ([`turborepo_trace_graph::audit`]), with the declared package dependencies
//! read from the workspace manifests.

use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::Path,
    time::Instant,
};

use anyhow::{Result, bail};
use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Change, ChangeSet, EdgeFilter, Engine, GraphView, NodeSet,
    PackageId, PackageVersion, RefFlags, Verdict,
    audit::{AuditOptions, Ecosystem, VerdictFilter},
};
use turborepo_trace_store::Graph;

/// The declared dependencies between the first-party packages of a graph.
#[derive(Debug, Default)]
pub struct PackageTable {
    /// Per package: the first-party packages its manifests name.
    pub deps: Vec<Vec<PackageId>>,
    /// Per package: the ecosystems it has a manifest for (a mask of
    /// [`Ecosystem::bit`]).
    pub ecosystems: Vec<u8>,
}

/// A quoted string's contents at the start of `s` (after its opening quote).
fn quoted(s: &str) -> Option<&str> {
    let q = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let rest = &s[1..];
    rest.find(q).map(|i| &rest[..i])
}

/// The value of `key = "value"` on a TOML line.
fn toml_string<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix(key)?.trim_start();
    quoted(rest.strip_prefix('=')?.trim_start())
}

/// Every `path = "…"` value on a TOML line (inline tables included).
fn toml_paths(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("path") {
        let after = rest[i + 4..].trim_start();
        if let Some(v) = after.strip_prefix('=').and_then(|a| quoted(a.trim_start())) {
            out.push(v);
        }
        rest = &rest[i + 4..];
    }
    out
}

/// PEP 503 normalization of a Python project name.
fn pep503(name: &str) -> String {
    let mut out = String::new();
    let mut sep = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            sep = true;
        } else {
            if sep && !out.is_empty() {
                out.push('-');
            }
            sep = false;
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// The leading distribution name of a requirement string (`name[extra]>=1.0`).
fn requirement_name(req: &str) -> &str {
    let end = req
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(req.len());
    &req[..end]
}

/// `dir` joined with a relative manifest path, normalized, relative to the
/// repository root.
fn join_rel(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

#[derive(Default)]
struct Manifests {
    /// What each package names: (ecosystem, name) and repo-relative
    /// directories.
    names: Vec<Vec<(Ecosystem, String)>>,
    dirs: Vec<Vec<String>>,
}

/// Reads the manifests of every first-party package: `package.json`, `go.mod`,
/// `Cargo.toml` and `pyproject.toml` in its root. A dependency counts when it
/// names another first-party package (by name, or by a local path for
/// `replace`, `path = …` and `file:`/`link:`).
pub fn package_table(g: &Graph) -> PackageTable {
    let root = Path::new(g.root());
    let np = g.package_count() as usize;
    let mut table = PackageTable {
        deps: vec![Vec::new(); np],
        ecosystems: vec![0; np],
    };
    let mut by_name: HashMap<(Ecosystem, String), PackageId> = HashMap::new();
    let mut by_root: HashMap<String, PackageId> = HashMap::new();
    let mut m = Manifests {
        names: vec![Vec::new(); np],
        dirs: vec![Vec::new(); np],
    };
    for p in 0..np {
        let id = PackageId(p as u32);
        let info = g.package_info(id);
        if info.version != PackageVersion::Workspace {
            continue;
        }
        by_root.insert(info.root.to_owned(), id);
        let dir = root.join(info.root);
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        if let Some(text) = read("package.json")
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        {
            table.ecosystems[p] |= Ecosystem::Js.bit();
            if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                by_name.entry((Ecosystem::Js, n.to_owned())).or_insert(id);
            }
            for field in [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ] {
                let Some(o) = v.get(field).and_then(|o| o.as_object()) else {
                    continue;
                };
                for (name, range) in o {
                    m.names[p].push((Ecosystem::Js, name.clone()));
                    let range = range.as_str().unwrap_or("");
                    // `npm:<other>@…` aliases, and local paths.
                    if let Some(alias) = range.strip_prefix("npm:") {
                        let cut = alias[1.min(alias.len())..]
                            .find('@')
                            .map_or(alias.len(), |i| i + 1);
                        m.names[p].push((Ecosystem::Js, alias[..cut].to_owned()));
                    }
                    for prefix in ["file:", "link:", "portal:"] {
                        if let Some(rel) = range.strip_prefix(prefix)
                            && let Some(d) = join_rel(info.root, rel)
                        {
                            m.dirs[p].push(d);
                        }
                    }
                }
            }
        }
        if let Some(text) = read("go.mod") {
            table.ecosystems[p] |= Ecosystem::Go.bit();
            let mut block = "";
            for line in text.lines() {
                let line = line.split("//").next().unwrap_or("").trim();
                if line.is_empty() {
                    continue;
                }
                if line == ")" {
                    block = "";
                    continue;
                }
                let (directive, rest) = if block.is_empty() {
                    let (d, r) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
                    if r.trim() == "(" {
                        block = match d {
                            "require" => "require",
                            "replace" => "replace",
                            _ => "other",
                        };
                        continue;
                    }
                    (d, r.trim())
                } else {
                    (block, line)
                };
                match directive {
                    "module" => {
                        by_name
                            .entry((Ecosystem::Go, rest.trim_matches('"').to_owned()))
                            .or_insert(id);
                    }
                    "require" => {
                        if let Some(module) = rest.split_whitespace().next() {
                            m.names[p].push((Ecosystem::Go, module.to_owned()));
                        }
                    }
                    "replace" => {
                        if let Some((_, to)) = rest.split_once("=>") {
                            let to = to.split_whitespace().next().unwrap_or("");
                            if to.starts_with('.') {
                                if let Some(d) = join_rel(info.root, to) {
                                    m.dirs[p].push(d);
                                }
                            } else {
                                m.names[p].push((Ecosystem::Go, to.to_owned()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(text) = read("Cargo.toml") {
            table.ecosystems[p] |= Ecosystem::Rust.bit();
            let mut section = String::new();
            for line in text.lines() {
                let t = line.trim();
                if let Some(s) = t.strip_prefix('[') {
                    section = s.trim_end_matches(']').trim_matches('[').to_owned();
                    // `[dependencies.name]` tables.
                    if let Some((kind, name)) = section.rsplit_once('.')
                        && kind.ends_with("dependencies")
                    {
                        m.names[p].push((Ecosystem::Rust, name.trim_matches('"').to_owned()));
                    }
                    continue;
                }
                if section == "package" {
                    if let Some(n) = toml_string(t, "name") {
                        by_name.entry((Ecosystem::Rust, n.to_owned())).or_insert(id);
                    }
                } else if section.contains("dependencies") {
                    if let Some((name, _)) = t.split_once('=')
                        && section.ends_with("dependencies")
                    {
                        let name = name.trim().trim_matches('"');
                        let name = name.split('.').next().unwrap_or(name);
                        m.names[p].push((Ecosystem::Rust, name.to_owned()));
                        if let Some(n) = line.find("package").and_then(|i| {
                            quoted(line[i + 7..].trim_start().strip_prefix('=')?.trim_start())
                        }) {
                            m.names[p].push((Ecosystem::Rust, n.to_owned()));
                        }
                    }
                    for rel in toml_paths(t) {
                        if let Some(d) = join_rel(info.root, rel) {
                            m.dirs[p].push(d);
                        }
                    }
                }
            }
        }
        let py = read("pyproject.toml");
        if py.is_some() || dir.join("setup.py").is_file() {
            table.ecosystems[p] |= Ecosystem::Python.bit();
        }
        if let Some(text) = py {
            let mut section = String::new();
            for line in text.lines() {
                let t = line.trim();
                if let Some(s) = t.strip_prefix('[') {
                    section = s.trim_end_matches(']').trim_matches('[').to_owned();
                    continue;
                }
                if (section == "project" || section == "tool.poetry")
                    && let Some(n) = toml_string(t, "name")
                {
                    by_name.entry((Ecosystem::Python, pep503(n))).or_insert(id);
                }
                // Requirement strings, dependency keys and local sources, wherever they are:
                // a string or key that names another first-party project is a dependency.
                let mut rest = t;
                while let Some(i) = rest.find(['"', '\'']) {
                    let Some(s) = quoted(&rest[i..]) else { break };
                    m.names[p].push((Ecosystem::Python, pep503(requirement_name(s))));
                    rest = &rest[i + s.len() + 2..];
                }
                if section.contains("dependencies") || section.contains("sources") {
                    if let Some((name, _)) = t.split_once('=') {
                        m.names[p].push((Ecosystem::Python, pep503(name.trim().trim_matches('"'))));
                    }
                    for rel in toml_paths(t) {
                        if let Some(d) = join_rel(info.root, rel) {
                            m.dirs[p].push(d);
                        }
                    }
                }
            }
        }
        // The graph's own package name, for manifests this reader doesn't know.
        if table.ecosystems[p] == 0 {
            let eco = match info.name.split_once(':') {
                Some(("go", n)) => Some((Ecosystem::Go, n.to_owned())),
                Some(("pypi", n)) => Some((Ecosystem::Python, pep503(n))),
                Some(("crates", n)) => Some((Ecosystem::Rust, n.to_owned())),
                _ => None,
            };
            if let Some((eco, name)) = eco {
                table.ecosystems[p] |= eco.bit();
                by_name.entry((eco, name)).or_insert(id);
            }
        }
    }
    for p in 0..np {
        let mut deps: Vec<PackageId> = m.names[p]
            .iter()
            .filter_map(|k| by_name.get(k).copied())
            .chain(m.dirs[p].iter().filter_map(|d| by_root.get(d).copied()))
            .filter(|d| d.0 as usize != p)
            .collect();
        deps.sort_unstable();
        deps.dedup();
        table.deps[p] = deps;
    }
    table
}

/// An evaluated `affected` query, as [`run`] audits it.
pub struct Selection<'a> {
    /// The change set the selection was evaluated with (without ignored
    /// steering files).
    pub changes: &'a ChangeSet,
    /// The entries.
    pub entries: &'a NodeSet,
    /// The final report.
    pub report: &'a AffectedReport,
    /// The options of the evaluation.
    pub opts: &'a AffectedOptions,
    /// What the pruning rules did: which entries were evaluated without
    /// type-only edges, and which erased-equal files were skipped for them.
    pub pruned: &'a turborepo_trace_graph::barrel::PruneReport,
    /// Changed repo steering files, which count only through explicit
    /// references.
    pub steering: &'a [Change],
}

/// What [`run`] prints.
#[derive(Copy, Clone, Debug)]
pub struct Output<'a> {
    /// Sources to rank (`--top`).
    pub top: usize,
    /// The entries the JSON lists (`--audit-entries`).
    pub entries: &'a str,
    /// JSON instead of text.
    pub json: bool,
    /// Timings on stderr.
    pub stats: bool,
}

/// Runs the audit for an evaluated `affected` query and prints it: the ranked
/// sources as text, or the full report as JSON. `g` is the stored graph,
/// `engine` the engine of the evaluation (on the derived view of `g`).
///
/// The audit follows the passes of the evaluation: tests whose runner
/// transpiles file by file are audited without type-only edges and without the
/// erased-equal changes that were skipped for them, every other entry with
/// every edge and change.
// Invariant: `passes` gets one pass when it would be empty (above).
#[allow(clippy::expect_used)]
pub fn run(
    g: &Graph,
    engine: &Engine<'_, turborepo_trace_graph::Patched<'_, Graph>>,
    sel: &Selection<'_>,
    how: &Output<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let Output {
        top,
        entries: which,
        json,
        stats,
    } = *how;
    let Some(filter) = VerdictFilter::parse(which) else {
        bail!("--audit-entries {which:?}: use all, selected, flagged or none");
    };
    let t0 = Instant::now();
    let table = package_table(g);
    let t_table = t0.elapsed();
    let mut selected = NodeSet::new(g.node_count());
    for e in sel.report.with_verdict(Verdict::Affected) {
        selected.insert(e.entry);
    }
    let ignored: Vec<String> = sel.steering.iter().map(|c| c.path.clone()).collect();
    // (entries, traversal filter, changes) per pass.
    let mut passes: Vec<(NodeSet, Option<EdgeFilter>, ChangeSet)> = Vec::new();
    if let Some(runtime) = &sel.pruned.runtime_entries {
        let mut isolated = sel.entries.clone();
        isolated.intersect_with(runtime);
        let mut rest = sel.entries.clone();
        rest.difference_with(runtime);
        if !isolated.is_empty() {
            // Erased-equal files nothing reads count for no transpiling test, except for
            // a changed test itself.
            let erased: HashSet<&str> = sel
                .pruned
                .erased
                .iter()
                .filter(|s| s.relevant == 0 && !sel.entries.contains(s.node))
                .map(|s| g.path(s.node))
                .collect();
            let mut changes = sel.changes.clone();
            changes.files.retain(|c| !erased.contains(c.path.as_str()));
            let no_types = engine
                .options()
                .traversal
                .without_flags(RefFlags::TYPE_ONLY);
            passes.push((isolated, Some(no_types), changes));
        }
        if !rest.is_empty() {
            passes.push((rest, None, sel.changes.clone()));
        }
    }
    if passes.is_empty() {
        passes.push((sel.entries.clone(), None, sel.changes.clone()));
    }
    let audit = passes
        .iter()
        .map(|(entries, edges, changes)| {
            engine.audit(
                changes,
                entries,
                &selected,
                sel.opts,
                &AuditOptions {
                    top,
                    package_deps: Some(&table.deps),
                    package_ecosystems: Some(&table.ecosystems),
                    chains: json && filter != VerdictFilter::None,
                    filter: *edges,
                    ignored_steering: &ignored,
                },
            )
        })
        .reduce(|a, b| a.merge(b))
        .expect("at least one pass");
    if stats {
        let t = audit.timing;
        eprintln!(
            "audit: {} node(s), {} edge(s); {} first-party package(s) with {} declared \
             dependenc(ies); manifests {:.1} ms, graph {:.1} ms, signals {:.1} ms, greedy {:.1} \
             ms, sources {:.1} ms, total {:.1} ms",
            audit.graph.0,
            audit.graph.1,
            table.ecosystems.iter().filter(|e| **e != 0).count(),
            table.deps.iter().map(Vec::len).sum::<usize>(),
            t_table.as_secs_f64() * 1e3,
            t.graph_ms,
            t.signals_ms,
            t.greedy_ms,
            t.sources_ms,
            t0.elapsed().as_secs_f64() * 1e3
        );
    }
    if json {
        serde_json::to_writer_pretty(&mut *out, &audit.to_value(engine.graph(), filter))?;
        writeln!(out)?;
    } else {
        audit.write_text(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_helpers() {
        assert_eq!(
            toml_string(r#"name = "my-crate""#, "name"),
            Some("my-crate")
        );
        assert_eq!(toml_string("name='x'", "name"), Some("x"));
        assert_eq!(toml_string("rename = \"x\"", "name"), None);
        assert_eq!(
            toml_paths(r#"a = { path = "../a" }, b = {path="../b", version = "1"}"#),
            ["../a", "../b"]
        );
        assert_eq!(pep503("My_Pkg.Name"), "my-pkg-name");
        assert_eq!(requirement_name("requests[socks]>=2"), "requests");
        assert_eq!(
            join_rel("services/a", "../../libs/x").as_deref(),
            Some("libs/x")
        );
        assert_eq!(join_rel("a", "../../x"), None);
        assert_eq!(join_rel("", "./x/").as_deref(), Some("x"));
    }
}
