//! POSIX path operations on partially known [`Value`]s, shared by language
//! evaluators (Node `path.*`, Python `os.path.*` / `pathlib`, Go `filepath.*`).
//!
//! Values are strings, patterns with wildcards, alternatives or unknown.
//! Operations work on the known parts and return `None` when the result cannot
//! be expressed (for example the directory of a pattern whose last literal has
//! no `/`). Languages keep their own anchors (file directory, cwd) around these
//! values: an *anchored* value is a path relative to such an anchor, where a
//! leading `/` is dropped and the empty string is the anchor itself.
//!
//! | Operation | Node | Python | Go |
//! |---|---|---|---|
//! | [`join`] then [`normalize`] | `path.join` | | `filepath.Join` |
//! | [`join_reset`] then [`normalize`] | | `os.path.join`, `pathlib /` | |
//! | [`normalize`] | `path.normalize` | `os.path.normpath` | `filepath.Clean` |
//! | [`dirname`] / [`parent`] | `path.dirname` | `os.path.dirname`, `.parent` | `filepath.Dir` |
//! | [`basename`] | `path.basename` | `os.path.basename`, `.name` | `filepath.Base` |
//! | [`extname`] | `path.extname` | `.suffix` | `filepath.Ext` |
//! | [`relative`] | `path.relative` | `os.path.relpath` | `filepath.Rel` |
//!
//! Edge-case differences between ecosystems (Python keeps a leading `//`, Go
//! returns `.` for an empty clean) are left to the caller.

use crate::eval::{PatPart, Value};

/// Whether the value starts with `/` (a known absolute path or a pattern whose
/// first literal is absolute).
pub fn starts_absolute(v: &Value) -> bool {
    match v {
        Value::Str(s) => s.starts_with('/'),
        Value::Pattern(p) => matches!(p.first(), Some(PatPart::Lit(s)) if s.starts_with('/')),
        _ => false,
    }
}

/// Joins segments with `/`, skipping empty known strings (Node `path.join`, Go
/// `filepath.Join`). The result is not normalized; see [`normalize`].
pub fn join(vals: impl IntoIterator<Item = Value>) -> Value {
    Value::join(vals.into_iter().filter(|v| v.as_str() != Some("")), "/")
}

/// [`join`] where an absolute segment discards everything before it (Python
/// `os.path.join`, `pathlib.PurePath.__truediv__`).
pub fn join_reset(vals: impl IntoIterator<Item = Value>) -> Value {
    let vals: Vec<Value> = vals.into_iter().collect();
    let start = vals.iter().rposition(starts_absolute).unwrap_or(0);
    join(vals.into_iter().skip(start))
}

/// POSIX normalization of a known string without the trailing-slash rule: `.`
/// segments and empty segments are dropped, `..` pops a segment (kept at the
/// start of a relative path, dropped at the root). An empty relative result is
/// `""`.
pub fn normalize_str(s: &str) -> String {
    let abs = s.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !abs {
                    out.push("..");
                }
            }
            _ => out.push(seg),
        }
    }
    let j = out.join("/");
    if abs { format!("/{j}") } else { j }
}

/// Normalizes a path value. `anchored`: relative to an anchor (a leading `/` is
/// dropped and the empty path is the anchor itself); otherwise like
/// `path.normalize` (`"."` when empty). In patterns the literal prefix before
/// the first wildcard is normalized; later literals lose `//` and `/./`. A `..`
/// after the first wildcard pops a literal segment as usual; a segment with a
/// wildcard may stand for several segments, so popping it leaves a wildcard
/// glued to what follows: `d/*/../x` is `d/*x`, which matches `d/x` and `d/a/x`
/// (see [`pop_after_wild`]). Without this the pattern would keep a literal `..`
/// and match no file.
pub fn normalize(v: Value, anchored: bool) -> Value {
    match v {
        Value::Str(s) => {
            let s = if anchored {
                s.trim_start_matches('/')
            } else {
                &s
            };
            let n = normalize_str(s);
            Value::Str(if n.is_empty() && !anchored {
                ".".into()
            } else {
                n
            })
        }
        Value::Pattern(mut parts) => {
            if let Some(PatPart::Lit(first)) = parts.first_mut() {
                let f: &str = if anchored {
                    first.trim_start_matches('/')
                } else {
                    first
                };
                let new_first = match f.rfind('/') {
                    Some(i) => {
                        let head = normalize_str(&f[..=i]);
                        let tail = &f[i + 1..];
                        if head.is_empty() {
                            tail.to_string()
                        } else if head.ends_with('/') {
                            format!("{head}{tail}")
                        } else {
                            format!("{head}/{tail}")
                        }
                    }
                    None => f.to_string(),
                };
                *first = new_first;
            }
            for p in parts.iter_mut().skip(1) {
                if let PatPart::Lit(s) = p {
                    while s.contains("//") {
                        *s = s.replace("//", "/");
                    }
                    while s.contains("/./") {
                        *s = s.replace("/./", "/");
                    }
                }
            }
            Value::pattern(pop_after_wild(parts))
        }
        Value::Alternatives(a) => {
            Value::alternatives(a.into_iter().map(|x| normalize(x, anchored)))
        }
        u @ Value::Unknown { .. } => u,
    }
}

