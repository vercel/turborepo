//! URL patterns: a path whose segments may contain unknown parts, as produced
//! by partially evaluating `page.goto(`/${team}/settings`)` or
//! `fetch(`/api/teams/${id}/members`)`.
//!
//! - A known segment is a literal (`settings`).
//! - A segment with unknown parts (`${id}`, `pre-${x}`) stands for one segment.
//! - An open tail (`/docs/${path}` where `path` may hold slashes, or a URL
//!   prefix like `/api/teams/` + unknown) stands for any number of further
//!   segments.
//!
//! Query strings and fragments are dropped. Origins are split off: a literal
//! `https://host` makes the URL absolute ([`Origin::Absolute`]); an unknown leading value
//! followed by `/` is an unknown origin ([`Origin::Unknown`]).

use std::fmt;

use serde::{Serialize, Serializer};
use turborepo_trace_core::eval::{PatPart, Value};

use super::{routes::Seg, source::Source};

/// One part of a segment.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// Known text.
    Lit(String),
    /// Unknown text (no `/`).
    Any,
}

/// Where the URL points.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "host")]
pub enum Origin {
    /// Same origin (a path).
    Relative,
    /// A literal `scheme://host`.
    Absolute(String),
    /// An unknown origin expression (`${baseURL}/x`).
    Unknown,
}

/// A URL pattern.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UrlPat {
    /// Origin.
    pub origin: Origin,
    /// Path segments.
    pub segs: Vec<Vec<Part>>,
    /// Further unknown segments may follow.
    pub open_tail: bool,
}

/// Why a value is not a usable URL pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Unresolved {
    /// Explanation (the source text of the unknown part).
    pub reason: String,
    /// Where the unknown path comes from.
    #[serde(skip)]
    pub src: Source,
}

impl Unresolved {
    /// The reason with its source ([`Source::tag`]).
    pub fn tagged(&self) -> String {
        self.src.tag(&self.reason)
    }
}

impl Serialize for UrlPat {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl fmt::Display for UrlPat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Origin::Absolute(h) = &self.origin {
            f.write_str(h)?;
        } else if self.origin == Origin::Unknown {
            f.write_str("<origin>")?;
        }
        if self.segs.is_empty() && !self.open_tail {
            return f.write_str("/");
        }
        for s in &self.segs {
            f.write_str("/")?;
            for p in s {
                match p {
                    Part::Lit(t) => f.write_str(t)?,
                    Part::Any => f.write_str("*")?,
                }
            }
        }
        if self.open_tail {
            f.write_str("/**")?;
        }
        Ok(())
    }
}

/// A piece of a partially known string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Known text.
    Lit(String),
    /// Unknown text; `multi` if it may contain `/`.
    Wild {
        /// May span segments.
        multi: bool,
        /// Source text.
        reason: String,
        /// Where the value comes from.
        src: Source,
    },
    /// The value of a function parameter that no caller has bound yet (symbolic
    /// evaluation, [`super::jseval::Eval::symbolic`]). As a pattern it is an
    /// unknown value.
    Param(ParamRef),
}

/// A function parameter, or a field of it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamRef {
    /// The file's id in the file cache.
    pub file: u32,
    /// The function's index in the file.
    pub func: u32,
    /// The parameter index.
    pub index: u8,
    /// The field path below the parameter (`opts.url` → `["url"]`).
    pub field: Vec<String>,
    /// The name in the source.
    pub name: String,
    /// Where the value comes from when no caller binds the parameter.
    pub src: Source,
}

impl Piece {
    /// An unknown value named `reason`; `multi` by the name
    /// ([`reason_is_pathish`]).
    pub fn wild(reason: &str) -> Piece {
        Piece::wild_from(reason, Source::Code)
    }

    /// An unknown value named `reason` that comes from `src`.
    pub fn wild_from(reason: &str, src: Source) -> Piece {
        Piece::Wild {
            multi: reason_is_pathish(reason),
            reason: reason.to_string(),
            src,
        }
    }

    /// Unbound parameters as unknown values.
    pub fn demote(&self) -> Piece {
        match self {
            Piece::Param(p) => Piece::wild_from(&p.name, p.src),
            other => other.clone(),
        }
    }
}

