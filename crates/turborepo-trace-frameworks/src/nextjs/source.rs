//! Where an unknown value comes from.
//!
//! The URL evaluator ([`super::jseval`]) stops at a leaf it cannot evaluate: a
//! parameter no caller binds, a call into a package, a browser global. The
//! leaf's [`Source`] says what kind of value that is, found by resolving the
//! identifiers of the leaf expression through the file's scopes (the
//! declaration in the innermost block, parameters, imports, then globals) and
//! following what they are bound to:
//!
//! - a `const` / `let` / `var`: its initializer and every assignment;
//! - React state (`const [x, setX] = useState(init)`): the initial value and
//!   every argument given to the setter;
//! - a first-party import: the exported binding's initializer, or the results
//!   of the exported function when it is called;
//! - a parameter: the request for the entry functions of a route (a page's
//!   props, a handler's request, `getServerSideProps`' context, a server
//!   action's arguments), an input for event handlers, otherwise a parameter;
//! - an import from a package and a global: the tables below.
//!
//! The source is that of the whole value, not of everything the expression
//! mentions:
//!
//! - a member or method of a value has the value's source
//!   (`params.get('next')`, `location.href`, `(await res.json()).next`);
//! - `a ?? b`, `c ? a : b` and `a + b` have the highest source of their
//!   alternatives or operands, in the order of the enum (the further outside
//!   the code a value comes from, the less the code says about it);
//! - a call has the source of what the function returns, and of its arguments
//!   where the function returns a parameter;
//! - a value the code assembles (a template or a concatenation with text, an
//!   object, an array, a function) is the code's, whatever goes into it:
//!   `/api/teams/${teamId}/members` is not a URL the request chooses. Read
//!   through a member, an object or an array is a container: the member can be
//!   any of its contents.
//!
//! What the relation does with it: an unknown URL computed by code is bounded
//! by the URL-like strings of the route's module closure (the code has to take
//! the URL from somewhere); one read from the request, from storage or from an
//! input ([`Source::outside`]) can be any URL, whatever the code contains.
//!
//! Limits: the tables of request, storage and response sources are finite (a
//! package that wraps storage and is not listed is a call); values that travel
//! through React context, a store or a ref are the code's.

use turborepo_trace_lang_js::lexer::TokKind;

use super::jseval::{Decl, Eval, Files, FnKind, Imported, JsFile};

/// Where an unknown part of a value comes from. Ordered: a later source wins
/// when several identifiers contribute.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    /// Computed by code the evaluator did not follow.
    #[default]
    Code,
    /// The result of a call that is not evaluated (a package function).
    Call,
    /// A binding imported from a package.
    Imported,
    /// A parameter that no known caller binds.
    Param,
    /// An environment variable.
    Env,
    /// A response body (`fetch`, an HTTP client, SWR / TanStack Query data).
    Response,
    /// What the user enters or a message delivers: form data, an input event,
    /// `prompt`, `postMessage`, the clipboard.
    Input,
    /// Browser storage and cookies.
    Storage,
    /// The request: the location, search and route parameters, headers, the
    /// request body, the arguments of a server action.
    Request,
}

impl Source {
    /// Every source, in order.
    pub const ALL: [Source; 9] = [
        Source::Code,
        Source::Call,
        Source::Imported,
        Source::Param,
        Source::Env,
        Source::Response,
        Source::Input,
        Source::Storage,
        Source::Request,
    ];

    /// The name used in reasons and statistics.
    pub fn name(self) -> &'static str {
        match self {
            Source::Code => "code",
            Source::Call => "call",
            Source::Imported => "imported",
            Source::Param => "param",
            Source::Env => "env",
            Source::Response => "response",
            Source::Input => "input",
            Source::Storage => "storage",
            Source::Request => "request",
        }
    }

    /// Whether the value is supplied from outside the code at runtime (the
    /// request, storage, an input): no string of the code has to equal it.
    pub fn outside(self) -> bool {
        self >= Source::Input
    }

    /// The origin class shared with uncertainty records.
    pub fn origin(self) -> turborepo_trace_core::Origin {
        use turborepo_trace_core::Origin;
        match self {
            Source::Code => Origin::Unknown,
            Source::Call => Origin::Call,
            Source::Imported => Origin::Imported,
            Source::Param => Origin::Param,
            Source::Env => Origin::Env,
            Source::Response | Source::Input | Source::Storage | Source::Request => Origin::Data,
        }
    }

    /// `reason` with the source appended as ` [name]` (nothing for
    /// [`Source::Code`]).
    pub fn tag(self, reason: &str) -> String {
        let (text, old) = Source::split(reason);
        match self.max(old) {
            Source::Code => text.to_string(),
            s => format!("{text} [{}]", s.name()),
        }
    }

    /// A tagged reason as `(text, source)`.
    pub fn split(reason: &str) -> (&str, Source) {
        if let Some(rest) = reason.strip_suffix(']')
            && let Some((text, name)) = rest.rsplit_once(" [")
            && let Some(s) = Source::ALL.iter().find(|s| s.name() == name)
        {
            return (text, *s);
        }
        (reason, Source::Code)
    }

    /// The source a tagged reason names.
    pub fn of_reason(reason: &str) -> Source {
        Source::split(reason).1
    }
}

