//! Origin marks: where an unknown part of a value comes from.
//!
//! The value domain ([`turborepo_trace_core::eval::Value`]) records for every
//! unknown part a reason string. The value-flow engine ([`crate::flow`])
//! prefixes that reason with a mark, `@<tag>[<payload>] <source text>`, so the
//! origin survives concatenation, joins and path operations without changing
//! the shared value types. When a site cannot be lowered to a reference,
//! [`combine`] turns the marks of the whole unknown part into the
//! [`turborepo_trace_core::Origin`] of its uncertainty record plus a payload
//! for the record's detail (the environment variable, the parameter, the
//! imported binding).
//!
//! Marks are internal: [`strip`] removes them before a reason reaches a record
//! or a specifier. Source text never contains a mark, because [`clean`]
//! replaces `@` in every text the engine turns into a reason.
//!
//! | tag | meaning | [`Origin`] |
//! |---|---|---|
//! | `param` | a parameter bound by callers the file cannot see; payload `<function>#<index>` | `Param` |
//! | `env` | `process.env.X`; payload `X` | `Env` |
//! | `argv` | `process.argv` | `Argv` |
//! | `import` | a binding imported from another module; payload `<specifier>#<name>` | `Imported` |
//! | `call` | the result of a call that cannot be evaluated; payload the callee | `Call` |
//! | `data` | runtime data (file contents, `JSON.parse`, network) | `Data` |
//! | `unknown` | anything else | `Unknown` |
//! | `listing` | an entry name of a directory listing; transparent (the directory decides) | |
//! | `ref` | follows a `call` mark: the index of the call in the value-flow engine's table of imported calls with evaluated arguments ([`crate::flow::env`]); transparent | |

use turborepo_trace_core::Origin;

/// The tag of an origin mark.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tag {
    Param,
    Env,
    Argv,
    Import,
    Call,
    Data,
    Unknown,
    Listing,
    Ref,
}

const TAGS: [(Tag, &str); 9] = [
    (Tag::Param, "param"),
    (Tag::Env, "env"),
    (Tag::Argv, "argv"),
    (Tag::Import, "import"),
    (Tag::Call, "call"),
    (Tag::Data, "data"),
    (Tag::Unknown, "unknown"),
    (Tag::Listing, "listing"),
    (Tag::Ref, "ref"),
];

impl Tag {
    fn name(self) -> &'static str {
        TAGS.iter()
            .find(|(t, _)| *t == self)
            .map_or("unknown", |(_, n)| n)
    }

    /// What the payload names, for record details.
    fn label(self) -> &'static str {
        match self {
            Tag::Param => "parameter",
            Tag::Env => "env",
            Tag::Argv => "argv",
            Tag::Import => "imported",
            Tag::Call => "call",
            Tag::Data => "data",
            Tag::Unknown | Tag::Listing | Tag::Ref => "",
        }
    }
}

/// Maximum length of a marked reason's text and payload.
const MAX_TEXT: usize = 120;

fn cap(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut n = max;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

/// Source text as reason text: whitespace collapsed, capped, and without `@`
/// (so it can never look like a mark).
pub(crate) fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT));
    for w in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in w.chars() {
            out.push(if c == '@' { '\u{FF20}' } else { c });
        }
        if out.len() > MAX_TEXT {
            break;
        }
    }
    if out.len() > MAX_TEXT {
        let n = cap(&out, MAX_TEXT).len();
        out.truncate(n);
        out.push('…');
    }
    out
}

/// A marked reason: `@<tag>[<payload>] <text>`. `text` must come from
/// [`clean`].
pub(crate) fn mark(tag: Tag, payload: &str, text: &str) -> Box<str> {
    let payload: String = cap(payload, MAX_TEXT)
        .chars()
        .map(|c| match c {
            ']' => ')',
            '@' => '\u{FF20}',
            c if c.is_whitespace() => '_',
            c => c,
        })
        .collect();
    format!("@{}[{payload}] {text}", tag.name()).into()
}