/// The parts of one `/`-separated segment of a pattern.
type Segment = Vec<PatPart>;

fn is_lit(seg: &Segment, text: &str) -> bool {
    matches!(seg.as_slice(), [PatPart::Lit(s)] if s == text)
}

fn wild_reason(seg: &Segment) -> Option<Box<str>> {
    seg.iter().find_map(|p| match p {
        PatPart::Wild { reason } => Some(reason.clone()),
        PatPart::Lit(_) => None,
    })
}

/// Resolves the `..` segments that follow the first wildcard of a pattern.
///
/// A literal segment before the `..` is popped. A segment with a wildcard may
/// be one segment or several, so after the pop anything from nothing to a path
/// with a trailing `/` can remain of it: it becomes a wildcard glued to the
/// next segment (`d/*/../x` is `d/*x`). A further `..` right after that pops
/// the segment before the wildcard as well (the wildcard may be used up) and
/// keeps the wildcard, which still covers the paths below the popped segment,
/// since wildcards cross `/`.
fn pop_after_wild(parts: Vec<PatPart>) -> Vec<PatPart> {
    let mut segs: Vec<Segment> = vec![Vec::new()];
    for p in &parts {
        match p {
            PatPart::Wild { .. } => {
                if let Some(last) = segs.last_mut() {
                    last.push(p.clone());
                }
            }
            PatPart::Lit(s) => {
                for (i, piece) in s.split('/').enumerate() {
                    if i > 0 {
                        segs.push(Vec::new());
                    }
                    if !piece.is_empty()
                        && let Some(last) = segs.last_mut()
                    {
                        last.push(PatPart::Lit(piece.to_owned()));
                    }
                }
            }
        }
    }
    let Some(first) = segs.iter().position(|s| wild_reason(s).is_some()) else {
        return parts;
    };
    if !segs.iter().skip(first + 1).any(|s| is_lit(s, "..")) {
        return parts;
    }
    let absolute = matches!(parts.first(), Some(PatPart::Lit(s)) if s.starts_with('/'));
    let trailing = matches!(parts.last(), Some(PatPart::Lit(s)) if s.ends_with('/'));
    enum Entry {
        Seg(Segment),
        /// What remains of a popped wildcard segment: nothing, or a path ending
        /// in `/`.
        Rest(Box<str>),
    }
    let mut stack: Vec<Entry> = Vec::new();
    for seg in segs {
        if seg.is_empty() || is_lit(&seg, ".") {
            continue;
        }
        if !is_lit(&seg, "..") {
            stack.push(Entry::Seg(seg));
            continue;
        }
        match stack.pop() {
            None => {
                if !absolute {
                    stack.push(Entry::Seg(seg));
                }
            }
            Some(Entry::Seg(top)) if is_lit(&top, "..") => {
                stack.push(Entry::Seg(top));
                stack.push(Entry::Seg(seg));
            }
            Some(Entry::Seg(top)) => {
                if let Some(reason) = wild_reason(&top) {
                    stack.push(Entry::Rest(reason));
                }
            }
            Some(Entry::Rest(reason)) => {
                match stack.pop() {
                    None => {
                        if !absolute {
                            stack.push(Entry::Seg(seg));
                        }
                    }
                    Some(Entry::Seg(below)) if is_lit(&below, "..") => {
                        stack.push(Entry::Seg(below));
                        stack.push(Entry::Seg(seg));
                    }
                    Some(_) => {}
                }
                stack.push(Entry::Rest(reason));
            }
        }
    }
    let mut out: Vec<PatPart> = Vec::new();
    if absolute {
        out.push(PatPart::Lit("/".into()));
    }
    // Whether the next entry follows without a `/` (the start, or after a glued
    // wildcard).
    let mut glue = true;
    for e in stack {
        if !glue {
            out.push(PatPart::Lit("/".into()));
        }
        match e {
            Entry::Seg(s) => {
                out.extend(s);
                glue = false;
            }
            Entry::Rest(reason) => {
                out.push(PatPart::Wild { reason });
                glue = true;
            }
        }
    }
    if trailing && !glue {
        out.push(PatPart::Lit("/".into()));
    }
    out
}

