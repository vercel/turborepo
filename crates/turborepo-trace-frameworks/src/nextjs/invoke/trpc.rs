//! tRPC: procedure calls, the router tree, and the HTTP entry.
//!
//! - **Server**: every `router({...})` / `createTRPCRouter({...})` /
//!   `t.router({...})` call is a router; its properties are procedures (a chain
//!   ending in `.query(…)`, `.mutation(…)` or `.subscription(…)`), nested
//!   routers (inline, a local binding or an import, also `lazy(() =>
//!   import(…))`), or spreads; `mergeRouters(a, b)` unites routers. The roots
//!   are the routers given to an HTTP adapter (`fetchRequestHandler({ router
//!   })`, `createNextApiHandler`, …) or, without one, the routers no other
//!   router nests. The tree gives every procedure path (`user.byId`) and the
//!   file that defines it.
//! - **Client**: a member chain `root.a.b.<terminal>(…)` whose terminal is a
//!   tRPC client method (`useQuery`, `useMutation`, `query`, `mutate`,
//!   `queryOptions`, `prefetch`, `invalidate`, …) and whose root is a tRPC
//!   client: a binding created by `createTRPCReact`, `createTRPCNext`,
//!   `createTRPCClient`, `createTRPCProxyClient`, `createTRPCOptionsProxy`,
//!   `createTRPCContext` (`useTRPC()`), `createServerSideHelpers`,
//!   `createHydrationHelpers`, `x.useUtils()` / `x.useContext()`,
//!   `createCaller(…)` / `createCallerFactory(…)(…)`, directly or through
//!   imports; or any root when the path is a procedure (or a router) of the
//!   tree. A server-side caller is called without a terminal:
//!   `caller.user.byId(input)`.
//! - **HTTP entry**: the file of the adapter call; when it is a route of the
//!   app, procedure targets name it as the serving route.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use turborepo_trace_lang_js::lexer::TokKind;

use super::model::{InvokeKind, Site, SiteTarget};
use crate::nextjs::jseval::{Decl, Files, JsFile, call_sites};

/// A procedure of the router tree.
#[derive(Clone, Debug)]
pub(crate) struct ProcDef {
    pub path: String,
    pub kind: String,
    pub file: PathBuf,
    pub line: u32,
}

/// The tRPC side of the scanned files.
#[derive(Clone, Debug, Default)]
pub(crate) struct TrpcScan {
    pub sites: Vec<Site>,
    pub procedures: Vec<ProcDef>,
    /// Files with an HTTP adapter call.
    pub handlers: Vec<PathBuf>,
}

const TERMINALS: &[&str] = &[
    "useQuery",
    "useSuspenseQuery",
    "useInfiniteQuery",
    "useSuspenseInfiniteQuery",
    "usePrefetchQuery",
    "usePrefetchInfiniteQuery",
    "useMutation",
    "useSubscription",
    "query",
    "mutate",
    "subscribe",
    "queryOptions",
    "infiniteQueryOptions",
    "mutationOptions",
    "subscriptionOptions",
    "queryKey",
    "infiniteQueryKey",
    "mutationKey",
    "queryFilter",
    "infiniteQueryFilter",
    "pathKey",
    "pathFilter",
    "fetch",
    "fetchInfinite",
    "prefetch",
    "prefetchInfinite",
    "ensureData",
    "invalidate",
    "refetch",
    "cancel",
    "reset",
    "setData",
    "setInfiniteData",
    "getData",
    "getInfiniteData",
    "revalidate",
];

/// Terminals no other common API has: enough to accept a chain whose path the
/// tree has.
const CREATORS: &[&str] = &[
    "createTRPCReact",
    "createTRPCNext",
    "createTRPCClient",
    "createTRPCProxyClient",
    "createTRPCOptionsProxy",
    "createTRPCContext",
    "createTRPCQueryUtils",
    "createServerSideHelpers",
    "createHydrationHelpers",
    "createCaller",
    "createCallerFactory",
    "useUtils",
    "useContext",
    "useTRPC",
    "useTRPCClient",
    "createSSGHelpers",
];

const ADAPTERS: &[&str] = &[
    "fetchRequestHandler",
    "createNextApiHandler",
    "createHTTPHandler",
    "createHTTPServer",
    "createExpressMiddleware",
    "nodeHTTPRequestHandler",
    "awsLambdaRequestHandler",
    "fastifyRequestHandler",
    "createOpenApiNextHandler",
    "createOpenApiFetchHandler",
];

