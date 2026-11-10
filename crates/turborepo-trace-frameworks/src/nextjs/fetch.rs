//! Static discovery of the URLs route code requests or navigates to at runtime.
//!
//! Every first-party file (server and client components, hooks, handlers; where
//! the call sits does not matter) is scanned for call sites in a data-driven
//! table of URL-producing APIs ([`UrlApi`], in the shape of
//! [`turborepo_trace_core::api::ApiEntry`]): `fetch`, `new Request`, SWR keys,
//! axios/ky, `navigator.sendBeacon`, `EventSource`, `new URL(x, base)`, server
//! `redirect()`, and client navigation (`router.push`, `<Link href>`, an `href`
//! given to any other component, `location.assign`). Project wrappers come from
//! `turbotrace.toml` where it is read ([`parse_url_apis_toml`]).
//!
//! Each argument is evaluated to URL patterns ([`super::jseval`]); the caller
//! matches them against the route table by overlap ([`super::select`]).

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use serde::{Deserialize, Serialize};

use super::{
    jseval::{Eval, Files, JsFile, Tpl, all_unknown, call_sites},
    source::Source,
    urlpat::{Origin, Part, Piece, UrlPat, path_source, tail_source},
};

/// What a URL is used for.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlUse {
    /// A request (`fetch`, SWR, axios): the target handler or page runs.
    Fetch,
    /// A server-side redirect (`redirect()` in a server component): the target
    /// renders.
    Redirect,
    /// A client navigation (`router.push`, `<Link>`): only a visit if a test
    /// performs it.
    Navigation,
    /// A URL is built (`new URL(x, base)`): informational, never an edge or an
    /// uncertainty. It is not a request by itself; the site that uses the
    /// URL (a `fetch`, a redirect) is evaluated on its own.
    Build,
}

/// How the API is invoked.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// `f(x)`.
    #[default]
    Call,
    /// `new F(x)`.
    New,
    /// A JSX attribute (`<Link href={x}>`); `field` is the attribute.
    Jsx,
}

/// One URL-producing API. Unknown keys are refused: a misspelt `prefix` in a
/// `turbotrace.toml` would otherwise give a wrapper without one, whose URLs
/// match no route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlApi {
    /// Module specifier; `None` for a global (`fetch`, `navigator.sendBeacon`).
    #[serde(default)]
    pub module: Option<String>,
    /// Dotted export path: `default` for a default import, `default.get` for
    /// `axios.get`, `useRouter().push` for a method of a hook result; for
    /// globals the dotted global (`navigator.sendBeacon`). For JSX: the
    /// component's export.
    pub name: String,
    /// Argument index.
    #[serde(default)]
    pub arg: u8,
    /// Read this field of an object argument (`url` of `axios.request({ url
    /// })`), or the JSX attribute.
    #[serde(default)]
    pub field: Option<String>,
    /// Use.
    #[serde(default = "default_use")]
    pub kind: UrlUse,
    /// Call style.
    #[serde(default)]
    pub style: Style,
    /// Prepended to paths that start with `/` but not with the prefix (wrappers
    /// that add `/api`).
    #[serde(default)]
    pub prefix: Option<String>,
    /// The index of a fetcher callback argument: its first parameter receives
    /// the URL (SWR: `useSWR(key, fetcher)`).
    #[serde(default)]
    pub fetcher: Option<u8>,
    /// The HTTP method the API implies (`axios.post` → `POST`).
    #[serde(default)]
    pub method: Option<String>,
}

fn default_use() -> UrlUse {
    UrlUse::Fetch
}

fn api(module: Option<&str>, name: &str, kind: UrlUse) -> UrlApi {
    UrlApi {
        module: module.map(String::from),
        name: name.to_string(),
        arg: 0,
        field: None,
        kind,
        style: Style::Call,
        prefix: None,
        fetcher: None,
        method: None,
    }
}

impl UrlApi {
    fn new_style(mut self) -> Self {
        self.style = Style::New;
        self
    }

    fn jsx(mut self, attr: &str) -> Self {
        self.style = Style::Jsx;
        self.field = Some(attr.to_string());
        self
    }