/// Binding initializers and function results are followed this deep.
const MAX_DEPTH: u32 = 6;
/// Tokens looked at per expression.
const MAX_STEPS: u32 = 20_000;

/// File names (without extension) whose exported functions Next.js calls with
/// the request.
const SEGMENT_FILES: &[&str] = &[
    "page",
    "layout",
    "template",
    "default",
    "route",
    "error",
    "global-error",
    "loading",
    "not-found",
    "forbidden",
    "unauthorized",
    "middleware",
    "proxy",
];

/// Whether Next.js calls the exported functions of `f` with request data: an
/// App Router segment file, the middleware, or a file below a `pages`
/// directory.
fn is_route_file(f: &JsFile) -> bool {
    let stem = f.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    SEGMENT_FILES.contains(&stem)
        || f.path.parent().is_some_and(|p| {
            // A `pages` directory of the repository, not one the checkout lives below.
            crate::fsx::local(p)
                .components()
                .any(|c| c.as_os_str() == "pages")
        })
}

/// Whether `f.funcs[func]` is exported by name, or is a function expression of
/// a module-level statement (`export const GET = withAuth(async (req) => ...)`,
/// `export default auth((req) => ...)`).
fn module_level_export(f: &JsFile, func: usize) -> bool {
    let fd = &f.funcs[func];
    if f.block_of(fd.start) != usize::MAX {
        return false;
    }
    match (&fd.name, fd.kind) {
        (Some(n), FnKind::Decl | FnKind::Bound) => f.exports.values().any(|l| l == n),
        (_, FnKind::Anon) => true,
        _ => false,
    }
}

/// Whether the parameters of `f.funcs[func]` are supplied by the framework from
/// a request: no code calls the function.
pub fn request_entry(f: &JsFile, func: usize) -> bool {
    let fd = &f.funcs[func];
    // A server action: an export of a `'use server'` module, or a function whose
    // body starts with the directive.
    if f.directive.as_deref() == Some("use server") && module_level_export(f, func) {
        return true;
    }
    if fd.block
        && (fd.body.0..fd.body.0 + 2)
            .any(|k| f.kind(k) == TokKind::Str && f.str_value(k).as_deref() == Some("use server"))
    {
        return true;
    }
    is_route_file(f) && module_level_export(f, func)
}

/// Whether the function starting at token `start` is a JSX event handler
/// (`onChange={(e) => ...}`) or a listener (`addEventListener('message', (e) =>
/// ...)`).
fn event_handler(f: &JsFile, start: usize) -> bool {
    if let Some(tok) = f.toks.get(start) {
        let before = &f.src[..tok.start as usize];
        let trimmed = before
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
            .map_or(&before[..0], |i| &before[..=i]);
        if let Some(attr) = trimmed.strip_suffix(b"={") {
            let n = attr
                .iter()
                .rev()
                .take_while(|b| b.is_ascii_alphanumeric())
                .count();
            let name = &attr[attr.len() - n..];
            if name.len() > 2 && name.starts_with(b"on") && name[2].is_ascii_uppercase() {
                return true;
            }
        }
    }
    f.enclosing_call(start)
        .is_some_and(|c| matches!(f.text(c), "addEventListener" | "on" | "onmessage"))
}

/// The source of a global by its name and the member chain after it.
fn global(name: &str, chain: &[&str]) -> Source {
    match name {
        "window" | "globalThis" | "self" | "top" | "parent" | "frames" => match chain.split_first()
        {
            Some((n, rest)) => global(n, rest),
            None => Source::Code,
        },
        "location" | "history" => Source::Request,
        "document" => match chain.first() {
            Some(&("location" | "URL" | "documentURI" | "referrer" | "baseURI")) => Source::Request,
            Some(&"cookie") => Source::Storage,
            Some(_) => Source::Input,
            None => Source::Code,
        },
        "localStorage" | "sessionStorage" | "indexedDB" | "caches" | "cookieStore" => {
            Source::Storage
        }
        "prompt" | "FormData" | "event" => Source::Input,
        "navigator" if chain.first() == Some(&"clipboard") => Source::Input,
        "process" if matches!(chain.first(), Some(&("env" | "argv"))) => Source::Env,
        "fetch" | "XMLHttpRequest" | "WebSocket" | "EventSource" => Source::Response,
        _ => Source::Code,
    }
}

