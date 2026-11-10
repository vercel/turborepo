//! Environment-variable facts: what a file says about paths that travel through
//! the process environment.
//!
//! A file read or module load whose path comes from an environment variable
//! cannot be resolved from the file alone: the value is assigned elsewhere (a
//! test's setup, the `env` object of a spawned process, a config file) or
//! outside the repository. A language plugin therefore records three kinds of
//! fact per file, each a pure function of the file's text, carried with its
//! extraction ([`crate::RefSink::fact`]):
//!
//! - [`EnvFact::Use`]: a reference whose path is built from environment
//!   variables or from calls of imported functions
//!   (`readFile(getEnv('TOKEN_PATH', DEFAULT))`);
//! - [`EnvFact::Set`]: an assignment of a variable (`process.env.X = v`, `{ X:
//!   v }` in an object literal, `stubEnv('X', v)`) with its evaluated value;
//! - [`EnvFact::Helper`]: an exported function that returns environment values,
//!   as a template over its arguments (`getEnv(name, fallback)` returns
//!   `env[name]` or `fallback`).
//!
//! The graph store joins the facts of all files into a registry (variable →
//! every value the repository assigns) and resolves the uses against it when a
//! graph is opened.
//!
//! Facts are serialized as tagged strings (`env:<json>`), so the extraction
//! cache and the shard format treat them as opaque.

use serde::{Deserialize, Serialize};

/// Prefix of a serialized [`EnvFact`].
pub const ENV_FACT_TAG: &str = "env:";

/// Where the text of an [`EnvTerm`] is anchored.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvBase {
    /// A plain string. As a path it is absolute when it starts with `/`, and
    /// otherwise relative to the working directory of whoever uses it.
    #[serde(rename = "s")]
    Str,
    /// Relative to the directory of the file the fact is in.
    #[serde(rename = "f")]
    FileDir,
    /// Relative to the process working directory.
    #[serde(rename = "c")]
    Cwd,
    /// Relative to the repository root. Not produced by a file's own facts (a
    /// file does not know where it is): the form in which a cross-file
    /// evaluator reports values.
    #[serde(rename = "r")]
    Root,
}

/// The name of an environment variable in a template.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvName {
    /// A literal name.
    #[serde(rename = "n")]
    Name(String),
    /// In a helper summary: the string passed as argument `i`.
    #[serde(rename = "a")]
    Arg(u32),
}

/// One part of an [`EnvTerm`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvPart {
    /// Literal text.
    #[serde(rename = "l")]
    Lit(String),
    /// Unknown text that is not the first part: path segments below a known
    /// prefix.
    #[serde(rename = "w")]
    Wild,
    /// The value of an environment variable.
    #[serde(rename = "v")]
    Var(EnvName),
    /// In a helper summary: the value of argument `i`.
    #[serde(rename = "a")]
    Arg(u32),
    /// In a helper summary: what calling argument `i` returns.
    #[serde(rename = "r")]
    ArgCall(u32),
    /// The result of call `i` of the enclosing [`EnvValue::calls`].
    #[serde(rename = "c")]
    Call(u32),
}

/// One alternative of a value: an anchor and the concatenation of its parts.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvTerm {
    /// The anchor of the text.
    #[serde(rename = "b")]
    pub base: EnvBase,
    /// The parts, concatenated. Empty: the anchor directory itself.
    #[serde(rename = "p")]
    pub parts: Vec<EnvPart>,
}

/// One alternative of an [`EnvValue`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvAlt {
    /// A string or path.
    #[serde(rename = "t")]
    Term(EnvTerm),
    /// A value the extractor could not evaluate; the reason.
    #[serde(rename = "u")]
    Unknown(String),
}

/// A call of an imported function, with evaluated arguments.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvCall {
    /// The module specifier of the import.
    #[serde(rename = "m")]
    pub module: String,
    /// The exported name (`default`, `getEnv`, `env.tokenPath`).
    #[serde(rename = "e")]
    pub export: String,
    /// The arguments.
    #[serde(rename = "a")]
    pub args: Vec<EnvArg>,
}

/// One argument of an [`EnvCall`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvArg {
    /// The alternatives of the argument's value. Empty: not passed, or never a
    /// string.
    #[serde(rename = "v")]
    pub value: Vec<EnvAlt>,
    /// When the argument can be a function: the alternatives of what it
    /// returns.
    #[serde(rename = "r", default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<Vec<EnvAlt>>,
}