    fn field(mut self, f: &str) -> Self {
        self.field = Some(f.to_string());
        self
    }

    fn fetcher(mut self, arg: u8) -> Self {
        self.fetcher = Some(arg);
        self
    }

    fn method(mut self, m: &str) -> Self {
        self.method = Some(m.to_ascii_uppercase());
        self
    }
}

/// The built-in table.
pub fn default_url_apis() -> Vec<UrlApi> {
    use UrlUse::*;
    let mut v = vec![
        api(None, "fetch", Fetch),
        api(None, "Request", Fetch).new_style(),
        api(None, "EventSource", Fetch).new_style(),
        api(None, "WebSocket", Fetch).new_style(),
        api(None, "URL", Build).new_style(),
        api(None, "navigator.sendBeacon", Fetch),
        api(None, "location.assign", Navigation),
        api(None, "location.replace", Navigation),
        api(None, "a", Navigation).jsx("href"),
        api(None, "form", Fetch).jsx("action"),
        api(Some("swr"), "default", Fetch).fetcher(1),
        api(Some("swr"), "useSWR", Fetch).fetcher(1),
        api(Some("swr"), "preload", Fetch).fetcher(1),
        api(Some("swr"), "mutate", Fetch),
        api(Some("swr"), "useSWRConfig().mutate", Fetch),
        api(Some("swr/immutable"), "default", Fetch).fetcher(1),
        api(Some("swr/infinite"), "default", Fetch).fetcher(1),
        api(Some("swr/mutation"), "default", Fetch).fetcher(1),
        api(Some("swr/subscription"), "default", Fetch),
        api(Some("next/navigation"), "redirect", Redirect),
        api(Some("next/navigation"), "permanentRedirect", Redirect),
        api(Some("next/server"), "NextResponse.redirect", Redirect),
        api(Some("next/server"), "NextResponse.rewrite", Redirect),
        api(None, "Response.redirect", Redirect),
        api(Some("next/link"), "default", Navigation).jsx("href"),
        api(Some("next/form"), "default", Navigation).jsx("action"),
        api(None, "open", Navigation),
    ];
    for m in [
        "node-fetch",
        "cross-fetch",
        "isomorphic-fetch",
        "isomorphic-unfetch",
    ] {
        v.push(api(Some(m), "default", Fetch));
    }
    v.push(api(Some("undici"), "fetch", Fetch));
    v.push(api(Some("undici"), "request", Fetch));
    for name in ["ofetch", "$fetch"] {
        v.push(api(Some("ofetch"), name, Fetch));
    }
    for m in ["next/navigation", "next/router", "next/compat/router"] {
        for meth in ["push", "replace", "prefetch"] {
            v.push(api(Some(m), &format!("useRouter().{meth}"), Navigation));
        }
    }
    for meth in ["push", "replace", "prefetch"] {
        v.push(api(
            Some("next/router"),
            &format!("default.{meth}"),
            Navigation,
        ));
    }
    for m in ["axios", "ky", "redaxios"] {
        v.push(api(Some(m), "default", Fetch));
        for meth in ["get", "post", "put", "patch", "delete", "head", "options"] {
            v.push(api(Some(m), &format!("default.{meth}"), Fetch).method(meth));
            v.push(api(Some(m), &format!("default.create().{meth}"), Fetch).method(meth));
        }
        v.push(api(Some(m), "default.request", Fetch).field("url"));
    }
    v
}

/// The wrappers a large Next.js app uses: an example of a project table.
/// `fetchAPI` prefixes paths with `/api` unless they already start with it.
pub const FRONT_URL_APIS_TOML: &str = r#"
# turbotrace.toml (apps/web): project fetch wrappers for Next.js fetch discovery.
[[nextjs.url_apis]]
module = "@acme/api-client/fetch-api"
name = "fetchAPI"
prefix = "/api"

[[nextjs.url_apis]]
module = "@acme/api-client/fetch-api"
name = "default"
prefix = "/api"

[[nextjs.url_apis]]
module = "@acme/api-client/fetch-api/app"
name = "fetchAPI"
prefix = "/api"

