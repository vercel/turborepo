//! What routes invoke at runtime: the static relation "this page (or handler,
//! layout, or a component in its closure) invokes these routes, procedures and
//! actions".
//!
//! Two layers:
//!
//! - **Sites** ([`Site`], per source file): `fetch` and fetch wrappers, SWR and
//!   TanStack Query hooks, tRPC procedure calls, server actions, redirects and
//!   navigations, each with its targets ([`SiteTarget`]: a URL pattern, a
//!   procedure path, an action, or unknown).
//! - **The relation** ([`InvokeRelation`], per route): the sites of the route's
//!   module closure, with URL targets mapped through the route table
//!   ([`relation`]).
//!
//! An unknown target stays in the relation as [`Target::Unknown`]. By default
//! it is bounded ([`RelationOptions::bound_unknown`]): a request whose URL
//! cannot be evaluated reaches only the handler routes that a URL-like string
//! of the route's module closure matches; without the bound it stands for every
//! handler. The unknown sites are the work order for further evaluation.
//!
//! Every unknown names where its value comes from
//! ([`crate::nextjs::source::Source`], the ` [request]`-style end of its
//! reason): a parameter, a call, the environment, a response, or, outside the
//! code, the request, storage and what the user enters. The bound rests on
//! the code taking the URL from somewhere in its closure, so it does not apply
//! to the last three: such a URL can be any, and the site stands for every
//! handler.
//!
//! | Idiom | Module |
//! |---|---|
//! | `fetch`, fetch wrappers (derived across files), axios/ky/ofetch, SWR keys and fetchers, redirects, navigations | `scan` |
//! | server actions | `actions` |
//! | tRPC procedures, router tree, HTTP entry | `trpc` |
//! | route mapping, module closure, bound | [`relation`] |
//!
//! The target of a link or a redirect is requested like any URL, so the app's
//! middleware runs for it: where it does, the target also maps to the routes
//! behind a prefix the middleware can put in front of the path (a locale, a
//! flags code, a tenant), with the logic that matches the URLs a spec names
//! ([`crate::nextjs::matcher::Matcher::behind_prefix`]). The routes Next's
//! precedence picks behind leading dynamic segments are ordinary targets; every
//! other route the prefixed path can hit is a [`Invocation::wide`] target,
//! unless the middleware's own code redirects or rewrites to a URL that is not
//! known (then a request can end on any page, and the relation says so with
//! that unknown target).
//!
//! A JSX element of a component with an `href` attribute is a navigation site,
//! like `<Link href>` and `<a href>`: an app's own link component hands its
//! props on (`<NextLink {...props} />`), so the URL is written where the
//! component is used.
//!
//! Debugging (stderr): `TURBO_TRACE_DEBUG_INVOKE_DEBUG=1` lists the derived
//! wrappers, `TURBO_TRACE_DEBUG_INVOKE_TRACE=<path substring>` shows how the
//! calls of matching files resolve and what their sites evaluate to,
//! `TURBO_TRACE_DEBUG_INVOKE_TIMING=1` prints phase timings, and
//! `TURBO_TRACE_DEBUG_EVAL_TRACE=1` reports where the evaluator gave up.

mod actions;
pub mod model;
pub mod relation;
mod scan;
pub mod shared;
mod trpc;

use std::path::{Path, PathBuf};

pub use model::{
    Invocation, InvokeKind, InvokeRelation, InvokeStats, Procedure, Site, SiteTarget, Target,
    UrlString,
};
pub use relation::{
    MappedSite, Prefix, RelationOptions, TrpcIndex, map_sites, outside_input, relation, route_key,
    spelled_prefixes, unknown_class, unknown_sites, unknown_source,
};
pub use shared::{FileInvokes, RouteSets, SharedRelation, shared_relation};

use super::{
    NextApp,
    fetch::{UrlApi, UrlSite, UrlUse},
    jseval::ImportResolver,
};

/// The result of scanning a set of files.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    /// Sites, sorted by file and line.
    pub sites: Vec<Site>,
    /// URL-like strings: the bound for unknown targets.
    pub strings: Vec<UrlString>,
    /// tRPC procedures of the router trees: `(path, kind, file, line)`.
    pub procedures: Vec<(String, String, PathBuf, u32)>,
    /// Files with a tRPC HTTP adapter call.
    pub trpc_handlers: Vec<PathBuf>,
    /// Number of derived wrappers.
    pub wrappers: usize,
}

impl Scan {
    /// The tRPC procedures and HTTP entries of `app`.
    pub fn trpc_index(&self, app: &NextApp, graph_root: &Path) -> TrpcIndex {
        let rel = |p: &Path| {
            p.strip_prefix(graph_root)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned()
        };
        let procedures = self
            .procedures
            .iter()
            .map(|(path, kind, file, line)| Procedure {
                path: path.clone(),
                kind: kind.clone(),
                file: rel(file),
                line: *line,
            })
            .collect();
        let handlers = app
            .table
            .routes
            .iter()
            .filter(|r| self.trpc_handlers.contains(&app.root.join(&r.file)))
            .map(route_key)
            .collect();
        TrpcIndex {
            procedures,
            handlers,
        }
    }
}

