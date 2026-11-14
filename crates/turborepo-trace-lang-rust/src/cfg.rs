//! `cfg` predicates: parsing from tokens and three-valued evaluation for a
//! target [`Platform`].
//!
//! Leaves the platform decides (`unix`, `windows`, `target_os`, `target_arch`,
//! `target_env`, `target_family`, `target_pointer_width`, `target_vendor`,
//! `target_endian`, `target_has_atomic`) evaluate to true or false. Everything
//! else (`feature`, `test`, `debug_assertions`, `doc`, `miri`, custom cfgs) is
//! [`Tri::Unknown`]: code behind it is kept and flagged CONDITIONAL.
//! `all`/`any`/`not` use Kleene logic, so `all(windows, feature = "x")` is
//! false on Linux and `any(unix, feature = "x")` is true.
//!
//! An optional enabled-feature set turns `feature = "…"` leaves into known
//! values (used for `[target.'cfg(…)'.dependencies]` keys, where features are
//! not allowed anyway, and by the resolver when it knows a crate's features).

use std::collections::BTreeSet;

use turborepo_trace_core::{
    Platform,
    platform::{Arch, Os},
};

use crate::lexer::{Tok, TokKind, tokenize};

/// A three-valued truth value.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tri {
    /// Known true for the target.
    True,
    /// Known false for the target.
    False,
    /// Depends on something the tracer does not know.
    Unknown,
}

impl std::ops::Not for Tri {
    type Output = Tri;

    /// Negation (unknown stays unknown).
    fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

impl Tri {
    /// Kleene conjunction.
    pub fn and(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::False, _) | (_, Tri::False) => Tri::False,
            (Tri::True, Tri::True) => Tri::True,
            _ => Tri::Unknown,
        }
    }

    /// Kleene disjunction.
    pub fn or(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::True, _) | (_, Tri::True) => Tri::True,
            (Tri::False, Tri::False) => Tri::False,
            _ => Tri::Unknown,
        }
    }

    /// From a known boolean.
    pub fn from_bool(b: bool) -> Tri {
        if b { Tri::True } else { Tri::False }
    }
}

/// What a predicate is evaluated against.
#[derive(Copy, Clone, Debug)]
pub struct CfgEnv<'a> {
    /// Target platform.
    pub platform: Platform,
    /// Enabled features, when known.
    pub features: Option<&'a BTreeSet<String>>,
    /// Every target (`--all-targets`): target-dependent predicates (`unix`,
    /// `windows`, `target_*`, target triples) are unknown, so items and
    /// dependencies behind them are kept as conditional.
    pub all_targets: bool,
}

impl<'a> CfgEnv<'a> {
    /// An environment for `platform` with unknown features.
    pub fn new(platform: Platform) -> CfgEnv<'static> {
        CfgEnv {
            platform,
            features: None,
            all_targets: false,
        }
    }

    /// The same environment, evaluating target predicates as unknown when
    /// `yes`.
    pub fn with_all_targets(mut self, yes: bool) -> CfgEnv<'a> {
        self.all_targets = yes;
        self
    }

    /// Evaluates one leaf `key` or `key = "value"`.
    pub fn leaf(&self, key: &str, value: Option<&str>) -> Tri {
        if self.all_targets && is_target_key(key) {
            return Tri::Unknown;
        }
        if let Some(b) = self.platform.rust_cfg(key, value) {
            return Tri::from_bool(b);
        }
        let p = &self.platform;
        match (key, value) {
            ("feature", Some(f)) => match self.features {
                Some(set) => Tri::from_bool(set.contains(f)),
                None => Tri::Unknown,
            },
            ("target_vendor", Some(v)) => {
                let vendor = match p.os {
                    Os::Darwin => "apple",
                    Os::Windows => "pc",
                    _ => "unknown",
                };
                Tri::from_bool(v == vendor)
            }
            ("target_endian", Some(v)) => match p.arch {
                Arch::S390x => Tri::from_bool(v == "big"),
                Arch::Other(_) => Tri::Unknown,
                _ => Tri::from_bool(v == "little"),
            },
            ("target_has_atomic", Some(v)) => match v {
                "8" | "16" | "32" | "ptr" => Tri::True,
                "64" => match p.arch {
                    Arch::X86_64 | Arch::Aarch64 | Arch::Ppc64 | Arch::S390x | Arch::Riscv64 => {
                        Tri::True
                    }
                    _ => Tri::Unknown,
                },
                _ => Tri::Unknown,
            },
            ("target_abi", Some(v)) => match (p.os, p.arch) {
                (Os::Linux, Arch::Arm) => Tri::from_bool(v == "eabihf"),
                (Os::Linux | Os::Darwin | Os::Windows, Arch::X86_64 | Arch::Aarch64) => {
                    Tri::from_bool(v.is_empty())
                }
                _ => Tri::Unknown,
            },
            _ => Tri::Unknown,
        }
    }
}

