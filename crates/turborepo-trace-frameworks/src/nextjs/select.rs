//! Route-level e2e selection: which Playwright specs a change set affects.
//!
//! [`select_e2e`] builds an [`Overlay`] over the repo graph and runs the
//! ordinary `affected` reverse BFS
//! ([`turborepo_trace_graph::Engine::affected`]) with the spec files as
//! entries. The overlay adds:
//!
//! | Node | Edges to | Meaning |
//! |---|---|---|
//! | spec file | `route:<file>` per visited route, `next:global`, `next:routing` | what the spec renders |
//! | `route:<file>` | the route's segment files (page, layouts, templates, boundaries, slot pages/defaults, metadata) | their import closures follow from the base graph |
//! | `route-soft:<file>` | the route plus intercepting routes of its slots | a soft (client) navigation to the route |
//! | `next:global` | global inputs, `public/**` and `.env*` memberships, probes, `next:tailwind` | inputs of every route |
//! | `next:routing` | `next.config` (and helpers), middleware, microfrontends; route-table memberships | inputs that decide which route a URL hits |
//! | `next:tailwind` | changed files Tailwind scans (strict) or whose class tokens changed (heuristic) | the generated stylesheet |
//! | a file with a `fetch`/SWR/`redirect()` site | `route:<file>` of every handler or page the URL overlaps | runtime requests found statically |
//! | `next:any-handler` | every handler route | target of fetch sites whose URL is unknown |
//! | `next:any-route` | every page route | target of specs with unresolved navigations |
//!
//! Route-table changes ([`RouteTableMode::Rematch`], the default): an added or
//! deleted `page`/`route` file becomes a node that specs depend on if one of
//! their visited URLs overlaps the file's route pattern; an added or deleted
//! layout-level file becomes a node the routes below its directory depend on.
//! [`RouteTableMode::Membership`] instead puts a membership over every special
//! file on `next:routing` (any added or removed route file selects every spec).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use serde::Serialize;
use turborepo_trace_graph::{
    AffectedOptions, Cause, ChangeKind, ChangeSet, Engine, GraphView, NodeId, NodeSet, Verdict,
};

use super::{
    NextApp,
    fetch::{UrlApi, UrlSite, UrlUse, Wrapper, scan_file},
    jseval::{Files, ImportResolver},
    matcher::{Ambiguity, Outcome, RequestContext},
    overlay::Overlay,
    routes::{DirKind, Route, RouteKind, Router, Seg, parse_dir_name, parse_pattern, special_name},
    tailwind::{TailwindMode, css_affected},
    urlpat::{Origin, Part, UrlPat, overlaps},
};
use crate::playwright::visits::SpecVisits;

/// Which routes a templated URL selects.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityPolicy {
    /// Every route some value of the unknown parts could hit (sound, default).
    #[default]
    Candidates,
    /// Assume unknown parts never equal a fixed segment name (Next's sorter
    /// picks the dynamic route); the assumption is reported per spec.
    Heuristic,
}

/// How added and removed route files are handled.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteTableMode {
    /// Re-match: only specs whose URLs the changed route pattern overlaps
    /// (default).
    #[default]
    Rematch,
    /// A membership over every route file: any added or removed one selects all
    /// specs.
    Membership,
}

/// Old and new file contents (for the Tailwind token heuristic).
pub trait Contents {
    /// Contents before the change (`None`: did not exist).
    fn before(&self, path: &str) -> Option<Vec<u8>>;
    /// Contents after the change (`None`: deleted).
    fn after(&self, path: &str) -> Option<Vec<u8>>;
}

/// Options for [`select_e2e`].
pub struct SelectOptions<'a> {
    /// The root the graph's paths are relative to.
    pub graph_root: PathBuf,
    /// Templated URLs.
    pub ambiguity: AmbiguityPolicy,
    /// Route-table changes.
    pub route_table: RouteTableMode,
    /// Tailwind model.
    pub tailwind: TailwindMode,
    /// Contents for [`TailwindMode::Tokens`] (without them, tokens mode acts as
    /// strict).
    pub contents: Option<&'a dyn Contents>,
    /// Tokens of every scanned file, for the "no other file has it" refinement.
    pub tailwind_others: Option<&'a BTreeSet<String>>,
    /// Resolved fetch sites ([`resolve_sites`]); `None`: no fetch discovery
    /// (handlers are then reachable only through visits).
    pub fetch: Option<&'a [ResolvedSite]>,
    /// Unknown fetch URLs may call any handler (default true).
    pub unresolved_fetch_any_handler: bool,
    /// Request context for rule conditions.
    pub context: RequestContext,
    /// `affected` options (uncertainty policy).
    pub affected: AffectedOptions,
}