/// Scans `files` (absolute paths of first-party code) for invocation sites.
pub fn scan(files: &[PathBuf], apis: &[UrlApi], resolver: &dyn ImportResolver) -> Scan {
    let cache = super::jseval::Files::new(resolver);
    let timing = turborepo_trace_core::debug::var_os("INVOKE_TIMING").is_some();
    let t0 = std::time::Instant::now();
    cache.preload(files);
    let t_parse = t0.elapsed();
    let urls = scan::scan_urls(&cache, files, apis);
    let t1 = std::time::Instant::now();
    let mut sites = urls.sites;
    sites.extend(actions::scan_actions(&cache, files));
    let t2 = std::time::Instant::now();
    let t = trpc::scan_trpc(&cache, files);
    if timing {
        eprintln!("invoke timing: parse {} ms", t_parse.as_millis());
        eprintln!(
            "invoke timing: urls {} ms, actions {} ms, trpc {} ms",
            (t1 - t0).as_millis(),
            (t2 - t1).as_millis(),
            t2.elapsed().as_millis()
        );
    }
    sites.extend(t.sites);
    sites.sort_by(|a, b| (&a.file, a.line, a.kind, &a.api).cmp(&(&b.file, b.line, b.kind, &b.api)));
    sites.dedup();
    Scan {
        sites,
        strings: urls.strings,
        procedures: t
            .procedures
            .into_iter()
            .map(|p| (p.path, p.kind, p.file, p.line))
            .collect(),
        trpc_handlers: t.handlers,
        wrappers: urls.wrappers,
    }
}

const CODE_EXT: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

/// The first-party code files of a graph (absolute paths): the files to scan.
pub fn graph_code_files<G: turborepo_trace_graph::GraphView>(
    g: &G,
    graph_root: &Path,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for i in 0..g.node_count() {
        let n = turborepo_trace_graph::NodeId(i);
        if g.is_external(n) {
            continue;
        }
        let p = g.path(n);
        if p.contains("node_modules/") || p.ends_with(".d.ts") {
            continue;
        }
        let built = ["/.next/", "/dist/", "/build/", "/out/", ".min.js"]
            .iter()
            .any(|x| p.contains(x));
        if !built
            && p.rsplit_once('.')
                .is_some_and(|(_, e)| CODE_EXT.contains(&e))
        {
            out.push(graph_root.join(p));
        }
    }
    out.sort();
    out
}

/// The relation of `app` over `graph` (paths relative to `graph_root`) in its
/// shared form, with the default API table plus the `[[nextjs.url_apis]]` of
/// `turbotrace.toml` in the graph root and in the app directory.
pub fn static_relation<G: turborepo_trace_graph::GraphView>(
    app: &NextApp,
    graph: &G,
    graph_root: &Path,
    bound: bool,
) -> (Scan, SharedRelation) {
    let files = graph_code_files(graph, graph_root);
    let resolver = super::select::GraphImports::from_graph(graph, graph_root);
    let scan = static_scan(&files, &resolver, graph_root, &[&app.root]);
    let mut rel = relation_of(app, graph, graph_root, &scan, bound);
    rel.stats.files = files.len();
    (scan, rel)
}

/// Scans `files` with the default API table plus the `[[nextjs.url_apis]]` of
/// `turbotrace.toml` in the graph root and in each of `app_roots`. One scan
/// serves the relations of several apps ([`relation_of`]).
pub fn static_scan(
    files: &[PathBuf],
    resolver: &dyn ImportResolver,
    graph_root: &Path,
    app_roots: &[&Path],
) -> Scan {
    scan(files, &url_apis(graph_root, app_roots).0, resolver)
}

/// The API table of a scan: the defaults plus the `[[nextjs.url_apis]]` of
/// `turbotrace.toml` in the graph root and in each of `app_roots`, and one
/// message per file whose `[nextjs]` table cannot be read (`<file>: <error>`).
/// The entries of such a file are not in the table, so a caller that selects by
/// the relation reports the message instead of going on without the wrappers
/// the file declares.
pub fn url_apis(graph_root: &Path, app_roots: &[&Path]) -> (Vec<UrlApi>, Vec<String>) {
    let mut apis = super::fetch::default_url_apis();
    let mut errors = Vec::new();
    let mut tomls = vec![graph_root.join("turbotrace.toml")];
    tomls.extend(app_roots.iter().map(|a| a.join("turbotrace.toml")));
    if !turborepo_trace_core::project::reads_turbotrace_toml() {
        tomls.clear();
    }
    tomls.sort();
    tomls.dedup();
    for f in tomls {
        if let Ok(text) = crate::fsx::read_to_string(&f) {
            match super::fetch::parse_url_apis_toml(&text) {
                Ok(more) => apis.extend(more),
                Err(e) => errors.push(format!("{}: {}", f.display(), e.trim_end())),
            }
        }
    }
    (apis, errors)
}

