//! Hints: declared dependencies and bounds for dynamic references, read from a
//! TOML file and applied to the composed graph ([`Graph::hints_patch`]). `turbo
//! trace` reads none; the tests name one (`TURBO_TRACE_DEBUG_HINTS`).
//!
//! A hint is the escape hatch for references no analysis can bound: a framework
//! that imports handler modules from path strings in route tables, a helper
//! that loads a module chosen at runtime. It applies to the files its `files`
//! glob matches:
//!
//! ```toml
//! # A router imports the handlers its route tables name: the tables depend on them.
//! [[hint]]
//! files = "services/*/src/routes.ts"
//! depends-on = ["./handlers/**"]        # "./": relative to each matched file's directory
//!
//! # What the dynamic loads of a file can reach, instead of "anything".
//! [[hint]]
//! files = "packages/framework/normalize.ts"
//! line = 42                             # optional: only the reference on this line
//! kinds = ["computed_specifier"]        # optional: only records of these kinds
//! loads = "module-strings"              # or a list of globs; [] = nothing in the repository
//!
//! # A helper that loads `<working directory>/src/routes` in a way the analysis can't follow.
//! [[hint]]
//! files = "packages/testing/setup.ts"
//! cwd-loads = ["./src/routes"]
//! ```
//!
//! | Key | Effect |
//! |---|---|
//! | `depends-on = [globs]` | An edge from each matched file to every graph file a glob matches, and a directory membership per glob, so files added or deleted there count too. A pattern without wildcards also becomes a config input (it may name a file that is not in the graph, or not there yet). |
//! | `loads = [globs]` | As `depends-on`, and the matched files' open uncertainty records (those raised by references; filtered by `line` and `kinds`) are dropped: the globs bound what the references can load. `[]` declares that they load nothing from the repository. |
//! | `loads = "module-strings"` | The records are dropped with nothing added: the references only load modules that string literals in the entry's closure name, and those are edges already. |
//! | `cwd-loads = [specifiers]` | The matched files load these modules relative to the working directory: resolved per test like extracted `cwd_dependent` references ([`Graph::cwd_patch`]). The records then stay open for entries without a known working directory. |
//!
//! A second table declares what something outside the repository's code (a CI
//! setting, a deploy system) sets an environment variable to, when that is a
//! path in the repository ([`super::env`]):
//!
//! ```toml
//! [[env]]
//! name = "FLAGS_DATAFILE"
//! values = ["fixtures/flags.json"]      # relative to the repository root; globs allowed
//! ```
//!
//! Hints are declarations: they are trusted. A hint that leaves out
//! something a reference can load makes `affected` miss it, so prefer wide
//! globs. Unknown keys are errors. The file has other owners too, whose tables
//! are skipped here and checked by them: `[lint]` belongs to the lint
//! and `[nextjs]` to the Next.js plugin ([`OTHER_TABLES`]). Hints are applied
//! when a graph is opened, not stored in it: editing the file takes effect at
//! once and invalidates nothing.

use std::{collections::HashMap, path::Path};

use serde::Deserialize;
use turborepo_trace_graph::{
    self as q, EdgeAttr, GraphView, NodeId, NodeSet, Patch,
    glob::{Glob, is_glob},
};

use super::{
    Graph,
    cwd::{ExtraSite, RESOLVED_NOTE},
};
use crate::paths::join_repo;

/// The top-level tables of a hints file that other readers of the file own:
/// accepted and not looked at by [`Hints::parse`]. A reader that gets a table
/// of its own is added here, or every command that opens a graph refuses a file
/// with that table.
pub const OTHER_TABLES: &[&str] = &["lint", "nextjs"];

/// What a hint says the dynamic references of its files can load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loads {
    /// Files matching these globs (none: nothing in the repository).
    Globs(Vec<String>),
    /// Only modules named by string literals, which are edges already.
    ModuleStrings,
}

/// One `[[hint]]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// Glob over repo-relative paths: the files the hint applies to.
    pub files: String,
    /// Only records on this line.
    pub line: Option<u32>,
    /// Only records of these kinds.
    pub kinds: Vec<q::UncertaintyKind>,
    /// Extra dependencies.
    pub depends_on: Vec<String>,
    /// The bound of the files' dynamic references.
    pub loads: Option<Loads>,
    /// Modules loaded relative to the working directory.
    pub cwd_loads: Vec<String>,
}