fn is_router_fn(name: &str) -> bool {
    name == "router"
        || name == "mergeRouters"
        || (name.starts_with("create") && name.ends_with("Router"))
}

#[derive(Debug)]
enum Entry {
    Proc {
        kind: String,
        line: u32,
    },
    /// A nested router defined inline: its index.
    Inline(usize),
    /// A reference to resolve: the token of the name.
    Ref(usize),
}

#[derive(Debug)]
struct RouterDef {
    file: Rc<JsFile>,
    /// `(key, entry)`; an empty key merges the entry's children.
    entries: Vec<(String, Entry)>,
}

struct Trees {
    routers: Vec<RouterDef>,
    /// `(file id, token of the call)` → router.
    by_call: HashMap<(u32, usize), usize>,
}

/// The kind of the procedure whose builder chain is `[a, b)`: the last
/// top-level `.query(` / `.mutation(` / `.subscription(`.
fn procedure_kind(f: &JsFile, a: usize, b: usize) -> Option<String> {
    let mut kind = None;
    let mut k = a;
    while k + 2 < b {
        match f.kind(k) {
            TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                k = f.closer(k).filter(|&c| c < b)?;
            }
            TokKind::Dot
                if f.kind(k + 2) == TokKind::LParen
                    && matches!(f.text(k + 1), "query" | "mutation" | "subscription") =>
            {
                kind = Some(f.text(k + 1).to_string());
            }
            _ => {}
        }
        k += 1;
    }
    kind
}

impl Trees {
    /// Reads the router call whose callee starts at `at` and whose `(` is
    /// `open`.
    fn router(&mut self, f: &Rc<JsFile>, at: usize, open: usize, merge: bool) -> Option<usize> {
        if let Some(&i) = self.by_call.get(&(f.id, at)) {
            return Some(i);
        }
        let close = f.closer(open)?;
        let args = f.split_commas(open + 1, close);
        let idx = self.routers.len();
        self.routers.push(RouterDef {
            file: f.clone(),
            entries: Vec::new(),
        });
        self.by_call.insert((f.id, at), idx);
        let mut entries = Vec::new();
        if merge {
            for &(x, y) in &args {
                if x < y {
                    entries.push((String::new(), self.value(f, x, y)?));
                }
            }
        } else {
            let &(x, y) = args.first()?;
            if f.kind(x) != TokKind::LBrace || f.closer(x) != Some(y - 1) {
                self.routers.pop();
                self.by_call.remove(&(f.id, at));
                return None;
            }
            for (p, q) in f.split_commas(x + 1, y - 1) {
                if p >= q {
                    continue;
                }
                if f.kind(p) == TokKind::Ellipsis {
                    if let Some(e) = self.value(f, p + 1, q) {
                        entries.push((String::new(), e));
                    }
                    continue;
                }
                if !matches!(f.kind(p), TokKind::Ident | TokKind::Str) {
                    continue;
                }
                let key = f.text(p).trim_matches(['"', '\'']).to_string();
                let entry = if f.kind(p + 1) == TokKind::Colon {
                    self.value(f, p + 2, q)
                } else if q == p + 1 {
                    Some(Entry::Ref(p))
                } else {
                    None
                };
                if let Some(e) = entry {
                    entries.push((key, e));
                }
            }
        }
        self.routers[idx].entries = entries;
        Some(idx)
    }

    /// A router property value `[a, b)`: a procedure, an inline router, or a
    /// reference.
    fn value(&mut self, f: &Rc<JsFile>, a: usize, b: usize) -> Option<Entry> {
        if let Some(kind) = procedure_kind(f, a, b) {
            return Some(Entry::Proc {
                kind,
                line: f.line(a),
            });
        }
        // `router({...})`, `t.router({...})`, `lazy(() => import('./x'))`.
        let mut k = a;
        while f.kind(k) == TokKind::Ident
            && matches!(f.kind(k + 1), TokKind::Dot | TokKind::OptChain)
        {
            k += 2;
        }
        if f.kind(k) == TokKind::Ident && f.kind(k + 1) == TokKind::LParen {
            let name = f.text(k);
            if is_router_fn(name) {
                return self
                    .router(f, a, k + 1, name == "mergeRouters")
                    .map(Entry::Inline);
            }
        }
        if b == a + 1 && f.kind(a) == TokKind::Ident {
            return Some(Entry::Ref(a));
        }
        None
    }
}