/// The source of an import from a package, if the package or the name says.
fn package(module: &str, name: &str) -> Option<Source> {
    let by_module = match module {
        "next/navigation" => matches!(
            name,
            "useSearchParams"
                | "useParams"
                | "usePathname"
                | "useSelectedLayoutSegment"
                | "useSelectedLayoutSegments"
        )
        .then_some(Source::Request),
        "next/router" | "next/compat/router" | "next/headers" | "nuqs" | "nuqs/server"
        | "next-usequerystate" | "use-query-params" => Some(Source::Request),
        "js-cookie" | "cookies-next" | "universal-cookie" | "react-cookie" | "nookies"
        | "cookie" | "idb-keyval" | "localforage" => Some(Source::Storage),
        "swr"
        | "swr/immutable"
        | "swr/infinite"
        | "swr/mutation"
        | "@tanstack/react-query"
        | "react-query"
        | "axios"
        | "ky"
        | "ofetch"
        | "got"
        | "node-fetch"
        | "undici"
        | "cross-fetch" => Some(Source::Response),
        _ => None,
    };
    by_module.or(match name {
        "useSearchParams" | "useParams" | "useLocation" | "useQueryState" | "useQueryStates"
        | "useQueryParam" | "useQueryParams" => Some(Source::Request),
        "useLocalStorage" | "useSessionStorage" | "useCookie" | "useCookies" => {
            Some(Source::Storage)
        }
        _ => None,
    })
}

struct Walk<'a, 'r> {
    files: &'a Files<'r>,
    /// For the expression helpers.
    ev: Eval<'a, 'r>,
    /// Bindings and functions already followed: `(file id, token)`.
    seen: Vec<(u32, usize)>,
    steps: u32,
    /// A parameter no caller binds was read since this was last cleared.
    param_read: bool,
}

/// Functions whose result is their (first) argument in another form.
const PASS_THROUGH: &[&str] = &[
    "String",
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "atob",
    "structuredClone",
    "use",
];

impl<'a, 'r> Walk<'a, 'r> {
    fn new(files: &'a Files<'r>) -> Self {
        Walk {
            files,
            ev: Eval::new(files),
            seen: Vec::new(),
            steps: 0,
            param_read: false,
        }
    }

    /// The source of the call of `g.funcs[fi]` with the arguments `args` of
    /// `f`: of what the function returns; where that includes a parameter
    /// of the function, of the arguments.
    fn call(
        &mut self,
        f: &JsFile,
        args: &[(usize, usize)],
        g: &JsFile,
        fi: usize,
        field: bool,
        depth: u32,
    ) -> Source {
        let outer = std::mem::replace(&mut self.param_read, false);
        let r = self.results(g, fi, field, depth + 1);
        let through = std::mem::replace(&mut self.param_read, outer);
        if !through {
            return r;
        }
        let of_args = args
            .iter()
            .map(|&(p, q)| self.value(f, p, q, field, depth))
            .max()
            .unwrap_or_default();
        if r == Source::Param {
            of_args
        } else {
            r.max(of_args)
        }
    }