/// The source of the unknown parts of a URL's path: the highest source of the
/// unknown pieces that can decide which route the URL names. Not counted: a
/// leading origin (`${origin}${path}`), what follows the start of a query
/// string, and, next to known text, a piece that fills one segment
/// (`/api/teams/${id}/members`: whatever `id` is, the code names the route).
pub fn path_source(pieces: &[Piece]) -> Source {
    sources(pieces, false)
}

/// [`path_source`] of the pieces that may span segments (what opens a pattern's
/// tail).
pub fn tail_source(pieces: &[Piece]) -> Source {
    sources(pieces, true)
}

fn sources(pieces: &[Piece], multi_only: bool) -> Source {
    let mut out = Source::Code;
    let mut path_started = false;
    // Known text: single-segment pieces only fill it in.
    let multi_only = multi_only
        || pieces
            .iter()
            .any(|p| matches!(p, Piece::Lit(l) if l.chars().any(char::is_alphanumeric)));
    for (i, p) in pieces.iter().enumerate() {
        match p {
            Piece::Lit(l) => {
                if l.contains(['?', '#']) {
                    break;
                }
                path_started |= !l.is_empty();
            }
            Piece::Wild { multi, reason, src } => {
                if i == 0 && pieces.len() > 1 && reason_is_origin(reason) {
                    continue;
                }
                if path_started && reason_is_query(reason) {
                    break;
                }
                if *multi || !multi_only {
                    out = out.max(*src);
                }
                path_started = true;
            }
            Piece::Param(r) => {
                if !multi_only || reason_is_pathish(&r.name) {
                    out = out.max(r.src);
                }
                path_started = true;
            }
        }
    }
    out
}

/// Whether a wildcard named `reason` likely holds a multi-segment path or a
/// full URL.
pub fn reason_is_pathish(reason: &str) -> bool {
    let r = reason.trim_end_matches("()").to_ascii_lowercase();
    let last = r.rsplit(['.', ' ']).next().unwrap_or(&r);
    [
        "path", "url", "href", "pathname", "route", "endpoint", "rest", "suffix", "uri", "link",
    ]
    .iter()
    .any(|k| last.ends_with(k))
}

/// Whether a wildcard named `reason` likely holds a query string
/// (`encode(query)`, `qs.stringify(params)`, `searchParams`): appended to a
/// path, it adds no path part.
pub fn reason_is_query(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    let name = r.split('(').next().unwrap_or(&r);
    let last = name.rsplit(['.', ' ']).next().unwrap_or(name);
    r.contains("searchparams")
        || r.contains("stringify(")
        || r.contains("querystring")
        || matches!(last, "query" | "qs" | "search" | "encode" | "encodequery")
        || last.ends_with("query")
        || last.ends_with("querystring")
        || last.ends_with("qs")
}

/// Whether a wildcard named `reason` likely holds an origin (`baseURL`,
/// `getBaseUrl()`).
pub fn reason_is_origin(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    [
        "baseurl",
        "base_url",
        "origin",
        "host",
        "base",
        "domain",
        "apiurl",
        "api_url",
        // An environment-configured base URL (`${process.env.API_URL}/v1/x`).
        "process.env.",
    ]
    .iter()
    .any(|k| r.contains(k))
}

impl UrlPat {
    /// A known path (`/a/b?x` → `/a/b`).
    pub fn literal(path: &str) -> Result<UrlPat, Unresolved> {
        UrlPat::from_pieces(&[Piece::Lit(path.to_string())])
    }

    /// From a partially evaluated value. Alternatives yield several patterns.
    pub fn from_value(v: &Value) -> Vec<Result<UrlPat, Unresolved>> {
        match v {
            Value::Str(s) => vec![UrlPat::literal(s)],
            Value::Pattern(parts) => {
                let pieces: Vec<Piece> = parts
                    .iter()
                    .map(|p| match p {
                        PatPart::Lit(s) => Piece::Lit(s.clone()),
                        PatPart::Wild { reason } => Piece::wild(reason),
                    })
                    .collect();
                vec![UrlPat::from_pieces(&pieces)]
            }
            Value::Alternatives(vs) => vs.iter().flat_map(UrlPat::from_value).collect(),
            Value::Unknown { reason } => vec![Err(Unresolved {
                reason: reason.to_string(),
                src: Source::Code,
            })],
        }
    }

