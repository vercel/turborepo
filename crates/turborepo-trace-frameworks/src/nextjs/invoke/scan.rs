//! The URL site scanner: `fetch`-like calls, fetch wrappers, SWR and navigation
//! sites.
//!
//! # Wrappers
//!
//! A site whose URL depends on a parameter of an enclosing named function
//! (after the evaluator followed bindings, callbacks and guards) and is not a
//! usable pattern on its own makes that function a *wrapper*: the site becomes
//! a summary of the function, and every call of the function (found through
//! imports and re-exports, in every scanned file) is evaluated instead, by
//! evaluating the wrapper's inner site again with the parameters bound to the
//! caller's argument expressions ([`Frame`]). A call that passes one of its own
//! parameters on makes its function a wrapper in turn. Nothing is assumed about
//! names: `fetchAPI(path)` adding an `/api` prefix behind a `startsWith` guard,
//! a class method, and a hook that forwards its argument to `useSWR` are the
//! same case.
//!
//! # Fetchers
//!
//! APIs with a fetcher callback (SWR) pass their key to it: an inline fetcher's
//! first parameter is bound to the key, a fetcher that is a wrapper is applied
//! to the key, and without a (resolvable) fetcher the key itself is the URL.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use super::model::{InvokeKind, Site, SiteTarget, UrlString};
use crate::nextjs::{
    fetch::{Style, UrlApi, UrlUse, apply_prefix, arg_value, is_self_url, scan_jsx, to_patterns},
    jseval::{
        CallSite, Decl, Eval, Files, FnKind, Frame, Imported, JsFile, Tpl, all_unknown, call_sites,
        demote, params_in, source_of,
    },
    source::Source,
    urlpat::{Part, Piece, UrlPat},
};

/// A function: `(file id, index in the file's functions)`.
pub(crate) type FnKey = (u32, u32);

/// Parameters as `(index, field path)`.
type ParamPaths = Vec<(u8, Vec<String>)>;

/// A call: file, callee token, `(` token, argument ranges.
type CallRow = (Rc<JsFile>, usize, usize, Vec<(usize, usize)>);

/// A use of a wrapper that is not a call: the wrapper and the place (`None`:
/// its definition).
type WrapperUse = (FnKey, Option<(Rc<JsFile>, usize)>);

/// A URL-consuming call: a table API, or a call of a wrapper.
struct Inner {
    file: Rc<JsFile>,
    /// Token of the callee.
    at: usize,
    args: Vec<(usize, usize)>,
    target: InnerTarget,
    kind: InvokeKind,
    label: String,
    /// Values bound directly to the callee's parameters instead of by an
    /// argument expression (a fetcher applied to an SWR key; one
    /// alternative of an argument): `(parameter index, field path, value)`.
    direct: Vec<(u8, Vec<String>, Vec<Tpl>)>,
    /// Wrapper nesting: 0 for a table API, one more than the wrapper's summary
    /// otherwise.
    level: u8,
    /// Set when the call's fetcher argument is a parameter of the enclosing
    /// function: the parameter's index. Callers that pass a fetcher of
    /// their own are bound through it.
    fetcher_fwd: Option<u8>,
}

/// An API that passes a key to a fetcher callback: a table entry, or a function
/// that forwards a key and a fetcher parameter to one (`useLazySWR(key,
/// fetcher)`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum FetcherApi {
    Table(usize),
    Derived(FnKey),
}

/// A function that forwards one of its parameters as the fetcher of a fetcher
/// API call.
struct Derived {
    /// The parameter that holds the fetcher.
    param: u8,
    /// The inner fetcher API call: its file, arguments and API.
    file: Rc<JsFile>,
    args: Vec<(usize, usize)>,
    inner: FetcherApi,
}

/// How a wrapper call is evaluated ([`Engine::variants`]).
enum Variants {
    /// No argument value: no request.
    None,
    /// The argument is unknown: its text and where it comes from.
    Unknown(String, Source),
    /// As it is.
    One,
    /// Once per alternative of the argument.
    Each(Vec<Rc<Inner>>),
}

/// The fetcher argument of a fetcher API call.
enum Fetcher {
    /// None, an options object, or something that cannot be resolved: the key
    /// is the URL.
    None,
    /// An inline function.
    Inline,
    /// A first-party function.
    Func(FnKey),
    /// A parameter of the enclosing named function: `(function, parameter
    /// index)`.
    Param(u32, u8),
}

const MAX_LEVEL: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InnerTarget {
    /// Index into the API table.
    Api(usize),
    /// A wrapper and the index of one of its summaries.
    Wrapper(FnKey, usize),
}

/// A call of a first-party function.
struct Call {
    file: Rc<JsFile>,
    at: usize,
    open: usize,
    args: Vec<(usize, usize)>,
    /// A JSX element (`<Comp a="x" b={y} />`): the component is called with its
    /// props.
    jsx: Option<Rc<JsxUse>>,
}

/// A JSX element of a component.
struct JsxUse {
    /// The tag (`Comp`, `Ns.Comp`).
    name: String,
    /// Byte offset of the `<`.
    off: usize,
    attrs: Vec<(String, JsxVal)>,
    /// `{...props}` is present: attributes not listed are unknown.
    spread: bool,
}

enum JsxVal {
    Lit(String),
    /// A token range.
    Expr(usize, usize),
    True,
}