[[nextjs.url_apis]]
module = "@acme/api-client/typed/fetch-api/client"
name = "clientTypedFetchData"
prefix = "/api"

[[nextjs.url_apis]]
module = "@acme/api-client/typed/fetch-api/client"
name = "clientTypedFetch"
prefix = "/api"

[[nextjs.url_apis]]
module = "@acme/api-client/typed/fetch-api/app"
name = "typedFetchData"
prefix = "/api"

[[nextjs.url_apis]]
module = "@acme/api-client/typed/fetch-api/app"
name = "typedFetch"
prefix = "/api"

[[nextjs.url_apis]]
module = "@acme/api-client/typed/swr"
name = "useTypedSWR"
prefix = "/api"
"#;

#[derive(Deserialize)]
struct TomlRoot {
    #[serde(default)]
    nextjs: Option<TomlNext>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlNext {
    #[serde(default)]
    url_apis: Vec<UrlApi>,
}

/// Reads `[[nextjs.url_apis]]` entries from a `turbotrace.toml` text. The other
/// tables of the file belong to other readers (the hints, the lint)
/// and are not looked at; an unknown key inside `[nextjs]` is an error.
pub fn parse_url_apis_toml(text: &str) -> Result<Vec<UrlApi>, String> {
    let r: TomlRoot = toml::from_str(text).map_err(|e| e.to_string())?;
    Ok(r.nextjs.map(|n| n.url_apis).unwrap_or_default())
}

/// One discovered site.
#[derive(Clone, Debug, Serialize)]
pub struct UrlSite {
    /// The file.
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// The API (`fetch`, `swr:default`).
    pub api: String,
    /// Use.
    pub kind: UrlUse,
    /// URL patterns.
    pub urls: Vec<UrlPat>,
    /// Alternatives that could not be pinned down (reasons).
    pub unresolved: Vec<String>,
    /// The URL is a parameter forwarded by the enclosing function: the wrapper
    /// it became (whose call sites are scanned instead), `"self"` for the
    /// request's own URL, or `"caller"` for a callback of another URL site
    /// (SWR fetchers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarded: Option<String>,
}

/// A function that forwards one of its parameters to a URL API: a derived URL
/// API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Wrapper {
    /// The file defining it.
    pub file: PathBuf,
    /// Its local name.
    pub name: String,
    /// Names it is exported under (`default` included).
    pub exports: Vec<String>,
    /// The parameter index.
    pub arg: u8,
    /// The field of a destructured object parameter.
    pub field: Option<String>,
    /// Use.
    pub kind: UrlUse,
    /// Prefix inherited from the wrapped API.
    pub prefix: Option<String>,
    /// The API it wraps (for explanations).
    pub via: String,
}

/// Whether an unknown URL expression is the current URL: the request's own URL
/// (server code reading the incoming request), or an empty or query-only URL
/// (relative to the current page). It adds no route.
pub(crate) fn is_self_url(reason: &str) -> bool {
    let reason = Source::split(reason).0;
    if reason == "empty URL" || reason.starts_with("query-only URL") {
        return true;
    }
    let r = reason.trim_start_matches("${").trim_end_matches('}');
    let r = r
        .strip_prefix("String(")
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(r);
    matches!(
        r,
        "req.url"
            | "request.url"
            | "req.nextUrl"
            | "request.nextUrl"
            | "location"
            | "window.location"
            | "document.location"
            | "location.href"
            | "window.location.href"
            | "location.pathname"
            | "window.location.pathname"
            | "document.location.href"
            | "nextUrl"
            | "req.nextUrl.href"
            | "request.nextUrl.href"
    )
}

pub(crate) fn apply_prefix(t: &mut Tpl, prefix: &str) {
    if let Some(Piece::Lit(s)) = t.first_mut()
        && s.starts_with('/')
        && !s.starts_with(prefix)
    {
        s.insert_str(0, prefix);
    }
}