/// The router a name at token `t` refers to, through bindings and imports.
fn resolve_router(
    files: &Files<'_>,
    trees: &Trees,
    f: &Rc<JsFile>,
    t: usize,
    depth: u32,
) -> Option<usize> {
    if depth > 6 {
        return None;
    }
    let name = f.text(t);
    let binding_router = |g: &Rc<JsFile>, init: (usize, usize)| -> Option<usize> {
        let mut a = init.0;
        if g.text(a) == "await" {
            a += 1;
        }
        if let Some(&i) = trees.by_call.get(&(g.id, a)) {
            return Some(i);
        }
        // An alias: `export const appRouter = base`.
        if init.1 == a + 1 && g.kind(a) == TokKind::Ident {
            return resolve_router(files, trees, g, a, depth + 1);
        }
        None
    };
    match f.lookup(name, t)? {
        Decl::Binding(i) => binding_router(f, f.bindings[name][i].init?),
        Decl::Import(_) => {
            let (g, n) = files.imported_decl(f, name)?;
            binding_router(&g, g.module_binding(&n)?.init?)
        }
        _ => None,
    }
}

fn chain(f: &JsFile, at: usize) -> Vec<String> {
    let mut v = vec![f.text(at).to_string()];
    let mut k = at + 1;
    while matches!(f.kind(k), TokKind::Dot | TokKind::OptChain) && f.kind(k + 1) == TokKind::Ident {
        v.push(f.text(k + 1).to_string());
        k += 2;
    }
    v
}

/// Whether the expression starting at `a` creates a tRPC client: a creator
/// call, or a method of a client (`trpc.useUtils()`, `createCaller(ctx)`).
fn creates_client(files: &Files<'_>, f: &Rc<JsFile>, init: (usize, usize), depth: u32) -> bool {
    let mut a = init.0;
    if f.text(a) == "await" {
        a += 1;
    }
    if f.kind(a) != TokKind::Ident {
        return false;
    }
    let c = chain(f, a);
    if c.iter().any(|n| CREATORS.contains(&n.as_str())) {
        return true;
    }
    // `useTRPC()` where `useTRPC` comes from `createTRPCContext()`,
    // `api.useUtils()`.
    is_client(files, f, a, depth + 1)
}

/// Whether the name at token `t` is a tRPC client (or a hook that returns one).
fn is_client(files: &Files<'_>, f: &Rc<JsFile>, t: usize, depth: u32) -> bool {
    if depth > 5 {
        return false;
    }
    let name = f.text(t);
    if CREATORS.contains(&name) {
        return true;
    }
    match f.lookup(name, t) {
        Some(Decl::Binding(i)) => f.bindings[name][i]
            .init
            .is_some_and(|init| creates_client(files, f, init, depth)),
        Some(Decl::Import(_)) => match files.imported_decl(f, name) {
            Some((g, n)) => g
                .module_binding(&n)
                .and_then(|b| b.init)
                .is_some_and(|init| creates_client(files, &g, init, depth)),
            None => false,
        },
        _ => false,
    }
}

