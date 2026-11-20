use std::borrow::Cow;

use crate::{Part, Specifier};

/// Maximum number of alternatives a [`Value`] keeps. Larger sets collapse into
/// a wildcard.
pub const MAX_ALTERNATIVES: usize = 8;

/// One piece of a [`Value::Pattern`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PatPart {
    /// Known text.
    Lit(String),
    /// Unknown text; `reason` records the source expression or why it is
    /// unknown.
    Wild {
        /// Source text or explanation.
        reason: Box<str>,
    },
}

/// The result of partially evaluating an expression.
///
/// Values are kept in normal form by the constructors and operations: patterns
/// never contain adjacent literals or adjacent wildcards, a pattern that is a
/// single literal becomes [`Value::Str`], a pattern that is a single wildcard
/// becomes [`Value::Unknown`], and alternatives are flattened, deduplicated and
/// hold at least two values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    /// A fully known string.
    Str(String),
    /// A partially known string, such as `["./locales/", Wild, ".json"]`.
    Pattern(Vec<PatPart>),
    /// One of several values (at most [`MAX_ALTERNATIVES`]).
    Alternatives(Vec<Value>),
    /// Nothing is known.
    Unknown {
        /// Source text or explanation.
        reason: Box<str>,
    },
}

impl Value {
    /// A known string.
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// An unknown value.
    pub fn unknown(reason: impl Into<Box<str>>) -> Value {
        Value::Unknown {
            reason: reason.into(),
        }
    }

    /// The known string, if fully known.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Whether the value is fully known.
    pub fn is_known(&self) -> bool {
        matches!(self, Value::Str(_))
    }

    /// Builds a pattern from parts and normalizes it.
    pub fn pattern(parts: impl IntoIterator<Item = PatPart>) -> Value {
        let mut out: Vec<PatPart> = Vec::new();
        for p in parts {
            push_part(&mut out, p);
        }
        match out.as_slice() {
            [] => Value::Str(String::new()),
            [PatPart::Lit(_)] => match out.pop() {
                Some(PatPart::Lit(s)) => Value::Str(s),
                _ => unreachable!("matched a single literal"),
            },
            [PatPart::Wild { .. }] => match out.pop() {
                Some(PatPart::Wild { reason }) => Value::Unknown { reason },
                _ => unreachable!("matched a single wildcard"),
            },
            _ => Value::Pattern(out),
        }
    }

