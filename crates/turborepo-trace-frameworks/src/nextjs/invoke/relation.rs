//! From sites to the per-route relation: URL targets are matched against the
//! route table (after the config's rules), and every route collects the sites
//! of its module closure.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use turborepo_trace_graph::{GraphView, RefFlags, RefKind};

use super::model::{InvokeKind, InvokeRelation, Procedure, Site, SiteTarget, Target, UrlString};
use crate::nextjs::{
    NextApp,
    matcher::{Ambiguity, MAX_PREFIX, Matcher, Outcome, RequestContext},
    routes::{Route, RouteKind, Seg},
    source::Source,
    urlpat::{Origin, Part, UrlPat, overlaps},
};

/// Options for [`relation`].
#[derive(Clone, Debug, Default)]
pub struct RelationOptions {
    /// The root the graph's paths are relative to.
    pub graph_root: PathBuf,
    /// Request context for rule conditions.
    pub context: RequestContext,
    /// Hosts that are this app (an absolute URL to one is matched like a path).
    /// The context's host always is.
    pub own_hosts: Vec<String>,
    /// Bound unknown targets: a request whose URL cannot be evaluated is taken
    /// to reach only the handler routes that a URL-like string of the
    /// invoking route's module closure matches (they are added as `fetch`
    /// invocations via `url-like string`), not every handler. What escapes
    /// the bound: URLs assembled from fragments that do not
    /// look like paths, and paths that come from outside the closure
    /// (environment, configuration, responses).
    pub bound_unknown: bool,
}

/// The key of a route: its file relative to the app root, without extension and
/// without a leading `src/` ([`crate::nextjs::build::invokes::source_key`]: the
/// build's manifest key).
pub fn route_key(r: &Route) -> String {
    crate::nextjs::build::invokes::source_key(&r.file)
}

/// The tRPC side of an app, for mapping procedure targets.
#[derive(Clone, Debug, Default)]
pub struct TrpcIndex {
    /// Procedures by path.
    pub procedures: Vec<Procedure>,
    /// Route keys of the HTTP entries.
    pub handlers: Vec<String>,
}

/// A site with its mapped targets.
#[derive(Clone, Debug)]
pub struct MappedSite {
    /// Index into the sites.
    pub site: usize,
    /// `(target, url pattern)` pairs.
    pub targets: Vec<(Target, Option<String>)>,
    /// Routes that a link or redirect target can also be served by when the
    /// middleware puts any segments in front of the path
    /// ([`Matcher::behind_prefix`], its sound set): not targets by Next's
    /// precedence, so they count only where nothing is assumed
    /// ([`super::model::Invocation::wide`]). Empty when the middleware itself
    /// redirects or rewrites to a URL that is not known: a request can then
    /// end on any page.
    pub wide: Vec<(Target, Option<String>)>,
}

/// A prefix the middleware puts in front of a path: the segments of a rewrite
/// or redirect target that ends in an open path.
pub type Prefix = Vec<Vec<Part>>;

/// The prefixes the redirect and rewrite sites `sites` (those of a middleware)
/// spell: the segments of every target that ends in an open path
/// (`/precomputed/<code>/<path>`).
pub fn spelled_prefixes<'a>(sites: impl IntoIterator<Item = &'a Site>) -> Vec<Prefix> {
    let mut prefixes: Vec<Prefix> = Vec::new();
    for site in sites {
        if site.kind != InvokeKind::Redirect {
            continue;
        }
        for t in &site.targets {
            if let SiteTarget::Url(u) = t
                && u.open_tail
                && !u.segs.is_empty()
                && u.segs.len() <= MAX_PREFIX + 1
                && !matches!(u.origin, Origin::Absolute(_))
                && !prefixes.contains(&u.segs)
            {
                prefixes.push(u.segs.clone());
            }
        }
    }
    prefixes
}

/// What the scan knows of an app's middleware, for mapping the targets of links
/// and redirects.
#[derive(Clone, Debug, Default)]
pub(crate) struct MiddlewareFacts {
    /// The prefixes its rewrites spell ([`spelled_prefixes`]).
    pub prefixes: Vec<Prefix>,
    /// Some redirect or rewrite of it has a target that is not known: a request
    /// it runs for can end on any page, so wide targets say nothing more
    /// and are not listed.
    pub any_page: bool,
}