impl SelectOptions<'_> {
    /// Defaults for a graph rooted at `graph_root`.
    pub fn new(graph_root: impl Into<PathBuf>) -> Self {
        SelectOptions {
            graph_root: graph_root.into(),
            ambiguity: AmbiguityPolicy::default(),
            route_table: RouteTableMode::default(),
            tailwind: TailwindMode::default(),
            contents: None,
            tailwind_others: None,
            fetch: None,
            unresolved_fetch_any_handler: true,
            context: RequestContext::default(),
            affected: AffectedOptions::default(),
        }
    }
}

/// Why a spec is affected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Reason {
    /// The spec file changed.
    SpecChanged,
    /// A module the spec imports (helper, page object, fixture) changed.
    SpecImports,
    /// A visited route's contents changed.
    Route(String),
    /// A route a visited route's code requests (fetch site) changed.
    FetchedRoute(String),
    /// A global input changed.
    GlobalInput,
    /// A route-matching input (config, middleware, microfrontends) changed.
    RouteMatching,
    /// A route file was added or removed where it can change which route a
    /// visited URL hits, or which layouts wrap a visited route.
    RouteTable,
    /// The Tailwind stylesheet changed.
    TailwindCss,
    /// The spec has unresolved navigations (may visit any route).
    IncompleteVisits,
    /// Code on the spec's routes requests unknown URLs (may call any handler).
    UnresolvedFetch,
    /// Uncertainty in the closure (policy).
    Uncertain,
}

/// One spec's verdict.
#[derive(Clone, Debug, Serialize)]
pub struct SpecVerdict {
    /// The spec (absolute).
    pub spec: PathBuf,
    /// Verdict.
    pub verdict: Verdict,
    /// Why (for affected specs).
    pub reason: Option<Reason>,
    /// The witness chain, changed node first.
    pub witness: Vec<String>,
    /// The cause of the first witness node.
    pub cause: Option<String>,
    /// Routes the spec visits (ids).
    pub routes: Vec<String>,
    /// Unresolved navigations.
    pub unresolved: Vec<String>,
    /// Ambiguities met while matching its URLs.
    pub ambiguities: Vec<String>,
}

/// The selection result.
#[derive(Clone, Debug, Serialize)]
pub struct E2eReport {
    /// Per spec, in input order.
    pub specs: Vec<SpecVerdict>,
    /// Changes that matched nothing.
    pub unmatched: Vec<String>,
}

impl E2eReport {
    /// Affected (or unknown) specs.
    pub fn selected(&self) -> impl Iterator<Item = &SpecVerdict> {
        self.specs
            .iter()
            .filter(|s| s.verdict != Verdict::Unaffected)
    }
}

/// Where a fetch site's URLs lead.
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedSite {
    /// The site.
    pub site: UrlSite,
    /// Route ids hit.
    pub routes: Vec<String>,
    /// URLs to external origins (absolute, or rewritten to one).
    pub external: usize,
    /// URLs rewritten or redirected internally.
    pub rewritten: usize,
    /// URLs served by another microfrontend.
    pub microfrontend: usize,
    /// URLs with no route.
    pub not_found: usize,
    /// Alternatives that could not be pinned down (plus the site's own
    /// unresolved list).
    pub unresolved: usize,
}

/// Scans files for URL sites, deriving wrappers (functions that forward a URL
/// parameter to a URL API) and scanning their call sites too, up to `levels`
/// rounds.
pub fn scan_sites(
    files: &[PathBuf],
    apis: &[UrlApi],
    resolver: &dyn ImportResolver,
) -> Vec<UrlSite> {
    scan_sites_with(files, apis, resolver, 4).0
}