/// The directory part of a path value: everything before the last `/` (`/` for
/// a top-level absolute path; `""` when anchored and `"."` otherwise if there
/// is no `/`). For a pattern, the last literal must contain a `/`. `None` for
/// unknowns, alternatives and patterns ending in a wildcard. Does not
/// normalize; see [`parent`] for anchored paths.
pub fn dirname(v: Value, anchored: bool) -> Option<Value> {
    match v {
        Value::Str(s) => Some(Value::Str(match s.rfind('/') {
            Some(0) => "/".into(),
            Some(i) => s[..i].into(),
            None if anchored => String::new(),
            None => ".".into(),
        })),
        Value::Pattern(mut parts) => {
            let Some(PatPart::Lit(last)) = parts.last_mut() else {
                return None;
            };
            let i = last.rfind('/')?;
            last.truncate(i);
            Some(Value::pattern(parts))
        }
        _ => None,
    }
}

/// The parent of an anchored path value, normalized: the anchor's parent (`..`)
/// for the anchor itself, `../..` for `..`, else [`dirname`].
pub fn parent(v: Value) -> Option<Value> {
    let v = normalize(v, true);
    Some(match &v {
        Value::Str(s) if s.is_empty() => Value::str(".."),
        Value::Str(s) if s == ".." || s.ends_with("/..") => Value::Str(format!("{s}/..")),
        _ => dirname(v, true)?,
    })
}

/// The last segment of a path value (trailing `/` ignored). `None` when it is
/// not known, is `..`, or the path is empty.
pub fn basename(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => {
            let t = s.trim_end_matches('/');
            if t.is_empty() || t == ".." || t.ends_with("/..") {
                return None;
            }
            t.rsplit('/').next().map(str::to_owned)
        }
        Value::Pattern(p) => match p.last()? {
            PatPart::Lit(l) => {
                let t = l.trim_end_matches('/');
                let i = t.rfind('/')?;
                Some(t[i + 1..].to_owned())
            }
            PatPart::Wild { .. } => None,
        },
        _ => None,
    }
}

/// The extension of one path segment, with the dot (`.json`); `""` for none or
/// dotfiles.
fn ext_of(seg: &str) -> String {
    match seg.rfind('.') {
        Some(i) if i > 0 => seg[i..].to_owned(),
        _ => String::new(),
    }
}

/// The extension of a path value's last segment (`.json`, `""` for none). For a
/// pattern the last literal must hold the dot. Does not normalize.
pub fn extname(v: &Value) -> Option<String> {
    match v {
        Value::Str(_) => Some(ext_of(&basename(v)?)),
        Value::Pattern(p) => match p.last()? {
            PatPart::Lit(l) => Some(match l.rfind('/') {
                Some(i) => ext_of(&l[i + 1..]),
                None => l[l.rfind('.')?..].to_owned(),
            }),
            PatPart::Wild { .. } => None,
        },
        _ => None,
    }
}