    /// From pieces. Fails for fully unknown values and values without any known
    /// path part (the rule for runtime fetch URLs: `/${x}` pins down
    /// nothing).
    pub fn from_pieces(pieces: &[Piece]) -> Result<UrlPat, Unresolved> {
        UrlPat::build(pieces, false)
    }

    /// From pieces, for test navigations: a path of unknown segments
    /// (`/${team}/${project}`) is kept, since its segment count still
    /// selects routes, and a leading unknown value followed by `/` that
    /// does not look like an origin is the first segment of a path relative
    /// to the base URL.
    pub fn from_pieces_lenient(pieces: &[Piece]) -> Result<UrlPat, Unresolved> {
        UrlPat::build(pieces, true)
    }

    fn build(pieces: &[Piece], lenient: bool) -> Result<UrlPat, Unresolved> {
        // Merge adjacent literals and split `{param}` placeholders out of literals.
        let mut ps: Vec<Piece> = Vec::new();
        for p in pieces {
            match p {
                Piece::Lit(s) => {
                    for (lit, is_param) in split_placeholders(s) {
                        if is_param {
                            ps.push(Piece::Wild {
                                multi: false,
                                reason: lit,
                                src: Source::Code,
                            });
                        } else if let Some(Piece::Lit(prev)) = ps.last_mut() {
                            prev.push_str(&lit);
                        } else if !lit.is_empty() {
                            ps.push(Piece::Lit(lit));
                        }
                    }
                }
                w => ps.push(w.demote()),
            }
        }
        let whole = ps
            .iter()
            .map(|p| match p {
                Piece::Lit(s) => s.clone(),
                Piece::Wild { reason, .. } => format!("${{{reason}}}"),
                Piece::Param(p) => format!("${{{}}}", p.name),
            })
            .collect::<String>();
        let src = path_source(&ps);
        let whole_reason = || whole.clone();
        if ps.is_empty() {
            return Err(Unresolved {
                reason: "empty URL".into(),
                src,
            });
        }
        // Origin.
        let mut origin = Origin::Relative;
        let mut i = 0;
        match &ps[0] {
            Piece::Lit(s) if s.starts_with("http://") || s.starts_with("https://") => {
                let after = s.find("://").map_or(0, |k| k + 3);
                match s[after..].find('/') {
                    Some(k) => {
                        origin = Origin::Absolute(s[..after + k].to_string());
                        ps[0] = Piece::Lit(s[after + k..].to_string());
                    }
                    None => {
                        origin = Origin::Absolute(s.clone());
                        if ps.len() == 1 {
                            return Ok(UrlPat {
                                origin,
                                segs: Vec::new(),
                                open_tail: false,
                            });
                        }
                        // An unknown part follows. After a complete host, a path-like value
                        // is the path (`https://host${path}`): an unknown path. Otherwise the
                        // host continues (`http://${host}/x`, `http://localhost:${port}/x`,
                        // `https://${sub}.example.com/x`) up to the next `/`.
                        let host_done = after < s.len()
                            && matches!(ps.get(1), Some(Piece::Wild { multi: true, .. }));
                        let mut label = s.clone();
                        let mut found = None;
                        if !host_done {
                            for (k, p) in ps.iter().enumerate().skip(1) {
                                match p {
                                    Piece::Lit(t) => match t.find('/') {
                                        Some(x) => {
                                            label.push_str(&t[..x]);
                                            found = Some((k, x));
                                            break;
                                        }
                                        None => label.push_str(t),
                                    },
                                    _ => label.push('*'),
                                }
                            }
                        }
                        let Some((k, x)) = found else {
                            return Ok(UrlPat {
                                origin,
                                segs: Vec::new(),
                                open_tail: true,
                            });
                        };
                        let host = &label[after..];
                        origin = if host
                            .chars()
                            .all(|c| c == '*' || c == ':' || c.is_ascii_digit())
                        {
                            Origin::Unknown
                        } else {
                            Origin::Absolute(label.clone())
                        };
                        if let Piece::Lit(t) = &ps[k] {
                            ps[k] = Piece::Lit(t[x..].to_string());
                        }
                        i = k;
                    }
                }
            }
            Piece::Lit(s) if s.starts_with("//") => {
                let rest = &s[2..];
                let k = rest.find('/').unwrap_or(rest.len());
                origin = Origin::Absolute(format!("//{}", &rest[..k]));
                ps[0] = Piece::Lit(rest[k..].to_string());
            }
            Piece::Wild { reason, .. } if matches!(ps.get(1), Some(Piece::Lit(t)) if t.starts_with('/')) =>
            {
                if reason_is_origin(reason) || reason_is_pathish(reason) {
                    origin = Origin::Unknown;
                    i = 1;
                } else if lenient {
                    // A relative path starting with an unknown segment.
                } else {
                    return Err(Unresolved {
                        reason: whole_reason(),
                        src,
                    });
                }
            }
            Piece::Wild { .. } if !lenient || ps.len() == 1 => {
                return Err(Unresolved {
                    reason: whole_reason(),
                    src,
                });
            }
            Piece::Wild { .. } | Piece::Param(_) => {}
            Piece::Lit(s) if s.starts_with('?') || s.starts_with('#') => {
                return Err(Unresolved {
                    reason: format!("query-only URL `{whole}`"),
                    src,
                });
            }
            Piece::Lit(_) => {}
        }
        // Path: stop at `?` or `#`.
        let mut segs: Vec<Vec<Part>> = Vec::new();
        let mut cur: Vec<Part> = Vec::new();
        let mut open_tail = false;
        let mut started = false;
        let mut done = false;
        for (idx, p) in ps.iter().enumerate().skip(i) {
            if done {
                break;
            }
            match p {
                Piece::Lit(s) => {
                    let mut s = s.as_str();
                    if let Some(k) = s.find(['?', '#']) {
                        s = &s[..k];
                        done = true;
                    }
                    for (n, chunk) in s.split('/').enumerate() {
                        if n > 0 {
                            if started {
                                segs.push(std::mem::take(&mut cur));
                            }
                            started = true;
                        } else if !started && idx == i && !chunk.is_empty() {
                            // A path without a leading slash: relative to the base URL.
                            started = true;
                        }
                        if !chunk.is_empty() {
                            cur.push(Part::Lit(decode(chunk)));
                        }
                    }
                }
                Piece::Param(_) => {}
                Piece::Wild { reason, .. }
                    if started && !cur.is_empty() && reason_is_query(reason) =>
                {
                    // A query string appended to a path.
                    done = true;
                }
                Piece::Wild { multi, .. } => {
                    if !started {
                        started = true;
                    }
                    if *multi {
                        let at_seg_start = cur.is_empty();
                        if !at_seg_start {
                            cur.push(Part::Any);
                            segs.push(std::mem::take(&mut cur));
                        }
                        open_tail = true;
                        done = true;
                    } else if cur.last() != Some(&Part::Any) {
                        cur.push(Part::Any);
                    }
                }
            }
        }
        if !cur.is_empty() {
            segs.push(cur);
        }
        // Trailing empty segments come from a trailing slash.
        let has_lit = segs
            .iter()
            .flatten()
            .any(|p| matches!(p, Part::Lit(t) if !t.is_empty()));
        if !has_lit && !matches!(origin, Origin::Absolute(_)) {
            if (segs.is_empty() && !open_tail) || (lenient && !segs.is_empty() && !open_tail) {
                // `/` itself.
                return Ok(UrlPat {
                    origin,
                    segs,
                    open_tail,
                });
            }
            return Err(Unresolved {
                reason: format!("no known path part in `{whole}`"),
                src,
            });
        }
        Ok(UrlPat {
            origin,
            segs,
            open_tail,
        })
    }