/// A value: its alternatives. Alternatives that can never be a path
/// (`undefined`, a boolean) or that lie in the temporary directory are left
/// out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvValue {
    /// The alternatives.
    #[serde(rename = "a")]
    pub alts: Vec<EnvAlt>,
    /// The calls [`EnvPart::Call`] parts refer to.
    #[serde(rename = "c", default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<EnvCall>,
}

/// Where a module load searches from, for the one kind of value whose meaning
/// depends on it: a relative specifier (`./x`), which Node resolves against the
/// place of the `require` function that is asked. An absolute or anchored value
/// names the same module for every `require` function.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvFrom {
    /// The file with the use: its own `require` or `import()`, or a `require`
    /// function made for the file or for a file in its directory.
    #[default]
    #[serde(rename = "f")]
    File,
    /// A `require` function made for a file in this directory, given relative
    /// to the directory of the file with the use
    /// (`createRequire(path.join(__dirname, '../other/x.js'))`).
    #[serde(rename = "d")]
    Dir(String),
    /// A `require` function whose place is not known (the working directory, a
    /// parameter): a relative value names no module that can be said.
    #[serde(rename = "u")]
    Unknown,
}

impl EnvFrom {
    /// Whether this is [`EnvFrom::File`] (the default, which is not
    /// serialized).
    pub fn is_file(&self) -> bool {
        *self == EnvFrom::File
    }
}

/// A fact about environment variables in one file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvFact {
    /// A reference whose path comes from the environment: the alternatives of
    /// one origin (`env` or `call`) of the reference at `span`, which
    /// raised an uncertainty record with that span and origin.
    #[serde(rename = "use")]
    Use {
        /// Byte span of the uncertainty record.
        span: [u32; 2],
        /// 1-based line.
        line: u32,
        /// Index into [`crate::RefKind::ALL`].
        kind: u8,
        /// Index into [`crate::Origin::ALL`] of the record.
        origin: u8,
        /// The value.
        value: EnvValue,
        /// For a module load asked of a `require` function: where it searches
        /// from.
        #[serde(default, skip_serializing_if = "EnvFrom::is_file")]
        from: EnvFrom,
        /// The record stands for other alternatives of the reference too
        /// (another record of its kind sits at the same place, and
        /// nothing tells the two apart): what the value names is added,
        /// and the records are left as they are. The reference
        /// stays open either way, by the other alternative.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        shared: bool,
    },
    /// An assignment of variable `name`.
    #[serde(rename = "set")]
    Set {
        /// The variable.
        name: String,
        /// 1-based line.
        line: u32,
        /// The assigned value.
        value: EnvValue,
        /// Byte span of the assigned expression, when the value has an
        /// alternative the extractor could not evaluate: a cross-file
        /// evaluator can be asked for it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<[u32; 2]>,
    },
    /// An exported function whose result is a template over its arguments.
    #[serde(rename = "helper")]
    Helper {
        /// The exported name.
        export: String,
        /// The result, with [`EnvPart::Arg`], [`EnvPart::ArgCall`] and
        /// [`EnvName::Arg`] for the arguments.
        value: EnvValue,
    },
}

impl EnvFact {
    /// The fact as a tagged string.
    pub fn encode(&self) -> String {
        let mut s = String::from(ENV_FACT_TAG);
        s.push_str(&serde_json::to_string(self).unwrap_or_default());
        s
    }

    /// Parses a tagged string. `None` for facts of other kinds and for
    /// malformed input.
    pub fn decode(s: &str) -> Option<EnvFact> {
        serde_json::from_str(s.strip_prefix(ENV_FACT_TAG)?).ok()
    }
}

impl EnvTerm {
    /// A plain string.
    pub fn str(s: impl Into<String>) -> EnvTerm {
        EnvTerm {
            base: EnvBase::Str,
            parts: vec![EnvPart::Lit(s.into())],
        }
    }

    /// The term's text when it is one literal.
    pub fn literal(&self) -> Option<&str> {
        match self.parts.as_slice() {
            [] => Some(""),
            [EnvPart::Lit(s)] => Some(s),
            _ => None,
        }
    }
}

/// Whether `name` looks like an environment variable written by convention:
/// upper-case letters, digits and underscores, at least three characters,
/// starting with a letter or an underscore.
pub fn is_env_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 3
        && (b[0].is_ascii_uppercase() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
        && b.iter().any(|c| c.is_ascii_uppercase())
}