/// The first-party code files (absolute paths) that the nodes `seeds` load:
/// their closure over runtime module edges (imports, `require`, `import()`,
/// re-exports; type-only edges are not followed). The files to scan for the
/// relation of the routes that start there.
pub fn code_closure<G: turborepo_trace_graph::GraphView>(
    g: &G,
    graph_root: &Path,
    seeds: &[turborepo_trace_graph::NodeId],
) -> Vec<PathBuf> {
    let mut seen = vec![false; g.node_count() as usize];
    let mut stack: Vec<turborepo_trace_graph::NodeId> = Vec::new();
    for &s in seeds {
        if !std::mem::replace(&mut seen[s.0 as usize], true) {
            stack.push(s);
        }
    }
    let mut out = Vec::new();
    while let Some(n) = stack.pop() {
        if g.is_external(n) {
            continue;
        }
        let p = g.path(n);
        if p.rsplit_once('.')
            .is_some_and(|(_, e)| CODE_EXT.contains(&e))
            && !p.ends_with(".d.ts")
        {
            out.push(graph_root.join(p));
        }
        for e in g.out_edges(n) {
            if let Some(to) = e.to
                && relation::follows(e.kind(), e.flags())
                && !std::mem::replace(&mut seen[to.0 as usize], true)
            {
                stack.push(to);
            }
        }
    }
    out.sort();
    out
}

/// The relation of `app` from a scan ([`static_scan`]) of at least the files
/// its routes load.
pub fn relation_of<G: turborepo_trace_graph::GraphView>(
    app: &NextApp,
    graph: &G,
    graph_root: &Path,
    scan: &Scan,
    bound: bool,
) -> SharedRelation {
    let opts = RelationOptions {
        graph_root: graph_root.to_path_buf(),
        bound_unknown: bound,
        ..RelationOptions::default()
    };
    let trpc = scan.trpc_index(app, graph_root);
    let t0 = std::time::Instant::now();
    let mut rel = shared::shared_relation(app, graph, &scan.sites, &scan.strings, &trpc, &opts);
    if turborepo_trace_core::debug::var_os("INVOKE_TIMING").is_some() {
        eprintln!(
            "invoke timing: relation {} ms ({} sites, {} url-like strings, {} files with sites, \
             {} sets)",
            t0.elapsed().as_millis(),
            scan.sites.len(),
            scan.strings.len(),
            rel.files.len(),
            rel.sets.len()
        );
    }
    rel.stats.wrappers = scan.wrappers;
    rel
}

/// External targets listed one by one per route in [`to_build`]; beyond it they
/// are summed per origin.
const EXTERNALS_LISTED: usize = 25;