/// Scans `paths` for the router tree, the HTTP entries and procedure calls.
pub(crate) fn scan_trpc(files: &Files<'_>, paths: &[PathBuf]) -> TrpcScan {
    let mut out = TrpcScan::default();
    // Only repositories that use tRPC.
    let mut any = false;
    let mut cands: Vec<Rc<JsFile>> = Vec::new();
    for p in paths {
        let Some(f) = files.get(p) else { continue };
        let has = |needle: &[u8]| f.src.windows(needle.len()).any(|w| w == needle);
        if has(b"@trpc/") {
            any = true;
        }
        if has(b"trpc") || has(b"TRPC") || has(b"outer") || has(b"api.") || has(b"caller") {
            cands.push(f);
        }
    }
    if !any {
        return out;
    }
    // Routers and adapters.
    let mut trees = Trees {
        routers: Vec::new(),
        by_call: HashMap::new(),
    };
    let mut roots: Vec<(Rc<JsFile>, usize)> = Vec::new();
    let mut per_file: Vec<(Rc<JsFile>, Vec<crate::nextjs::jseval::CallSite>)> = Vec::new();
    for f in &cands {
        let css = call_sites(f);
        for cs in &css {
            if is_router_fn(&cs.method)
                && !cs.is_new
                && cs.args.first().is_some_and(|&(x, _)| {
                    f.kind(x) == TokKind::LBrace || cs.method == "mergeRouters"
                })
            {
                trees.router(f, cs.at, cs.open, cs.method == "mergeRouters");
            }
            if ADAPTERS.contains(&cs.method.as_str())
                && let Some(&(x, y)) = cs.args.first()
                && f.kind(x) == TokKind::LBrace
                && f.closer(x) == Some(y - 1)
            {
                if !out.handlers.contains(&f.path) {
                    out.handlers.push(f.path.clone());
                }
                for (p, q) in f.split_commas(x + 1, y - 1) {
                    if f.text(p) == "router" {
                        // `router: appRouter` or the shorthand `router`.
                        let t = if f.kind(p + 1) == TokKind::Colon {
                            p + 2
                        } else {
                            p
                        };
                        if t < q && f.kind(t) == TokKind::Ident {
                            roots.push((f.clone(), t));
                        }
                    }
                }
            }
        }
        per_file.push((f.clone(), css));
    }
    // The tree from the roots.
    let mut root_ids: Vec<usize> = roots
        .iter()
        .filter_map(|(f, t)| resolve_router(files, &trees, f, *t, 0))
        .collect();
    if root_ids.is_empty() {
        // No adapter: the routers no other router nests.
        let mut nested: HashSet<usize> = HashSet::new();
        for r in &trees.routers {
            for (_, e) in &r.entries {
                match e {
                    Entry::Inline(i) => {
                        nested.insert(*i);
                    }
                    Entry::Ref(t) => {
                        if let Some(i) = resolve_router(files, &trees, &r.file, *t, 0) {
                            nested.insert(i);
                        }
                    }
                    Entry::Proc { .. } => {}
                }
            }
        }
        root_ids = (0..trees.routers.len())
            .filter(|i| !nested.contains(i))
            .collect();
    }
    root_ids.sort_unstable();
    root_ids.dedup();
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<(usize, String, u32)> =
        root_ids.iter().map(|&i| (i, String::new(), 0)).collect();
    while let Some((i, prefix, depth)) = stack.pop() {
        if depth > 12 {
            continue;
        }
        let r = &trees.routers[i];
        for (key, e) in &r.entries {
            let path = match (prefix.is_empty(), key.is_empty()) {
                (_, true) => prefix.clone(),
                (true, false) => key.clone(),
                (false, false) => format!("{prefix}.{key}"),
            };
            match e {
                Entry::Proc { kind, line } => {
                    if seen.insert(path.clone()) {
                        out.procedures.push(ProcDef {
                            path,
                            kind: kind.clone(),
                            file: r.file.path.clone(),
                            line: *line,
                        });
                    }
                }
                Entry::Inline(j) => stack.push((*j, path, depth + 1)),
                Entry::Ref(t) => {
                    if let Some(j) = resolve_router(files, &trees, &r.file, *t, 0) {
                        stack.push((j, path, depth + 1));
                    }
                }
            }
        }
    }
    out.procedures.sort_by(|a, b| a.path.cmp(&b.path));
    let known = |p: &str| {
        seen.contains(p)
            || out.procedures.iter().any(|d| {
                d.path.len() > p.len()
                    && d.path.starts_with(p)
                    && d.path.as_bytes()[p.len()] == b'.'
            })
    };
    // Calls.
    for (f, css) in &per_file {
        for cs in css {
            if cs.is_new || cs.path.starts_with("?.") {
                continue;
            }
            let c = chain(f, cs.at);
            if c.len() < 2 || c[0] == "this" {
                continue;
            }
            let terminal = c.last().map(String::as_str).unwrap_or("");
            let (path, api) = if TERMINALS.contains(&terminal) && c.len() >= 3 {
                (c[1..c.len() - 1].join("."), terminal.to_string())
            } else {
                // A server-side caller: `caller.user.byId(input)`.
                (c[1..].join("."), "caller".to_string())
            };
            let client = is_client(files, f, cs.at, 0);
            let accept = if api == "caller" {
                client && known(&path) && seen.contains(&path)
            } else {
                client || known(&path)
            };
            if !accept || path.is_empty() {
                continue;
            }
            out.sites.push(Site {
                file: f.path.clone(),
                line: f.line(cs.at),
                kind: InvokeKind::Trpc,
                api: api.clone(),
                method: None,
                targets: vec![SiteTarget::Procedure(path)],
                via: vec![format!("{}.{api}", c[0])],
            });
        }
    }
    out
}