/// `NAME=value` assignments in command-line text (`X=a.json Y=1 node run.js`,
/// also inside a longer string): `(name, value, at_end)`. The value ends at
/// white space, a quote or a shell separator. `at_end`: the text ends right
/// after `=` (the value follows in another piece, a template expression).
pub fn command_assignments(text: &str) -> Vec<(&str, &str, bool)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = b[from..].iter().position(|c| *c == b'=') {
        let eq = from + off;
        from = eq + 1;
        if b.get(eq + 1) == Some(&b'=') {
            from = eq + 2;
            continue;
        }
        let mut s = eq;
        while s > 0 && (b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_') {
            s -= 1;
        }
        let name = &text[s..eq];
        if !is_env_name(name) {
            continue;
        }
        let boundary = s == 0
            || matches!(
                b[s - 1],
                b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'(' | b'"' | b'\''
            );
        if !boundary {
            continue;
        }
        let mut e = eq + 1;
        while e < b.len()
            && !matches!(
                b[e],
                b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b')' | b'"' | b'\''
            )
        {
            e += 1;
        }
        out.push((name, &text[eq + 1..e], e == b.len() && e == eq + 1));
        from = e;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_assignments() {
        assert_eq!(
            command_assignments("A_FILE=../x/a.ts B_MODE=test tsx run.ts --flag=1"),
            [("A_FILE", "../x/a.ts", false), ("B_MODE", "test", false)]
        );
        assert_eq!(
            command_assignments("cross-env DATA_DIR=./d jest && X_Y=1 node b.js"),
            [("DATA_DIR", "./d", false), ("X_Y", "1", false)]
        );
        assert_eq!(
            command_assignments("run DATA_FILE="),
            [("DATA_FILE", "", true)]
        );
        assert!(command_assignments("a==b lower=1 url?FOO_BAR=1 x.FOO=1").is_empty());
    }

    #[test]
    fn facts_round_trip() {
        let f = EnvFact::Use {
            span: [10, 42],
            line: 3,
            kind: 7,
            origin: 2,
            value: EnvValue {
                alts: vec![
                    EnvAlt::Term(EnvTerm {
                        base: EnvBase::Str,
                        parts: vec![
                            EnvPart::Var(EnvName::Name("DATA_DIR".into())),
                            EnvPart::Lit("/x.json".into()),
                        ],
                    }),
                    EnvAlt::Term(EnvTerm {
                        base: EnvBase::Str,
                        parts: vec![EnvPart::Call(0)],
                    }),
                    EnvAlt::Unknown("f()".into()),
                ],
                calls: vec![EnvCall {
                    module: "@scope/env".into(),
                    export: "getEnv".into(),
                    args: vec![
                        EnvArg {
                            value: vec![EnvAlt::Term(EnvTerm::str("TOKEN_PATH"))],
                            returns: None,
                        },
                        EnvArg {
                            value: vec![],
                            returns: Some(vec![EnvAlt::Term(EnvTerm::str("/var/run/t"))]),
                        },
                    ],
                }],
            },
            from: EnvFrom::File,
            shared: false,
        };
        let s = f.encode();
        assert!(s.starts_with("env:{\"use\""), "{s}");
        // The file's own request and a record of its own are the defaults: not written.
        assert!(!s.contains("from") && !s.contains("shared"), "{s}");
        assert_eq!(EnvFact::decode(&s), Some(f.clone()));
        for from in [EnvFrom::Dir("../other".into()), EnvFrom::Unknown] {
            let EnvFact::Use {
                span,
                line,
                kind,
                origin,
                value,
                ..
            } = f.clone()
            else {
                unreachable!()
            };
            let g = EnvFact::Use {
                span,
                line,
                kind,
                origin,
                value,
                from,
                shared: true,
            };
            let s = g.encode();
            assert!(s.contains("\"from\"") && s.contains("\"shared\""), "{s}");
            assert_eq!(EnvFact::decode(&s), Some(g));
        }
        assert_eq!(EnvFact::decode("other:{}"), None);
        assert_eq!(EnvFact::decode("env:{"), None);
    }

    #[test]
    fn env_names() {
        for n in ["PORT", "DATA_FILE", "_X1", "A_B_C", "NODE_ENV"] {
            assert!(is_env_name(n), "{n}");
        }
        for n in ["Port", "dataFile", "AB", "1ABC", "A-B", "___", "", "getEnv"] {
            assert!(!is_env_name(n), "{n}");
        }
    }
}