/// [`scan_sites`] with the round limit; also returns the derived wrappers.
pub fn scan_sites_with(
    files: &[PathBuf],
    apis: &[UrlApi],
    resolver: &dyn ImportResolver,
    levels: u32,
) -> (Vec<UrlSite>, Vec<Wrapper>) {
    let cache = Files::new(resolver);
    let mut out = Vec::new();
    let mut wrappers: Vec<Wrapper> = Vec::new();
    let mut new: Vec<Wrapper> = Vec::new();
    for f in files {
        let (s, w) = scan_file(&cache, f, apis, &[]);
        out.extend(s);
        for x in w {
            if !new.contains(&x) {
                new.push(x);
            }
        }
    }
    for _ in 1..levels {
        if new.is_empty() {
            break;
        }
        let round = std::mem::take(&mut new);
        wrappers.extend(round.iter().cloned());
        for f in files {
            let (s, w) = scan_file(&cache, f, &[], &round);
            out.extend(s.into_iter().filter(|x| x.api.contains(" (wraps ")));
            for x in w {
                if !wrappers.contains(&x) && !new.contains(&x) {
                    new.push(x);
                }
            }
        }
    }
    // Wrappers derived in the last round stay unscanned: their sites count as
    // unresolved.
    if !new.is_empty() {
        for s in &mut out {
            if let Some(w) = &s.forwarded
                && new.iter().any(|n| n.name == *w && n.file == s.file)
            {
                s.unresolved
                    .push(format!("forwarded to `{w}` (wrapper depth limit)"));
                s.forwarded = None;
            }
        }
    }
    (out, wrappers)
}

/// Resolves sites against the app: overlap matching after rules.
pub fn resolve_sites(app: &NextApp, sites: &[UrlSite], ctx: &RequestContext) -> Vec<ResolvedSite> {
    let m = app.matcher().with_context(ctx.clone());
    sites
        .iter()
        .map(|s| {
            let mut r = ResolvedSite {
                site: s.clone(),
                routes: Vec::new(),
                external: 0,
                rewritten: 0,
                microfrontend: 0,
                not_found: 0,
                unresolved: s.unresolved.len(),
            };
            for u in &s.urls {
                let mm = m.match_pat(u, s.kind == UrlUse::Navigation);
                if mm
                    .via
                    .iter()
                    .any(|v| v.starts_with("rewrite") || v.starts_with("redirect"))
                {
                    r.rewritten += 1;
                }
                match mm.outcome {
                    Outcome::External(_) => r.external += 1,
                    Outcome::Microfrontend(_) => r.microfrontend += 1,
                    Outcome::Public(_) => {}
                    Outcome::NotFound => {
                        if mm.candidates.is_empty() {
                            r.not_found += 1;
                        }
                    }
                    Outcome::Routes => {}
                }
                // Overlap semantics: every candidate, ...
                let mut ids: Vec<&String> = mm.candidates.iter().chain(&mm.routes).collect();
                // ... except that a literal segment of the URL that names a static route
                // directory goes there: candidates reaching it only through a dynamic segment
                // (`/api/aliases/**` vs `/[team]/[project]/[[...rest]]`) would need the
                // whole static subtree to miss, which a request the code wrote for a handler
                // does not intend. Not applied through rewrites.
                // The static directory is looked up in the whole route table, so a request
                // for a missing handler (`/api/search`) finds nothing rather than the dynamic
                // page.
                if s.kind != UrlUse::Navigation && mm.via.is_empty() {
                    let mut prefix: Vec<&String> = Vec::new();
                    for (i, seg) in u.segs.iter().enumerate() {
                        let [super::urlpat::Part::Lit(l)] = seg.as_slice() else {
                            break;
                        };
                        let static_at = |rt: &Route| {
                            matches!(rt.segs.get(i), Some(Seg::Static(x)) if x == l)
                                && prefix.iter().enumerate().all(|(j, p)| {
                                    matches!(rt.segs.get(j), Some(Seg::Static(x)) if x == *p)
                                })
                        };
                        if !app.table.routes.iter().any(static_at) {
                            break;
                        }
                        ids.retain(|id| app.table.route(id).is_some_and(static_at));
                        prefix.push(l);
                    }
                    // A data request (`fetch`, SWR) targets a handler; a page only if the URL
                    // names it by a static segment (an RSC/HTML fetch). A page matched purely
                    // through dynamic segments (`/v9/projects/*` vs `/[team]/[project]/[id]`)
                    // is an API path on another base, and an unknown origin (`${base}/v1/x`)
                    // is an API base.
                    if s.kind == UrlUse::Fetch {
                        let lits: Vec<(usize, &String)> = u
                            .segs
                            .iter()
                            .enumerate()
                            .filter_map(|(i, seg)| match seg.as_slice() {
                                [super::urlpat::Part::Lit(l)] => Some((i, l)),
                                _ => None,
                            })
                            .collect();
                        ids.retain(|id| {
                            let Some(rt) = app.table.route(id) else {
                                return false;
                            };
                            rt.kind == RouteKind::Handler
                                || (u.origin != super::urlpat::Origin::Unknown
                                    && lits.iter().any(|(i, l)| {
                                        matches!(rt.segs.get(*i), Some(Seg::Static(x)) if x == *l)
                                    }))
                        });
                    }
                    if ids.is_empty() && mm.candidates.iter().chain(&mm.routes).next().is_some() {
                        r.not_found += 1;
                    }
                }
                for id in ids {
                    if !r.routes.contains(id) {
                        r.routes.push(id.clone());
                    }
                }
            }
            r.routes.sort();
            r
        })
        .collect()
}