    /// The source of the value of `[a, b)`: of what the whole value is taken
    /// from. A value the code assembles (a template with text, a
    /// concatenation with a literal, an object, an array, a function) is
    /// the code's, whatever its parts are. With `field`, a member
    /// of the value is read: an object or an array is then a container, and the
    /// member can be any of its contents.
    fn value(&mut self, f: &JsFile, a: usize, b: usize, field: bool, depth: u32) -> Source {
        self.steps += 1;
        if self.steps > MAX_STEPS || a >= b || b > f.toks.len() {
            return Source::Code;
        }
        let (a, b) = self.ev.strip(f, a, b);
        if a >= b {
            return Source::Code;
        }
        let (q, plus, orr, and) = self.ev.top_ops(f, a, b);
        if let Some(qi) = q
            && let Some(k) = self.ev.ternary_colon(f, qi, b)
        {
            return self.value(f, qi + 1, k, field, depth).max(self.value(
                f,
                k + 1,
                b,
                field,
                depth,
            ));
        }
        if !orr.is_empty() {
            let mut out = Source::Code;
            let mut s = a;
            for &o in orr.iter().chain(std::iter::once(&b)) {
                out = out.max(self.value(f, s, o, field, depth));
                s = o + 1;
            }
            return out;
        }
        if let Some(&last) = and.last() {
            return self.value(f, last + 1, b, field, depth);
        }
        if !plus.is_empty() {
            let mut parts = Vec::new();
            let mut s = a;
            for &o in plus.iter().chain(std::iter::once(&b)) {
                parts.push((s, o));
                s = o + 1;
            }
            // Known text in a concatenation: the code gives the value its shape.
            if parts.iter().any(|&(x, y)| {
                y == x + 1
                    && matches!(f.kind(x), TokKind::Str | TokKind::Template)
                    && f.toks[x].end - f.toks[x].start > 2
            }) {
                return Source::Code;
            }
            return parts
                .into_iter()
                .map(|(x, y)| self.value(f, x, y, false, depth))
                .max()
                .unwrap_or_default();
        }
        match f.kind(a) {
            // `${x}` alone is `x`; text around substitutions is the code's.
            TokKind::TemplateHead if f.closer(a) == Some(b - 1) => {
                let mut subs = Vec::new();
                let mut text = !f.template_text(a).is_empty();
                let mut s = a + 1;
                let mut k = a + 1;
                while k < b {
                    match f.kind(k) {
                        TokKind::LParen
                        | TokKind::LBracket
                        | TokKind::LBrace
                        | TokKind::TemplateHead => {
                            if let Some(c) = f.closer(k).filter(|&c| c < b - 1) {
                                k = c;
                            }
                        }
                        TokKind::TemplateMiddle | TokKind::TemplateTail => {
                            subs.push((s, k));
                            text |= !f.template_text(k).is_empty();
                            s = k + 1;
                        }
                        _ => {}
                    }
                    k += 1;
                }
                if text {
                    return Source::Code;
                }
                subs.into_iter()
                    .map(|(x, y)| self.value(f, x, y, false, depth))
                    .max()
                    .unwrap_or_default()
            }
            // A container a member is read from: any of its contents.
            TokKind::LBrace | TokKind::LBracket if field && f.closer(a) == Some(b - 1) => {
                let object = f.kind(a) == TokKind::LBrace;
                let mut out = Source::Code;
                for (x, y) in f.split_commas(a + 1, b - 1) {
                    if x >= y {
                        continue;
                    }
                    let x = if f.kind(x) == TokKind::Ellipsis {
                        x + 1
                    } else {
                        x
                    };
                    // `key: value`; a shorthand property is its own value.
                    let colon = (x..y).find(|&k| {
                        f.kind(k) == TokKind::Colon && f.toks[k].depth == f.toks[x].depth
                    });
                    let from = match colon {
                        Some(c) if object && !matches!(f.kind(x), TokKind::LBracket) => c + 1,
                        _ => x,
                    };
                    out = out.max(self.value(f, from, y, true, depth));
                }
                out
            }
            TokKind::Ident => self.chain(f, a, b, field, depth),
            // `(a ?? b).x`, `(await res.json()).next`: a member of the parenthesized value.
            TokKind::LParen => match f.closer(a) {
                Some(c) if c + 1 < b && f.kind(c + 1) != TokKind::Arrow => {
                    self.value(f, a + 1, c, true, depth)
                }
                _ => Source::Code,
            },
            _ => Source::Code,
        }
    }