/// Converts alternatives to patterns and unresolved reasons.
pub fn to_patterns(tpls: &[Tpl], prefix: Option<&str>) -> (Vec<UrlPat>, Vec<String>) {
    let mut urls = Vec::new();
    let mut unresolved = Vec::new();
    for t in tpls {
        let mut t = t.clone();
        if let Some(p) = prefix {
            apply_prefix(&mut t, p);
        }
        // Not a URL: an opaque string (an SWR cache key, a name).
        if let Some(Piece::Lit(l)) = t.first() {
            let url_like = l.is_empty()
                || ["/", "?", "#", "http:", "https:"]
                    .iter()
                    .any(|p| l.starts_with(p));
            if !url_like {
                continue;
            }
        }
        match UrlPat::from_pieces(&t) {
            // Only an origin-relative wildcard (`${origin}${path}`, `http://localhost:${port}/${p}`):
            // pins down nothing, so an uncertainty rather than a match of every route.
            Ok(u)
                if u.open_tail
                    && !u.segs.iter().flatten().any(|p| matches!(p, Part::Lit(_)))
                    && match &u.origin {
                        Origin::Absolute(h) => {
                            let host = h.split("://").nth(1).unwrap_or("");
                            let host = host.split(':').next().unwrap_or("");
                            host.is_empty() || host == "localhost" || host == "127.0.0.1"
                        }
                        _ => true,
                    } =>
            {
                unresolved.push(path_source(&t).tag(&format!("origin-relative wildcard `{u}`")));
            }
            // A prefix glued to an unknown path that the request, storage or an input
            // supplies (`/api${location.pathname}`): any URL, whatever the code contains.
            Ok(u)
                if u.open_tail
                    && matches!(
                        u.segs.last().map(Vec::as_slice),
                        Some([Part::Lit(_), Part::Any])
                    )
                    && tail_source(&t).outside() =>
            {
                unresolved.push(tail_source(&t).tag(&format!("open pattern `{u}`")));
            }
            Ok(u) => {
                if !urls.contains(&u) {
                    urls.push(u)
                }
            }
            Err(e) => unresolved.push(e.tagged()),
        }
    }
    (urls, unresolved)
}

pub(crate) fn arg_value(
    ev: &Eval<'_, '_>,
    f: &Rc<JsFile>,
    range: (usize, usize),
    field: Option<&str>,
) -> Vec<Tpl> {
    let (a, b) = range;
    // A function argument (SWR infinite `getKey`): its results.
    if let Some(rs) = f.function_results(a, b) {
        let mut v = Vec::new();
        for (x, y) in rs {
            v.extend(arg_value(ev, f, (x, y), field));
        }
        return v;
    }
    use turborepo_trace_lang_js::lexer::TokKind;
    // An array key (`useSWR([url, id], ...)`): the first element.
    if f.kind(a) == TokKind::LBracket
        && f.closer(a) == Some(b - 1)
        && let Some(&(x, y)) = f.split_commas(a + 1, b - 1).first()
    {
        return ev.eval(f, x, y);
    }
    if let Some(fld) = field
        && f.kind(a) == TokKind::LBrace
        && f.closer(a) == Some(b - 1)
    {
        for (x, y) in f.split_commas(a + 1, b - 1) {
            if f.text(x).trim_matches(['"', '\'']) == fld && f.kind(x + 1) == TokKind::Colon {
                return ev.eval(f, x + 2, y);
            }
            if f.text(x) == fld && y == x + 1 {
                return ev.eval(f, x, y);
            }
        }
        return vec![vec![Piece::Wild {
            multi: false,
            reason: format!("no `{fld}` field"),
            src: Source::Code,
        }]];
    }
    ev.eval(f, a, b)
}