/// An [`ImportResolver`] from a graph's resolved import edges (specifier →
/// target), with relative paths as a fallback.
pub struct GraphImports {
    map: HashMap<PathBuf, HashMap<String, PathBuf>>,
    simple: crate::common::SimpleResolver,
}

impl GraphImports {
    /// Indexes `g` (paths relative to `root`).
    pub fn from_graph<G: GraphView>(g: &G, root: &Path) -> GraphImports {
        let mut map: HashMap<PathBuf, HashMap<String, PathBuf>> = HashMap::new();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            if g.is_external(n) {
                continue;
            }
            let mut m = HashMap::new();
            for e in g.out_edges(n) {
                // Module edges only: a package specifier also has an edge to the manifest
                // (`package.json`) consulted to resolve it.
                if !matches!(
                    e.kind(),
                    turborepo_trace_graph::RefKind::Import
                        | turborepo_trace_graph::RefKind::Require
                        | turborepo_trace_graph::RefKind::DynamicImport
                        | turborepo_trace_graph::RefKind::ReExport
                ) {
                    continue;
                }
                if let (Some(to), Some(spec)) = (e.to, e.specifier)
                    && !g.is_external(to)
                {
                    m.insert(spec.to_string(), root.join(g.path(to)));
                }
            }
            if !m.is_empty() {
                map.insert(root.join(g.path(n)), m);
            }
        }
        GraphImports {
            map,
            simple: crate::common::SimpleResolver::default(),
        }
    }
}

impl ImportResolver for GraphImports {
    fn resolve(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        self.map
            .get(from)
            .and_then(|m| m.get(spec).cloned())
            .or_else(|| ImportResolver::resolve(&self.simple, from, spec))
    }
}

/// The URL segments a route file at `rel` (relative to the app root) would
/// have, for route-table changes. `None` if the file is not under a router
/// directory or is private.
fn file_route(app: &NextApp, rel: &str) -> Option<(Router, String, Vec<Seg>, String)> {
    let t = &app.table;
    let (router, dir) = if let Some(a) = t
        .app_dir
        .as_deref()
        .filter(|a| rel.starts_with(&format!("{a}/")))
    {
        (Router::App, a)
    } else {
        let p = t
            .pages_dir
            .as_deref()
            .filter(|p| rel.starts_with(&format!("{p}/")))?;
        (Router::Pages, p)
    };
    let inner = &rel[dir.len() + 1..];
    let (parent, name) = match inner.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", inner),
    };
    let mut segs = Vec::new();
    for c in parent.split('/').filter(|s| !s.is_empty()) {
        match parse_dir_name(c) {
            DirKind::Url(s) => segs.push(s),
            DirKind::Private => return None,
            DirKind::Intercept(_, rest) => {
                if let DirKind::Url(s) = parse_dir_name(&rest) {
                    segs.push(s);
                }
            }
            DirKind::Group | DirKind::Slot => {}
        }
    }
    let special = match router {
        Router::App => special_name(name, &t.page_exts)?.to_string(),
        Router::Pages => {
            let base = t.page_exts.iter().find_map(|e| {
                name.strip_suffix(e.as_str())
                    .and_then(|b| b.strip_suffix('.'))
            })?;
            if parent.is_empty() && super::routes::PAGES_SPECIAL.contains(&base) {
                "pages-special".to_string()
            } else {
                if base != "index"
                    && let DirKind::Url(s) = parse_dir_name(base)
                {
                    segs.push(s);
                }
                "page".to_string()
            }
        }
    };
    let dir_rel = if parent.is_empty() {
        dir.to_string()
    } else {
        format!("{dir}/{parent}")
    };
    Some((router, special, segs, dir_rel))
}

