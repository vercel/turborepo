//! Anchored path values, shared by the language evaluators.
//!
//! Every language has expressions that denote a path relative to something the
//! static analysis knows: JS `__dirname`, Python `__file__`, Rust
//! `env!("CARGO_MANIFEST_DIR")`, Go `runtime.Caller(0)`. [`Anchored`] is such a
//! value: an optional language-defined anchor `A` plus a [`Value`] relative to
//! it (`""` is the anchor itself). The operations are the language-neutral
//! parts of each evaluator:
//!
//! | Operation | JS | Python | Go | Rust |
//! |---|---|---|---|---|
//! | [`Anchored::concat`] | `__dirname + '/x'`, templates | `+`, f-strings | `+`, `fmt.Sprintf` | `concat!`, `format!` |
//! | [`Anchored::join`] with [`JoinStyle::Append`] | `path.join` | | `filepath.Join` | |
//! | [`Anchored::join`] with [`JoinStyle::Reset`] | | `os.path.join`, `/` | | `Path::join` |
//! | [`Anchored::parent`] | `path.dirname` | `os.path.dirname`, `.parent` | `filepath.Dir` | `Path::parent` |
//! | [`Anchored::alt`] | `a \|\| b`, `?:` | `or`, conditional expressions | | |
//!
//! [`is_trivial`] is the shared rule for which anchored values are worth
//! emitting as assets when they never reach a tracked API (nft's "static child"
//! emission, and the `validWildcard` rule that the anchor directory, its
//! ancestors and the file itself are never assets). The resolution policy for
//! such assets lives in `turborepo_trace_core::resolve_path::assets`
//! (`AssetRules::resolve_anchored_asset`).

use crate::eval::{PatPart, Value, path};

/// How [`Anchored::join`] treats an absolute or anchored right operand.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinStyle {
    /// Concatenate with `/` (Node `path.join`, Go `filepath.Join`): `join("a",
    /// "/b")` is `a/b`. An anchored right operand still replaces the left
    /// side, because its anchor is an absolute path.
    Append,
    /// An absolute or anchored right operand replaces the left side (Python
    /// `os.path.join`, `pathlib /`, Rust `Path::join`).
    Reset,
}

/// A path value relative to an anchor `A`, or a plain string when `anchor` is
/// `None`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Anchored<A> {
    /// The anchor, or `None` for a plain string (a cwd-relative path when used
    /// as one).
    pub anchor: Option<A>,
    /// The value relative to the anchor; `""` is the anchor itself. May still
    /// contain `.` and `..` segments (see [`Anchored::normalized`]).
    pub val: Value,
}

impl<A: Copy + Eq> Anchored<A> {
    /// A plain known string.
    pub fn str(s: impl Into<String>) -> Self {
        Anchored {
            anchor: None,
            val: Value::str(s),
        }
    }

    /// A plain value.
    pub fn plain(val: Value) -> Self {
        Anchored { anchor: None, val }
    }

    /// A plain unknown value.
    pub fn unknown(reason: impl Into<Box<str>>) -> Self {
        Anchored {
            anchor: None,
            val: Value::unknown(reason),
        }
    }

    /// The anchor itself.
    pub fn at(anchor: A) -> Self {
        Anchored {
            anchor: Some(anchor),
            val: Value::str(""),
        }
    }

    /// `rel` relative to `anchor`.
    pub fn rel(anchor: A, val: Value) -> Self {
        Anchored {
            anchor: Some(anchor),
            val,
        }
    }

    /// Whether the value is relative to an anchor.
    pub fn is_anchored(&self) -> bool {
        self.anchor.is_some()
    }

    fn is_empty_plain(&self) -> bool {
        self.anchor.is_none() && self.val.as_str() == Some("")
    }

    /// String concatenation. An anchor only counts at the start: `"" + anchor`
    /// is the anchor, `anchor + "/x"` stays anchored, and an anchored value
    /// after a non-empty prefix becomes a wildcard (an absolute path in the
    /// middle of a string).
    pub fn concat(self, b: Self) -> Self {
        if b.anchor.is_some() {
            if self.is_empty_plain() {
                return b;
            }
            return Anchored {
                anchor: self.anchor,
                val: self.val.concat(Value::unknown("path anchor in the middle")),
            };
        }
        Anchored {
            anchor: self.anchor,
            val: self.val.concat(b.val),
        }
    }

    /// Path join (see [`JoinStyle`]). An anchored right operand always replaces
    /// the left side; an absolute plain one does so with
    /// [`JoinStyle::Reset`]. The result is not normalized.
    pub fn join(self, b: Self, style: JoinStyle) -> Self {
        if b.anchor.is_some() || (style == JoinStyle::Reset && path::starts_absolute(&b.val)) {
            return b;
        }
        if self.val.as_str() == Some("") && self.anchor.is_some() {
            return Anchored {
                anchor: self.anchor,
                val: b.val,
            };
        }
        let val = match style {
            JoinStyle::Append => path::join([self.val, b.val]),
            JoinStyle::Reset => path::join_reset([self.val, b.val]),
        };
        Anchored {
            anchor: self.anchor,
            val,
        }
    }

    /// The parent directory: [`path::parent`] for anchored values (the anchor's
    /// parent is `..`), [`path::dirname`] otherwise. `None` when it cannot
    /// be expressed.
    pub fn parent(self) -> Option<Self> {
        let val = match self.anchor {
            Some(_) => path::parent(self.val)?,
            None => path::dirname(self.val, false)?,
        };
        Some(Anchored {
            anchor: self.anchor,
            val,
        })
    }