impl MiddlewareFacts {
    /// From the sites the middleware's module closure reaches.
    pub(crate) fn of(sites: &[&Site]) -> MiddlewareFacts {
        MiddlewareFacts {
            prefixes: spelled_prefixes(sites.iter().copied()),
            any_page: sites
                .iter()
                .any(|s| s.kind == InvokeKind::Redirect && s.has_unknown()),
        }
    }
}

pub(crate) fn rel_path(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Maps one URL pattern to targets: `(targets, wide)` ([`MappedSite::wide`]).
///
/// The target of a link or a redirect is requested like a URL a spec names, so
/// the app's middleware runs for it too. When it does, the routes behind a
/// prefix are targets as well: behind the prefixes its rewrites spell, and
/// behind up to [`MAX_PREFIX`] leading dynamic segments
/// ([`Matcher::behind_prefix`]); not for a path that names a static route as it
/// is.
fn map_url(
    app: &NextApp,
    m: &Matcher<'_>,
    keys: &HashMap<&str, (String, &Route)>,
    u: &UrlPat,
    kind: InvokeKind,
    own_hosts: &[String],
    mw: &MiddlewareFacts,
) -> (Vec<Target>, Vec<Target>) {
    let mut u = u.clone();
    if let Origin::Absolute(h) = &u.origin {
        let host = h.split("://").nth(1).unwrap_or(h).trim_start_matches("//");
        let host = host.split(':').next().unwrap_or(host);
        if own_hosts.iter().any(|o| o == host) {
            u.origin = Origin::Relative;
        }
    }
    let nav = kind == InvokeKind::Navigation;
    let mm = m.match_pat(&u, nav);
    let pattern = u.path_string();
    match &mm.outcome {
        Outcome::External(h) => {
            let t = Target::External {
                origin: h.clone(),
                pattern: if mm.via.is_empty() {
                    pattern
                } else {
                    mm.target.clone()
                },
            };
            return (vec![t], Vec::new());
        }
        Outcome::Microfrontend(a) => {
            let t = Target::External {
                origin: format!("mfe:{a}"),
                pattern,
            };
            return (vec![t], Vec::new());
        }
        // A static asset: no code runs.
        Outcome::Public(_) => return (Vec::new(), Vec::new()),
        Outcome::Routes | Outcome::NotFound => {}
    }
    let mut ids: Vec<&String> = mm.candidates.iter().chain(&mm.routes).collect();
    // Same refinements as `select::resolve_sites`: a literal segment that names a
    // static route directory goes there, and a data request targets a handler
    // (a page only when the URL names it by a static segment).
    if !nav && mm.via.is_empty() {
        let mut prefix: Vec<&String> = Vec::new();
        for (i, seg) in u.segs.iter().enumerate() {
            let [Part::Lit(l)] = seg.as_slice() else {
                break;
            };
            let static_at = |rt: &Route| {
                matches!(rt.segs.get(i), Some(Seg::Static(x)) if x == l)
                    && prefix
                        .iter()
                        .enumerate()
                        .all(|(j, p)| matches!(rt.segs.get(j), Some(Seg::Static(x)) if x == *p))
            };
            if !app.table.routes.iter().any(static_at) {
                break;
            }
            ids.retain(|id| keys.get(id.as_str()).is_some_and(|x| static_at(x.1)));
            prefix.push(l);
        }
        if kind != InvokeKind::Redirect {
            let lits: Vec<(usize, &String)> = u
                .segs
                .iter()
                .enumerate()
                .filter_map(|(i, seg)| match seg.as_slice() {
                    [Part::Lit(l)] => Some((i, l)),
                    _ => None,
                })
                .collect();
            ids.retain(|id| {
                let Some(&(_, rt)) = keys.get(id.as_str()) else {
                    return false;
                };
                rt.kind == RouteKind::Handler
                    || (u.origin != Origin::Unknown
                        && lits.iter().any(
                            |(i, l)| matches!(rt.segs.get(*i), Some(Seg::Static(x)) if x == *l),
                        ))
            });
        }
    }
    let mut out: Vec<Target> = Vec::new();
    let mut wide: Vec<Target> = Vec::new();
    let route = |id: &String| keys.get(id.as_str()).map(|(k, _)| Target::Route(k.clone()));
    for t in ids.iter().filter_map(|id| route(id)) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    // A page request: the middleware that runs for the path can rewrite it behind a
    // prefix.
    if matches!(kind, InvokeKind::Navigation | InvokeKind::Redirect)
        && mm.ambiguities.contains(&Ambiguity::Middleware)
    {
        let names_static = ids
            .iter()
            .any(|id| keys.get(id.as_str()).is_some_and(|x| x.1.is_static()));
        let mut likely: Vec<String> = Vec::new();
        let mut sound: Vec<String> = Vec::new();
        if !names_static {
            for prefix in &mw.prefixes {
                let mut prefixed = u.clone();
                let mut segs = prefix.clone();
                segs.extend(prefixed.segs);
                prefixed.segs = segs;
                likely.extend(m.match_pat(&prefixed, nav).candidates);
            }
        }
        for p in m.behind_prefix(&u, nav) {
            if !names_static {
                likely.extend(p.likely);
            }
            sound.extend(p.sound);
        }
        for t in likely.iter().filter_map(route) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        if !mw.any_page {
            for t in sound.iter().filter_map(route) {
                if !out.contains(&t) && !wide.contains(&t) {
                    wide.push(t);
                }
            }
        }
    }
    if u.origin == Origin::Unknown {
        out.push(Target::External {
            origin: "<origin>".into(),
            pattern,
        });
    } else if out.is_empty() {
        // No route of this app serves it: a backend behind the same origin (platform
        // rewrites), or a 404.
        out.push(Target::External {
            origin: String::new(),
            pattern,
        });
    }
    (out, wide)
}

/// Maps every site's targets.
pub fn map_sites(
    app: &NextApp,
    sites: &[Site],
    trpc: &TrpcIndex,
    opts: &RelationOptions,
) -> Vec<MappedSite> {
    let mw = MiddlewareFacts::default();
    map_sites_kept(app, sites, trpc, opts, &vec![true; sites.len()], &mw)
}

/// [`map_sites`] for the sites with `keep` set; the others get no targets.
pub(crate) fn map_sites_kept(
    app: &NextApp,
    sites: &[Site],
    trpc: &TrpcIndex,
    opts: &RelationOptions,
    keep: &[bool],
    mw: &MiddlewareFacts,
) -> Vec<MappedSite> {
    let m = app.matcher().with_context(opts.context.clone());
    let keys: HashMap<&str, (String, &Route)> = app
        .table
        .routes
        .iter()
        .map(|r| (r.id.as_str(), (route_key(r), r)))
        .collect();
    let mut own = opts.own_hosts.clone();
    own.push(opts.context.host.clone());
    let procs: HashMap<&str, &Procedure> = trpc
        .procedures
        .iter()
        .map(|p| (p.path.as_str(), p))
        .collect();
    let mut cache: HashMap<(UrlPat, InvokeKind), (Vec<Target>, Vec<Target>)> = HashMap::new();
    let mut out = Vec::with_capacity(sites.len());
    for (i, s) in sites.iter().enumerate() {
        let mut targets: Vec<(Target, Option<String>)> = Vec::new();
        let mut wide: Vec<(Target, Option<String>)> = Vec::new();
        if !keep[i] {
            out.push(MappedSite {
                site: i,
                targets,
                wide,
            });
            continue;
        }
        let mut push = |t: Target, u: Option<String>| {
            if !targets.iter().any(|(x, _)| *x == t) {
                targets.push((t, u));
            }
        };
        for t in &s.targets {
            match t {
                SiteTarget::Url(u) => {
                    let (ts, ws) = cache
                        .entry((u.clone(), s.kind))
                        .or_insert_with(|| map_url(app, &m, &keys, u, s.kind, &own, mw));
                    for t in ts.iter() {
                        push(t.clone(), Some(u.to_string()));
                    }
                    for t in ws.iter() {
                        if !wide.iter().any(|(x, _)| x == t) {
                            wide.push((t.clone(), Some(u.to_string())));
                        }
                    }
                }
                SiteTarget::Procedure(p) => {
                    let file = procs.get(p.as_str()).map(|x| x.file.clone());
                    push(
                        Target::Procedure {
                            path: p.clone(),
                            file,
                            serving: trpc.handlers.first().cloned(),
                        },
                        None,
                    );
                }
                SiteTarget::Action { file, export } => {
                    push(
                        Target::Action(format!("{}#{export}", rel_path(&opts.graph_root, file))),
                        None,
                    );
                }
                SiteTarget::Unknown(r) => push(Target::Unknown(r.clone()), None),
            }
        }
        // A route that another alternative of the site targets by precedence is not
        // wide.
        wide.retain(|(t, _)| !targets.iter().any(|(x, _)| x == t));
        out.push(MappedSite {
            site: i,
            targets,
            wide,
        });
    }
    out
}

/// The literal prefix of a pattern that is a prefix glued to an unknown path:
/// every segment known except the last, which is known text followed by an
/// unknown part, and an open tail (`/api*/**` from `/api${path}`).
fn open_prefix(u: &UrlPat) -> Option<Vec<String>> {
    if !u.open_tail || matches!(u.origin, Origin::Absolute(_)) {
        return None;
    }
    let (last, init) = u.segs.split_last()?;
    let mut pre: Vec<String> = Vec::new();
    for seg in init {
        match seg.as_slice() {
            [Part::Lit(l)] => pre.push(l.clone()),
            _ => return None,
        }
    }
    match last.as_slice() {
        [Part::Lit(l), Part::Any] => pre.push(l.clone()),
        _ => return None,
    }
    Some(pre)
}

/// Whether an unknown URL is supplied from outside the code: read from the
/// request (location, search and route parameters, headers, a body), from
/// storage, or from what the user enters or a message delivers
/// ([`Source::outside`], found by resolving the expression's identifiers:
/// [`crate::nextjs::source`]). No string of the module closure has to match
/// such a URL (a test can supply any), so the bound does not apply to it.
pub fn outside_input(reason: &str) -> bool {
    Source::of_reason(reason).outside()
}

/// Where an unknown reason says its value comes from ([`Source::name`]).
pub fn unknown_source(reason: &str) -> &'static str {
    Source::of_reason(reason).name()
}