    /// The path as a string with `*` for unknown parts (no origin).
    pub fn path_string(&self) -> String {
        let mut p = self.clone();
        p.origin = Origin::Relative;
        p.to_string()
    }

    /// Whether every segment is known and there is no open tail.
    pub fn is_concrete(&self) -> bool {
        !self.open_tail
            && self
                .segs
                .iter()
                .all(|s| s.iter().all(|p| matches!(p, Part::Lit(_))))
    }

    /// The literal text of segment `i`, if fully known.
    pub fn lit(&self, i: usize) -> Option<String> {
        let s = self.segs.get(i)?;
        let mut out = String::new();
        for p in s {
            match p {
                Part::Lit(t) => out.push_str(t),
                Part::Any => return None,
            }
        }
        Some(out)
    }

    /// Drops the first `n` segments if they equal `prefix` (basePath, locale).
    pub fn strip_prefix_segs(&self, prefix: &[&str]) -> Option<UrlPat> {
        if self.segs.len() < prefix.len() {
            return None;
        }
        for (i, p) in prefix.iter().enumerate() {
            if self.lit(i).as_deref() != Some(*p) {
                return None;
            }
        }
        Some(UrlPat {
            origin: self.origin.clone(),
            segs: self.segs[prefix.len()..].to_vec(),
            open_tail: self.open_tail,
        })
    }