    /// One of `self` or `other`: alternatives when both have the same anchor
    /// (or none), else `None` (the caller decides how to represent mixed
    /// anchors).
    pub fn alt(self, other: Self) -> Option<Self> {
        (self.anchor == other.anchor).then(|| Anchored {
            anchor: self.anchor,
            val: Value::alternatives([self.val, other.val]),
        })
    }

    /// The value normalized relative to its anchor ([`path::normalize`];
    /// anchored values drop a leading `/` and use `""` for the anchor
    /// itself).
    pub fn normalized(&self) -> Value {
        path::normalize(self.val.clone(), self.anchor.is_some())
    }
}

/// Whether a normalized anchored value is not worth emitting as an asset: the
/// anchor itself or one of its ancestors (only `.`/`..`/empty segments), the
/// referencing file itself (`file_name`; pass it only for file-directory
/// anchors), a pattern without any literal alphanumeric name, or an unknown
/// value. Alternatives are trivial when every branch is.
///
/// This is nft's rule for "static child" assets and its `validWildcard` rule,
/// used by every language's anchored-asset emission.
pub fn is_trivial(v: &Value, file_name: Option<&str>) -> bool {
    match v {
        Value::Str(s) => {
            s.split('/').all(|seg| matches!(seg, "" | "." | ".."))
                || (file_name.is_some() && Some(s.as_str()) == file_name)
        }
        Value::Pattern(p) => !p.iter().any(|x| match x {
            PatPart::Lit(s) => s.bytes().any(|b| b.is_ascii_alphanumeric()),
            PatPart::Wild { .. } => false,
        }),
        Value::Alternatives(a) => a.iter().all(|x| is_trivial(x, file_name)),
        Value::Unknown { .. } => true,
    }
}

/// Whether a value can be globbed from a static prefix: it is known, or a
/// pattern starting with a literal, or alternatives where some branch is.
/// Patterns starting with a wildcard are refused by the glob expander.
pub fn has_static_prefix(v: &Value) -> bool {
    match v {
        Value::Str(_) => true,
        Value::Pattern(p) => matches!(p.first(), Some(PatPart::Lit(_))),
        Value::Alternatives(a) => a.iter().any(has_static_prefix),
        Value::Unknown { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    enum A {
        Dir,
        Root,
    }

    type V = Anchored<A>;

    #[test]
    fn concat_keeps_a_leading_anchor() {
        let v = V::at(A::Dir).concat(V::str("/data.json"));
        assert_eq!(v, V::rel(A::Dir, Value::str("/data.json")));
        assert_eq!(v.normalized(), Value::str("data.json"));
        let v = V::str("").concat(V::at(A::Root)).concat(V::str("/x"));
        assert_eq!(v.anchor, Some(A::Root));
        let mid = V::str("x").concat(V::at(A::Root));
        assert_eq!(mid.anchor, None);
        assert!(matches!(mid.val, Value::Pattern(_)));
    }

    #[test]
    fn join_styles() {
        let base = V::at(A::Dir);
        assert_eq!(
            base.clone()
                .join(V::str("a"), JoinStyle::Reset)
                .join(V::str("b.txt"), JoinStyle::Reset)
                .normalized(),
            Value::str("a/b.txt")
        );
        assert_eq!(
            base.clone().join(V::str("/etc/x"), JoinStyle::Reset),
            V::str("/etc/x")
        );
        assert_eq!(
            base.clone()
                .join(V::str("/x"), JoinStyle::Append)
                .normalized(),
            Value::str("x")
        );
        assert_eq!(
            V::str("a").join(V::at(A::Root), JoinStyle::Append),
            V::at(A::Root)
        );
    }

    #[test]
    fn parents_and_alternatives() {
        let f = V::rel(A::Dir, Value::str("main.py"));
        assert_eq!(
            f.clone().parent().map(|p| p.normalized()),
            Some(Value::str(""))
        );
        assert_eq!(
            V::at(A::Dir).parent().map(|p| p.val),
            Some(Value::str(".."))
        );
        assert_eq!(V::str("a/b").parent().map(|p| p.val), Some(Value::str("a")));
        let a = V::rel(A::Dir, Value::str("a")).alt(V::rel(A::Dir, Value::str("b")));
        assert!(matches!(a.map(|x| x.val), Some(Value::Alternatives(_))));
        assert!(V::at(A::Dir).alt(V::at(A::Root)).is_none());
    }

    #[test]
    fn triviality() {
        assert!(is_trivial(&Value::str(""), None));
        assert!(is_trivial(&Value::str("../.."), None));
        assert!(is_trivial(&Value::str("main.py"), Some("main.py")));
        assert!(!is_trivial(&Value::str("main.py"), None));
        assert!(!is_trivial(&Value::str("data/x.json"), None));
        let p = Value::pattern([
            PatPart::Lit("-".into()),
            PatPart::Wild { reason: "x".into() },
        ]);
        assert!(is_trivial(&p, None));
        let p = Value::pattern([
            PatPart::Lit("data/".into()),
            PatPart::Wild { reason: "x".into() },
        ]);
        assert!(!is_trivial(&p, None));
        assert!(has_static_prefix(&p));
        assert!(is_trivial(&Value::unknown("x"), None));
    }
}