/// A parsed predicate (kept for display and re-evaluation with known features).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pred {
    /// `key` or `key = "value"`.
    Leaf(String, Option<String>),
    /// `all(…)`.
    All(Vec<Pred>),
    /// `any(…)`.
    Any(Vec<Pred>),
    /// `not(…)`.
    Not(Box<Pred>),
    /// Something unparsable.
    Invalid,
}

impl Pred {
    /// Evaluates the predicate.
    pub fn eval(&self, env: &CfgEnv<'_>) -> Tri {
        match self {
            Pred::Leaf(k, v) => env.leaf(k, v.as_deref()),
            Pred::All(ps) => ps.iter().fold(Tri::True, |a, p| a.and(p.eval(env))),
            Pred::Any(ps) => ps.iter().fold(Tri::False, |a, p| a.or(p.eval(env))),
            Pred::Not(p) => !p.eval(env),
            Pred::Invalid => Tri::Unknown,
        }
    }

    /// Whether any leaf has key `key`.
    pub fn mentions(&self, key: &str) -> bool {
        match self {
            Pred::Leaf(k, _) => k == key,
            Pred::All(ps) | Pred::Any(ps) => ps.iter().any(|p| p.mentions(key)),
            Pred::Not(p) => p.mentions(key),
            Pred::Invalid => false,
        }
    }
}

impl std::fmt::Display for Pred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = |f: &mut std::fmt::Formatter<'_>, name: &str, ps: &[Pred]| {
            write!(f, "{name}(")?;
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{p}")?;
            }
            f.write_str(")")
        };
        match self {
            Pred::Leaf(k, None) => f.write_str(k),
            Pred::Leaf(k, Some(v)) => write!(f, "{k} = {v:?}"),
            Pred::All(ps) => list(f, "all", ps),
            Pred::Any(ps) => list(f, "any", ps),
            Pred::Not(p) => write!(f, "not({p})"),
            Pred::Invalid => f.write_str("<invalid>"),
        }
    }
}

/// Maximum predicate nesting; deeper input is [`Pred::Invalid`].
const MAX_DEPTH: usize = 32;

/// Parses one predicate from `toks[*i..end]`, advancing `*i` past it.
pub fn parse(toks: &[Tok], src: &[u8], i: &mut usize, end: usize) -> Pred {
    parse_depth(toks, src, i, end, 0)
}

fn parse_depth(toks: &[Tok], src: &[u8], i: &mut usize, end: usize, depth: usize) -> Pred {
    let Some(t) = toks.get(*i).filter(|_| *i < end) else {
        return Pred::Invalid;
    };
    if !matches!(t.kind, TokKind::Ident | TokKind::RawIdent) || depth > MAX_DEPTH {
        *i += 1;
        return Pred::Invalid;
    }
    let name = String::from_utf8_lossy(t.bytes(src)).into_owned();
    *i += 1;
    let next = toks.get(*i).filter(|_| *i < end);
    match next {
        Some(n) if n.is_punct(b'=') => {
            *i += 1;
            let v = toks
                .get(*i)
                .filter(|_| *i < end)
                .and_then(|t| t.text(src))
                .map(|s| s.into_owned());
            if v.is_some() {
                *i += 1;
            }
            match v {
                Some(v) => Pred::Leaf(name, Some(v)),
                None => Pred::Invalid,
            }
        }
        Some(n) if n.is_punct(b'(') && matches!(name.as_str(), "all" | "any" | "not") => {
            *i += 1;
            let mut items = Vec::new();
            loop {
                match toks.get(*i).filter(|_| *i < end) {
                    None => break,
                    Some(t) if t.is_punct(b')') => {
                        *i += 1;
                        break;
                    }
                    Some(t) if t.is_punct(b',') => *i += 1,
                    Some(_) => {
                        let before = *i;
                        items.push(parse_depth(toks, src, i, end, depth + 1));
                        if *i == before {
                            *i += 1;
                        }
                    }
                }
            }
            match name.as_str() {
                "all" => Pred::All(items),
                "any" => Pred::Any(items),
                _ => match <[Pred; 1]>::try_from(items) {
                    Ok([p]) => Pred::Not(Box::new(p)),
                    Err(_) => Pred::Invalid,
                },
            }
        }
        _ => Pred::Leaf(name, None),
    }
}

/// Parses a predicate written as text: `cfg(unix)` or a bare predicate `unix`.
pub fn parse_str(s: &str) -> Pred {
    let src = s.as_bytes();
    let mut toks = Vec::new();
    let mut issues = Vec::new();
    tokenize(src, &mut toks, &mut issues);
    let mut i = 0;
    if toks.len() >= 2 && toks[0].is_ident(src, "cfg") && toks[1].is_punct(b'(') {
        i = 2;
    }
    parse(&toks, src, &mut i, toks.len())
}

/// Evaluates a `[target.<key>.dependencies]` key: a `cfg(…)` expression or a
/// target triple.
pub fn target_key_matches(key: &str, env: &CfgEnv<'_>) -> Tri {
    let k = key.trim();
    if k.starts_with("cfg(") {
        parse_str(k).eval(env)
    } else if env.all_targets {
        Tri::Unknown
    } else {
        Tri::from_bool(k == env.platform.rust_triple())
    }
}