    /// A member / call chain starting at the identifier `a`.
    fn chain(&mut self, f: &JsFile, a: usize, b: usize, field: bool, depth: u32) -> Source {
        let is_new = f.text(a) == "new";
        let i = if is_new { a + 1 } else { a };
        if i >= b || f.kind(i) != TokKind::Ident {
            return Source::Code;
        }
        let name = f.text(i);
        if matches!(name, "function" | "async" | "class" | "typeof" | "delete")
            || f.kind(i + 1) == TokKind::Arrow
        {
            return Source::Code;
        }
        let mut members: Vec<&str> = Vec::new();
        // The arguments of a call of the root itself.
        let mut call: Option<(usize, usize)> = None;
        // The last `(` of the chain.
        let mut last = 0;
        let mut k = i + 1;
        while k < b {
            match f.kind(k) {
                TokKind::Dot | TokKind::OptChain
                    if matches!(f.kind(k + 1), TokKind::Ident | TokKind::PrivateName) =>
                {
                    members.push(f.text(k + 1));
                    k += 2;
                }
                TokKind::OptChain | TokKind::Bang => k += 1,
                TokKind::Lt => match f.type_args_end(k) {
                    Some(e) if e > k => k = e + 1,
                    _ => return Source::Code,
                },
                TokKind::LParen => {
                    let Some(c) = f.closer(k) else {
                        return Source::Code;
                    };
                    if members.is_empty() && call.is_none() {
                        call = Some((k + 1, c));
                    }
                    last = k;
                    k = c + 1;
                }
                TokKind::LBracket => {
                    let Some(c) = f.closer(k) else {
                        return Source::Code;
                    };
                    members.push("[]");
                    k = c + 1;
                }
                // Something this walk does not read (an operator, a tagged template).
                _ => return Source::Code,
            }
        }
        let Some((x, y)) = call else {
            // A value and what is read from it: `params.get('next')`, `location.href`,
            // `res.json()`.
            return self.name(f, name, i, &members, field || !members.is_empty(), depth);
        };
        // Something is read from the call's result (`build(x).url`).
        let field = field || last != x - 1 || f.closer(x - 1).is_some_and(|c| c + 1 < b);
        let args = f.split_commas(x, y);
        let of_args = |w: &mut Self| {
            args.iter()
                .map(|&(p, q)| w.value(f, p, q, field, depth))
                .max()
                .unwrap_or_default()
        };
        if is_new {
            // `new URL(x, base)`, `new URLSearchParams(x)`: what they are built from.
            return if matches!(name, "URL" | "URLSearchParams" | "Request") {
                of_args(self)
            } else {
                global(name, &members)
            };
        }
        if PASS_THROUGH.contains(&name) && f.lookup(name, i).is_none() {
            return of_args(self);
        }
        // `useMemo(() => value, deps)`: the function's results.
        if matches!(name, "useMemo")
            && let Some(&(p, q)) = args.first()
            && let Some(fi) = f.fn_at(p, q)
        {
            return self.results(f, fi, field, depth);
        }
        match f.lookup(name, i) {
            None => global(name, &members),
            Some(Decl::Func(fi)) => self.call(f, &args, f, fi, field, depth),
            Some(Decl::Import(ii)) => {
                let imp = &f.imports[ii];
                let imported = match &imp.imported {
                    Imported::Named(n) => n.as_str(),
                    Imported::Default => "default",
                    Imported::Namespace => "*",
                };
                if let Some(s) = package(&imp.module, imported) {
                    return s;
                }
                if let Some((g, n)) = self.files.imported_decl(f, &imp.local) {
                    if let Some(fi) = g.module_fn(&n) {
                        return self.call(f, &args, &g, fi, field, depth);
                    }
                    return Source::Code;
                }
                if self.files.resolve(&f.path, &imp.module).is_some() {
                    Source::Code
                } else {
                    Source::Call
                }
            }
            // A function held in a binding or a parameter: what it returns is its.
            Some(Decl::Binding(_) | Decl::Param { .. }) => {
                self.name(f, name, i, &members, true, depth)
            }
            Some(Decl::Class(_)) => Source::Code,
        }
    }

    /// The source of `name` as resolved at token `at`, read through the member
    /// `chain`.
    fn name(
        &mut self,
        f: &JsFile,
        name: &str,
        at: usize,
        chain: &[&str],
        field: bool,
        depth: u32,
    ) -> Source {
        match f.lookup(name, at) {
            None => global(name, chain),
            Some(Decl::Param { func, param }) => {
                let s = param_source(f, func, param, chain);
                self.param_read |= s == Source::Param;
                s
            }
            Some(Decl::Binding(bi)) => self.binding(f, name, bi, at, field, depth),
            Some(Decl::Func(_) | Decl::Class(_)) => Source::Code,
            Some(Decl::Import(ii)) => {
                let imp = &f.imports[ii];
                let imported = match &imp.imported {
                    Imported::Named(n) => n.as_str(),
                    Imported::Default => "default",
                    Imported::Namespace => chain.first().copied().unwrap_or("*"),
                };
                if let Some(s) = package(&imp.module, imported) {
                    return s;
                }
                if imp.imported != Imported::Namespace
                    && let Some((g, n)) = self.files.imported_decl(f, &imp.local)
                {
                    if depth < MAX_DEPTH
                        && let Some(bnd) = g.module_binding(&n)
                        && let Some((x, y)) = bnd.init
                        && g.fn_at(x, y).is_none()
                    {
                        let key = (g.id, bnd.at);
                        if self.seen.contains(&key) {
                            return Source::Code;
                        }
                        self.seen.push(key);
                        return self.value(&g, x, y, field || !bnd.path.is_empty(), depth + 1);
                    }
                    return Source::Code;
                }
                if self.files.resolve(&f.path, &imp.module).is_some() {
                    Source::Code
                } else {
                    Source::Imported
                }
            }
        }
    }