/// A coarse class of an unknown reason, for the work order.
pub fn unknown_class(reason: &str) -> String {
    match reason.split_once(':') {
        Some((c, _))
            if !c.is_empty()
                && c.len() <= 24
                && c.chars().all(|x| x.is_ascii_lowercase() || x == '-') =>
        {
            c.to_string()
        }
        _ => "expression".to_string(),
    }
}

pub(crate) fn follows(kind: RefKind, flags: RefFlags) -> bool {
    matches!(
        kind,
        RefKind::Import | RefKind::Require | RefKind::DynamicImport | RefKind::ReExport
    ) && !flags.contains(RefFlags::TYPE_ONLY)
}

pub(crate) fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// What the routes and the middleware of an app reach over module edges: what
/// is mapped.
pub(crate) struct Reached {
    /// Per site: some route (or the middleware) reaches its file.
    pub sites: Vec<bool>,
    /// The same per URL-like string.
    pub strings: Vec<bool>,
    /// The middleware's own redirects and rewrites.
    pub middleware: MiddlewareFacts,
}

/// Sites with their mapped targets, and the handler routes each URL-like string
/// matches.
pub(crate) struct Prepared {
    /// Per site.
    pub mapped: Vec<MappedSite>,
    /// Per URL-like string: route keys of the handlers it matches (alone, and
    /// after each wrapper prefix).
    pub string_routes: Vec<Vec<String>>,
}