/// Scans one file for URL sites: calls of `apis` and of derived `wrappers`.
/// Returns the sites and the wrappers newly derived from them (sites whose URL
/// is a parameter of a named enclosing function).
pub fn scan_file(
    files: &Files<'_>,
    path: &Path,
    apis: &[UrlApi],
    wrappers: &[Wrapper],
) -> (Vec<UrlSite>, Vec<Wrapper>) {
    let Some(f) = files.get(path) else {
        return (Vec::new(), Vec::new());
    };
    let ev = Eval::new(files);
    let mut out = Vec::new();
    let mut derived = Vec::new();
    let sites = call_sites(&f);
    for cs in &sites {
        let style = if cs.is_new { Style::New } else { Style::Call };
        // A table entry, or a wrapper.
        let (api_name, kind, arg, field, prefix) = if let Some(a) = apis.iter().find(|a| {
            a.style == style && a.name == cs.path && a.module.as_deref() == cs.module.as_deref()
        }) {
            let name = match &a.module {
                Some(m) => format!("{m}:{}", a.name),
                None => a.name.clone(),
            };
            (name, a.kind, a.arg, a.field.clone(), a.prefix.clone())
        } else if let Some(w) = (!cs.is_new)
            .then(|| wrappers.iter().find(|w| wrapper_matches(files, &f, cs, w)))
            .flatten()
        {
            (
                format!("{} (wraps {})", w.name, w.via),
                w.kind,
                w.arg,
                w.field.clone(),
                w.prefix.clone(),
            )
        } else {
            continue;
        };
        let Some(&range) = cs.args.get(arg as usize) else {
            continue;
        };
        if kind == UrlUse::Build && cs.args.len() < 2 {
            continue;
        }
        let tpls = arg_value(&ev, &f, range, field.as_deref());
        let text = f.range_text(range.0, range.1);
        let (urls, mut unresolved) = if all_unknown(&tpls) {
            (Vec::new(), vec![text.clone()])
        } else {
            to_patterns(&tpls, prefix.as_deref())
        };
        unresolved.dedup();
        let mut forwarded = None;
        if urls.is_empty() && !unresolved.is_empty() {
            if unresolved.iter().all(|r| is_self_url(r)) {
                forwarded = Some("self".to_string());
            } else if let Some(w) = forward(
                files,
                &f,
                cs.at,
                range,
                kind,
                prefix.clone(),
                &api_name,
                apis,
                wrappers,
            ) {
                forwarded = Some(match &w {
                    Some(w) => w.name.clone(),
                    None => "caller".to_string(),
                });
                if let Some(w) = w
                    && !wrappers.contains(&w)
                    && !derived.contains(&w)
                {
                    derived.push(w);
                }
            }
            if forwarded.is_some() {
                unresolved.clear();
            }
        }
        out.push(UrlSite {
            file: path.to_path_buf(),
            line: f.line(cs.at),
            api: api_name,
            kind,
            urls,
            unresolved,
            forwarded,
        });
    }
    out.extend(scan_jsx(&ev, &f, apis));
    (out, derived)
}

fn wrapper_matches(
    files: &Files<'_>,
    f: &JsFile,
    cs: &super::jseval::CallSite,
    w: &Wrapper,
) -> bool {
    match &cs.module {
        None => f.path == w.file && cs.path == w.name,
        Some(m) => {
            (w.exports.contains(&cs.path)
                || (cs.path == "default" && w.exports.iter().any(|e| e == "default")))
                && files.resolve(&f.path, m).as_deref() == Some(w.file.as_path())
        }
    }
}