    fn binding(
        &mut self,
        f: &JsFile,
        name: &str,
        bi: usize,
        at: usize,
        field: bool,
        depth: u32,
    ) -> Source {
        let bnd = &f.bindings[name][bi];
        // A destructured name is a member of the initializer's value.
        let of_init = field || !bnd.path.is_empty();
        let key = (f.id, bnd.at);
        if depth >= MAX_DEPTH || self.seen.contains(&key) {
            return Source::Code;
        }
        self.seen.push(key);
        let mut out = Source::Code;
        if let Some((x, y)) = bnd.init
            && !(x <= at && at < y)
        {
            out = out.max(self.value(f, x, y, of_init, depth + 1));
            // React state: the values given to the setter.
            if is_state(f, name, bi) {
                let setter = f.bindings.iter().find_map(|(n, bs)| {
                    bs.iter()
                        .any(|b| b.init == Some((x, y)) && b.path.len() == 1 && b.path[0] == "1")
                        .then_some(n.as_str())
                });
                if let Some(setter) = setter {
                    for k in 0..f.toks.len().saturating_sub(1) {
                        if f.kind(k) == TokKind::Ident
                            && f.kind(k + 1) == TokKind::LParen
                            && f.text(k) == setter
                            && let Some(c) = f.closer(k + 1)
                            && !(k + 2 <= at && at < c)
                        {
                            out = out.max(self.value(f, k + 2, c, field, depth + 1));
                        }
                    }
                }
            }
        }
        if bnd.mutable
            && let Some(asg) = f.assigns.get(name)
        {
            let (s, e) = if bnd.scope == usize::MAX {
                (0, f.toks.len())
            } else {
                (bnd.scope, f.closer(bnd.scope).unwrap_or(f.toks.len()))
            };
            for x in asg.iter().filter(|x| s <= x.at && x.at <= e) {
                if !(x.value.0 <= at && at < x.value.1) {
                    out = out.max(self.value(f, x.value.0, x.value.1, field, depth + 1));
                }
            }
        }
        out
    }

    /// The source of what `f.funcs[fi]` returns.
    fn results(&mut self, f: &JsFile, fi: usize, field: bool, depth: u32) -> Source {
        let fd = &f.funcs[fi];
        let key = (f.id, fd.start);
        if depth >= MAX_DEPTH || self.seen.contains(&key) {
            return Source::Code;
        }
        self.seen.push(key);
        if !fd.block {
            return self.value(f, fd.body.0, fd.body.1, field, depth + 1);
        }
        let mut out = Source::Code;
        for (_, (x, y)) in f.guarded_returns(fd.body.0, fd.body.1) {
            out = out.max(self.value(f, x, y, field, depth + 1));
        }
        out
    }
}

/// The source of the parameter `f.funcs[func].params[param]`, read through
/// `chain`.
pub fn param_source(f: &JsFile, func: usize, param: usize, chain: &[&str]) -> Source {
    let fd = &f.funcs[func];
    let Some(p) = fd.params.get(param) else {
        return Source::Param;
    };
    if request_entry(f, func) {
        return Source::Request;
    }
    // An event: what the user entered, dropped or pasted, or what a message
    // delivered.
    let first = p.field.as_deref().or(chain.first().copied());
    if matches!(
        first,
        Some("target" | "currentTarget" | "clipboardData" | "dataTransfer")
    ) {
        return Source::Input;
    }
    if fd.kind == FnKind::Anon && event_handler(f, fd.start) {
        return Source::Input;
    }
    Source::Param
}

/// The source of the expression `[a, b)` of `f`.
pub fn of_expr(files: &Files<'_>, f: &JsFile, a: usize, b: usize) -> Source {
    Walk::new(files).value(f, a, b, false, 0)
}

/// The source of a member of the value of `[a, b)` of `f`.
pub fn of_member(files: &Files<'_>, f: &JsFile, a: usize, b: usize) -> Source {
    Walk::new(files).value(f, a, b, true, 0)
}

/// Whether the binding is the value of a React state hook (`const [x, setX] =
/// useState(..)`).
pub fn is_state(f: &JsFile, name: &str, bi: usize) -> bool {
    let bnd = &f.bindings[name][bi];
    let Some((x, _)) = bnd.init else {
        return false;
    };
    let hook = if f.text(x) == "React" { x + 2 } else { x };
    bnd.path.len() == 1 && bnd.path[0] == "0" && matches!(f.text(hook), "useState" | "useReducer")
}