/// The marks in `reason` (a single reason or several merged ones), in order,
/// and whether the reason starts with a mark.
fn marks(reason: &str) -> (Vec<(Tag, &str)>, bool) {
    let mut out = Vec::new();
    let mut leading = false;
    let bytes = reason.as_bytes();
    let mut i = 0;
    while let Some(off) = memchr::memchr(b'@', &bytes[i..]) {
        let at = i + off;
        i = at + 1;
        if at > 0 && bytes[at - 1] != b' ' {
            continue;
        }
        let rest = &reason[at + 1..];
        let Some((tag, name)) = TAGS.iter().find(|(_, n)| {
            rest.len() > n.len() && rest.starts_with(n) && rest.as_bytes()[n.len()] == b'['
        }) else {
            continue;
        };
        let body = &rest[name.len() + 1..];
        let Some(close) = body.find(']') else {
            continue;
        };
        if at == 0 {
            leading = true;
        }
        out.push((*tag, &body[..close]));
        i = at + 1 + name.len() + 1 + close + 1;
    }
    (out, leading)
}

/// The marks of `reason` as `(tag, payload)` when the reason starts with a
/// mark, and the text after the last mark.
pub(crate) fn parse(reason: &str) -> Option<(Vec<(Tag, &str)>, &str)> {
    let (ms, leading) = marks(reason);
    if !leading || ms.is_empty() {
        return None;
    }
    let mut rest = reason;
    while let Some((a, b)) = find_mark(rest) {
        if a != 0 {
            break;
        }
        rest = rest[b..].trim_start_matches(' ');
    }
    Some((ms, rest))
}

/// Whether `reason` carries a mark with `tag`.
pub(crate) fn has_tag(reason: &str, tag: Tag) -> bool {
    marks(reason).0.iter().any(|(t, _)| *t == tag)
}

/// The marks of `reason` re-applied to `text`: the reason of a value derived
/// from the one `reason` belongs to (a property of it, the result of a method
/// on it). Unmarked reasons derive an `unknown` mark.
pub(crate) fn derive(reason: &str, text: &str) -> Box<str> {
    let (ms, leading) = marks(reason);
    if ms.is_empty() || !leading {
        return mark(Tag::Unknown, "", text);
    }
    let mut out = String::new();
    let mut seen: Vec<(Tag, &str)> = Vec::new();
    for (t, p) in ms {
        if seen.contains(&(t, p)) || seen.len() >= 4 {
            continue;
        }
        seen.push((t, p));
        out.push_str(&format!("@{}[{p}] ", t.name()));
    }
    out.push_str(text);
    out.into()
}

/// `reason` without its marks, for records and specifiers.
pub(crate) fn strip(reason: &str) -> String {
    if !reason.contains('@') {
        return reason.to_owned();
    }
    let mut out = String::with_capacity(reason.len());
    let mut rest = reason;
    while let Some((a, b)) = find_mark(rest) {
        out.push_str(&rest[..a]);
        rest = rest[b..].trim_start_matches(' ');
    }
    out.push_str(rest);
    out
}

/// Byte range of the first mark in `reason`.
fn find_mark(reason: &str) -> Option<(usize, usize)> {
    let bytes = reason.as_bytes();
    let mut i = 0;
    while let Some(off) = memchr::memchr(b'@', &bytes[i..]) {
        let at = i + off;
        i = at + 1;
        if at > 0 && bytes[at - 1] != b' ' {
            continue;
        }
        let rest = &reason[at + 1..];
        let Some((_, name)) = TAGS.iter().find(|(_, n)| {
            rest.len() > n.len() && rest.starts_with(n) && rest.as_bytes()[n.len()] == b'['
        }) else {
            continue;
        };
        let body = &rest[name.len() + 1..];
        if let Some(close) = body.find(']') {
            return Some((at, at + 1 + name.len() + 1 + close + 1));
        }
    }
    None
}