/// The JSX elements of components (capitalized tags) in `f`.
fn jsx_uses(f: &JsFile) -> Vec<JsxUse> {
    let src = &f.src;
    let n = src.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if src[i] != b'<' {
            i += 1;
            continue;
        }
        let k = i;
        i += 1;
        if !src.get(k + 1).is_some_and(u8::is_ascii_uppercase) {
            continue;
        }
        let mut e = k + 1;
        while e < n && (src[e].is_ascii_alphanumeric() || src[e] == b'.' || src[e] == b'_') {
            e += 1;
        }
        if e >= n || !(src[e].is_ascii_whitespace() || src[e] == b'>' || src[e] == b'/') {
            continue;
        }
        let name = String::from_utf8_lossy(&src[k + 1..e]).into_owned();
        let mut attrs = Vec::new();
        let mut spread = false;
        let mut j = e;
        let mut ok = true;
        let mut steps = 0;
        loop {
            steps += 1;
            while j < n && src[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= n || steps > 200 {
                ok = false;
                break;
            }
            match src[j] {
                b'>' => break,
                b'/' if src.get(j + 1) == Some(&b'>') => break,
                b'{' => {
                    // `{...spread}` or a comment.
                    let mut d = 0;
                    while j < n {
                        match src[j] {
                            b'{' => d += 1,
                            b'}' => {
                                d -= 1;
                                if d == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    spread = true;
                    j += 1;
                }
                c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                    let a = j;
                    while j < n
                        && (src[j].is_ascii_alphanumeric()
                            || matches!(src[j], b'_' | b'-' | b':' | b'$'))
                    {
                        j += 1;
                    }
                    let attr = String::from_utf8_lossy(&src[a..j]).into_owned();
                    let mut v = j;
                    while v < n && src[v].is_ascii_whitespace() {
                        v += 1;
                    }
                    if src.get(v) != Some(&b'=') {
                        attrs.push((attr, JsxVal::True));
                        continue;
                    }
                    v += 1;
                    while v < n && src[v].is_ascii_whitespace() {
                        v += 1;
                    }
                    match src.get(v) {
                        Some(&q @ (b'"' | b'\'')) => {
                            let Some(end) = src[v + 1..].iter().position(|&b| b == q) else {
                                spread = true;
                                break;
                            };
                            attrs.push((
                                attr,
                                JsxVal::Lit(
                                    String::from_utf8_lossy(&src[v + 1..v + 1 + end]).into_owned(),
                                ),
                            ));
                            j = v + end + 2;
                        }
                        Some(b'{') => {
                            let a_tok = f.toks.partition_point(|t| (t.start as usize) <= v);
                            let b_tok = f.expr_end(a_tok, f.toks.len());
                            let Some(end) = f.toks.get(b_tok) else {
                                spread = true;
                                break;
                            };
                            if b_tok <= a_tok || src.get(end.start as usize) != Some(&b'}') {
                                spread = true;
                                break;
                            }
                            attrs.push((attr, JsxVal::Expr(a_tok, b_tok)));
                            j = end.start as usize + 1;
                        }
                        _ => {
                            spread = true;
                            break;
                        }
                    }
                }
                _ => {
                    // Not an element (`Array<Foo | Bar>`).
                    ok = attrs.is_empty() && !spread;
                    ok = !ok;
                    spread = true;
                    break;
                }
            }
        }
        if ok {
            out.push(JsxUse {
                name,
                off: k,
                attrs,
                spread,
            });
        }
    }
    out
}

/// A fetcher API call whose fetcher is a first-party function.
struct FetcherRef {
    file: Rc<JsFile>,
    at: usize,
    /// Token of the fetcher argument.
    arg: usize,
    api: FetcherApi,
    args: Vec<(usize, usize)>,
    label: String,
}

struct Engine<'a, 'r> {
    files: &'a Files<'r>,
    apis: &'a [UrlApi],
    api_index: HashMap<(Option<String>, String, bool), usize>,
    summaries: HashMap<FnKey, Vec<Rc<Inner>>>,
    calls: HashMap<FnKey, Vec<Call>>,
    fetcher_refs: HashMap<FnKey, Vec<FetcherRef>>,
    /// Per file: argument ranges of SWR / TanStack Query calls, with the kind.
    contexts: HashMap<u32, Vec<(usize, usize, InvokeKind)>>,
    /// Per file: fetcher API calls: `(open paren, api)`.
    fetcher_calls: HashMap<u32, Vec<(usize, FetcherApi)>>,
    derived: HashMap<FnKey, Derived>,
    /// Per summary: the parameters (index, field path) its URL depends on.
    summary_params: HashMap<(FnKey, usize), ParamPaths>,
    /// `TURBO_TRACE_DEBUG_INVOKE_TRACE=<path substring>`: print how the calls
    /// of matching files resolve (stderr).
    trace: Option<String>,
    /// The SWR / TanStack Query call around the calls of the evaluation in
    /// progress.
    ctx_kind: std::cell::Cell<Option<InvokeKind>>,
    queue: Vec<Rc<Inner>>,
    sites: Vec<Site>,
    seen: HashSet<(u32, usize, u32, u32, usize)>,
}

const QUERY_HOOKS: &[&str] = &[
    "useQuery",
    "useSuspenseQuery",
    "useInfiniteQuery",
    "useSuspenseInfiniteQuery",
    "useMutation",
    "useQueries",
    "useSuspenseQueries",
    "usePrefetchQuery",
    "queryOptions",
    "infiniteQueryOptions",
    "mutationOptions",
];

const QUERY_CLIENT_METHODS: &[&str] = &[
    "prefetchQuery",
    "fetchQuery",
    "ensureQueryData",
    "prefetchInfiniteQuery",
    "fetchInfiniteQuery",
    "ensureInfiniteQueryData",
];

fn is_query_module(m: &str) -> bool {
    m.starts_with("@tanstack/") && m.contains("query") || m == "react-query"
}

fn api_kind(api: &UrlApi) -> InvokeKind {
    match api.kind {
        UrlUse::Redirect => InvokeKind::Redirect,
        UrlUse::Navigation | UrlUse::Build => InvokeKind::Navigation,
        UrlUse::Fetch => {
            if api.module.as_deref().is_some_and(|m| m.starts_with("swr")) {
                InvokeKind::Swr
            } else {
                InvokeKind::Fetch
            }
        }
    }
}

fn api_label(api: &UrlApi) -> String {
    match &api.module {
        Some(m) => format!("{m}:{}", api.name),
        None => api.name.clone(),
    }
}

/// The coarse class of an unevaluated expression, as a `class: text` reason;
/// the source of a tagged text ([`Source::tag`]) stays at the end.
pub(crate) fn reason(text: &str) -> String {
    let (t, src) = Source::split(text.trim());
    if t.starts_with("wrapper-value: ") {
        return src.tag(t);
    }
    let ident = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    };
    let inner = t
        .strip_prefix("${")
        .and_then(|x| x.strip_suffix('}'))
        .filter(|x| !x.contains("${"))
        .unwrap_or(t);
    let class = if t.starts_with("no known path part") {
        "no-path"
    } else if t.starts_with("origin-relative wildcard") {
        "origin-only"
    } else if t.starts_with("open pattern") {
        "open-prefix"
    } else if t.starts_with("more than ") {
        "alternatives"
    } else if ident(inner) {
        "identifier"
    } else if inner.split('.').all(ident) {
        "member"
    } else if inner.ends_with(')') && !inner.contains("${") {
        "call"
    } else if t.contains("${") || t.starts_with('`') {
        "template"
    } else {
        "expression"
    };
    src.tag(&format!("{class}: {t}"))
}

impl<'a, 'r> Engine<'a, 'r> {
    fn named(f: &JsFile, func: u32) -> bool {
        f.funcs
            .get(func as usize)
            .is_some_and(|fd| fd.kind != FnKind::Anon && fd.name.is_some())
    }

    /// The chain of names of a call's callee, from the token `at`.
    fn chain(f: &JsFile, at: usize) -> Vec<String> {
        use turborepo_trace_lang_js::lexer::TokKind;
        let mut v = vec![f.text(at).to_string()];
        let mut k = at + 1;
        while matches!(f.kind(k), TokKind::Dot | TokKind::OptChain)
            && f.kind(k + 1) == TokKind::Ident
        {
            v.push(f.text(k + 1).to_string());
            k += 2;
        }
        v
    }

    /// The first-party function a name chain at token `at` refers to.
    fn resolve_fn(&self, f: &Rc<JsFile>, at: usize, chain: &[String]) -> Option<FnKey> {
        use turborepo_trace_lang_js::lexer::TokKind;
        let ev = Eval::new(self.files);
        let root = chain.first()?;
        if chain.len() == 1 {
            return match f.lookup(root, at)? {
                Decl::Func(fi) => Some((f.id, fi as u32)),
                Decl::Import(_) => {
                    let (g, n) = self.files.imported_decl(f, root)?;
                    let fi = g.module_fn(&n)?;
                    Some((g.id, fi as u32))
                }
                _ => None,
            };
        }
        if root == "this" && chain.len() == 2 {
            // A method of the enclosing class or object.
            let me = f
                .funcs
                .iter()
                .filter(|fd| fd.kind == FnKind::Method && fd.body.0 <= at && at < fd.body.1)
                .min_by_key(|fd| fd.start)?;
            let container = f.open_of[me.start];
            let fi = f.funcs.iter().position(|fd| {
                fd.kind == FnKind::Method
                    && fd.name.as_deref() == Some(chain[1].as_str())
                    && f.open_of[fd.start] == container
            })?;
            return Some((f.id, fi as u32));
        }
        let path = &chain[1..];
        match f.lookup(root, at) {
            Some(Decl::Binding(bi)) => {
                let b = &f.bindings[root.as_str()][bi];
                let (x, _) = b.init?;
                if f.kind(x) == TokKind::LBrace && b.path.is_empty() {
                    let fi = ev.object_fn(f, x, path)?;
                    return Some((f.id, fi as u32));
                }
            }
            Some(Decl::Import(ii)) => {
                if f.imports[ii].imported == Imported::Namespace {
                    let g = self.files.imported_namespace(f, root)?;
                    let (h, n) = self.files.resolve_export(&g, &path[0])?;
                    if path.len() == 1 {
                        let fi = h.module_fn(&n)?;
                        return Some((h.id, fi as u32));
                    }
                    let b = h.module_binding(&n)?;
                    let (x, _) = b.init?;
                    let fi = ev.object_fn(&h, x, &path[1..])?;
                    return Some((h.id, fi as u32));
                }
                let (g, n) = self.files.imported_decl(f, root)?;
                if let Some(b) = g.module_binding(&n)
                    && let Some((x, _)) = b.init
                    && g.kind(x) == TokKind::LBrace
                {
                    let fi = ev.object_fn(&g, x, path)?;
                    return Some((g.id, fi as u32));
                }
                // A static method of an imported class.
                if path.len() == 1
                    && let Some(c) = g.class(&n)
                {
                    return Self::class_method(&g, c.body, &path[0]).map(|fi| (g.id, fi));
                }
            }
            Some(Decl::Class(ci)) if path.len() == 1 => {
                return Self::class_method(f, f.classes[ci].body, &path[0]).map(|fi| (f.id, fi));
            }
            _ => {}
        }
        // `obj.method()` where `obj` is an instance of a class.
        if path.len() == 1
            && let Some(cls) = f.var_class.get(root)
            && let Some((cf, ci)) = self.files.find_class(f, cls)
        {
            return Self::class_method(&cf, cf.classes[ci].body, &path[0]).map(|fi| (cf.id, fi));
        }
        None
    }

    fn class_method(f: &JsFile, body: (usize, usize), name: &str) -> Option<u32> {
        f.funcs
            .iter()
            .position(|fd| {
                fd.name.as_deref() == Some(name)
                    && matches!(fd.kind, FnKind::Method | FnKind::Bound)
                    && f.open_of.get(fd.start).copied() == Some(body.0 as u32)
            })
            .map(|i| i as u32)
    }

    fn match_api(&self, f: &JsFile, cs: &CallSite) -> Option<usize> {
        let &i = self
            .api_index
            .get(&(cs.module.clone(), cs.path.clone(), cs.is_new))?;
        if cs.module.is_none() {
            // A global: not a local declaration of the same name.
            let root = f.text(cs.at);
            let global_prefix = matches!(root, "window" | "globalThis" | "self" | "global");
            if !global_prefix && f.lookup(root, cs.at).is_some() {
                return None;
            }
        }
        Some(i)
    }

    /// Indexes one file: API sites, calls of first-party functions, fetcher
    /// references.
    fn index_file(&mut self, f: &Rc<JsFile>) {
        for cs in call_sites(f) {
            if let Some(ai) = self.match_api(f, &cs) {
                let api = &self.apis[ai];
                if api.kind == UrlUse::Build {
                    // `new URL(x, base)` builds a URL; the site that uses it is evaluated.
                    continue;
                }
                let kind = api_kind(api);
                if let (Some(&(a, _)), Some(&(_, b))) = (cs.args.first(), cs.args.last())
                    && kind == InvokeKind::Swr
                {
                    self.contexts
                        .entry(f.id)
                        .or_default()
                        .push((a, b, InvokeKind::Swr));
                }
                let mut fwd = None;
                if let Some(j) = api.fetcher {
                    self.fetcher_calls
                        .entry(f.id)
                        .or_default()
                        .push((cs.open, FetcherApi::Table(ai)));
                    match self.classify_fetcher(f, &cs.args, j) {
                        Fetcher::Inline => continue,
                        Fetcher::Func(key) => {
                            self.fetcher_refs.entry(key).or_default().push(FetcherRef {
                                file: f.clone(),
                                at: cs.at,
                                arg: cs.args[j as usize].0,
                                api: FetcherApi::Table(ai),
                                args: cs.args.clone(),
                                label: api_label(api),
                            });
                            continue;
                        }
                        Fetcher::Param(func, idx) => {
                            self.derived.entry((f.id, func)).or_insert_with(|| Derived {
                                param: idx,
                                file: f.clone(),
                                args: cs.args.clone(),
                                inner: FetcherApi::Table(ai),
                            });
                            fwd = Some(idx);
                        }
                        Fetcher::None => {}
                    }
                }
                self.queue.push(Rc::new(Inner {
                    file: f.clone(),
                    at: cs.at,
                    args: cs.args.clone(),
                    target: InnerTarget::Api(ai),
                    kind,
                    label: api_label(api),
                    direct: Vec::new(),
                    level: 0,
                    fetcher_fwd: fwd,
                }));
                continue;
            }
            // TanStack Query: the calls whose arguments hold `queryFn` / `mutationFn`.
            let query = match &cs.module {
                Some(m) => is_query_module(m) && QUERY_HOOKS.contains(&cs.method.as_str()),
                None => cs.receiver.is_some() && QUERY_CLIENT_METHODS.contains(&cs.method.as_str()),
            } || (cs.path.starts_with("?.")
                && QUERY_CLIENT_METHODS.contains(&cs.method.as_str()));
            if query && let (Some(&(a, _)), Some(&(_, b))) = (cs.args.first(), cs.args.last()) {
                self.contexts
                    .entry(f.id)
                    .or_default()
                    .push((a, b, InvokeKind::Query));
            }
            if cs.is_new || cs.path.starts_with("?.") {
                continue;
            }
            let chain = Self::chain(f, cs.at);
            if let Some(t) = &self.trace
                && f.path.to_string_lossy().contains(t.as_str())
            {
                let imp = f.import(&chain[0]).map(|i| {
                    let p = self.files.resolve(&f.path, &i.module);
                    let d = self.files.imported_decl(f, &chain[0]);
                    format!(
                        "import {} -> {:?} decl {:?}",
                        i.module,
                        p,
                        d.map(|(g, n)| (g.path.clone(), n.clone(), g.module_fn(&n)))
                    )
                });
                eprintln!(
                    "call {}:{} {:?} -> {:?} [{:?}]",
                    f.path.display(),
                    f.line(cs.at),
                    chain,
                    self.resolve_fn(f, cs.at, &chain),
                    imp
                );
            }
            if let Some(key) = self.resolve_fn(f, cs.at, &chain) {
                self.calls.entry(key).or_default().push(Call {
                    file: f.clone(),
                    at: cs.at,
                    open: cs.open,
                    args: cs.args,
                    jsx: None,
                });
            }
        }
        // JSX elements call their components with the props.
        if f.src.contains(&b'<') {
            for u in jsx_uses(f) {
                let at = f.toks.partition_point(|t| (t.start as usize) < u.off);
                let chain: Vec<String> = u.name.split('.').map(String::from).collect();
                if let Some(key) =
                    self.resolve_fn(f, at.min(f.toks.len().saturating_sub(1)), &chain)
                {
                    self.calls.entry(key).or_default().push(Call {
                        file: f.clone(),
                        at: at.min(f.toks.len().saturating_sub(1)),
                        open: 0,
                        args: Vec::new(),
                        jsx: Some(Rc::new(u)),
                    });
                }
            }
        }
    }

    /// The props of a JSX element as direct bindings of the component's first
    /// parameter, for the property paths `ps` a summary depends on.
    fn jsx_direct(
        &self,
        file: &Rc<JsFile>,
        u: &JsxUse,
        ps: &[(u8, Vec<String>)],
    ) -> Vec<(u8, Vec<String>, Vec<Tpl>)> {
        let ev = Eval::symbolic(self.files);
        let mut out = Vec::new();
        for (idx, path) in ps {
            if *idx != 0 {
                continue;
            }
            let Some(name) = path.first() else {
                out.push((0, Vec::new(), vec![vec![Piece::wild("props")]]));
                continue;
            };
            let v = match u.attrs.iter().rev().find(|(n, _)| n == name) {
                Some((_, JsxVal::Lit(s))) if path.len() == 1 => vec![vec![Piece::Lit(s.clone())]],
                Some((_, JsxVal::Expr(a, b))) => ev.eval_field(file, *a, *b, &path[1..]),
                Some(_) => vec![vec![Piece::wild(name)]],
                None if u.spread => vec![vec![Piece::wild(name)]],
                // Not passed: the parameter's default applies.
                None => continue,
            };
            out.push((0, path.clone(), v));
        }
        out
    }

    /// The value an SWR-style API passes to its fetcher for the key expression
    /// `[a, b)`: `(field path, value)` pairs (the whole key, and the
    /// elements of an array key).
    fn key_values(
        &self,
        ev: &Eval<'_, '_>,
        f: &Rc<JsFile>,
        (a, b): (usize, usize),
    ) -> Vec<(Vec<String>, Vec<Tpl>)> {
        use turborepo_trace_lang_js::lexer::TokKind;
        let mut out = Vec::new();
        // A function key: its results; a conditional key: its branches.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        Self::key_exprs(f, (a, b), 0, &mut ranges);
        let mut whole: Vec<Tpl> = Vec::new();
        let mut elems: Vec<Vec<Tpl>> = Vec::new();
        for (x, y) in ranges {
            if f.kind(x) == TokKind::LBracket && f.closer(x) == Some(y - 1) {
                for (n, (p, q)) in f.split_commas(x + 1, y - 1).into_iter().enumerate() {
                    if elems.len() <= n {
                        elems.resize(n + 1, Vec::new());
                    }
                    elems[n].extend(ev.eval(f, p, q));
                }
            } else {
                whole.extend(ev.eval(f, x, y));
            }
        }
        if whole.is_empty()
            && let Some(first) = elems.first()
        {
            // An array key with a fetcher that takes the URL as its first argument.
            whole = first.clone();
        }
        if !whole.is_empty() {
            out.push((Vec::new(), whole));
        }
        for (n, v) in elems.into_iter().enumerate() {
            if !v.is_empty() {
                out.push((vec![n.to_string()], v));
            }
        }
        out
    }

    /// What the fetcher argument (index `j`) of a fetcher API call is.
    fn classify_fetcher(&self, f: &Rc<JsFile>, args: &[(usize, usize)], j: u8) -> Fetcher {
        use turborepo_trace_lang_js::lexer::TokKind;
        let Some(&(x, y)) = args.get(j as usize) else {
            return Fetcher::None;
        };
        if x >= y {
            return Fetcher::None;
        }
        if f.fn_at(x, y).is_some() {
            return Fetcher::Inline;
        }
        if f.kind(x) != TokKind::Ident {
            return Fetcher::None;
        }
        let chain = Self::chain(f, x);
        // `fetcher ?? null`.
        let mut y = y;
        if y >= x + 3
            && matches!(f.text(y - 2), "??" | "||")
            && matches!(f.text(y - 1), "null" | "undefined")
        {
            y -= 2;
        }
        if x + 2 * chain.len() - 1 != y {
            return Fetcher::None;
        }
        if chain.len() == 1
            && let Some(Decl::Param { func, param }) = f.lookup(&chain[0], x)
        {
            let p = &f.funcs[func].params[param];
            if p.field.is_none() && !p.rest && Self::named(f, func as u32) {
                return Fetcher::Param(func as u32, p.index);
            }
            return Fetcher::None;
        }
        match self.resolve_fn(f, x, &chain) {
            Some(key) => Fetcher::Func(key),
            None => Fetcher::None,
        }
    }

    /// Functions that forward a fetcher parameter to a fetcher API are fetcher
    /// APIs themselves: their calls bind inline fetchers and fetcher
    /// functions too.
    fn expand_derived(&mut self) {
        let mut work: Vec<FnKey> = self.derived.keys().copied().collect();
        let mut rounds = 0;
        while let Some(k) = work.pop() {
            rounds += 1;
            if rounds > 10_000 {
                break;
            }
            let param = self.derived[&k].param;
            let label = self
                .files
                .by_id(k.0)
                .and_then(|g| g.funcs[k.1 as usize].name.clone())
                .unwrap_or_else(|| "wrapper".into());
            let calls: Vec<CallRow> = self
                .calls
                .get(&k)
                .map(|v| {
                    v.iter()
                        .map(|c| (c.file.clone(), c.at, c.open, c.args.clone()))
                        .collect()
                })
                .unwrap_or_default();
            for (file, at, open, args) in calls {
                self.fetcher_calls
                    .entry(file.id)
                    .or_default()
                    .push((open, FetcherApi::Derived(k)));
                if let (Some(&(a, _)), Some(&(_, b))) = (args.first(), args.last()) {
                    self.contexts
                        .entry(file.id)
                        .or_default()
                        .push((a, b, InvokeKind::Swr));
                }
                match self.classify_fetcher(&file, &args, param) {
                    Fetcher::Func(g) => {
                        self.fetcher_refs.entry(g).or_default().push(FetcherRef {
                            file: file.clone(),
                            at,
                            arg: args[param as usize].0,
                            api: FetcherApi::Derived(k),
                            args,
                            label: label.clone(),
                        });
                    }
                    Fetcher::Param(func, idx) => {
                        let key2 = (file.id, func);
                        if let std::collections::hash_map::Entry::Vacant(e) =
                            self.derived.entry(key2)
                        {
                            e.insert(Derived {
                                param: idx,
                                file: file.clone(),
                                args,
                                inner: FetcherApi::Derived(k),
                            });
                            work.push(key2);
                        }
                    }
                    Fetcher::Inline | Fetcher::None => {}
                }
            }
        }
    }

    /// The key values a fetcher API call (arguments `args` in `f`) passes to
    /// its fetcher.
    fn fetcher_key(
        &self,
        api: FetcherApi,
        f: &Rc<JsFile>,
        args: &[(usize, usize)],
        ev: &Eval<'_, '_>,
        depth: u32,
    ) -> Vec<(Vec<String>, Vec<Tpl>)> {
        match api {
            FetcherApi::Table(ai) => match args.get(self.apis[ai].arg as usize) {
                Some(&r) => self.key_values(ev, f, r),
                None => Vec::new(),
            },
            FetcherApi::Derived(k) => {
                let Some(d) = self.derived.get(&k) else {
                    return Vec::new();
                };
                if depth > 6 {
                    return Vec::new();
                }
                let frame = Frame {
                    file: d.file.clone(),
                    func: k.1 as usize,
                    caller: f.clone(),
                    args: args.to_vec(),
                    parent: ev.env,
                    direct: Vec::new(),
                    id: ev.frame_id(),
                };
                let e2 = ev.within(Some(&frame));
                self.fetcher_key(d.inner, &d.file, &d.args, &e2, depth + 1)
            }
        }
    }

    /// The expressions a key can be: the results of a function key, the
    /// branches of `cond ? [url, id] : null`, the right side of `cond &&
    /// key`.
    fn key_exprs(f: &JsFile, (a, b): (usize, usize), depth: u32, out: &mut Vec<(usize, usize)>) {
        use turborepo_trace_lang_js::lexer::TokKind;
        if a >= b || depth > 6 {
            return;
        }
        if let Some(rs) = f.function_results(a, b) {
            for r in rs {
                Self::key_exprs(f, r, depth + 1, out);
            }
            return;
        }
        if f.kind(a) == TokKind::LParen && f.closer(a) == Some(b - 1) {
            return Self::key_exprs(f, (a + 1, b - 1), depth + 1, out);
        }
        // Top-level `?:`, `&&`.
        let mut q = None;
        let mut and = None;
        let mut k = a;
        while k < b {
            match f.kind(k) {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    match f.closer(k).filter(|&c| c < b) {
                        Some(c) => k = c,
                        None => break,
                    }
                }
                TokKind::Arrow => {
                    q = None;
                    and = None;
                    break;
                }
                TokKind::Question if q.is_none() => q = Some(k),
                TokKind::Op if f.text(k) == "&&" => and = Some(k),
                _ => {}
            }
            k += 1;
        }
        if let Some(qi) = q {
            // The matching `:`.
            let mut k = qi + 1;
            let mut nest = 0;
            while k < b {
                match f.kind(k) {
                    TokKind::LParen
                    | TokKind::LBracket
                    | TokKind::LBrace
                    | TokKind::TemplateHead => match f.closer(k).filter(|&c| c < b) {
                        Some(c) => k = c,
                        None => break,
                    },
                    TokKind::Question => nest += 1,
                    TokKind::Colon if nest == 0 => {
                        Self::key_exprs(f, (qi + 1, k), depth + 1, out);
                        Self::key_exprs(f, (k + 1, b), depth + 1, out);
                        return;
                    }
                    TokKind::Colon => nest -= 1,
                    _ => {}
                }
                k += 1;
            }
        } else if let Some(ai) = and {
            return Self::key_exprs(f, (ai + 1, b), depth + 1, out);
        }
        if b == a + 1 && matches!(f.text(a), "null" | "undefined") {
            return;
        }
        out.push((a, b));
    }

    /// If token `at` is inside an inline fetcher of a fetcher API call: the
    /// fetcher's function index and the key values bound to its first
    /// parameter.
    #[allow(clippy::type_complexity)]
    fn fetcher_binding(
        &self,
        ev: &Eval<'_, '_>,
        f: &Rc<JsFile>,
        at: usize,
    ) -> Option<(usize, Vec<(Vec<String>, Vec<Tpl>)>)> {
        let calls = self.fetcher_calls.get(&f.id)?;
        let mut best: Option<(usize, usize, FetcherApi, Vec<(usize, usize)>)> = None;
        for &(open, api) in calls {
            let Some(close) = f.closer(open) else {
                continue;
            };
            if !(open < at && at < close) {
                continue;
            }
            let j = match api {
                FetcherApi::Table(ai) => match self.apis[ai].fetcher {
                    Some(j) => j,
                    None => continue,
                },
                FetcherApi::Derived(k) => match self.derived.get(&k) {
                    Some(d) => d.param,
                    None => continue,
                },
            } as usize;
            let args = f.split_commas(open + 1, close);
            let Some(&(x, y)) = args.get(j) else { continue };
            if !(x <= at && at < y) {
                continue;
            }
            let Some(fi) = f.fn_at(x, y) else { continue };
            if best.as_ref().is_none_or(|b| close - open < b.0) {
                best = Some((close - open, fi, api, args));
            }
        }
        let (_, fi, api, args) = best?;
        Some((fi, self.fetcher_key(api, f, &args, ev, 0)))
    }

    /// The URLs, method values and wrapper chain of `inner` in the environment
    /// of `ev`.
    fn inner_eval(
        &self,
        inner: &Inner,
        ev: &Eval<'_, '_>,
        depth: u32,
    ) -> (Vec<Tpl>, Vec<Tpl>, Vec<String>) {
        // An SWR or TanStack Query call around this call, here or in a wrapper's body.
        if self.ctx_kind.get().is_none()
            && let Some(cx) = self.contexts.get(&inner.file.id)
            && let Some(&(_, _, k)) = cx
                .iter()
                .filter(|(a, b, _)| *a <= inner.at && inner.at < *b)
                .min_by_key(|(a, b, _)| b - a)
        {
            self.ctx_kind.set(Some(k));
        }
        // Inside an inline fetcher: its first parameter is the key.
        if let Some((fi, vals)) = self.fetcher_binding(ev, &inner.file, inner.at) {
            let frame = Frame {
                file: inner.file.clone(),
                func: fi,
                caller: inner.file.clone(),
                args: Vec::new(),
                parent: ev.env,
                direct: vals.into_iter().map(|(p, v)| (0u8, p, v)).collect(),
                id: ev.frame_id(),
            };
            let e2 = ev.within(Some(&frame));
            return self.inner_eval_bound(inner, &e2, depth);
        }
        self.inner_eval_bound(inner, ev, depth)
    }

    fn inner_eval_bound(
        &self,
        inner: &Inner,
        ev: &Eval<'_, '_>,
        depth: u32,
    ) -> (Vec<Tpl>, Vec<Tpl>, Vec<String>) {
        match inner.target {
            InnerTarget::Api(ai) => {
                let api = &self.apis[ai];
                let Some(&range) = inner.args.get(api.arg as usize) else {
                    return (Vec::new(), Vec::new(), vec![inner.label.clone()]);
                };
                let mut tpls = arg_value(ev, &inner.file, range, api.field.as_deref());
                if let Some(p) = &api.prefix {
                    for t in &mut tpls {
                        apply_prefix(t, p);
                    }
                }
                let method = match &api.method {
                    Some(m) => vec![vec![Piece::Lit(m.clone())]],
                    None => match inner.args.get(api.arg as usize + 1) {
                        Some(&(x, y))
                            if x < y
                                && api.kind == UrlUse::Fetch
                                && api.style != Style::Jsx
                                && api.field.is_none() =>
                        {
                            ev.eval_field(&inner.file, x, y, &["method".to_string()])
                        }
                        _ => Vec::new(),
                    },
                };
                (tpls, method, vec![inner.label.clone()])
            }
            InnerTarget::Wrapper(key, j) => {
                let unknown = || {
                    (
                        vec![vec![Piece::wild(&inner.label)]],
                        Vec::new(),
                        vec![inner.label.clone()],
                    )
                };
                if depth > 12 {
                    return unknown();
                }
                let (Some(g), Some(callee)) = (
                    self.files.by_id(key.0),
                    self.summaries.get(&key).and_then(|v| v.get(j)).cloned(),
                ) else {
                    return unknown();
                };
                let frame = Frame {
                    file: g,
                    func: key.1 as usize,
                    caller: inner.file.clone(),
                    args: inner.args.clone(),
                    parent: ev.env,
                    direct: inner.direct.clone(),
                    id: ev.frame_id(),
                };
                let e2 = ev.within(Some(&frame));
                let (u, m, mut via) = self.inner_eval(&callee, &e2, depth + 1);
                via.insert(0, inner.label.clone());
                (u, m, via)
            }
        }
    }

    fn patterns(tpls: &[Tpl], text: &str) -> (Vec<UrlPat>, Vec<String>) {
        let d = demote(tpls);
        if all_unknown(&d) {
            let r = match d.first().and_then(|t| t.first()) {
                Some(Piece::Wild { reason, .. }) if d.len() == 1 && !reason.is_empty() => {
                    reason.clone()
                }
                _ => text.to_string(),
            };
            return (Vec::new(), vec![source_of(&d).tag(&r)]);
        }
        to_patterns(&d, None)
    }

    fn weak(urls: &[UrlPat], unresolved: &[String]) -> bool {
        !unresolved.is_empty()
            || urls.is_empty()
            || urls.iter().any(|u| {
                u.open_tail
                    || !u
                        .segs
                        .iter()
                        .flatten()
                        .any(|p| matches!(p, Part::Lit(t) if !t.is_empty()))
            })
    }

    /// How to evaluate a wrapper call: when the wrapper's URL depends on one
    /// parameter, an unknown argument is the unknown (the wrapper is not
    /// evaluated), no argument value is no request, and several
    /// alternatives are evaluated one by one, so that the wrapper's
    /// guards decide for each.
    fn variants(&self, inner: &Rc<Inner>, ev: &Eval<'_, '_>, forward: bool) -> Variants {
        let InnerTarget::Wrapper(key, j) = inner.target else {
            return Variants::One;
        };
        let Some(ps) = self.summary_params.get(&(key, j)) else {
            return Variants::One;
        };
        let f = &inner.file;
        // Inside an inline fetcher the argument may be the fetcher's parameter.
        let binding = self.fetcher_binding(ev, f, inner.at);
        let frame = binding.map(|(fi, vals)| Frame {
            file: f.clone(),
            func: fi,
            caller: f.clone(),
            args: Vec::new(),
            parent: ev.env,
            direct: vals.into_iter().map(|(p, v)| (0u8, p, v)).collect(),
            id: ev.frame_id(),
        });
        let e2 = ev.within(frame.as_ref().or(ev.env));
        let value = |idx: u8, path: &[String]| -> Option<(Vec<Tpl>, String)> {
            if let Some((_, _, v)) = inner
                .direct
                .iter()
                .find(|(i, p, _)| *i == idx && p.as_slice() == path)
            {
                return Some((v.clone(), inner.label.clone()));
            }
            if !inner.direct.is_empty() {
                return None;
            }
            let &(a, b) = inner.args.get(idx as usize)?;
            Some((e2.eval_field(f, a, b, path), f.range_text(a, b)))
        };
        if ps.len() != 1 {
            // Several parameters: unknown when each of them is.
            let mut text = None;
            let mut src = Source::Code;
            for (idx, path) in ps {
                match value(*idx, path) {
                    Some((v, t))
                        if !v.is_empty()
                            && !(forward
                                && params_in(&v).iter().any(|r| {
                                    self.files
                                        .by_id(r.file)
                                        .is_some_and(|g| Self::named(&g, r.func))
                                }))
                            && all_unknown(&demote(&v)) =>
                    {
                        src = src.max(source_of(&v));
                        text.get_or_insert(t);
                    }
                    _ => return Variants::One,
                }
            }
            return match text {
                Some(t) => Variants::Unknown(t, src),
                None => Variants::One,
            };
        }
        let (idx, path) = &ps[0];
        let Some((vals, text)) = value(*idx, path) else {
            return Variants::One;
        };
        if vals.is_empty() {
            return Variants::None;
        }
        // The path is unknown when nothing before a query string is known.
        let opaque = |v: &[Tpl]| {
            demote(v).iter().all(|t| {
                for p in t {
                    match p {
                        Piece::Lit(l) if l.is_empty() => {}
                        Piece::Lit(l) if l.starts_with(['?', '#']) => return true,
                        Piece::Lit(_) => return false,
                        _ => {}
                    }
                }
                true
            })
        };
        // Parameters of callbacks have no callers to bind them: unknown values. So are
        // all parameters in the conservative form of a wrapper (a use that is
        // not a call).
        let bindable = forward
            && params_in(&vals).iter().any(|r| {
                self.files
                    .by_id(r.file)
                    .is_some_and(|g| Self::named(&g, r.func))
            });
        if !bindable && opaque(&vals) {
            let text = match vals.first().and_then(|t| t.first()) {
                Some(Piece::Wild { reason, .. }) if vals.len() == 1 && !reason.is_empty() => {
                    reason.clone()
                }
                _ => text,
            };
            return Variants::Unknown(text, source_of(&vals));
        }
        if vals.len() < 2 || vals.len() > 12 {
            return Variants::One;
        }
        Variants::Each(
            vals.into_iter()
                .map(|v| {
                    Rc::new(Inner {
                        file: inner.file.clone(),
                        at: inner.at,
                        args: inner.args.clone(),
                        target: inner.target,
                        kind: inner.kind,
                        label: inner.label.clone(),
                        direct: vec![(*idx, path.clone(), vec![v])],
                        level: inner.level,
                        fetcher_fwd: inner.fetcher_fwd,
                    })
                })
                .collect(),
        )
    }

    /// Evaluates one site: it becomes a summary of an enclosing function (a
    /// wrapper), or a site. With `forward` off it is always a site (the
    /// conservative form of a wrapper's inner call, for uses of the wrapper
    /// that are not calls).
    fn process(&mut self, inner: Rc<Inner>, forward: bool, at: Option<(Rc<JsFile>, usize)>) {
        let f = inner.file.clone();
        let ev = Eval::symbolic(self.files);
        self.ctx_kind.set(None);
        let (tpls, method, via) = match self.variants(&inner, &ev, forward) {
            Variants::None => return,
            Variants::Unknown(text, src) => (
                vec![vec![Piece::Wild {
                    multi: true,
                    reason: text,
                    src,
                }]],
                Vec::new(),
                vec![inner.label.clone()],
            ),
            Variants::One => self.inner_eval(&inner, &ev, 0),
            Variants::Each(vs) => {
                let mut tpls = Vec::new();
                let mut method = Vec::new();
                let mut via = Vec::new();
                for v in vs {
                    let (t, m, w) = self.inner_eval(&v, &ev, 0);
                    for x in t {
                        if !tpls.contains(&x) {
                            tpls.push(x);
                        }
                    }
                    for x in m {
                        if !method.contains(&x) {
                            method.push(x);
                        }
                    }
                    via = w;
                }
                (tpls, method, via)
            }
        };
        // The conservative form of a wrapper that takes whole URLs (some alternative is
        // the parameter itself, after an optional origin): any URL, an unknown
        // target. A wrapper that adds a path prefix keeps its open pattern.
        let (tpls, method, via) = if !forward
            && tpls.iter().any(|t| {
                let mut it = t
                    .iter()
                    .filter(|p| !matches!(p, Piece::Lit(l) if l.is_empty()));
                match it.next() {
                    Some(Piece::Param(_)) => true,
                    Some(Piece::Wild { reason, .. })
                        if crate::nextjs::urlpat::reason_is_origin(reason) =>
                    {
                        matches!(it.next(), Some(Piece::Param(_)))
                    }
                    Some(Piece::Lit(l))
                        if l.starts_with("http")
                            && l.split("://").nth(1).is_some_and(|h| !h.contains('/')) =>
                    {
                        matches!(it.next(), Some(Piece::Param(_)))
                    }
                    _ => false,
                }
            }) {
            let name = via.first().cloned().unwrap_or_else(|| inner.label.clone());
            (
                vec![vec![Piece::Wild {
                    multi: true,
                    reason: format!("wrapper-value: a URL passed to `{name}` used as a value"),
                    src: Source::Code,
                }]],
                Vec::new(),
                via,
            )
        } else {
            (tpls, method, via)
        };
        if let Some(t) = &self.trace
            && f.path.to_string_lossy().contains(t.as_str())
        {
            let refs: Vec<String> = params_in(&tpls)
                .iter()
                .map(|r| {
                    let g = self.files.by_id(r.file);
                    format!(
                        "{}@{}:{}",
                        r.name,
                        g.as_ref()
                            .map(|g| g.path.display().to_string())
                            .unwrap_or_default(),
                        g.as_ref()
                            .map(|g| g.line(g.funcs[r.func as usize].start))
                            .unwrap_or(0)
                    )
                })
                .collect();
            eprintln!(
                "site {}:{} {} params {:?} -> {:?}",
                f.path.display(),
                f.line(inner.at),
                inner.label,
                refs,
                demote(&tpls)
            );
        }
        if tpls.is_empty() {
            return;
        }
        let text = match inner.target {
            InnerTarget::Api(ai) => inner
                .args
                .get(self.apis[ai].arg as usize)
                .map(|&(a, b)| f.range_text(a, b))
                .unwrap_or_default(),
            InnerTarget::Wrapper(..) => inner.label.clone(),
        };
        let (urls, mut unresolved) = Self::patterns(&tpls, &text);
        if urls.is_empty()
            && unresolved.len() > 1
            && matches!(inner.target, InnerTarget::Wrapper(..))
        {
            // Every alternative of the wrapper is unknown: the argument is.
            let arg = inner
                .args
                .first()
                .map(|&(a, b)| f.range_text(a, b))
                .unwrap_or_else(|| inner.label.clone());
            let src = unresolved
                .iter()
                .map(|r| Source::of_reason(r))
                .max()
                .unwrap_or_default();
            unresolved = vec![src.tag(&arg)];
        }
        // A wrapper: the URL depends on a parameter of an enclosing named function and
        // is not a usable pattern without it.
        if forward && inner.level < MAX_LEVEL && Self::weak(&urls, &unresolved) {
            let refs = params_in(&tpls);
            let owner = refs
                .iter()
                .filter(|r| r.file == f.id && Self::named(&f, r.func))
                .min_by_key(|r| {
                    let fd = &f.funcs[r.func as usize];
                    fd.body.1 - fd.start
                })
                .map(|r| r.func);
            // A recursive call adds nothing to its own function's summaries.
            if let Some(func) = owner
                && matches!(inner.target, InnerTarget::Wrapper(k, _) if k == (f.id, func))
            {
                return;
            }
            // The framework calls a route's entry functions (a page with its props, a
            // handler with the request, a server action with its arguments): no call binds
            // their parameters, so the site stays here, unknown from the request.
            let owner =
                owner.filter(|&func| !crate::nextjs::source::request_entry(&f, func as usize));
            if let Some(func) = owner {
                // Would binding the parameters help? Probe with a literal path.
                let probe: Vec<Tpl> = tpls
                    .iter()
                    .map(|t| {
                        t.iter()
                            .map(|p| match p {
                                Piece::Param(r) if r.file == f.id && r.func == func => {
                                    Piece::Lit("/probe".into())
                                }
                                other => other.clone(),
                            })
                            .collect()
                    })
                    .collect();
                let (pu, punres) = Self::patterns(&probe, &text);
                let open = |v: &[UrlPat]| v.iter().filter(|u| u.open_tail).count();
                let helps = !pu.is_empty()
                    && (punres.len() < unresolved.len()
                        || pu.len() > urls.len()
                        || open(&pu) < open(&urls));
                if helps {
                    let key = (f.id, func);
                    let id = match inner.target {
                        InnerTarget::Api(ai) => (f.id, inner.at, u32::MAX, ai as u32, 0),
                        InnerTarget::Wrapper(k, j) => (f.id, inner.at, k.0, k.1, j + 1),
                    };
                    if !self.seen.insert(id) {
                        return;
                    }
                    let list = self.summaries.entry(key).or_default();
                    list.push(inner.clone());
                    let j = list.len() - 1;
                    let mut ps: Vec<(u8, Vec<String>)> = refs
                        .iter()
                        .filter(|r| r.file == f.id && r.func == func)
                        .map(|r| (r.index, r.field.clone()))
                        .collect();
                    ps.sort();
                    ps.dedup();
                    if turborepo_trace_core::debug::var_os("INVOKE_DEBUG").is_some() {
                        eprintln!(
                            "wrapper {}:{} {} params {:?} via {:?}",
                            f.path.display(),
                            f.line(inner.at),
                            f.funcs[func as usize].name.as_deref().unwrap_or("?"),
                            ps,
                            via
                        );
                    }
                    self.summary_params.insert((key, j), ps);
                    let label = f.funcs[func as usize]
                        .name
                        .clone()
                        .unwrap_or_else(|| "wrapper".into());
                    // Calls of the new wrapper.
                    let mut new: Vec<Rc<Inner>> = Vec::new();
                    if let Some(cs) = self.calls.get(&key) {
                        for c in cs {
                            // A caller that passes its own fetcher is bound through the
                            // fetcher, not through the key.
                            let mut fwd = None;
                            if let Some(idx) = inner.fetcher_fwd {
                                match self.classify_fetcher(&c.file, &c.args, idx) {
                                    Fetcher::Inline | Fetcher::Func(_) => continue,
                                    Fetcher::Param(_, i2) => fwd = Some(i2),
                                    Fetcher::None => {}
                                }
                            }
                            let direct = match &c.jsx {
                                Some(u) => {
                                    let ps = self
                                        .summary_params
                                        .get(&(key, j))
                                        .cloned()
                                        .unwrap_or_default();
                                    self.jsx_direct(&c.file, u, &ps)
                                }
                                None => Vec::new(),
                            };
                            new.push(Rc::new(Inner {
                                file: c.file.clone(),
                                at: c.at,
                                args: c.args.clone(),
                                target: InnerTarget::Wrapper(key, j),
                                kind: inner.kind,
                                label: label.clone(),
                                direct,
                                level: inner.level + 1,
                                fetcher_fwd: fwd,
                            }));
                        }
                    }
                    // Fetcher positions: the key is the first argument.
                    if let Some(rs) = self.fetcher_refs.get(&key) {
                        let e0 = Eval::symbolic(self.files);
                        for r in rs {
                            let direct: Vec<(u8, Vec<String>, Vec<Tpl>)> = self
                                .fetcher_key(r.api, &r.file, &r.args, &e0, 0)
                                .into_iter()
                                .map(|(p, v)| (0u8, p, v))
                                .collect();
                            if direct.is_empty() {
                                continue;
                            }
                            new.push(Rc::new(Inner {
                                file: r.file.clone(),
                                at: r.at,
                                args: Vec::new(),
                                target: InnerTarget::Wrapper(key, j),
                                kind: InvokeKind::Swr,
                                label: format!("{} → {label}", r.label),
                                direct,
                                level: inner.level + 1,
                                fetcher_fwd: None,
                            }));
                        }
                    }
                    self.queue.extend(new);
                    return;
                }
            }
        }
        // The request's own URL (a handler reading `req.url`) adds no route.
        if urls.is_empty() && unresolved.iter().all(|r| is_self_url(r)) {
            return;
        }
        let mut targets: Vec<SiteTarget> = Vec::new();
        for u in urls {
            let t = SiteTarget::Url(u);
            if !targets.contains(&t) {
                targets.push(t);
            }
        }
        for r in unresolved {
            if is_self_url(&r) {
                continue;
            }
            let t = SiteTarget::Unknown(reason(&r));
            if !targets.contains(&t) {
                targets.push(t);
            }
        }
        if targets.is_empty() {
            return;
        }
        // The idiom: an SWR or TanStack Query call around the site wins over `fetch`.
        let mut kind = inner.kind;
        if kind == InvokeKind::Fetch
            && let Some(k) = self.ctx_kind.get()
        {
            kind = k;
        }
        let method = {
            let lits: Vec<String> = method
                .iter()
                .filter_map(|t| match t.as_slice() {
                    [Piece::Lit(m)] => Some(m.to_ascii_uppercase()),
                    _ => None,
                })
                .collect();
            if !method.is_empty()
                && lits.len() == method.len()
                && lits.iter().all(|m| *m == lits[0])
            {
                Some(lits[0].clone())
            } else if method.is_empty()
                && matches!(
                    kind,
                    InvokeKind::Fetch | InvokeKind::Swr | InvokeKind::Query
                )
                && matches!(via.last().map(String::as_str), Some("fetch" | "Request"))
            {
                Some("GET".to_string())
            } else {
                None
            }
        };
        let (file, line) = match &at {
            Some((g, t)) => (g.path.clone(), g.line(*t)),
            None => (f.path.clone(), f.line(inner.at)),
        };
        self.sites.push(Site {
            file,
            line,
            kind,
            api: inner.label.clone(),
            method,
            targets,
            via,
        });
    }

    /// Uses of wrappers that are not calls: the wrapper passed or stored as a
    /// value may be called with any URL, so its inner call is a site (in
    /// its conservative form) at the place of the use. A wrapper that is a
    /// method or an object property, with no resolved call at all, is a
    /// site at its definition.
    fn escapes(&mut self, paths: &[PathBuf]) {
        use turborepo_trace_lang_js::lexer::{Kw, TokKind};
        let keys: HashSet<FnKey> = self.summaries.keys().copied().collect();
        if keys.is_empty() {
            return;
        }
        let mut uses: Vec<WrapperUse> = Vec::new();
        for &k in &keys {
            let Some(f) = self.files.by_id(k.0) else {
                continue;
            };
            let fd = &f.funcs[k.1 as usize];
            if matches!(fd.kind, FnKind::Method | FnKind::Prop)
                && self.calls.get(&k).is_none_or(Vec::is_empty)
                && self.fetcher_refs.get(&k).is_none_or(Vec::is_empty)
            {
                uses.push((k, None));
            }
        }
        let mut fetcher_args: HashSet<(u32, usize)> = HashSet::new();
        for rs in self.fetcher_refs.values() {
            for r in rs {
                fetcher_args.insert((r.file.id, r.arg));
            }
        }
        // The names a wrapper can be imported under: an import of another name is not
        // one.
        let mut wrapper_names: HashSet<String> = HashSet::new();
        for &k in &keys {
            if let Some(f) = self.files.by_id(k.0)
                && let Some(n) = &f.funcs[k.1 as usize].name
            {
                wrapper_names.insert(n.clone());
                for (e, l) in &f.exports {
                    if l == n {
                        wrapper_names.insert(e.clone());
                    }
                }
            }
        }
        let wrapper_names = self.files.export_aliases(paths, wrapper_names);
        for p in paths {
            let Some(f) = self.files.get(p) else { continue };
            let mut names: Vec<(&str, FnKey)> = Vec::new();
            for (fi, fd) in f.funcs.iter().enumerate() {
                if matches!(fd.kind, FnKind::Decl | FnKind::Bound)
                    && keys.contains(&(f.id, fi as u32))
                    && let Some(n) = &fd.name
                {
                    names.push((n.as_str(), (f.id, fi as u32)));
                }
            }
            for imp in &f.imports {
                match &imp.imported {
                    Imported::Namespace => continue,
                    Imported::Named(n) if !wrapper_names.contains(n) => continue,
                    Imported::Default if !wrapper_names.contains("default") => continue,
                    _ => {}
                }
                if let Some((g, n)) = self.files.imported_decl(&f, &imp.local)
                    && let Some(fi) = g.module_fn(&n)
                    && keys.contains(&(g.id, fi as u32))
                {
                    names.push((imp.local.as_str(), (g.id, fi as u32)));
                }
            }
            if names.is_empty() {
                continue;
            }
            for t in 0..f.toks.len() {
                if f.kind(t) != TokKind::Ident {
                    continue;
                }
                let text = f.text(t);
                let Some(&(_, key)) = names.iter().find(|(n, _)| *n == text) else {
                    continue;
                };
                let prev = if t > 0 { f.kind(t - 1) } else { TokKind::Eof };
                let next = f.kind(t + 1);
                // Not a reference: a member name, a call, a declaration, a type query, a
                // property key, an import or export specifier.
                if matches!(prev, TokKind::Dot | TokKind::OptChain)
                    || matches!(next, TokKind::LParen | TokKind::Lt | TokKind::Colon)
                    || (t > 0
                        && matches!(
                            f.toks[t - 1].kw,
                            Kw::Function
                                | Kw::Const
                                | Kw::Let
                                | Kw::Var
                                | Kw::Typeof
                                | Kw::Import
                                | Kw::As
                                | Kw::Default
                        ))
                    || fetcher_args.contains(&(f.id, t))
                {
                    continue;
                }
                // `window.name = name`: a global installed for debugging is not followed.
                if prev == TokKind::Assign
                    && (t.saturating_sub(9)..t)
                        .rev()
                        .take_while(|&k| f.kind(k) != TokKind::Semi)
                        .any(|k| matches!(f.text(k), "window" | "globalThis" | "self" | "global"))
                {
                    continue;
                }
                let o = f.open_of[t];
                if o != u32::MAX
                    && f.kind(o as usize) == TokKind::LBrace
                    && o > 0
                    && (matches!(
                        f.toks[o as usize - 1].kw,
                        Kw::Import | Kw::Export | Kw::Type
                    ) || f.kind(o as usize - 1) == TokKind::Comma
                        && f.toks[o as usize].depth == 0
                        && f.block_of(o as usize) == usize::MAX
                        && (0..o as usize)
                            .rev()
                            .take(6)
                            .any(|k| f.toks[k].kw == Kw::Import))
                {
                    continue;
                }
                // The same declaration, not a shadowing one.
                let same = match f.lookup(text, t) {
                    Some(Decl::Func(fi)) => (f.id, fi as u32) == key,
                    Some(Decl::Import(_)) => key.0 != f.id || !keys.contains(&(f.id, key.1)),
                    _ => false,
                };
                if same {
                    uses.push((key, Some((f.clone(), t))));
                }
            }
        }
        // A wrapper's conservative form is the same at every use: evaluated once.
        let mut forms: HashMap<(FnKey, usize), Vec<Site>> = HashMap::new();
        for (key, at) in uses {
            let Some(list) = self.summaries.get(&key).cloned() else {
                continue;
            };
            for (j, inner) in list.into_iter().enumerate() {
                let form = match forms.entry((key, j)) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let n = self.sites.len();
                        self.process(inner, false, None);
                        e.insert(self.sites.split_off(n))
                    }
                };
                for site in form.iter() {
                    let mut site = site.clone();
                    if let Some((g, t)) = &at {
                        site.file = g.path.clone();
                        site.line = g.line(*t);
                    }
                    self.sites.push(site);
                }
            }
        }
    }
}