fn pattern_as_url(segs: &[Seg]) -> UrlPat {
    let mut out = Vec::new();
    let mut open = false;
    for s in segs {
        match s {
            Seg::Static(v) => out.push(vec![Part::Lit(v.clone())]),
            Seg::Dynamic(_) => out.push(vec![Part::Any]),
            Seg::CatchAll(_) | Seg::OptCatchAll(_) => {
                open = true;
                break;
            }
        }
    }
    UrlPat {
        origin: Origin::Relative,
        segs: out,
        open_tail: open,
    }
}

/// Selects the specs a change set affects.
pub fn select_e2e<G: GraphView>(
    changes: &ChangeSet,
    specs: &[SpecVisits],
    app: &NextApp,
    graph: &G,
    opts: &SelectOptions<'_>,
) -> E2eReport {
    let root = &opts.graph_root;
    let rel = |p: &Path| -> String {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    };
    let app_rel = rel(&app.root);
    let in_app = |p: &str| -> Option<String> {
        if app_rel.is_empty() {
            Some(p.to_string())
        } else {
            p.strip_prefix(&format!("{app_rel}/")).map(String::from)
        }
    };
    let matcher = app.matcher().with_context(opts.context.clone());
    let mut ov = Overlay::new(graph);
    let tag = if app_rel.is_empty() {
        ".".to_string()
    } else {
        app_rel.clone()
    };
    let g_node = ov.node(&format!("next:global:{tag}"));
    let r_node = ov.node(&format!("next:routing:{tag}"));
    let any_route = ov.node(&format!("next:any-route:{tag}"));
    let any_handler = ov.node(&format!("next:any-handler:{tag}"));

    // Global and routing inputs.
    for f in &app.globals.files {
        let n = ov.node(&rel(&f.file));
        ov.edge(g_node, n);
        if f.routing {
            ov.edge(r_node, n);
        }
    }
    for p in &app.globals.probes {
        ov.probe(g_node, &rel(p));
    }
    for m in &app.globals.memberships {
        ov.membership(g_node, &rel(&m.dir), &m.pattern);
    }
    // Modified env files and public assets are not graph nodes: link changed ones.
    for c in &changes.files {
        if let Some(r) = in_app(&c.path)
            && (r.starts_with("public/")
                || r.rsplit('/').next().is_some_and(|n| n.starts_with(".env")) && !r.contains('/'))
        {
            let n = ov.node(&c.path);
            ov.edge(g_node, n);
        }
    }
    if opts.route_table == RouteTableMode::Membership {
        for m in app.route_table_memberships() {
            ov.membership(r_node, &rel(&m.dir), &m.pattern);
        }
    }

    // Route nodes.
    let mut route_node: HashMap<String, NodeId> = HashMap::new();
    let mut soft_node: HashMap<String, NodeId> = HashMap::new();
    for r in &app.table.routes {
        let n = ov.node(&format!("route:{}", rel(&app.root.join(&r.file))));
        let cl = app.table.segment_closure(r);
        for f in &cl.files {
            let fnode = ov.node(&rel(&app.root.join(f)));
            ov.edge(n, fnode);
        }
        if !cl.soft.is_empty() {
            let s = ov.node(&format!("route-soft:{}", rel(&app.root.join(&r.file))));
            ov.edge(s, n);
            for f in &cl.soft {
                let fnode = ov.node(&rel(&app.root.join(f)));
                ov.edge(s, fnode);
            }
            soft_node.insert(r.id.clone(), s);
        }
        if r.intercept.is_none() && r.slot.is_none() {
            match r.kind {
                RouteKind::Page => ov.edge(any_route, n),
                RouteKind::Handler => ov.edge(any_handler, n),
            }
        }
        route_node.insert(r.id.clone(), n);
    }
    // Pages may call any handler too: an unresolved navigation reaches handlers
    // through fetch sites only, so `any-route` does not imply `any-handler`.

    // Fetch sites.
    if let Some(sites) = opts.fetch {
        for s in sites {
            // Navigations are visits only if a test performs them; built URLs are not
            // requests (the site that uses one is evaluated on its own), and linking them
            // would chain page closures through redirect helpers.
            if matches!(s.site.kind, UrlUse::Navigation | UrlUse::Build) {
                continue;
            }
            let Some(fnode) = graph.node_by_path(&rel(&s.site.file)) else {
                continue;
            };
            for id in &s.routes {
                if let Some(&rn) = route_node.get(id) {
                    ov.edge(fnode, rn);
                }
            }
            if s.unresolved > 0 && s.site.kind != UrlUse::Build && opts.unresolved_fetch_any_handler
            {
                ov.edge(fnode, any_handler);
            }
        }
    }

    // Tailwind.
    if app.tailwind.enabled {
        let t_node = ov.node(&format!("next:tailwind:{tag}"));
        ov.edge(g_node, t_node);
        for c in &changes.files {
            let abs = root.join(&c.path);
            if !app.tailwind.scans(&abs) {
                continue;
            }
            let hit = match (opts.tailwind, opts.contents) {
                (TailwindMode::Tokens, Some(ct)) => {
                    let old = if c.kind == ChangeKind::Added {
                        None
                    } else {
                        ct.before(&c.path)
                    };
                    let new = if c.kind == ChangeKind::Deleted {
                        None
                    } else {
                        ct.after(&c.path)
                    };
                    css_affected(
                        old.as_deref(),
                        new.as_deref(),
                        opts.tailwind_others,
                        &app.tailwind.custom_utilities,
                    )
                }
                (TailwindMode::Closure, _) => false,
                _ => true,
            };
            if hit {
                let n = ov.node(&c.path);
                ov.edge(t_node, n);
            }
        }
    }

    // Specs.
    let mut spec_nodes = Vec::new();
    let mut verdict_info: Vec<(Vec<String>, Vec<String>, Vec<String>)> = Vec::new();
    let mut spec_urls: Vec<Vec<UrlPat>> = Vec::new();
    let mut spec_any: Vec<bool> = Vec::new();
    for s in specs {
        let sn = ov.node(&rel(&s.spec));
        spec_nodes.push(sn);
        ov.edge(sn, g_node);
        ov.edge(sn, r_node);
        let mut routes: BTreeSet<String> = BTreeSet::new();
        let mut amb: BTreeSet<String> = BTreeSet::new();
        let mut urls = Vec::new();
        let replace = s
            .annotation
            .as_ref()
            .is_some_and(|a| a.replace && !a.routes.is_empty());
        let mut unresolved: Vec<String> = if replace {
            Vec::new()
        } else {
            s.unresolved
                .iter()
                .filter(|u| u.kind.counts_for_completeness())
                .map(|u| format!("{}:{}: {}", rel(&u.file), u.line, u.text))
                .collect()
        };
        if let Some(a) = &s.annotation {
            for p in &a.routes {
                let segs = parse_pattern(p);
                let hits: Vec<&Route> = app
                    .table
                    .routes
                    .iter()
                    .filter(|r| r.segs == segs && r.intercept.is_none())
                    .collect();
                if hits.is_empty() {
                    unresolved.push(format!("annotation route `{p}` is not in the route table"));
                }
                for r in hits {
                    routes.insert(r.id.clone());
                }
                urls.push(pattern_as_url(&segs));
            }
            for u in &a.unreadable {
                unresolved.push(format!("annotation entry `{u}`"));
            }
        }
        if !replace {
            for v in &s.visits {
                let m = matcher.match_pat(&v.url, v.soft);
                urls.push(v.url.clone());
                for a in &m.ambiguities {
                    amb.insert(match a {
                        Ambiguity::ParamMayEqualStatic { extra } => {
                            format!("param-may-equal-static({}): {}", v.url, extra.join(", "))
                        }
                        Ambiguity::ConditionalRule { phase, source } => {
                            format!("conditional-{phase}: {source}")
                        }
                        Ambiguity::UnsupportedRule { source } => {
                            format!("unsupported-rule: {source}")
                        }
                        Ambiguity::MicrofrontendFlag { app, flag } => {
                            format!("microfrontend-flag: {app}/{flag}")
                        }
                        other => format!("{other:?}").to_ascii_lowercase(),
                    });
                }
                let ids: Vec<&String> = match opts.ambiguity {
                    AmbiguityPolicy::Candidates => m.candidates.iter().collect(),
                    AmbiguityPolicy::Heuristic => m.routes.iter().collect(),
                };
                for id in ids {
                    routes.insert(id.clone());
                    let n = if v.soft {
                        soft_node.get(id).or(route_node.get(id))
                    } else {
                        route_node.get(id)
                    };
                    if let Some(&n) = n {
                        ov.edge(sn, n);
                    }
                }
            }
            let navs = s.visits.iter().any(|v| v.kind.counts_for_completeness());
            if !navs && s.annotation.is_none() {
                unresolved.push("no navigation found".into());
            }
        }
        for id in &routes {
            if let Some(&n) = route_node.get(id) {
                ov.edge(sn, n);
            }
        }
        let any = !unresolved.is_empty();
        if any {
            ov.edge(sn, any_route);
        }
        spec_any.push(any);
        spec_urls.push(urls);
        verdict_info.push((
            routes.into_iter().collect(),
            unresolved,
            amb.into_iter().collect(),
        ));
    }

    // Route-table changes.
    let mut table_nodes: Vec<NodeId> = Vec::new();
    if opts.route_table == RouteTableMode::Rematch {
        for c in &changes.files {
            if c.kind == ChangeKind::Modified {
                continue;
            }
            let Some(r) = in_app(&c.path) else { continue };
            let Some((router, special, segs, dir_rel)) = file_route(app, &r) else {
                continue;
            };
            let x = ov.node(&c.path);
            table_nodes.push(x);
            match special.as_str() {
                "page" | "route" => {
                    let u = pattern_as_url(&segs);
                    for (i, urls) in spec_urls.iter().enumerate() {
                        let hit = urls
                            .iter()
                            .any(|v| overlaps(&segs, v) || overlaps(&parse_route_segs(v), &u));
                        if hit {
                            ov.edge(spec_nodes[i], x);
                        }
                    }
                    ov.edge(any_route, x);
                }
                "pages-special" => ov.edge(g_node, x),
                _ => {
                    // A layout-level file: every route below its directory.
                    let is_root = Some(dir_rel.as_str()) == app.table.app_dir.as_deref();
                    if is_root || router == Router::Pages {
                        ov.edge(g_node, x);
                    }
                    for rt in &app.table.routes {
                        if (rt.dir == dir_rel || rt.dir.starts_with(&format!("{dir_rel}/")))
                            && let Some(&rn) = route_node.get(&rt.id)
                        {
                            ov.edge(rn, x);
                        }
                    }
                }
            }
        }
    }

    // Run `affected`.
    let engine = Engine::new(&ov);
    let mut entries = NodeSet::new(ov.node_count());
    for &n in &spec_nodes {
        entries.insert(n);
    }
    let report = engine.affected(changes, &entries, &opts.affected);
    let by_entry: HashMap<NodeId, &turborepo_trace_graph::EntryResult> =
        report.entries.iter().map(|e| (e.entry, e)).collect();
    let mut out = Vec::new();
    for (i, s) in specs.iter().enumerate() {
        let n = spec_nodes[i];
        let (routes, unresolved, ambiguities) = verdict_info[i].clone();
        let (verdict, witness, cause, reason) = match by_entry.get(&n) {
            None => (Verdict::Unaffected, Vec::new(), None, None),
            Some(e) => {
                let w: Vec<String> = e.witness.iter().map(|&x| ov.path(x).to_string()).collect();
                let cause = e.cause.as_ref().map(|c| match c {
                    Cause::Modified { path } => format!("modified {path}"),
                    Cause::Deleted { path } => format!("deleted {path}"),
                    Cause::Probe { added, probe } => format!("added {added} (probe {probe})"),
                    Cause::Membership {
                        path,
                        kind,
                        dir,
                        pattern,
                    } => {
                        format!("{} {path} (membership {dir}/{pattern})", kind.as_str())
                    }
                    Cause::Package { package } => format!("package {}", package.0),
                    Cause::Scope {
                        path,
                        kind,
                        dir,
                        pattern,
                    } => format!("{} {path} (scope {dir}/{pattern})", kind.as_str()),
                    Cause::Named { path, by, .. } => format!("{path} (named by {by})"),
                });
                let reason = if e.uncertain.is_some() {
                    Reason::Uncertain
                } else if e.witness.len() > 1
                    && table_nodes.contains(&e.witness[0])
                    && !w.iter().any(|x| x.starts_with("next:global"))
                {
                    Reason::RouteTable
                } else {
                    classify(&w)
                };
                (e.verdict, w, cause, Some(reason))
            }
        };
        out.push(SpecVerdict {
            spec: s.spec.clone(),
            verdict,
            reason,
            witness,
            cause,
            routes,
            unresolved,
            ambiguities,
        });
    }
    E2eReport {
        specs: out,
        unmatched: report.unmatched.iter().map(|c| c.path.clone()).collect(),
    }
}