/// The origin of an unknown value whose unknown parts have `reasons`, and a
/// payload for the record detail (`env TMP_FOLDER`, `parameter load#0`).
///
/// The classes the policy may ignore (`Param`, `Env`, `Argv`) are only reported
/// when every unknown part has one of them; a part of any other origin wins, in
/// the order `Unknown`, `Data`, `Call`, `Imported`. Unmarked reasons (values
/// the token evaluator produced) are `Unknown`, except the evaluator's own "one
/// of N alternatives", which stands for known strings.
pub(crate) fn combine<'a>(reasons: impl IntoIterator<Item = &'a str>) -> (Origin, String) {
    let mut all: Vec<(Tag, &str)> = Vec::new();
    for r in reasons {
        let (ms, leading) = marks(r);
        if !leading && !r.starts_with("one of ") {
            all.push((Tag::Unknown, ""));
        }
        all.extend(ms);
    }
    all.retain(|(t, _)| !matches!(t, Tag::Listing | Tag::Ref));
    let pick = [Tag::Unknown, Tag::Data, Tag::Call, Tag::Import]
        .into_iter()
        .find_map(|t| all.iter().find(|(x, _)| *x == t))
        .or(all.first());
    let Some((tag, payload)) = pick else {
        return (Origin::Unknown, String::new());
    };
    let origin = match tag {
        Tag::Param => Origin::Param,
        Tag::Env => Origin::Env,
        Tag::Argv => Origin::Argv,
        Tag::Import => Origin::Imported,
        Tag::Call => Origin::Call,
        Tag::Data => Origin::Data,
        Tag::Unknown | Tag::Listing | Tag::Ref => Origin::Unknown,
    };
    let payload = if payload.is_empty() || tag.label().is_empty() {
        String::new()
    } else {
        format!("{} {payload}", tag.label())
    };
    (origin, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_round_trip() {
        let r = mark(Tag::Env, "TMP_FOLDER", "process.env.TMP_FOLDER");
        assert_eq!(&*r, "@env[TMP_FOLDER] process.env.TMP_FOLDER");
        assert_eq!(strip(&r), "process.env.TMP_FOLDER");
        assert_eq!(combine([&*r]), (Origin::Env, "env TMP_FOLDER".to_owned()));
        assert!(has_tag(&r, Tag::Env));
        assert!(!has_tag(&r, Tag::Param));
        // Merged reasons keep every mark.
        let merged = format!("{r} {}", mark(Tag::Param, "f#0", "name"));
        assert_eq!(strip(&merged), "process.env.TMP_FOLDER name");
        assert_eq!(combine([merged.as_str()]).0, Origin::Env);
    }

    #[test]
    fn combine_rules() {
        let p = mark(Tag::Param, "load#0", "dir");
        let d = mark(Tag::Data, "", "JSON.parse(x).file");
        let l = mark(Tag::Listing, "", "entry");
        let c = mark(Tag::Call, "./x#get", "get()");
        assert_eq!(combine([&*p]).0, Origin::Param);
        assert_eq!(combine([&*p, &*l]).0, Origin::Param);
        assert_eq!(combine([&*p, &*d]).0, Origin::Data);
        assert_eq!(combine([&*c, &*p]).0, Origin::Call);
        assert_eq!(combine([&*d, &*c]).0, Origin::Data);
        // An unmarked part is unknown and wins.
        assert_eq!(combine([&*p, "foo()"]).0, Origin::Unknown);
        assert_eq!(combine(["foo()"]), (Origin::Unknown, String::new()));
        assert_eq!(combine([&*p, "one of 9 alternatives"]).0, Origin::Param);
        assert_eq!(combine([&*l]).0, Origin::Unknown);
    }

    #[test]
    fn source_text_cannot_forge_a_mark() {
        let text = clean("f('x @param[a] y')");
        assert!(!text.contains('@'));
        let r = mark(Tag::Unknown, "", &text);
        assert_eq!(combine([&*r]).0, Origin::Unknown);
        assert_eq!(&*derive(&p(), "x.y"), "@param[f#0] x.y");
        assert_eq!(&*derive("plain", "x.y"), "@unknown[] x.y");
        assert_eq!(strip("no marks @ here"), "no marks @ here");
    }

    fn p() -> Box<str> {
        mark(Tag::Param, "f#0", "x")
    }
}