/// The result of the URL scan.
pub(crate) struct UrlScan {
    pub sites: Vec<Site>,
    pub strings: Vec<UrlString>,
    pub wrappers: usize,
}

/// The URL-like strings of `f`: string literals and templates that start with
/// `/` (template holes are evaluated; unknown holes are unknown segment parts).
/// A literal that ends in `/` is a prefix.
fn url_strings(ev: &Eval<'_, '_>, f: &Rc<JsFile>, out: &mut Vec<UrlString>) {
    use turborepo_trace_lang_js::lexer::TokKind;
    let mut seen: HashSet<UrlPat> = HashSet::new();
    let mut push = |out: &mut Vec<UrlString>, t: usize, mut tpl: Tpl| {
        match tpl.first() {
            Some(Piece::Lit(l)) if l.starts_with('/') && !l.starts_with("//") && l.len() > 1 => {}
            _ => return,
        }
        // A prefix: `'/api/teams/' + id`.
        if let Some(Piece::Lit(l)) = tpl.last()
            && l.ends_with('/')
            && !l.contains('?')
        {
            tpl.push(Piece::Wild {
                multi: true,
                reason: "path".into(),
                src: Source::Code,
            });
        }
        let (urls, _) = to_patterns(&[tpl], None);
        for u in urls {
            let known = u
                .segs
                .iter()
                .flatten()
                .any(|p| matches!(p, Part::Lit(t) if !t.is_empty()));
            if known && seen.insert(u.clone()) {
                out.push(UrlString {
                    file: f.path.clone(),
                    line: f.line(t),
                    pattern: u,
                });
            }
        }
    };
    let mut t = 0;
    let n = f.toks.len();
    while t < n {
        match f.kind(t) {
            TokKind::Str => {
                if let Some(v) = f.str_value(t)
                    && v.starts_with('/')
                {
                    push(out, t, vec![Piece::Lit(v)]);
                }
            }
            TokKind::Template => {
                for tpl in ev.eval(f, t, t + 1) {
                    push(out, t, tpl);
                }
            }
            TokKind::TemplateHead => {
                if let Some(c) = f.closer(t) {
                    let raw = f.toks[t].text(&f.src);
                    // Only templates that start with `/`, or with a name followed by `/`
                    // (a constant base: `${API_TEAMS}/${id}`).
                    let base_hole = raw.len() >= 2 && raw[1] == b'$' && {
                        let mut k = t + 1;
                        while f.kind(k) == TokKind::Ident
                            && f.kind(k + 1) == TokKind::Dot
                            && f.kind(k + 2) == TokKind::Ident
                        {
                            k += 2;
                        }
                        f.kind(k) == TokKind::Ident
                            && matches!(
                                f.kind(k + 1),
                                TokKind::TemplateMiddle | TokKind::TemplateTail
                            )
                            && f.toks[k + 1].text(&f.src).starts_with(b"}/")
                    };
                    if raw.len() >= 2 && (raw[1] == b'/' || base_hole) {
                        let v = ev.eval(f, t, c + 1);
                        // A hole that is a parameter of a named function is the caller's
                        // string: the caller's closure has it (or a site evaluates it).
                        let caller_supplied = params_in(&v).iter().any(|r| {
                            r.file == f.id
                                && f.funcs
                                    .get(r.func as usize)
                                    .is_some_and(|fd| fd.kind != FnKind::Anon && fd.name.is_some())
                        });
                        if !caller_supplied {
                            for tpl in demote(&v) {
                                push(out, t, tpl);
                            }
                        }
                    }
                    t = c;
                }
            }
            _ => {}
        }
        t += 1;
    }
}