/// A URL pattern read as route segments (unknown segments as dynamic ones).
fn parse_route_segs(u: &UrlPat) -> Vec<Seg> {
    let mut v: Vec<Seg> = u
        .segs
        .iter()
        .map(|s| match s.as_slice() {
            [Part::Lit(t)] => Seg::Static(t.clone()),
            _ => Seg::Dynamic("_".into()),
        })
        .collect();
    if u.open_tail {
        v.push(Seg::OptCatchAll("_".into()));
    }
    v
}

fn classify(w: &[String]) -> Reason {
    if w.len() == 1 {
        return Reason::SpecChanged;
    }
    let has = |p: &str| w.iter().any(|x| x.starts_with(p));
    if has("next:tailwind") {
        return Reason::TailwindCss;
    }
    if has("next:global") {
        return Reason::GlobalInput;
    }
    if has("next:routing") {
        return Reason::RouteMatching;
    }
    if has("next:any-handler") {
        return Reason::UnresolvedFetch;
    }
    if has("next:any-route") {
        return Reason::IncompleteVisits;
    }
    // The route nearest the spec is the visited one; a route deeper in the chain
    // was reached through a fetch site.
    let routes: Vec<&String> = w
        .iter()
        .filter(|x| x.starts_with("route:") || x.starts_with("route-soft:"))
        .collect();
    match routes.as_slice() {
        [] => Reason::SpecImports,
        [only] => Reason::Route(only.to_string()),
        [first, ..] => Reason::FetchedRoute(first.to_string()),
    }
}