/// The source of the binding `f.bindings[name][bi]`: of its initializer, its
/// assignments and, for React state, of what its setter is given.
pub fn of_binding(files: &Files<'_>, f: &JsFile, name: &str, bi: usize) -> Source {
    Walk::new(files).binding(f, name, bi, usize::MAX, false, 0)
}

/// The source of `name` (resolved at token `at`) read through the field `path`.
pub fn of_name(files: &Files<'_>, f: &JsFile, name: &str, at: usize, path: &[String]) -> Source {
    let chain: Vec<&str> = path.iter().map(String::as_str).collect();
    Walk::new(files).name(f, name, at, &chain, !chain.is_empty(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::SimpleResolver;

    fn src(path: &str, code: &str, needle: &str) -> Source {
        let resolver = SimpleResolver::default();
        let files = Files::new(&resolver);
        let f = files.insert(JsFile::parse(
            std::path::Path::new(path),
            code.as_bytes().to_vec(),
        ));
        // The argument of the call of `needle`.
        let k = (0..f.toks.len())
            .find(|&k| f.text(k) == needle && f.kind(k + 1) == TokKind::LParen)
            .expect("the call");
        let c = f.closer(k + 1).expect("its closer");
        of_expr(&files, &f, k + 2, c)
    }

    #[test]
    fn tags() {
        assert_eq!(Source::Request.tag("x"), "x [request]");
        assert_eq!(Source::Code.tag("x"), "x");
        assert_eq!(Source::Code.tag("x [storage]"), "x [storage]");
        assert_eq!(Source::Request.tag("x [param]"), "x [request]");
        assert_eq!(Source::split("a [b] [env]"), ("a [b]", Source::Env));
        assert_eq!(Source::of_reason("items[0]"), Source::Code);
        assert!(Source::Storage.outside() && Source::Input.outside());
        assert!(!Source::Response.outside() && !Source::Env.outside());
    }

    #[test]
    fn globals_and_bindings() {
        let s = |code: &str| src("/app/components/x.tsx", code, "sink");
        assert_eq!(s("sink(window.location.search)"), Source::Request);
        assert_eq!(
            s("const u = new URLSearchParams(location.search).get('u'); sink(u)"),
            Source::Request
        );
        assert_eq!(
            s("const v = localStorage.getItem('k'); sink(v ?? '')"),
            Source::Storage
        );
        assert_eq!(s("sink(document.cookie)"), Source::Storage);
        assert_eq!(s("sink(process.env.API_URL)"), Source::Env);
        assert_eq!(
            s("const r = await fetch('/a'); const j = await r.json(); sink(j.next)"),
            Source::Response
        );
        assert_eq!(s("function f(url) { sink(url) }"), Source::Param);
        assert_eq!(
            s("let u = '/a'; u = prompt('where'); sink(u)"),
            Source::Input
        );
        // A local of the same name as a global is not the global.
        assert_eq!(
            s("function f() { const location = make(); sink(location.href) }"),
            Source::Code
        );
        // A member named like a source is not one.
        assert_eq!(
            s("function f(client) { sink(client.query) }"),
            Source::Param
        );
    }

    #[test]
    fn whole_values() {
        let s = |code: &str| src("/app/components/x.tsx", code, "sink");
        let nav = "import { useParams, useSearchParams } from 'next/navigation';\n";
        // A request value that fills a segment of a URL the code writes is not the URL.
        assert_eq!(
            s(&format!(
                "{nav}function C() {{ const {{ id }} = useParams(); \
                 sink(`/api/teams/${{id}}/members`) }}"
            )),
            Source::Code
        );
        assert_eq!(
            s(&format!(
                "{nav}function url(id) {{ return `/api/teams/${{id}}`; }}\nfunction C() {{ const \
                 {{ id }} = useParams(); sink(url(id)) }}"
            )),
            Source::Code
        );
        assert_eq!(
            s(&format!(
                "{nav}function C() {{ const {{ id }} = useParams(); sink(['team', id]) }}"
            )),
            Source::Code
        );
        assert_eq!(
            s(&format!(
                "{nav}function C() {{ const {{ id }} = useParams(); sink('/api/' + id) }}"
            )),
            Source::Code
        );
        // The whole value is the request's.
        assert_eq!(
            s(&format!(
                "{nav}function C() {{ const q = useSearchParams(); sink(`${{q.get('u')}}`) }}"
            )),
            Source::Request
        );
        assert_eq!(
            s(&format!(
                "{nav}function C() {{ const q = useSearchParams(); \
                 sink(decodeURIComponent(q.get('u') ?? '')) }}"
            )),
            Source::Request
        );
        assert_eq!(
            s(&format!(
                "{nav}function C({{ fallback }}) {{ const q = useSearchParams(); sink(fallback ? \
                 '/a' : q.get('u')!) }}"
            )),
            Source::Request
        );
        assert_eq!(
            s(
                "function same(x) { return x; }\nfunction C() { \
                 sink(same(window.location.search)) }"
            ),
            Source::Request
        );
        assert_eq!(
            s("function C() { sink(new URL(localStorage.getItem('u')!).pathname) }"),
            Source::Storage
        );
        // A member of an object the code assembles is one of the object's contents.
        assert_eq!(
            s(
                "function parse(q) { return { client: { logo: q.logo }, ok: true }; }\nfunction \
                 C() { const r = parse(window.location); sink(r.client.logo) }"
            ),
            Source::Request
        );
        assert_eq!(
            s("function C() { const { u } = { u: localStorage.getItem('u'), v: 1 }; sink(u) }"),
            Source::Storage
        );
        // What a function returns can include its argument next to other things.
        assert_eq!(
            s(
                "async function load(q) { const r = await fetch('/meta'); const meta = await \
                 r.json(); return { meta, logo: q.logo }; }\nasync function C() { const r = await \
                 load(window.location); sink(r.logo) }"
            ),
            Source::Request
        );
        assert_eq!(
            s("async function C() { const r = await fetch('/a'); sink((await r.json()).next) }"),
            Source::Response
        );
        assert_eq!(
            s(
                "function C({ other }) { sink((new URLSearchParams(location.search) ?? \
                 other).get('u')) }"
            ),
            Source::Request
        );
        // A function is the code's, whatever its parameters are called.
        assert_eq!(
            s("function C() { sink((location, search) => '/a' + search) }"),
            Source::Code
        );
        // A condition does not supply the value.
        assert_eq!(
            s("function C() { sink(location.hash ? '/a' : '/b') }"),
            Source::Code
        );
    }

    #[test]
    fn imports_and_state() {
        let s = |code: &str| src("/app/components/x.tsx", code, "sink");
        assert_eq!(
            s(
                "import { useSearchParams } from 'next/navigation';\nfunction C() { const p = \
                 useSearchParams(); const next = p.get('next'); sink(next) }"
            ),
            Source::Request
        );
        assert_eq!(
            s(
                "import { useRouter } from 'next/router';\nfunction C() { const { query } = \
                 useRouter(); sink(query.u) }"
            ),
            Source::Request
        );
        assert_eq!(
            s("import Cookies from 'js-cookie';\nfunction C() { sink(Cookies.get('u')) }"),
            Source::Storage
        );
        assert_eq!(
            s(
                "import useSWR from 'swr';\nfunction C() { const { data } = useSWR('/a'); \
                 sink(data.next) }"
            ),
            Source::Response
        );
        assert_eq!(
            s("import { make } from 'some-package';\nfunction C() { sink(make()) }"),
            Source::Call
        );
        // React state: what the setter is given.
        assert_eq!(
            s(
                "import { useState } from 'react';\nfunction C() { const [url, setUrl] = \
                 useState(''); sink(url); return <input onChange={(e) => setUrl(e.target.value)} \
                 /> }"
            ),
            Source::Input
        );
        assert_eq!(
            s(
                "import { useState } from 'react';\nfunction C({ list }) { const [url, setUrl] = \
                 useState('/a'); sink(url); setUrl(list[0]) }"
            ),
            Source::Param
        );
    }

    #[test]
    fn entry_parameters() {
        let page = "export default async function Page({ searchParams }) { sink(searchParams.u) }";
        assert_eq!(src("/app/x/page.tsx", page, "sink"), Source::Request);
        assert_eq!(src("/app/components/list.tsx", page, "sink"), Source::Param);
        let handler = "export const GET = withAuth(async (req) => { \
                       sink(req.nextUrl.searchParams.get('u')) })";
        assert_eq!(src("/app/api/x/route.ts", handler, "sink"), Source::Request);
        let action = "'use server';\nexport async function save(data) { sink(data.get('u')) }";
        assert_eq!(src("/app/x/actions.ts", action, "sink"), Source::Request);
        let inline =
            "function C() { async function save(data) { 'use server'; sink(data.get('u')) } }";
        assert_eq!(
            src("/app/components/c.tsx", inline, "sink"),
            Source::Request
        );
        let gssp = "export async function getServerSideProps(ctx) { sink(ctx.query.u) }";
        assert_eq!(src("/pages/x.tsx", gssp, "sink"), Source::Request);
        let listener = "window.addEventListener('message', (ev) => { sink(ev.data.url) })";
        assert_eq!(
            src("/app/components/c.tsx", listener, "sink"),
            Source::Input
        );
    }
}