/// Scans `files` for URL sites.
pub(crate) fn scan_urls(files: &Files<'_>, paths: &[PathBuf], apis: &[UrlApi]) -> UrlScan {
    let mut api_index = HashMap::new();
    for (i, a) in apis.iter().enumerate() {
        if a.style == Style::Jsx {
            continue;
        }
        api_index
            .entry((a.module.clone(), a.name.clone(), a.style == Style::New))
            .or_insert(i);
    }
    let mut e = Engine {
        files,
        apis,
        api_index,
        summaries: HashMap::new(),
        calls: HashMap::new(),
        fetcher_refs: HashMap::new(),
        contexts: HashMap::new(),
        fetcher_calls: HashMap::new(),
        derived: HashMap::new(),
        summary_params: HashMap::new(),
        trace: turborepo_trace_core::debug::var("INVOKE_TRACE").ok(),
        ctx_kind: std::cell::Cell::new(None),
        queue: Vec::new(),
        sites: Vec::new(),
        seen: HashSet::new(),
    };
    let jsx_ev = Eval::new(files);
    let str_ev = Eval::symbolic(files).with_budget(400);
    let mut jsx_sites = Vec::new();
    let mut strings = Vec::new();
    let timing = turborepo_trace_core::debug::var_os("INVOKE_TIMING").is_some();
    let t0 = std::time::Instant::now();
    let mut t_index = std::time::Duration::ZERO;
    let mut t_jsx = std::time::Duration::ZERO;
    let mut t_str = std::time::Duration::ZERO;
    for p in paths {
        let Some(f) = files.get(p) else { continue };
        let a = std::time::Instant::now();
        e.index_file(&f);
        let b = std::time::Instant::now();
        jsx_sites.extend(scan_jsx(&jsx_ev, &f, apis));
        let c = std::time::Instant::now();
        url_strings(&str_ev, &f, &mut strings);
        t_index += b - a;
        t_jsx += c - b;
        t_str += c.elapsed();
    }
    if timing {
        eprintln!(
            "invoke timing: files {} ms (index {} ms, jsx {} ms, strings {} ms)",
            t0.elapsed().as_millis(),
            t_index.as_millis(),
            t_jsx.as_millis(),
            t_str.as_millis()
        );
    }
    let t1 = std::time::Instant::now();
    e.expand_derived();
    // Sites and wrappers to a fixpoint: a new wrapper queues its calls.
    let mut steps = 0usize;
    while let Some(inner) = e.queue.pop() {
        steps += 1;
        if steps > 2_000_000 {
            break;
        }
        e.process(inner, true, None);
    }
    let t2 = std::time::Instant::now();
    e.escapes(paths);
    if timing {
        eprintln!(
            "invoke timing: derived {} ms, fixpoint {} ms ({} steps), escapes {} ms",
            0,
            (t2 - t1).as_millis(),
            steps,
            t2.elapsed().as_millis()
        );
    }
    let mut sites = e.sites;
    for s in jsx_sites {
        let kind = match s.kind {
            UrlUse::Fetch => InvokeKind::Fetch,
            UrlUse::Redirect => InvokeKind::Redirect,
            UrlUse::Navigation | UrlUse::Build => InvokeKind::Navigation,
        };
        let mut targets: Vec<SiteTarget> = s.urls.into_iter().map(SiteTarget::Url).collect();
        // `href="#top"`, `href="?tab=usage"`: the page the visitor is on, no other
        // route.
        targets.extend(
            s.unresolved
                .iter()
                .filter(|r| !is_self_url(r))
                .map(|r| SiteTarget::Unknown(reason(r))),
        );
        if targets.is_empty() {
            continue;
        }
        sites.push(Site {
            file: s.file,
            line: s.line,
            kind,
            api: s.api.clone(),
            method: None,
            targets,
            via: vec![s.api],
        });
    }
    UrlScan {
        sites,
        strings,
        wrappers: e.summaries.len(),
    }
}