/// The hints of a repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hints {
    /// In file order.
    pub hints: Vec<Hint>,
    /// `[[env]]`: values that something outside the repository's code gives to
    /// environment variables ([`super::env`]).
    pub env: Vec<super::env::EnvDecl>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    hint: Vec<RawHint>,
    #[serde(default)]
    env: Vec<RawEnv>,
    /// `[lint]`: the configuration of the lint, which parses it
    /// itself. Accepted here so that a file with that table still loads.
    #[serde(default, rename = "lint")]
    _lint: serde::de::IgnoredAny,
    /// `[nextjs]`: the tables of the Next.js plugin (`[[nextjs.url_apis]]`),
    /// which parses them itself
    /// (`turborepo_trace_frameworks::nextjs::fetch::parse_url_apis_toml`).
    #[serde(default, rename = "nextjs")]
    _nextjs: serde::de::IgnoredAny,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnv {
    name: String,
    values: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawHint {
    files: String,
    line: Option<u32>,
    #[serde(default)]
    kinds: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    loads: Option<RawLoads>,
    #[serde(default)]
    cwd_loads: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawLoads {
    Word(String),
    Globs(Vec<String>),
}

impl Hints {
    /// Parses a hints file.
    pub fn parse(text: &str) -> Result<Hints, String> {
        let raw: RawFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut hints = Vec::new();
        for (i, h) in raw.hint.into_iter().enumerate() {
            let files = h.files.clone();
            let at = move |m: String| format!("hint {} ({files}): {m}", i + 1);
            Glob::new(&h.files).map_err(|e| at(format!("files: {e}")))?;
            let mut kinds = Vec::new();
            for k in &h.kinds {
                kinds.push(
                    q::UncertaintyKind::parse(k)
                        .ok_or_else(|| at(format!("unknown uncertainty kind `{k}`")))?,
                );
            }
            let loads = match h.loads {
                None => None,
                Some(RawLoads::Globs(g)) => Some(Loads::Globs(g)),
                Some(RawLoads::Word(w)) if w == "module-strings" => Some(Loads::ModuleStrings),
                Some(RawLoads::Word(w)) => {
                    return Err(at(format!(
                        "loads: expected a list of globs or \"module-strings\", found \"{w}\""
                    )));
                }
            };
            let mut globs: Vec<&String> = h.depends_on.iter().collect();
            if let Some(Loads::Globs(g)) = &loads {
                globs.extend(g.iter());
            }
            for g in globs {
                if g.starts_with('/') {
                    return Err(at(format!(
                        "`{g}`: paths are relative to the repository root"
                    )));
                }
                Glob::new(g.trim_start_matches("./")).map_err(|e| at(format!("`{g}`: {e}")))?;
            }
            for s in &h.cwd_loads {
                if s.starts_with('/') || is_glob(s) {
                    return Err(at(format!(
                        "cwd-loads `{s}`: expected a path relative to the working directory"
                    )));
                }
            }
            if h.depends_on.is_empty() && loads.is_none() && h.cwd_loads.is_empty() {
                return Err(at("needs depends-on, loads or cwd-loads".to_owned()));
            }
            if (h.line.is_some() || !h.kinds.is_empty())
                && loads.is_none()
                && h.cwd_loads.is_empty()
            {
                return Err(at("line and kinds select uncertainty records: they need \
                               loads or cwd-loads"
                    .to_owned()));
            }
            hints.push(Hint {
                files: h.files,
                line: h.line,
                kinds,
                depends_on: h.depends_on,
                loads,
                cwd_loads: h.cwd_loads,
            });
        }
        let mut env = Vec::new();
        for e in raw.env {
            for v in &e.values {
                if v.starts_with('/') {
                    return Err(format!(
                        "env {}: `{v}`: paths are relative to the repository root",
                        e.name
                    ));
                }
                Glob::new(v).map_err(|err| format!("env {}: `{v}`: {err}", e.name))?;
            }
            env.push(super::env::EnvDecl {
                name: e.name,
                values: e.values,
            });
        }
        Ok(Hints { hints, env })
    }

    /// Reads and parses `path`. `Ok(None)` when the file does not exist.
    pub fn load(path: &Path) -> Result<Option<Hints>, String> {
        match std::fs::read_to_string(path) {
            Ok(t) => Hints::parse(&t)
                .map(Some)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Whether there are no hints.
    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }
}

impl Hint {
    /// Whether the hint bounds uncertainty records.
    fn closes(&self) -> bool {
        self.loads.is_some() || !self.cwd_loads.is_empty()
    }

    /// Whether the hint applies to record `u`.
    fn selects(&self, u: &q::Uncertainty<'_>) -> bool {
        if u.is_covered() || self.line.is_some_and(|l| l != u.line) {
            return false;
        }
        if self.kinds.is_empty() {
            // By default: the records references raised.
            u.effect != q::Effect::None
        } else {
            self.kinds.contains(&u.kind)
        }
    }
}

/// The directory of a repo-relative path.
fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Splits a glob into its static directory and the pattern below it.
fn split_glob(p: &str) -> (String, String) {
    let segs: Vec<&str> = p.split('/').collect();
    let k = segs.iter().position(|s| is_glob(s)).unwrap_or(segs.len());
    (segs[..k].join("/"), segs[k..].join("/"))
}

impl Graph {
    /// The patch for `hints`, together with [`Graph::cwd_patch`] (the two share
    /// the per-test resolution), and warnings: hints that match no file or
    /// whose `cwd-loads` could not be resolved.
    pub fn hints_patch(&self, hints: &Hints) -> (Patch, Vec<String>) {
        let n = self.node_count();
        let mut warnings = Vec::new();
        // Files per hint: first-party source files.
        let mut targets: Vec<Vec<NodeId>> = Vec::new();
        let mut extra: Vec<ExtraSite> = Vec::new();
        for (i, h) in hints.hints.iter().enumerate() {
            let mut set = NodeSet::new(n);
            if let Ok(g) = Glob::new(&h.files) {
                self.glob(&g, &mut set);
            }
            let nodes: Vec<NodeId> = set
                .iter()
                .filter(|&x| {
                    !self.is_external(x)
                        && self.lang(x).is_some()
                        && !crate::tasks::is_pseudo_lang(self.lang(x))
                })
                .collect();
            if nodes.is_empty() {
                warnings.push(format!("hint {} ({}): matches no file", i + 1, h.files));
            }
            if !h.cwd_loads.is_empty() {
                let specs: Vec<String> = h
                    .cwd_loads
                    .iter()
                    .map(|s| {
                        if s.starts_with("./") || s.starts_with("../") {
                            s.clone()
                        } else {
                            format!("./{s}")
                        }
                    })
                    .collect();
                for x in &nodes {
                    extra.push(ExtraSite {
                        node: x.0,
                        specs: specs.clone(),
                        line: h.line.unwrap_or(0),
                        key: [u32::MAX, i as u32],
                    });
                }
            }
            targets.push(nodes);
        }
        let (mut patch, failed) = self.cwd_patch_with(&extra);
        let attr = EdgeAttr(crate::convert::edge_attr(
            turborepo_trace_core::RefKind::DynamicImport,
            turborepo_trace_core::Phase::Runtime,
            turborepo_trace_core::RefFlags::LAZY,
        ));
        // Matches of a resolved glob, shared between files.
        let mut matches: HashMap<String, Vec<NodeId>> = HashMap::new();
        for (i, h) in hints.hints.iter().enumerate() {
            let mut globs: Vec<&String> = h.depends_on.iter().collect();
            if let Some(Loads::Globs(g)) = &h.loads {
                globs.extend(g.iter());
            }
            for &f in &targets[i] {
                let from = self.path(f);
                for g in &globs {
                    let resolved = if g.starts_with("./") || g.starts_with("../") {
                        join_repo(dir_of(from), g)
                    } else {
                        Some((*g).clone())
                    };
                    let Some(p) = resolved else {
                        warnings.push(format!(
                            "hint {} ({}): `{g}` leaves the repository from {from}",
                            i + 1,
                            h.files
                        ));
                        continue;
                    };
                    let label = format!("hint: {p}");
                    let ms = matches.entry(p.clone()).or_insert_with(|| {
                        let mut set = NodeSet::new(n);
                        if let Ok(gl) = Glob::new(&p) {
                            self.glob(&gl, &mut set);
                        }
                        set.iter()
                            .filter(|&x| !crate::tasks::is_pseudo_lang(self.lang(x)))
                            .collect()
                    });
                    for &t in ms.iter() {
                        patch.edge(f, t, attr, &label);
                    }
                    if is_glob(&p) {
                        let (dir, pattern) = split_glob(&p);
                        patch.membership(f, &dir, &pattern);
                    } else {
                        patch.input(f, &p);
                    }
                }
                if !h.closes() {
                    continue;
                }
                let per_test = !h.cwd_loads.is_empty();
                if per_test && failed.contains(&(f.0, [u32::MAX, i as u32])) {
                    warnings.push(format!(
                        "hint {} ({}): cwd-loads could not be resolved for every test that \
                         reaches {from}; its records stay open",
                        i + 1,
                        h.files
                    ));
                    continue;
                }
                let records: Vec<(q::UncertaintyKind, u32, Option<q::Span>, String)> =
                    GraphView::uncertainties(self, f)
                        .filter(|u| h.selects(u))
                        .map(|u| (u.kind, u.line, u.span, u.detail.to_owned()))
                        .collect();
                for (kind, line, span, detail) in records {
                    if per_test {
                        if detail.ends_with(RESOLVED_NOTE) {
                            continue;
                        }
                        patch.resolve_for_entries(
                            f,
                            kind,
                            line,
                            span,
                            &format!("{detail}; hint: {RESOLVED_NOTE}"),
                        );
                    } else {
                        patch.drop_uncertainty(f, kind, line, span);
                    }
                }
            }
        }
        (patch, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hints() {
        let h = Hints::parse(
            r#"
[[hint]]
files = "services/*/src/routes.ts"
depends-on = ["./handlers/**"]

[[hint]]
files = "packages/framework/normalize.ts"
line = 42
kinds = ["computed_specifier"]
loads = "module-strings"

[[hint]]
files = "packages/x/load.ts"
loads = []
cwd-loads = ["src/routes"]
"#,
        )
        .unwrap();
        assert_eq!(h.hints.len(), 3);
        assert_eq!(h.hints[0].depends_on, ["./handlers/**"]);
        assert_eq!(h.hints[1].loads, Some(Loads::ModuleStrings));
        assert_eq!(h.hints[1].line, Some(42));
        assert_eq!(h.hints[1].kinds, [q::UncertaintyKind::ComputedSpecifier]);
        assert_eq!(h.hints[2].loads, Some(Loads::Globs(vec![])));
        assert_eq!(h.hints[2].cwd_loads, ["src/routes"]);
        assert!(Hints::parse("").unwrap().is_empty());
        let e = Hints::parse("[[env]]\nname = \"DATA_FILE\"\nvalues = [\"fixtures/*.json\"]\n")
            .unwrap();
        assert_eq!(e.env[0].name, "DATA_FILE");
        assert_eq!(e.env[0].values, ["fixtures/*.json"]);
        assert!(
            Hints::parse("[[env]]\nname = \"A\"\nvalues = [\"/abs\"]\n")
                .unwrap_err()
                .contains("repository root")
        );
    }

    #[test]
    fn tables_of_the_other_readers_are_skipped() {
        // `turbotrace.toml` is read by the lint (`[lint]`) and by the Next.js
        // plugin (`[[nextjs.url_apis]]`) too: a file with their tables loads,
        // whatever is in them.
        let h = Hints::parse(
            r#"
[[hint]]
files = "a.ts"
loads = []

[lint]
baseline = "ci/lint-baseline.json"

[lint.rules]
unconstrained-load = "warn"

[[nextjs.url_apis]]
module = "@acme/fetch"
name = "apiFetch"
prefix = "/api"
"#,
        )
        .unwrap();
        assert_eq!(h.hints.len(), 1);
        for t in OTHER_TABLES {
            assert!(
                Hints::parse(&format!("[{t}]\nwhatever = 1\n")).is_ok(),
                "[{t}]"
            );
            assert!(Hints::parse(&format!("[[{t}.list]]\nx = 1\n")).is_ok());
        }
        // A table nobody owns is still an error.
        let e = Hints::parse("[nextj]\nurl_apis = []\n").unwrap_err();
        assert!(e.contains("nextj"), "{e}");
    }

    #[test]
    fn rejects_bad_hints() {
        let err = |t: &str| Hints::parse(t).unwrap_err();
        assert!(err("[[hint]]\nfiles = \"a\"\n").contains("needs depends-on"));
        assert!(err("[[hint]]\nfiles = \"a\"\nload = []\n").contains("unknown field"));
        assert!(err("[[hint]]\nfiles = \"a\"\nloads = \"anything\"\n").contains("module-strings"));
        assert!(
            err("[[hint]]\nfiles = \"a\"\nkinds = [\"nope\"]\nloads = []\n")
                .contains("unknown uncertainty kind")
        );
        assert!(
            err("[[hint]]\nfiles = \"a\"\nline = 3\ndepends-on = [\"b\"]\n")
                .contains("line and kinds")
        );
        assert!(
            err("[[hint]]\nfiles = \"a\"\ncwd-loads = [\"/abs\"]\n").contains("working directory")
        );
        assert!(
            err("[[hint]]\nfiles = \"a\"\ndepends-on = [\"/abs/**\"]\n")
                .contains("repository root")
        );
        assert!(err("[[hints]]\nfiles = \"a\"\n").contains("unknown field"));
    }

    #[test]
    fn skips_the_lint_table() {
        let h = Hints::parse(
            "[lint]\nbaseline = \"b.json\"\nanything = 1\n\n[lint.rules]\nunconstrained-load = \
             \"warn\"\n\n[[hint]]\nfiles = \"a\"\nloads = []\n",
        )
        .unwrap();
        assert_eq!(h.hints.len(), 1);
        assert!(Hints::parse("[lint]\n").unwrap().is_empty());
    }

    #[test]
    fn splits_globs() {
        assert_eq!(
            split_glob("services/a/src/handlers/**"),
            ("services/a/src/handlers".to_owned(), "**".to_owned())
        );
        assert_eq!(split_glob("*/x.ts"), (String::new(), "*/x.ts".to_owned()));
        assert_eq!(dir_of("a/b/c.ts"), "a/b");
        assert_eq!(dir_of("c.ts"), "");
    }
}