/// A relation in the shape the build fingerprints combine
/// ([`crate::nextjs::build::invokes`]): per route, each target once.
///
/// - A route's invocation of itself is left out.
/// - An unknown redirect target is left out (the visitor is sent on to a page;
///   whoever follows visits it).
/// - In a bounded relation an unknown target is left out, the routes of its
///   bound stand for it; a URL supplied by the request, storage or an input
///   ([`outside_input`]) stays unknown.
/// - A target that only an arbitrary middleware prefix reaches
///   ([`Invocation::wide`]) is left out.
/// - External targets do not enter a fingerprint. A route with more than a few
///   is given one entry per origin with the number of endpoints, so that a
///   relation of a large app stays small; `next routes --source` has every
///   endpoint.
pub fn to_build(rel: &SharedRelation) -> crate::nextjs::build::invokes::InvokeRelation {
    use crate::nextjs::build::invokes as b;
    let kind = |k: InvokeKind| match k {
        InvokeKind::Fetch => b::InvokeKind::Fetch,
        InvokeKind::Swr => b::InvokeKind::Swr,
        InvokeKind::Query => b::InvokeKind::Query,
        InvokeKind::Trpc => b::InvokeKind::Trpc,
        InvokeKind::Action => b::InvokeKind::Action,
        InvokeKind::Redirect => b::InvokeKind::Redirect,
        InvokeKind::Navigation => b::InvokeKind::Navigation,
    };
    let mut out = b::InvokeRelation::new();
    for (key, r) in &rel.routes {
        let entries = rel.entries(r);
        let externals = entries
            .iter()
            .filter(|e| matches!(e.first.target, Target::External { .. }))
            .count();
        let mut origins: std::collections::BTreeMap<
            (InvokeKind, &str),
            (usize, &shared::Entry<'_>),
        > = std::collections::BTreeMap::new();
        let mut list = Vec::new();
        for e in &entries {
            let i = e.first;
            // Not a target by Next's precedence.
            if e.wide {
                continue;
            }
            let target = match &i.target {
                Target::Route(k) if k == key => continue,
                Target::Unknown(_) if i.kind == InvokeKind::Redirect => continue,
                Target::Unknown(why)
                    if rel.stats.bounded && i.kind.is_dependency() && !outside_input(why) =>
                {
                    continue;
                }
                Target::Route(k) => b::InvokeTarget::Route(k.clone()),
                Target::Action(a) => b::InvokeTarget::Action(a.clone()),
                Target::Procedure {
                    path,
                    file,
                    serving,
                } => b::InvokeTarget::Procedure {
                    path: path.clone(),
                    file: file.clone().unwrap_or_default(),
                    serving: serving.clone(),
                },
                Target::External { origin, .. } if externals > EXTERNALS_LISTED => {
                    let o = origins.entry((i.kind, origin.as_str())).or_insert((0, e));
                    o.0 += 1;
                    continue;
                }
                Target::External { origin, pattern } => {
                    b::InvokeTarget::External(format!("{origin}{pattern}"))
                }
                Target::Unknown(r) => b::InvokeTarget::Unknown(r.clone()),
            };
            list.push(b::Invocation {
                kind: kind(i.kind),
                target,
                found: b::Found::Static,
                file: e.file.to_string(),
                line: i.line,
                method: if e.one_method { i.method.clone() } else { None },
                via: i.via.clone(),
            });
        }
        for ((k, origin), (n, e)) in origins {
            let name = if origin.is_empty() {
                "same origin"
            } else {
                origin
            };
            list.push(b::Invocation {
                kind: kind(k),
                target: b::InvokeTarget::External(format!("{name}: {n} endpoints")),
                found: b::Found::Static,
                file: e.file.to_string(),
                line: e.first.line,
                method: None,
                via: e.first.via.clone(),
            });
        }
        for (k, file, line, _) in rel.bound_witnesses(r) {
            let target = &rel.route_keys[k as usize];
            if target == key {
                continue;
            }
            list.push(b::Invocation {
                kind: b::InvokeKind::Fetch,
                target: b::InvokeTarget::Route(target.clone()),
                found: b::Found::Static,
                file: file.to_string(),
                line,
                method: None,
                via: vec!["url-like string".to_string()],
            });
        }
        if !list.is_empty() {
            out.routes.insert(key.clone(), list);
        }
    }
    out
}

/// [`static_relation`] (bounded) in the shape the build fingerprints combine
/// ([`to_build`]).
pub fn build_relation<G: turborepo_trace_graph::GraphView>(
    app: &NextApp,
    graph: &G,
    graph_root: &Path,
) -> crate::nextjs::build::invokes::InvokeRelation {
    to_build(&static_relation(app, graph, graph_root, true).1)
}

/// Reads a relation file for the build fingerprints: the shared form written by
/// `next routes --source --format json` (version 2), or per-route lists
/// (version 1: the expanded form, or a relation written from recorded
/// requests).
pub fn read_relation(
    bytes: &[u8],
) -> Result<crate::nextjs::build::invokes::InvokeRelation, String> {
    #[derive(serde::Deserialize)]
    struct Version {
        #[serde(default)]
        version: u32,
    }
    let v: Version = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if v.version >= 2 {
        let rel: SharedRelation = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        return Ok(to_build(&rel));
    }
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

/// Converts the sites of [`super::select::scan_sites`] (the first-generation
/// discovery) to invocation sites. Forwarded sites (a wrapper's inner call,
/// whose callers are the sites) and built URLs are dropped.
pub fn from_url_sites(sites: &[UrlSite]) -> Vec<Site> {
    let mut out = Vec::new();
    for s in sites {
        if s.forwarded.is_some() || s.kind == UrlUse::Build {
            continue;
        }
        let kind = match s.kind {
            UrlUse::Fetch if s.api.starts_with("swr") => InvokeKind::Swr,
            UrlUse::Fetch => InvokeKind::Fetch,
            UrlUse::Redirect => InvokeKind::Redirect,
            UrlUse::Navigation | UrlUse::Build => InvokeKind::Navigation,
        };
        let mut targets: Vec<SiteTarget> = s.urls.iter().cloned().map(SiteTarget::Url).collect();
        targets.extend(s.unresolved.iter().cloned().map(SiteTarget::Unknown));
        if targets.is_empty() {
            continue;
        }
        out.push(Site {
            file: s.file.clone(),
            line: s.line,
            kind,
            api: s.api.clone(),
            method: None,
            targets,
            via: vec![s.api.clone()],
        });
    }
    out
}