/// Summary counts over resolved sites (for reports).
#[derive(Clone, Debug, Default, Serialize)]
pub struct SiteStats {
    /// Sites (fetch + redirect).
    pub sites: usize,
    /// Navigation sites (not dependencies).
    pub navigation_sites: usize,
    /// `new URL(..)` sites (informational; not counted below).
    pub build_sites: usize,
    /// Sites whose URL is a parameter forwarded to a wrapper, a callback or the
    /// request's own URL (their callers are sites instead).
    pub forwarded: usize,
    /// Sites with at least one route.
    pub to_route: usize,
    /// Sites hitting at least one handler route.
    pub to_handler: usize,
    /// Sites with an external URL.
    pub external: usize,
    /// Sites rewritten or redirected.
    pub rewritten: usize,
    /// Sites served by another microfrontend.
    pub microfrontend: usize,
    /// Sites with an unresolved alternative.
    pub unresolved: usize,
    /// Sites whose URLs hit nothing.
    pub not_found: usize,
}

/// Counts.
pub fn site_stats(app: &NextApp, sites: &[ResolvedSite]) -> SiteStats {
    let handler: BTreeMap<&str, bool> = app
        .table
        .routes
        .iter()
        .map(|r| (r.id.as_str(), r.kind == RouteKind::Handler))
        .collect();
    let mut s = SiteStats::default();
    for r in sites {
        if r.site.kind == UrlUse::Navigation {
            s.navigation_sites += 1;
            continue;
        }
        if r.site.kind == UrlUse::Build {
            s.build_sites += 1;
            continue;
        }
        if r.site.forwarded.is_some() {
            s.forwarded += 1;
            continue;
        }
        s.sites += 1;
        if !r.routes.is_empty() {
            s.to_route += 1;
        }
        if r.routes
            .iter()
            .any(|id| handler.get(id.as_str()) == Some(&true))
        {
            s.to_handler += 1;
        }
        if r.external > 0 {
            s.external += 1;
        }
        if r.rewritten > 0 {
            s.rewritten += 1;
        }
        if r.microfrontend > 0 {
            s.microfrontend += 1;
        }
        if r.unresolved > 0 {
            s.unresolved += 1;
        }
        if r.not_found > 0 && r.routes.is_empty() {
            s.not_found += 1;
        }
    }
    s
}