/// Maps the sites and the URL-like strings to targets.
pub(crate) fn prepare(
    app: &NextApp,
    sites: &[Site],
    strings: &[UrlString],
    trpc: &TrpcIndex,
    opts: &RelationOptions,
    reached: &Reached,
) -> Prepared {
    let (keep_sites, keep_strings) = (&reached.sites, &reached.strings);
    let mut mapped = map_sites_kept(app, sites, trpc, opts, keep_sites, &reached.middleware);
    // A pattern that is a prefix glued to an unknown path (`/api${path}`: a wrapper
    // that adds a prefix, called with an unknown URL) is an unknown target too;
    // under the bound it reaches the routes of `prefix + s` for the URL-like
    // strings `s` of the closure.
    let mut prefixes: Vec<Vec<String>> = vec![Vec::new()];
    if opts.bound_unknown {
        for (i, site) in sites.iter().enumerate() {
            if !site.kind.is_dependency() || !keep_sites[i] {
                continue;
            }
            for t in &site.targets {
                let SiteTarget::Url(u) = t else { continue };
                let Some(pre) = open_prefix(u) else { continue };
                let us = u.to_string();
                mapped[i].targets.retain(|(t, url)| {
                    !(url.as_deref() == Some(us.as_str()) && matches!(t, Target::Route(_)))
                });
                mapped[i]
                    .wide
                    .retain(|(_, url)| url.as_deref() != Some(us.as_str()));
                let unk = Target::Unknown(format!("open-prefix: {us}"));
                if !mapped[i].targets.iter().any(|(t, _)| *t == unk) {
                    mapped[i].targets.push((unk, None));
                }
                if !prefixes.contains(&pre) {
                    prefixes.push(pre);
                }
            }
        }
    }
    // URL-like strings: the handler routes each one matches (alone, and after each
    // prefix).
    let string_routes: Vec<Vec<String>> = {
        let m = app.matcher().with_context(opts.context.clone());
        let keys: HashMap<&str, (String, &Route)> = app
            .table
            .routes
            .iter()
            .map(|r| (r.id.as_str(), (route_key(r), r)))
            .collect();
        let handlers: std::collections::HashSet<String> = app
            .table
            .routes
            .iter()
            .filter(|r| r.kind == RouteKind::Handler)
            .map(route_key)
            .collect();
        let own = [opts.context.host.clone()];
        let no_mw = MiddlewareFacts::default();
        let mut cache: HashMap<UrlPat, Vec<String>> = HashMap::new();
        // Handlers by their first segment: a string that overlaps no handler and that
        // no rule rewrites matches none, without asking the matcher.
        let mut by_first: HashMap<&str, Vec<&Route>> = HashMap::new();
        let mut dynamic_first: Vec<&Route> = Vec::new();
        for r in app
            .table
            .routes
            .iter()
            .filter(|r| r.kind == RouteKind::Handler)
        {
            match r.segs.first() {
                Some(Seg::Static(v)) => by_first.entry(v.as_str()).or_default().push(r),
                _ => dynamic_first.push(r),
            }
        }
        let base_or_locale = app.config.base_path.is_some() || app.config.i18n.is_some();
        let may_match = |pat: &UrlPat| -> bool {
            if base_or_locale {
                return true;
            }
            let direct = match pat.lit(0) {
                Some(l) => by_first
                    .get(l.as_str())
                    .is_some_and(|rs| rs.iter().any(|r| overlaps(&r.segs, pat))),
                None => by_first.values().flatten().any(|r| overlaps(&r.segs, pat)),
            } || dynamic_first.iter().any(|r| overlaps(&r.segs, pat));
            direct || m.may_rewrite(pat)
        };
        strings
            .iter()
            .enumerate()
            .map(|(si, s)| {
                let mut all: Vec<String> = Vec::new();
                if !keep_strings[si] {
                    return all;
                }
                for pre in &prefixes {
                    let mut pat = s.pattern.clone();
                    if !pre.is_empty() {
                        let mut segs: Vec<Vec<Part>> =
                            pre.iter().map(|x| vec![Part::Lit(x.clone())]).collect();
                        segs.extend(pat.segs);
                        pat.segs = segs;
                    }
                    if !cache.contains_key(&pat) && !may_match(&pat) {
                        continue;
                    }
                    let rs = cache.entry(pat.clone()).or_insert_with(|| {
                        map_url(app, &m, &keys, &pat, InvokeKind::Fetch, &own, &no_mw)
                            .0
                            .into_iter()
                            .filter_map(|t| match t {
                                Target::Route(k) if handlers.contains(&k) => Some(k),
                                _ => None,
                            })
                            .collect()
                    });
                    for k in rs.iter() {
                        if !all.contains(k) {
                            all.push(k.clone());
                        }
                    }
                }
                all
            })
            .collect()
    };
    Prepared {
        mapped,
        string_routes,
    }
}

/// Builds the relation in its expanded form: per route, the invocations over
/// its module closure in `g` (runtime import edges from the route's segment
/// files), deduplicated by kind and target. This repeats the invocations of
/// shared code for every route; the shared form
/// ([`super::shared::shared_relation`]) is the one to compute, store and pass
/// on.
pub fn relation<G: GraphView>(
    app: &NextApp,
    g: &G,
    sites: &[Site],
    strings: &[UrlString],
    trpc: &TrpcIndex,
    opts: &RelationOptions,
) -> InvokeRelation {
    super::shared::shared_relation(app, g, sites, strings, trpc, opts).expand()
}

/// The sites some route (or the middleware) of the relation reaches, as `(file
/// relative to the graph root, line)` of every unknown dependency site: the
/// work order.
pub fn unknown_sites(sites: &[Site], root: &Path) -> Vec<(String, u32, String, String)> {
    let mut out = Vec::new();
    for s in sites {
        if !s.kind.is_dependency() {
            continue;
        }
        for t in &s.targets {
            if let SiteTarget::Unknown(r) = t {
                out.push((rel_path(root, &s.file), s.line, s.api.clone(), r.clone()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}