/// The relative path from `from` to `to` (both known, both absolute or both
/// relative). `None` if they differ in absoluteness or `from` climbs out with
/// `..`.
pub fn relative(from: &str, to: &str) -> Option<String> {
    let (f, t) = (normalize_str(from), normalize_str(to));
    if f.starts_with('/') != t.starts_with('/') {
        return None;
    }
    let fs: Vec<&str> = f.split('/').filter(|s| !s.is_empty()).collect();
    let ts: Vec<&str> = t.split('/').filter(|s| !s.is_empty()).collect();
    let common = fs.iter().zip(&ts).take_while(|(a, b)| a == b).count();
    if fs[common..].contains(&"..") {
        return None;
    }
    let mut out: Vec<&str> = vec![".."; fs.len() - common];
    out.extend(&ts[common..]);
    Some(out.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::str(x)
    }

    #[test]
    fn dotdot_after_a_wildcard() {
        let n = |parts: &[Option<&str>]| normalize(pat(parts), true).to_specifier().to_string();
        // A literal segment after the wildcard is popped as usual.
        assert_eq!(
            n(&[
                Some("fixtures/"),
                None,
                Some("/.vercel/output/../builders/x")
            ]),
            "fixtures/*/.vercel/builders/x"
        );
        // The wildcard segment itself: one segment or several.
        assert_eq!(
            n(&[Some("fx/"), None, Some("/../sib.json")]),
            "fx/*sib.json"
        );
        assert_eq!(
            n(&[Some("fx/pre-"), None, Some("/../sib.json")]),
            "fx/*sib.json"
        );
        // Twice: the segment before the wildcard may go too.
        assert_eq!(n(&[Some("fx/"), None, Some("/../../up.json")]), "*up.json");
        assert_eq!(
            n(&[Some("a/fx/"), None, Some("/../../up.json")]),
            "a/*up.json"
        );
        // Nothing to pop into: the parent of the anchor.
        assert_eq!(
            n(&[Some("fx/"), None, Some("/../../../up.json")]),
            "../*up.json"
        );
        // A trailing `..` leaves the wildcard.
        assert_eq!(n(&[Some("fx/"), None, Some("/..")]), "fx/*");
        // No `..` after the wildcard: unchanged.
        assert_eq!(n(&[Some("a/../fx/"), None, Some("/x.json")]), "fx/*/x.json");
    }
    fn pat(parts: &[Option<&str>]) -> Value {
        Value::pattern(parts.iter().map(|p| match p {
            Some(l) => PatPart::Lit((*l).into()),
            None => PatPart::Wild { reason: "x".into() },
        }))
    }

    #[test]
    fn join_and_normalize() {
        assert_eq!(
            normalize(join([s("a"), s(""), s("../b"), s("./c")]), false),
            s("b/c")
        );
        assert_eq!(normalize(join([s("/a"), s("..")]), false), s("/"));
        assert_eq!(normalize(join([s("")]), false), s("."));
        assert_eq!(normalize(join([s(""), s("x")]), true), s("x"));
        assert_eq!(normalize(s("/x/../y"), true), s("y"));
        assert_eq!(normalize(s("../../a"), false), s("../../a"));
        assert_eq!(
            normalize(join([s("a/./b/"), pat(&[None, Some("//c/./d")])]), true),
            pat(&[Some("a/b/"), None, Some("/c/d")])
        );
        assert_eq!(
            normalize(join_reset([s("a"), s("/etc"), s("x")]), false),
            s("/etc/x")
        );
        assert_eq!(normalize(join_reset([s("a"), s("x")]), false), s("a/x"));
        assert!(starts_absolute(&pat(&[Some("/a/"), None])));
    }

    #[test]
    fn dirs_and_names() {
        assert_eq!(dirname(s("/a"), false), Some(s("/")));
        assert_eq!(dirname(s("a/b"), false), Some(s("a")));
        assert_eq!(dirname(s("a"), false), Some(s(".")));
        assert_eq!(dirname(s("a"), true), Some(s("")));
        assert_eq!(
            dirname(pat(&[Some("x/"), None, Some("/y")]), true),
            Some(pat(&[Some("x/"), None]))
        );
        assert_eq!(dirname(pat(&[Some("x/"), None]), true), None);
        assert_eq!(parent(s("")), Some(s("..")));
        assert_eq!(parent(s("..")), Some(s("../..")));
        assert_eq!(parent(s("a/b/")), Some(s("a")));
        assert_eq!(basename(&s("a/b.json/")), Some("b.json".into()));
        assert_eq!(basename(&s("..")), None);
        assert_eq!(
            basename(&pat(&[None, Some("/c.txt")])),
            Some("c.txt".into())
        );
        assert_eq!(extname(&s("a/b.tar.gz")), Some(".gz".into()));
        assert_eq!(extname(&s("a/.env")), Some(String::new()));
        assert_eq!(
            extname(&pat(&[Some("a/"), None, Some(".json")])),
            Some(".json".into())
        );
        assert_eq!(extname(&pat(&[Some("a/"), None])), None);
        assert_eq!(relative("/a/b", "/a/c/d"), Some("../c/d".into()));
        assert_eq!(relative("a", "/a"), None);
    }
}
