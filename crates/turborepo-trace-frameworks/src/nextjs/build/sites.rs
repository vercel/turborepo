//! The invoked-routes relation from the static URL-site discovery that exists
//! today ([`crate::nextjs::fetch`], [`crate::nextjs::select::scan_sites`] and
//! [`crate::nextjs::select::resolve_sites`]): an adapter, not an analysis of
//! its own.
//!
//! For each route, every URL site in the import closure of its segment files
//! (page or handler, layouts, boundaries, slot pages) becomes an invocation:
//!
//! | Site | Invocation |
//! |---|---|
//! | a request (`fetch`, SWR, axios, wrappers) hitting routes | `fetch` (`swr` for SWR) per route |
//! | a server-side redirect | `redirect` per route |
//! | a client navigation | `navigation` per route (listed, not combined) |
//! | a URL to another origin or another microfrontend | external |
//! | a request whose URL could not be evaluated | unknown (stands for every handler) |
//!
//! A dedicated extraction (wrappers through cross-file summaries, React Query,
//! tRPC, server action call sites) replaces this when it exists; the relation's
//! shape stays.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
};

use turborepo_trace_graph::{GraphView, NodeId};

use super::invokes::{Found, Invocation, InvokeKind, InvokeRelation, InvokeTarget, source_key};
use crate::nextjs::{
    NextApp,
    fetch::{UrlUse, default_url_apis},
    matcher::RequestContext,
    select::{GraphImports, ResolvedSite, resolve_sites, scan_sites},
};

const CODE_EXT: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

fn closure<G: GraphView>(g: &G, seeds: &[NodeId]) -> HashSet<u32> {
    let mut seen: HashSet<u32> = seeds.iter().map(|n| n.0).collect();
    let mut stack: Vec<NodeId> = seeds.to_vec();
    while let Some(n) = stack.pop() {
        for e in g.out_edges(n) {
            if let Some(t) = e.to
                && !g.is_external(t)
                && seen.insert(t.0)
            {
                stack.push(t);
            }
        }
    }
    seen
}

/// The relation for `app` over `graph` (paths relative to `graph_root`).
pub fn relation_from_sites<G: GraphView>(
    app: &NextApp,
    graph: &G,
    graph_root: &Path,
) -> InvokeRelation {
    // First-party code files of the graph.
    let mut scan: Vec<PathBuf> = Vec::new();
    for i in 0..graph.node_count() {
        let n = NodeId(i);
        let p = graph.path(n);
        if graph.is_external(n) || p.contains("node_modules/") || p.ends_with(".d.ts") {
            continue;
        }
        let built = ["/.next/", "/dist/", "/build/", "/out/", ".min.js"]
            .iter()
            .any(|x| p.contains(x));
        if !built
            && p.rsplit_once('.')
                .is_some_and(|(_, e)| CODE_EXT.contains(&e))
        {
            scan.push(graph_root.join(p));
        }
    }
    let imports = GraphImports::from_graph(graph, graph_root);
    let raw = scan_sites(&scan, &default_url_apis(), &imports);
    let resolved: Vec<ResolvedSite> = resolve_sites(app, &raw, &RequestContext::default());
    let rel = |p: &Path| -> String {
        p.strip_prefix(graph_root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    };
    let mut by_file: HashMap<u32, Vec<&ResolvedSite>> = HashMap::new();
    for s in &resolved {
        if s.site.kind == UrlUse::Build || s.site.forwarded.is_some() {
            continue;
        }
        if let Some(n) = graph.node_by_path(&rel(&s.site.file)) {
            by_file.entry(n.0).or_default().push(s);
        }
    }

    let mut out = InvokeRelation::new();
    for r in &app.table.routes {
        // Slot pages and defaults are part of the routes that render them.
        if r.slot.is_some() && r.intercept.is_none() {
            continue;
        }
        let seeds: Vec<NodeId> = app
            .table
            .segment_closure(r)
            .files
            .iter()
            .filter_map(|f| graph.node_by_path(&rel(&app.root.join(f))))
            .collect();
        let mut invs: BTreeSet<Invocation> = BTreeSet::new();
        for f in closure(graph, &seeds) {
            for s in by_file.get(&f).map(Vec::as_slice).unwrap_or(&[]) {
                let kind = match s.site.kind {
                    UrlUse::Fetch if s.site.api.starts_with("swr") => InvokeKind::Swr,
                    UrlUse::Fetch => InvokeKind::Fetch,
                    UrlUse::Redirect => InvokeKind::Redirect,
                    UrlUse::Navigation => InvokeKind::Navigation,
                    UrlUse::Build => continue,
                };
                let mut add = |target: InvokeTarget| {
                    invs.insert(Invocation {
                        kind,
                        target,
                        found: Found::Static,
                        file: rel(&s.site.file),
                        line: s.site.line,
                        method: None,
                        via: vec![s.site.api.clone()],
                    });
                };
                for id in &s.routes {
                    if let Some(t) = app.table.route(id)
                        && t.id != r.id
                    {
                        add(InvokeTarget::Route(source_key(&t.file)));
                    }
                }
                if s.external > 0 {
                    add(InvokeTarget::External("another origin".to_string()));
                }
                if s.microfrontend > 0 {
                    add(InvokeTarget::External("another microfrontend".to_string()));
                }
                if s.unresolved > 0 {
                    add(InvokeTarget::Unknown(
                        s.site
                            .unresolved
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "URL not evaluable".to_string()),
                    ));
                }
            }
        }
        if !invs.is_empty() {
            out.routes
                .insert(source_key(&r.file), invs.into_iter().collect());
        }
    }
    out
}