    /// The path rendered with `sentinel` for unknown parts (for regex rules).
    pub fn render(&self, sentinel: &str) -> String {
        let mut s = String::new();
        for seg in &self.segs {
            s.push('/');
            for p in seg {
                match p {
                    Part::Lit(t) => s.push_str(t),
                    Part::Any => s.push_str(sentinel),
                }
            }
        }
        if self.open_tail {
            s.push('/');
            s.push_str(sentinel);
        }
        if s.is_empty() {
            s.push('/');
        }
        s
    }

    /// Parses a rendered path back (inverse of [`UrlPat::render`]).
    pub fn parse_rendered(s: &str, sentinel: &str) -> Result<UrlPat, Unresolved> {
        let mut pieces = Vec::new();
        for (i, chunk) in s.split(sentinel).enumerate() {
            if i > 0 {
                pieces.push(Piece::Wild {
                    multi: false,
                    reason: "param".into(),
                    src: Source::Code,
                });
            }
            if !chunk.is_empty() {
                pieces.push(Piece::Lit(chunk.to_string()));
            }
        }
        UrlPat::from_pieces(&pieces)
    }
}

fn decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Splits `{param}` placeholders (typed API paths: `/v9/projects/{idOrName}`)
/// out of a literal: `(text, is_param)` pairs.
fn split_placeholders(s: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        let Some(b) = rest[a..].find('}') else { break };
        let name = &rest[a + 1..a + b];
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            break;
        }
        out.push((rest[..a].to_string(), false));
        out.push((name.to_string(), true));
        rest = &rest[a + b + 1..];
    }
    out.push((rest.to_string(), false));
    out
}

/// Whether a pattern segment can equal the literal `v`.
fn seg_can_equal(seg: &[Part], v: &str) -> bool {
    fn go(parts: &[Part], s: &str) -> bool {
        match parts.first() {
            None => s.is_empty(),
            Some(Part::Lit(t)) => s.starts_with(t.as_str()) && go(&parts[1..], &s[t.len()..]),
            Some(Part::Any) => (0..=s.len())
                .filter(|&k| s.is_char_boundary(k))
                .any(|k| go(&parts[1..], &s[k..])),
        }
    }
    go(seg, v)
}

/// Sound overlap: some URL the pattern stands for is matched by the route
/// segments.
pub fn overlaps(route: &[Seg], pat: &UrlPat) -> bool {
    fn go(route: &[Seg], segs: &[Vec<Part>], open: bool) -> bool {
        match route.first() {
            None => segs.is_empty(),
            Some(Seg::OptCatchAll(_)) => true,
            Some(Seg::CatchAll(_)) => !segs.is_empty() || open,
            Some(s) => {
                if segs.is_empty() {
                    // The open tail may supply the remaining segments.
                    return open;
                }
                let ok = match s {
                    Seg::Static(v) => seg_can_equal(&segs[0], v),
                    _ => true,
                };
                ok && go(&route[1..], &segs[1..], open)
            }
        }
    }
    go(route, &pat.segs, pat.open_tail)
}

/// Heuristic match: unknown segment parts never equal a fixed segment name.
/// Returns the rank vector if `route` matches every URL of the (closed) pattern
/// under that assumption.
pub fn heuristic_rank(route: &[Seg], pat: &UrlPat) -> Option<Vec<u8>> {
    if pat.open_tail {
        return None;
    }
    let parts: Vec<Option<String>> = (0..pat.segs.len()).map(|i| pat.lit(i)).collect();
    let refs: Vec<Option<&str>> = parts.iter().map(|p| p.as_deref()).collect();
    super::routes::rank_against(route, &refs)
}