/// If the URL argument is a parameter of the enclosing function:
/// `Some(Some(wrapper))` for a named function, `Some(None)` for an anonymous
/// callback passed to another URL site, `None` otherwise.
#[allow(clippy::too_many_arguments)]
fn forward(
    files: &Files<'_>,
    f: &Rc<JsFile>,
    at: usize,
    range: (usize, usize),
    kind: UrlUse,
    prefix: Option<String>,
    via: &str,
    apis: &[UrlApi],
    wrappers: &[Wrapper],
) -> Option<Option<Wrapper>> {
    use turborepo_trace_lang_js::lexer::TokKind;
    let (mut a, mut b) = range;
    // `x as string`, `x!`, `String(x)`, `x.toString()`.
    while b > a + 1 {
        if f.kind(b - 1) == TokKind::Bang {
            b -= 1;
        } else if b >= a + 3 && matches!(f.text(b - 2), "as" | "satisfies") {
            b -= 2;
        } else if b >= a + 4 && f.text(b - 3) == "toString" {
            b -= 4;
        } else {
            break;
        }
    }
    if f.text(a) == "String" && f.kind(a + 1) == TokKind::LParen && f.closer(a + 1) == Some(b - 1) {
        a += 2;
        b -= 1;
    }
    if b != a + 1 || f.kind(a) != TokKind::Ident {
        return None;
    }
    let x = f.text(a);
    let fd = f.enclosing_fn(at)?;
    let p = fd.params.iter().find(|p| p.name == x)?;
    match &fd.name {
        Some(name) => {
            let mut exports: Vec<String> = f
                .exports
                .iter()
                .filter(|(_, local)| *local == name)
                .map(|(e, _)| e.clone())
                .collect();
            exports.sort();
            Some(Some(Wrapper {
                file: f.path.clone(),
                name: name.clone(),
                exports,
                arg: p.index,
                field: p.field.clone(),
                kind,
                prefix,
                via: via.to_string(),
            }))
        }
        None => {
            // An anonymous callback: forwarded if it is an argument of a URL site.
            let c = f.enclosing_call(fd.start)?;
            let mut start = c;
            while start >= 2
                && matches!(f.kind(start - 1), TokKind::Dot | TokKind::OptChain)
                && f.kind(start - 2) == TokKind::Ident
            {
                start -= 2;
            }
            let chain: Vec<String> = (start..=c)
                .step_by(2)
                .map(|k| f.text(k).to_string())
                .collect();
            let (module, path) = super::jseval::resolve_chain(f, &chain);
            let is_api = apis.iter().any(|a| a.name == path && a.module == module);
            let cs = super::jseval::CallSite {
                module,
                path,
                is_new: false,
                args: Vec::new(),
                at: start,
                receiver: None,
                method: String::new(),
                open: c + 1,
            };
            let is_wrapper = wrappers.iter().any(|w| wrapper_matches(files, f, &cs, w));
            (is_api || is_wrapper).then_some(None)
        }
    }
}