    /// One of `values`. Flattens nested alternatives and removes duplicates
    /// (keeping first occurrence order). More than [`MAX_ALTERNATIVES`]
    /// distinct values collapse into a single wildcard, which
    /// over-approximates. A single value is returned as-is.
    pub fn alternatives(values: impl IntoIterator<Item = Value>) -> Value {
        let mut out: Vec<Value> = Vec::new();
        for v in values {
            match v {
                Value::Alternatives(inner) => {
                    for i in inner {
                        if !out.contains(&i) {
                            out.push(i);
                        }
                    }
                }
                v => {
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
        match out.len() {
            0 => Value::unknown("no alternatives"),
            1 => out
                .pop()
                .unwrap_or_else(|| Value::unknown("no alternatives")),
            n if n > MAX_ALTERNATIVES => Value::Alternatives(out).collapse(),
            _ => Value::Alternatives(out),
        }
    }

    /// Concatenates two values, like `a + b` on strings.
    ///
    /// Unknown operands become wildcards, so `x + ".json"` is the pattern `[*,
    /// ".json"]`. Alternatives distribute over concatenation (`(a | b) + c
    /// = ac | bc`), capped by [`MAX_ALTERNATIVES`].
    pub fn concat(self, other: Value) -> Value {
        match (self, other) {
            (Value::Str(mut a), Value::Str(b)) => {
                a.push_str(&b);
                Value::Str(a)
            }
            (Value::Alternatives(alts), b) => {
                if alts.len() * b.alt_count() > MAX_ALTERNATIVES {
                    return Value::Alternatives(alts).collapse().concat(b);
                }
                Value::alternatives(alts.into_iter().map(|a| a.concat(b.clone())))
            }
            (a, Value::Alternatives(alts)) => {
                if alts.len() * a.alt_count() > MAX_ALTERNATIVES {
                    return a.concat(Value::Alternatives(alts).collapse());
                }
                Value::alternatives(alts.into_iter().map(|b| a.clone().concat(b)))
            }
            (a, b) => Value::pattern(a.into_parts().into_iter().chain(b.into_parts())),
        }
    }

    /// Concatenates all values in order. An empty iterator yields the empty
    /// string.
    pub fn concat_all(values: impl IntoIterator<Item = Value>) -> Value {
        values
            .into_iter()
            .fold(Value::Str(String::new()), Value::concat)
    }

    /// Joins values with `sep` between them, like `[a, b].join(sep)`.
    pub fn join(values: impl IntoIterator<Item = Value>, sep: &str) -> Value {
        let mut acc: Option<Value> = None;
        for v in values {
            acc = Some(match acc {
                None => v,
                Some(a) => a.concat(Value::str(sep)).concat(v),
            });
        }
        acc.unwrap_or_else(|| Value::Str(String::new()))
    }

    fn alt_count(&self) -> usize {
        match self {
            Value::Alternatives(a) => a.len(),
            _ => 1,
        }
    }

    /// Replaces alternatives with a single pattern that over-approximates them:
    /// when all alternatives are known strings, their common prefix and
    /// suffix around a wildcard; otherwise a bare wildcard.
    fn collapse(self) -> Value {
        let Value::Alternatives(alts) = self else {
            return self;
        };
        let reason = format!("one of {} alternatives", alts.len());
        let strs: Option<Vec<&str>> = alts.iter().map(Value::as_str).collect();
        let Some(strs) = strs.filter(|s| !s.is_empty()) else {
            return Value::unknown(reason);
        };
        let first = strs[0].as_bytes();
        let min_len = strs.iter().map(|s| s.len()).min().unwrap_or(0);
        let mut pre = strs
            .iter()
            .map(|s| {
                first
                    .iter()
                    .zip(s.as_bytes())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
            .min()
            .unwrap_or(0);
        while !strs[0].is_char_boundary(pre) {
            pre -= 1;
        }
        let max_suf = min_len - pre;
        let mut suf = strs
            .iter()
            .map(|s| {
                first
                    .iter()
                    .rev()
                    .zip(s.as_bytes().iter().rev())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
            .min()
            .unwrap_or(0)
            .min(max_suf);
        while !strs[0].is_char_boundary(strs[0].len() - suf) {
            suf -= 1;
        }
        let s0 = strs[0];
        Value::pattern([
            PatPart::Lit(s0[..pre].to_string()),
            PatPart::Wild {
                reason: reason.into(),
            },
            PatPart::Lit(s0[s0.len() - suf..].to_string()),
        ])
    }

    /// Pattern parts of a non-alternative value.
    fn into_parts(self) -> Vec<PatPart> {
        match self {
            Value::Str(s) => vec![PatPart::Lit(s)],
            Value::Pattern(p) => p,
            Value::Unknown { reason } => vec![PatPart::Wild { reason }],
            Value::Alternatives(a) => vec![PatPart::Wild {
                reason: format!("one of {} alternatives", a.len()).into(),
            }],
        }
    }

    /// Converts to a core [`Specifier`].
    pub fn to_specifier(&self) -> Specifier<'static> {
        match self {
            Value::Str(s) => Specifier::Literal(Cow::Owned(s.clone())),
            Value::Pattern(parts) => Specifier::Pattern(
                parts
                    .iter()
                    .map(|p| match p {
                        PatPart::Lit(s) => Part::Lit(Cow::Owned(s.clone())),
                        PatPart::Wild { reason } => Part::Wild {
                            reason: Cow::Owned(reason.to_string()),
                        },
                    })
                    .collect(),
            ),
            Value::Alternatives(alts) => {
                Specifier::Alternatives(alts.iter().map(Value::to_specifier).collect())
            }
            Value::Unknown { .. } => Specifier::Unknown,
        }
    }

    /// Converts from a core [`Specifier`] (normalizing it).
    pub fn from_specifier(spec: &Specifier<'_>) -> Value {
        match spec {
            Specifier::Literal(s) => Value::Str(s.to_string()),
            Specifier::Pattern(parts) => Value::pattern(parts.iter().map(PatPart::from)),
            Specifier::Alternatives(alts) => {
                Value::alternatives(alts.iter().map(Value::from_specifier))
            }
            Specifier::Unknown => Value::unknown("unknown"),
        }
    }
}

impl From<&Part<'_>> for PatPart {
    fn from(p: &Part<'_>) -> Self {
        match p {
            Part::Lit(s) => PatPart::Lit(s.to_string()),
            Part::Wild { reason } => PatPart::Wild {
                reason: reason.as_ref().into(),
            },
        }
    }
}

fn push_part(out: &mut Vec<PatPart>, p: PatPart) {
    match (out.last_mut(), p) {
        (_, PatPart::Lit(s)) if s.is_empty() => {}
        (Some(PatPart::Lit(a)), PatPart::Lit(b)) => a.push_str(&b),
        (Some(PatPart::Wild { reason: a }), PatPart::Wild { reason: b }) => {
            *a = format!("{a} {b}").into();
        }
        (_, p) => out.push(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> PatPart {
        PatPart::Lit(s.into())
    }
    fn wild(r: &str) -> PatPart {
        PatPart::Wild { reason: r.into() }
    }

    #[test]
    fn concat_strings() {
        assert_eq!(Value::str("a").concat(Value::str("b")), Value::str("ab"));
        assert_eq!(
            Value::concat_all([Value::str("./"), Value::str("x"), Value::str(".js")]),
            Value::str("./x.js")
        );
        assert_eq!(Value::concat_all([]), Value::str(""));
    }

    #[test]
    fn concat_with_unknown_makes_pattern() {
        let v = Value::str("./locales/")
            .concat(Value::unknown("lang"))
            .concat(Value::str(".json"));
        assert_eq!(
            v,
            Value::Pattern(vec![lit("./locales/"), wild("lang"), lit(".json")])
        );
        assert_eq!(v.to_specifier().to_string(), "./locales/*.json");
        let lead = Value::unknown("x").concat(Value::str(".js"));
        assert_eq!(lead, Value::Pattern(vec![wild("x"), lit(".js")]));
    }

    #[test]
    fn pattern_normalization() {
        assert_eq!(Value::pattern([lit("a"), lit("b")]), Value::str("ab"));
        assert_eq!(
            Value::pattern([wild("x"), wild("y")]),
            Value::unknown("x y")
        );
        assert_eq!(Value::pattern([]), Value::str(""));
        assert_eq!(
            Value::pattern([lit(""), wild("x"), lit(""), lit("a")]),
            Value::Pattern(vec![wild("x"), lit("a")])
        );
        // Pattern + pattern merges the boundary literals.
        let a = Value::pattern([lit("a/"), wild("x"), lit("/")]);
        let b = Value::pattern([lit("b"), wild("y")]);
        assert_eq!(
            a.concat(b),
            Value::Pattern(vec![lit("a/"), wild("x"), lit("/b"), wild("y")])
        );
    }

    #[test]
    fn alternatives_distribute_and_cap() {
        let v = Value::alternatives([Value::str("a"), Value::str("b")]).concat(Value::str(".js"));
        assert_eq!(
            v,
            Value::Alternatives(vec![Value::str("a.js"), Value::str("b.js")])
        );
        let v = Value::str("./").concat(Value::alternatives([Value::str("x"), Value::str("y")]));
        assert_eq!(v.to_specifier().to_string(), "./x | ./y");

        // Dedup and flatten.
        let v = Value::alternatives([
            Value::str("a"),
            Value::alternatives([Value::str("a"), Value::str("b")]),
        ]);
        assert_eq!(
            v,
            Value::Alternatives(vec![Value::str("a"), Value::str("b")])
        );
        assert_eq!(Value::alternatives([Value::str("a")]), Value::str("a"));

        // Over the cap: collapse to common prefix/suffix around a wildcard.
        let many = Value::alternatives((10..30).map(|i| Value::str(format!("./l/{i}.json"))));
        assert_eq!(
            many,
            Value::Pattern(vec![
                lit("./l/"),
                wild("one of 20 alternatives"),
                lit(".json")
            ])
        );
        let mixed = Value::alternatives(
            (0..10)
                .map(|i| Value::str(i.to_string()))
                .chain([Value::unknown("x")]),
        );
        assert!(matches!(mixed, Value::Unknown { .. }));

        // Product over the cap collapses instead of exploding.
        let a = Value::alternatives((0..4).map(|i| Value::str(format!("a{i}"))));
        let b = Value::alternatives((0..4).map(|i| Value::str(format!("b{i}"))));
        let v = Value::str("./").concat(a).concat(b);
        let Value::Alternatives(alts) = v else {
            panic!("{v:?}");
        };
        assert!(alts.len() <= MAX_ALTERNATIVES);
        for a in alts {
            let Value::Pattern(p) = a else {
                panic!("{a:?}")
            };
            assert_eq!(p[0], lit("./a"));
        }
    }

    #[test]
    fn join_values() {
        let v = Value::join([Value::str("a"), Value::unknown("x"), Value::str("c")], "/");
        assert_eq!(v, Value::Pattern(vec![lit("a/"), wild("x"), lit("/c")]));
        assert_eq!(Value::join([], "/"), Value::str(""));
        assert_eq!(Value::join([Value::str("only")], "/"), Value::str("only"));
    }

    #[test]
    fn specifier_round_trip() {
        let v = Value::Pattern(vec![lit("./a/"), wild("x"), lit(".txt")]);
        assert_eq!(Value::from_specifier(&v.to_specifier()), v);
        let s = Value::str("x");
        assert_eq!(Value::from_specifier(&s.to_specifier()), s);
        assert!(matches!(
            Value::from_specifier(&Specifier::Unknown),
            Value::Unknown { .. }
        ));
        assert!(!Value::unknown("x").is_known());
        assert_eq!(Value::str("q").as_str(), Some("q"));
    }
}