/// Whether `route` matches every instantiation of the pattern (unknowns fall
/// only on its dynamic segments).
pub fn always_matches(route: &[Seg], pat: &UrlPat) -> bool {
    if pat.open_tail {
        return matches!(route.last(), Some(Seg::CatchAll(_) | Seg::OptCatchAll(_)))
            && route.len() <= pat.segs.len() + 1
            && route[..route.len() - 1]
                .iter()
                .enumerate()
                .all(|(i, s)| match s {
                    Seg::Static(v) => pat.lit(i).as_deref() == Some(v.as_str()),
                    Seg::Dynamic(_) => i < pat.segs.len(),
                    _ => false,
                });
    }
    let mut i = 0;
    for s in route {
        match s {
            Seg::Static(v) => {
                if pat.lit(i).as_deref() != Some(v.as_str()) {
                    return false;
                }
                i += 1;
            }
            Seg::Dynamic(_) => {
                if i >= pat.segs.len() {
                    return false;
                }
                i += 1;
            }
            Seg::CatchAll(_) => return i < pat.segs.len(),
            Seg::OptCatchAll(_) => return true,
        }
    }
    i == pat.segs.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nextjs::routes::parse_pattern;

    fn pat(pieces: &[&str]) -> Result<UrlPat, Unresolved> {
        let ps: Vec<Piece> = pieces
            .iter()
            .map(|p| match p.strip_prefix('$') {
                Some(r) => Piece::wild(r),
                None => Piece::Lit(p.to_string()),
            })
            .collect();
        UrlPat::from_pieces(&ps)
    }

    #[test]
    fn parsing() {
        assert_eq!(pat(&["/a/b?x=1"]).unwrap().to_string(), "/a/b");
        assert_eq!(
            pat(&["/", "$team", "/settings"]).unwrap().to_string(),
            "/*/settings"
        );
        assert_eq!(
            pat(&["/api/teams/", "$id"]).unwrap().to_string(),
            "/api/teams/*"
        );
        assert_eq!(
            pat(&["/api/teams/", "$rest"]).unwrap().to_string(),
            "/api/teams/**"
        );
        assert_eq!(pat(&["/docs/", "$path"]).unwrap().to_string(), "/docs/**");
        assert_eq!(
            pat(&["$baseURL", "/x/", "$id"]).unwrap().to_string(),
            "<origin>/x/*"
        );
        assert_eq!(
            pat(&["https://api.example.com/v1/", "$id"])
                .unwrap()
                .to_string(),
            "https://api.example.com/v1/*"
        );
        assert_eq!(
            pat(&["/v9/projects/{idOrName}"]).unwrap().to_string(),
            "/v9/projects/*"
        );
        assert!(pat(&["$x"]).is_err());
        assert!(pat(&["$team", "/x"]).is_err());
        assert!(pat(&["/", "$a"]).is_err());
        assert_eq!(pat(&["/"]).unwrap().to_string(), "/");
        assert_eq!(pat(&["settings"]).unwrap().to_string(), "/settings");
    }

    #[test]
    fn overlap_and_heuristic() {
        let p = pat(&["/api/teams/", "$id", "/members"]).unwrap();
        assert!(overlaps(&parse_pattern("/api/teams/[teamId]/members"), &p));
        assert!(overlaps(&parse_pattern("/api/teams/new/members"), &p));
        assert!(!overlaps(&parse_pattern("/api/teams/[teamId]"), &p));
        let prefix = pat(&["/api/teams/", "$endpoint"]).unwrap();
        assert!(overlaps(
            &parse_pattern("/api/teams/[teamId]/members"),
            &prefix
        ));
        assert!(overlaps(&parse_pattern("/api/teams"), &prefix));
        assert!(heuristic_rank(&parse_pattern("/api/teams/new/members"), &p).is_none());
        assert!(heuristic_rank(&parse_pattern("/api/teams/[t]/members"), &p).is_some());
        let pre = pat(&["/x/v", "$n"]).unwrap();
        assert!(overlaps(&parse_pattern("/x/v2"), &pre));
        assert!(!overlaps(&parse_pattern("/x/w2"), &pre));
    }
}