/// `<Tag ... attr=...>` sites for JSX table entries.
pub(crate) fn scan_jsx(ev: &Eval<'_, '_>, f: &Rc<JsFile>, apis: &[UrlApi]) -> Vec<UrlSite> {
    let jsx: Vec<&UrlApi> = apis.iter().filter(|a| a.style == Style::Jsx).collect();
    if jsx.is_empty() {
        return Vec::new();
    }
    let src = &f.src;
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(k) = memchr_lt(src, i) {
        i = k + 1;
        // Tag name.
        let mut e = k + 1;
        while e < src.len() && (src[e].is_ascii_alphanumeric() || src[e] == b'.' || src[e] == b'_')
        {
            e += 1;
        }
        if e == k + 1 || e >= src.len() || !src[e].is_ascii_whitespace() {
            continue;
        }
        let tag = String::from_utf8_lossy(&src[k + 1..e]).into_owned();
        // Resolve the tag: an import, or a lowercase intrinsic.
        let (module, name) = if tag.chars().next().is_some_and(char::is_lowercase) {
            (None, tag.clone())
        } else {
            let chain: Vec<String> = tag.split('.').map(String::from).collect();
            super::jseval::resolve_chain(f, &chain)
        };
        let known = jsx
            .iter()
            .find(|a| a.module.as_deref() == module.as_deref() && a.name == name);
        // A component that is given an `href` is a link of the app: a wrapper that
        // hands its props to `next/link` (`<Link {...props}>`), a link or
        // button of a design system, a package's component. Where it sends the
        // visitor is the attribute at its use, whatever the component does with
        // it, so the use is the site; nothing is assumed about names other than
        // `href`, the DOM's own.
        let component = tag.contains('.') || tag.starts_with(|c: char| c.is_ascii_uppercase());
        let (attr, kind, prefix) = match known {
            Some(a) => (
                a.field.as_deref().unwrap_or("href"),
                a.kind,
                a.prefix.as_deref(),
            ),
            None if component => ("href", UrlUse::Navigation, None),
            None => continue,
        };
        // Attributes until `>` at brace depth 0.
        let mut j = e;
        let mut depth = 0i32;
        let mut found = None;
        while j < src.len() {
            match src[j] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                b'>' if depth == 0 => break,
                b'"' | b'\'' if depth == 0 => {
                    let q = src[j];
                    j += 1;
                    while j < src.len() && src[j] != q {
                        j += 1;
                    }
                }
                _ if depth == 0
                    && src[j..].starts_with(attr.as_bytes())
                    && src[j - 1].is_ascii_whitespace() =>
                {
                    let mut v = j + attr.len();
                    while v < src.len() && src[v].is_ascii_whitespace() {
                        v += 1;
                    }
                    if src.get(v) == Some(&b'=') {
                        v += 1;
                        while v < src.len() && src[v].is_ascii_whitespace() {
                            v += 1;
                        }
                        found = Some(v);
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let Some(v) = found else { continue };
        let line = 1 + src[..v].iter().filter(|&&b| b == b'\n').count() as u32;
        let tpls: Vec<Tpl> = match src.get(v) {
            Some(&q @ (b'"' | b'\'')) => {
                let end = src[v + 1..].iter().position(|&b| b == q).map(|p| v + 1 + p);
                let Some(end) = end else { continue };
                vec![vec![Piece::Lit(
                    String::from_utf8_lossy(&src[v + 1..end]).into_owned(),
                )]]
            }
            Some(b'{') => {
                // Find the token range inside the braces.
                let start_off = v as u32 + 1;
                let a_tok = f.toks.iter().position(|t| t.start >= start_off);
                let Some(a_tok) = a_tok else { continue };
                // The opening brace token may or may not exist; use the expression end.
                let b_tok = f.expr_end(a_tok, f.toks.len());
                if b_tok <= a_tok {
                    continue;
                }
                ev.eval(f, a_tok, b_tok)
            }
            _ => continue,
        };
        let (urls, unresolved) = if all_unknown(&tpls) {
            // `<form action={serverAction}>`: a function (a server action reached by
            // import), not a URL.
            if attr == "action" {
                continue;
            }
            (Vec::new(), vec![format!("<{tag} {attr}>")])
        } else {
            to_patterns(&tpls, prefix)
        };
        out.push(UrlSite {
            file: f.path.clone(),
            line,
            api: format!("<{tag} {attr}>"),
            kind,
            urls,
            unresolved,
            forwarded: None,
        });
    }
    out
}

fn memchr_lt(src: &[u8], from: usize) -> Option<usize> {
    src.get(from..)?
        .iter()
        .position(|&b| b == b'<')
        .map(|p| from + p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_read_next_to_the_other_readers_tables() {
        // The hints (`[[hint]]`, `[[env]]`) and the lint (`[lint]`) read the
        // same file: their tables are not this reader's business.
        let apis = parse_url_apis_toml(
            r#"
[[hint]]
files = "a.ts"
loads = []

[[env]]
name = "DATA"
values = ["data/*.json"]

[lint]
baseline = "ci/lint-baseline.json"

[[nextjs.url_apis]]
module = "@acme/fetch"
name = "apiFetch"
prefix = "/api"
"#,
        )
        .unwrap();
        assert_eq!(apis.len(), 1);
        assert_eq!(apis[0].prefix.as_deref(), Some("/api"));
        assert_eq!(parse_url_apis_toml("[lint]\n").unwrap(), []);
        // A misspelt key is an error, not a wrapper without a prefix or no table at
        // all.
        let e = parse_url_apis_toml(
            "[[nextjs.url_apis]]\nmodule = \"@acme/fetch\"\nname = \"apiFetch\"\nprefx = \
             \"/api\"\n",
        )
        .unwrap_err();
        assert!(e.contains("prefx"), "{e}");
        let e = parse_url_apis_toml("[[nextjs.url_api]]\nname = \"x\"\n").unwrap_err();
        assert!(e.contains("url_api"), "{e}");
        assert!(parse_url_apis_toml(FRONT_URL_APIS_TOML).unwrap().len() >= 4);
    }
}