/// Whether a cfg key depends on the compilation target.
fn is_target_key(key: &str) -> bool {
    matches!(key, "unix" | "windows") || key.starts_with("target_")
}

#[cfg(test)]
mod tests {
    use turborepo_trace_core::platform::Libc;

    use super::*;

    fn linux() -> CfgEnv<'static> {
        CfgEnv::new(Platform {
            os: Os::Linux,
            arch: Arch::X86_64,
            libc: Libc::Glibc,
        })
    }

    fn mac() -> CfgEnv<'static> {
        CfgEnv::new(Platform {
            os: Os::Darwin,
            arch: Arch::Aarch64,
            libc: Libc::None,
        })
    }

    fn ev(s: &str, env: &CfgEnv<'_>) -> Tri {
        parse_str(s).eval(env)
    }

    #[test]
    fn all_targets_makes_target_predicates_unknown() {
        let l = linux().with_all_targets(true);
        assert_eq!(ev("cfg(unix)", &l), Tri::Unknown);
        assert_eq!(ev("target_os = \"macos\"", &l), Tri::Unknown);
        assert_eq!(ev("test", &l), ev("test", &linux()));
        assert_eq!(
            target_key_matches("x86_64-pc-windows-msvc", &l),
            Tri::Unknown
        );
        assert_eq!(
            target_key_matches("x86_64-pc-windows-msvc", &linux()),
            Tri::False
        );
    }

    #[test]
    fn platform_leaves() {
        let l = linux();
        assert_eq!(ev("cfg(unix)", &l), Tri::True);
        assert_eq!(ev("windows", &l), Tri::False);
        assert_eq!(ev("target_os = \"linux\"", &l), Tri::True);
        assert_eq!(ev("target_os = \"macos\"", &l), Tri::False);
        assert_eq!(ev("target_os = \"macos\"", &mac()), Tri::True);
        assert_eq!(ev("target_arch = \"x86_64\"", &l), Tri::True);
        assert_eq!(ev("target_arch = \"aarch64\"", &mac()), Tri::True);
        assert_eq!(ev("target_env = \"musl\"", &l), Tri::False);
        assert_eq!(ev("target_family = \"unix\"", &mac()), Tri::True);
        assert_eq!(ev("target_pointer_width = \"64\"", &l), Tri::True);
        assert_eq!(ev("target_vendor = \"apple\"", &mac()), Tri::True);
        assert_eq!(ev("target_endian = \"little\"", &l), Tri::True);
    }

    #[test]
    fn unknown_leaves_and_kleene_logic() {
        let l = linux();
        assert_eq!(ev("feature = \"x\"", &l), Tri::Unknown);
        assert_eq!(ev("test", &l), Tri::Unknown);
        assert_eq!(ev("debug_assertions", &l), Tri::Unknown);
        assert_eq!(ev("all(windows, feature = \"x\")", &l), Tri::False);
        assert_eq!(ev("all(unix, feature = \"x\")", &l), Tri::Unknown);
        assert_eq!(ev("any(unix, feature = \"x\")", &l), Tri::True);
        assert_eq!(ev("any(windows, test)", &l), Tri::Unknown);
        assert_eq!(ev("not(windows)", &l), Tri::True);
        assert_eq!(ev("not(test)", &l), Tri::Unknown);
        assert_eq!(ev("all()", &l), Tri::True);
        assert_eq!(ev("any()", &l), Tri::False);
        assert_eq!(
            ev("all(target_os = \"linux\", not(target_env = \"musl\"))", &l),
            Tri::True
        );
    }

    #[test]
    fn known_features() {
        let set: BTreeSet<String> = ["std".to_owned()].into();
        let env = CfgEnv {
            features: Some(&set),
            ..linux()
        };
        assert_eq!(ev("feature = \"std\"", &env), Tri::True);
        assert_eq!(ev("feature = \"alloc\"", &env), Tri::False);
    }

    #[test]
    fn malformed_predicates_are_unknown() {
        let l = linux();
        for s in [
            "cfg(",
            "not(a, b)",
            "= \"x\"",
            "all(unix",
            "target_os =",
            "",
        ] {
            assert_ne!(ev(s, &l), Tri::False, "{s}");
        }
        let deep = format!("{}unix{}", "not(".repeat(100), ")".repeat(100));
        assert_eq!(ev(&deep, &l), Tri::Unknown);
    }

    #[test]
    fn target_keys() {
        let l = linux();
        assert_eq!(target_key_matches("cfg(windows)", &l), Tri::False);
        assert_eq!(
            target_key_matches("x86_64-unknown-linux-gnu", &l),
            Tri::True
        );
        assert_eq!(target_key_matches("x86_64-pc-windows-msvc", &l), Tri::False);
        assert_eq!(
            target_key_matches(
                "cfg(all(target_arch = \"x86_64\", target_env = \"musl\"))",
                &l
            ),
            Tri::False
        );
    }

    #[test]
    fn display() {
        assert_eq!(
            parse_str("cfg(all(unix, not(feature = \"x\")))").to_string(),
            "all(unix, not(feature = \"x\"))"
        );
    }
}
