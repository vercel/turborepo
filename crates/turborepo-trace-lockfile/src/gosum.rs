//! Go modules: `go.mod`, `go.work` and `go.sum` parsing, and the mapping of a
//! module's build list into the neutral [`Lockfile`] model.
//!
//! # Model mapping
//!
//! | Go | Model |
//! |---|---|
//! | main module (`go.mod` directory) | [`Importer`] `.`, named by the `module` path |
//! | `require M V` | an importer [`DepEdge`] `M` → package `M@V` of kind [`DepKind::Prod`]; `// indirect` requirements too, because Go ≥ 1.17 lists every module that provides a package in the build |
//! | module version in the build list | [`LockedPackage`] with id `M@V`, name `M`, version `V`, integrity = the `go.sum` `h1:` hash of the module zip (else of its `go.mod`) |
//! | `replace M [V] => N W` | the package keeps id `M@V`; [`LockedPackage::resolution`] is `N@W` and the integrity is `N@W`'s hash |
//! | `replace M [V] => ./dir` | first-party code: the edge targets [`Target::Importer`] `dir` (traced through, like a workspace package) |
//! | `exclude M V` | recorded in [`GoMod::excludes`]; excluded versions never enter the model |
//! | `go.work` `use` directories | not part of [`to_lockfile`]: each module's `go.mod` is its own importer when the caller merges them |
//!
//! `go.sum` has no dependency graph, so packages carry no edges and a version
//! bump surfaces as an identity change (`M@V1` gone, `M@V2` new) through
//! [`crate::diff`]. A changed `replace` shows as a changed resolution (and
//! usually integrity).
//!
//! [`parse_go_mod`], [`parse_go_work`] and [`parse_go_sum`] are lenient:
//! unknown directives are skipped, so newer `go.mod` syntax never fails a
//! parse. Malformed lines of known directives are [`crate::Error::Syntax`]
//! errors.

use std::collections::BTreeMap;

use crate::model::{
    DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta, Target,
};

/// One `require` line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GoRequire {
    /// Module path.
    pub path: String,
    /// Required (minimum) version.
    pub version: String,
    /// Marked `// indirect`.
    pub indirect: bool,
}

/// Where a `replace` directive points.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReplaceTarget {
    /// A directory (`./x`, `../x`, `/abs`), relative to the `go.mod`/`go.work`
    /// directory.
    Local(String),
    /// Another module version.
    Module {
        /// Module path.
        path: String,
        /// Version.
        version: String,
    },
}

/// One `replace` line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GoReplace {
    /// The replaced module path.
    pub old: String,
    /// The replaced version, or `None` for every version.
    pub old_version: Option<String>,
    /// The replacement.
    pub new: ReplaceTarget,
}

/// A parsed `go.mod`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoMod {
    /// `module` path.
    pub module: Option<String>,
    /// `go` version.
    pub go: Option<String>,
    /// `toolchain` name.
    pub toolchain: Option<String>,
    /// `require` lines, in file order.
    pub requires: Vec<GoRequire>,
    /// `replace` lines, in file order.
    pub replaces: Vec<GoReplace>,
    /// `exclude` lines (`path`, `version`).
    pub excludes: Vec<(String, String)>,
    /// `tool` lines (package paths).
    pub tools: Vec<String>,
}

/// A parsed `go.work`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoWork {
    /// `go` version.
    pub go: Option<String>,
    /// `use` directories, relative to the `go.work` directory.
    pub uses: Vec<String>,
    /// `replace` lines; they override the modules' own replacements.
    pub replaces: Vec<GoReplace>,
}

/// The hashes `go.sum` records for one module version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct GoSumEntry {
    /// `h1:` hash of the module zip (`M V h1:…`).
    pub h1: Option<String>,
    /// `h1:` hash of the module's `go.mod` (`M V/go.mod h1:…`).
    pub go_mod_h1: Option<String>,
}

/// A parsed `go.sum`, keyed by `(module path, version)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoSum {
    /// Entries, sorted by module path then version string.
    pub entries: BTreeMap<(String, String), GoSumEntry>,
}

impl GoSum {
    /// The hashes for `path@version`.
    pub fn get(&self, path: &str, version: &str) -> Option<&GoSumEntry> {
        self.entries.get(&(path.to_owned(), version.to_owned()))
    }

    /// The content hash used as integrity: the zip hash, else the `go.mod`
    /// hash.
    pub fn integrity(&self, path: &str, version: &str) -> Option<String> {
        let e = self.get(path, version)?;
        e.h1.clone().or_else(|| e.go_mod_h1.clone())
    }
}

impl GoMod {
    /// The replacement for `path@version`: a version-specific `replace` wins
    /// over a wildcard one (Go's rule).
    pub fn replacement(&self, path: &str, version: &str) -> Option<&ReplaceTarget> {
        find_replacement(&self.replaces, path, version)
    }

    /// Whether `path@version` is excluded.
    pub fn is_excluded(&self, path: &str, version: &str) -> bool {
        self.excludes.iter().any(|(p, v)| p == path && v == version)
    }

    /// The required version of `path`, if required directly (the last line
    /// wins, as Go's `go.mod` editing keeps one line per module).
    pub fn required_version(&self, path: &str) -> Option<&str> {
        self.requires
            .iter()
            .rev()
            .find(|r| r.path == path)
            .map(|r| r.version.as_str())
    }
}

/// The replacement for `path@version` among `replaces` (version-specific
/// first).
pub fn find_replacement<'r>(
    replaces: &'r [GoReplace],
    path: &str,
    version: &str,
) -> Option<&'r ReplaceTarget> {
    replaces
        .iter()
        .rev()
        .find(|r| r.old == path && r.old_version.as_deref() == Some(version))
        .or_else(|| {
            replaces
                .iter()
                .rev()
                .find(|r| r.old == path && r.old_version.is_none())
        })
        .map(|r| &r.new)
}

/// Whether a `replace` right-hand side is a directory (Go: starts with `./`,
/// `../` or `/`, or is `.`/`..`).
pub fn is_local_path(s: &str) -> bool {
    s == "." || s == ".." || s.starts_with("./") || s.starts_with("../") || s.starts_with('/')
}

/// Splits one line into tokens: bare words, `"interpreted"` and `` `raw` ``
/// strings, and the `=>`, `(` and `)` punctuation. Returns the tokens and the
/// trailing `//` comment, if any.
fn tokenize(line: &str) -> Result<(Vec<String>, Option<&str>), String> {
    let b = line.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\r' => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                return Ok((toks, Some(line[i + 2..].trim())));
            }
            b'(' | b')' => {
                toks.push((b[i] as char).to_string());
                i += 1;
            }
            b'=' if b.get(i + 1) == Some(&b'>') => {
                toks.push("=>".into());
                i += 2;
            }
            b'"' => {
                let mut out = String::new();
                let mut j = i + 1;
                loop {
                    match b.get(j) {
                        None => return Err("unterminated string".into()),
                        Some(b'"') => break,
                        Some(b'\\') => {
                            let Some(&e) = b.get(j + 1) else {
                                return Err("unterminated string".into());
                            };
                            out.push(match e {
                                b'n' => '\n',
                                b't' => '\t',
                                other => other as char,
                            });
                            j += 2;
                        }
                        Some(_) => {
                            let ch = line[j..].chars().next().unwrap_or('\u{fffd}');
                            out.push(ch);
                            j += ch.len_utf8();
                        }
                    }
                }
                toks.push(out);
                i = j + 1;
            }
            b'`' => {
                let Some(end) = line[i + 1..].find('`') else {
                    return Err("unterminated raw string".into());
                };
                toks.push(line[i + 1..i + 1 + end].to_owned());
                i += end + 2;
            }
            _ => {
                let start = i;
                while i < b.len()
                    && !matches!(b[i], b' ' | b'\t' | b'\r' | b'(' | b')' | b'"' | b'`')
                    && !(b[i] == b'/' && b.get(i + 1) == Some(&b'/'))
                    && !(b[i] == b'=' && b.get(i + 1) == Some(&b'>'))
                {
                    i += 1;
                }
                toks.push(line[start..i].to_owned());
            }
        }
    }
    Ok((toks, None))
}

/// One logical directive line: the verb (from the line or the enclosing block),
/// its arguments, the trailing comment and the 1-based line number.
struct Directive<'a> {
    verb: String,
    args: Vec<String>,
    comment: Option<&'a str>,
    line: usize,
}

/// Splits a `go.mod`/`go.work` file into directives, expanding `verb ( ... )`
/// blocks.
fn directives(src: &str) -> Result<Vec<Directive<'_>>, (usize, String)> {
    let mut out = Vec::new();
    let mut block: Option<String> = None;
    for (n, line) in src.lines().enumerate() {
        let lineno = n + 1;
        let (toks, comment) = tokenize(line).map_err(|e| (lineno, e))?;
        if toks.is_empty() {
            continue;
        }
        if let Some(verb) = &block {
            if toks.len() == 1 && toks[0] == ")" {
                block = None;
                continue;
            }
            out.push(Directive {
                verb: verb.clone(),
                args: toks,
                comment,
                line: lineno,
            });
            continue;
        }
        if toks.len() == 2 && toks[1] == "(" {
            block = Some(toks[0].clone());
            continue;
        }
        // `verb ( )` on one line, or a one-line block `verb ( a b )`.
        if toks.len() >= 3 && toks[1] == "(" && toks.last().is_some_and(|t| t == ")") {
            if toks.len() > 3 {
                out.push(Directive {
                    verb: toks[0].clone(),
                    args: toks[2..toks.len() - 1].to_vec(),
                    comment,
                    line: lineno,
                });
            }
            continue;
        }
        let mut it = toks.into_iter();
        let verb = it.next().unwrap_or_default();
        out.push(Directive {
            verb,
            args: it.collect(),
            comment,
            line: lineno,
        });
    }
    if block.is_some() {
        return Err((src.lines().count(), "unterminated block".into()));
    }
    Ok(out)
}

fn syntax(line: usize, message: impl Into<String>) -> crate::Error {
    crate::Error::Syntax {
        format: LockfileFormat::Go,
        line,
        message: message.into(),
    }
}

fn parse_replace(d: &Directive<'_>) -> crate::Result<GoReplace> {
    let a = &d.args;
    let arrow = a
        .iter()
        .position(|t| t == "=>")
        .ok_or_else(|| syntax(d.line, "replace without =>"))?;
    let (lhs, rhs) = (&a[..arrow], &a[arrow + 1..]);
    let (old, old_version) = match lhs {
        [p] => (p.clone(), None),
        [p, v] => (p.clone(), Some(v.clone())),
        _ => return Err(syntax(d.line, "replace: expected `path [version] =>`")),
    };
    let new = match rhs {
        [p] if is_local_path(p) => ReplaceTarget::Local(p.clone()),
        [p] => {
            return Err(syntax(
                d.line,
                format!("replace: module replacement {p} needs a version"),
            ));
        }
        [p, v] => ReplaceTarget::Module {
            path: p.clone(),
            version: v.clone(),
        },
        _ => return Err(syntax(d.line, "replace: expected `=> path [version]`")),
    };
    Ok(GoReplace {
        old,
        old_version,
        new,
    })
}

fn is_indirect(comment: Option<&str>) -> bool {
    comment.is_some_and(|c| c.split(';').any(|p| p.trim() == "indirect"))
}

/// Parses a `go.mod` file.
pub fn parse_go_mod(src: &str) -> crate::Result<GoMod> {
    let mut m = GoMod::default();
    for d in directives(src).map_err(|(l, e)| syntax(l, e))? {
        match d.verb.as_str() {
            "module" => {
                m.module = Some(
                    d.args
                        .first()
                        .cloned()
                        .ok_or_else(|| syntax(d.line, "module without path"))?,
                );
            }
            "go" => m.go = d.args.first().cloned(),
            "toolchain" => m.toolchain = d.args.first().cloned(),
            "require" => match d.args.as_slice() {
                [p, v] => m.requires.push(GoRequire {
                    path: p.clone(),
                    version: v.clone(),
                    indirect: is_indirect(d.comment),
                }),
                _ => return Err(syntax(d.line, "require: expected `path version`")),
            },
            "exclude" => match d.args.as_slice() {
                [p, v] => m.excludes.push((p.clone(), v.clone())),
                _ => return Err(syntax(d.line, "exclude: expected `path version`")),
            },
            "replace" => m.replaces.push(parse_replace(&d)?),
            "tool" => m.tools.extend(d.args.first().cloned()),
            // retract, godebug, ignore and future directives do not affect the build list.
            _ => {}
        }
    }
    Ok(m)
}

/// Parses a `go.work` file.
pub fn parse_go_work(src: &str) -> crate::Result<GoWork> {
    let mut w = GoWork::default();
    for d in directives(src).map_err(|(l, e)| syntax(l, e))? {
        match d.verb.as_str() {
            "go" => w.go = d.args.first().cloned(),
            "use" => w.uses.extend(d.args.first().cloned()),
            "replace" => w.replaces.push(parse_replace(&d)?),
            _ => {}
        }
    }
    Ok(w)
}

/// Parses a `go.sum` file. Lines that are not `path version[/go.mod] hash` are
/// syntax errors; blank lines are skipped.
pub fn parse_go_sum(src: &str) -> crate::Result<GoSum> {
    let mut s = GoSum::default();
    for (n, line) in src.lines().enumerate() {
        let mut f = line.split_ascii_whitespace();
        let (Some(path), Some(ver), Some(hash)) = (f.next(), f.next(), f.next()) else {
            if line.trim().is_empty() {
                continue;
            }
            return Err(syntax(n + 1, "go.sum: expected `path version hash`"));
        };
        let (version, is_mod) = match ver.strip_suffix("/go.mod") {
            Some(v) => (v, true),
            None => (ver, false),
        };
        let e = s
            .entries
            .entry((path.to_owned(), version.to_owned()))
            .or_default();
        if is_mod {
            e.go_mod_h1 = Some(hash.to_owned());
        } else {
            e.h1 = Some(hash.to_owned());
        }
    }
    Ok(s)
}

/// Normalizes a local replacement directory into an importer path (`./a/b/` →
/// `a/b`).
fn local_importer_path(dir: &str) -> String {
    crate::model::normalize_importer_path(dir.trim_end_matches('/'))
}

/// The neutral model of one main module: its requirements resolved through
/// `replace` directives and `go.sum` hashes. `go_sum` may be absent (no
/// hashes).
pub fn to_lockfile(go_mod: &GoMod, go_sum: Option<&GoSum>) -> Lockfile {
    let mut root = Importer::new(".");
    root.name = go_mod.module.clone();
    let mut packages: BTreeMap<String, LockedPackage> = BTreeMap::new();
    let mut importers = Vec::new();
    for r in &go_mod.requires {
        if go_mod.is_excluded(&r.path, &r.version) {
            continue;
        }
        let target = match go_mod.replacement(&r.path, &r.version) {
            Some(ReplaceTarget::Local(dir)) => {
                let path = local_importer_path(dir);
                if !importers.iter().any(|i: &Importer| i.path == path) {
                    let mut imp = Importer::new(&path);
                    imp.name = Some(r.path.clone());
                    importers.push(imp);
                }
                Target::Importer(path)
            }
            repl => {
                let id = format!("{}@{}", r.path, r.version);
                let mut p = LockedPackage::new(id.clone(), r.path.clone(), r.version.clone());
                match repl {
                    Some(ReplaceTarget::Module { path, version }) => {
                        p.resolution = Some(format!("{path}@{version}"));
                        p.integrity = go_sum.and_then(|s| s.integrity(path, version));
                    }
                    _ => p.integrity = go_sum.and_then(|s| s.integrity(&r.path, &r.version)),
                }
                packages.insert(id.clone(), p);
                Target::Package(id)
            }
        };
        root.dependencies.push(DepEdge {
            name: r.path.clone(),
            kind: DepKind::Prod,
            target,
            specifier: Some(r.version.clone()),
        });
    }
    importers.push(root);
    Lockfile::from_parts(
        LockfileFormat::Go,
        go_mod.go.clone().unwrap_or_default(),
        packages.into_values().collect(),
        importers,
        LockfileMeta::default(),
    )
}

/// The model of a `go.sum` on its own: every module version it lists (with its
/// zip hash, else its `go.mod` hash), no importers. Useful when only `go.sum`
/// changed or `go.mod` is missing; it over-approximates the build list
/// (`go.sum` also lists versions MVS did not select).
pub fn go_sum_lockfile(go_sum: &GoSum) -> Lockfile {
    let packages = go_sum
        .entries
        .iter()
        .map(|((path, version), e)| {
            let mut p = LockedPackage::new(format!("{path}@{version}"), path, version);
            p.integrity = e.h1.clone().or_else(|| e.go_mod_h1.clone());
            p
        })
        .collect();
    Lockfile::from_parts(
        LockfileFormat::Go,
        "",
        packages,
        Vec::new(),
        LockfileMeta::default(),
    )
}

/// Parses a `go.mod` and its optional `go.sum` into the neutral model
/// ([`to_lockfile`]).
pub fn parse(go_mod: &str, go_sum: Option<&str>) -> crate::Result<Lockfile> {
    let m = parse_go_mod(go_mod)?;
    let s = go_sum.map(parse_go_sum).transpose()?;
    Ok(to_lockfile(&m, s.as_ref()))
}

/// Compares two semantic versions as Go does (`v1.2.3`, prereleases before
/// releases, `+incompatible` and build metadata ignored). Non-semver strings
/// sort before semver ones and among themselves by bytes.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn parse(v: &str) -> Option<(Vec<u64>, Option<&str>)> {
        let v = v.strip_prefix('v')?;
        let v = v.split('+').next().unwrap_or(v);
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let nums: Option<Vec<u64>> = core.split('.').map(|n| n.parse().ok()).collect();
        Some((nums?, pre))
    }
    fn cmp_pre(a: &str, b: &str) -> Ordering {
        for (x, y) in a.split('.').zip(b.split('.')) {
            let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                (Ok(i), Ok(j)) => i.cmp(&j),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => x.cmp(y),
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        a.split('.').count().cmp(&b.split('.').count())
    }
    match (parse(a), parse(b)) {
        (Some((na, pa)), Some((nb, pb))) => na.cmp(&nb).then_with(|| match (pa, pb) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => cmp_pre(x, y),
        }),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff;

    const MOD: &str = r#"
// A comment.
module example.com/app

go 1.22.0

toolchain go1.22.4

require (
	github.com/a/b v1.2.3
	github.com/c/d v0.1.0 // indirect
	"example.com/quoted" v1.0.0
)

require example.com/local v0.0.0

replace example.com/local => ./local/
replace github.com/c/d v0.1.0 => github.com/fork/d v0.1.1
exclude github.com/a/b v1.2.2
retract [v0.9.0, v0.9.5]
godebug default=go1.21
"#;

    const SUM: &str = "github.com/a/b v1.2.3 h1:AAA=\ngithub.com/a/b v1.2.3/go.mod \
                       h1:AAM=\ngithub.com/fork/d v0.1.1 h1:FFF=\ngithub.com/fork/d v0.1.1/go.mod \
                       h1:FFM=\nexample.com/quoted v1.0.0/go.mod h1:QQM=\ngithub.com/old/x \
                       v0.0.1/go.mod h1:XXM=\n";

    #[test]
    fn parses_go_mod() {
        let m = parse_go_mod(MOD).unwrap();
        assert_eq!(m.module.as_deref(), Some("example.com/app"));
        assert_eq!(m.go.as_deref(), Some("1.22.0"));
        assert_eq!(m.toolchain.as_deref(), Some("go1.22.4"));
        assert_eq!(m.requires.len(), 4);
        assert!(m.requires[1].indirect);
        assert!(!m.requires[0].indirect);
        assert_eq!(m.requires[2].path, "example.com/quoted");
        assert_eq!(
            m.replacement("example.com/local", "v0.0.0"),
            Some(&ReplaceTarget::Local("./local/".into()))
        );
        assert_eq!(
            m.replacement("github.com/c/d", "v0.1.0"),
            Some(&ReplaceTarget::Module {
                path: "github.com/fork/d".into(),
                version: "v0.1.1".into()
            })
        );
        assert_eq!(m.replacement("github.com/c/d", "v0.2.0"), None);
        assert!(m.is_excluded("github.com/a/b", "v1.2.2"));
        assert_eq!(m.required_version("github.com/a/b"), Some("v1.2.3"));
    }

    #[test]
    fn version_specific_replace_wins() {
        let m = parse_go_mod(
            "module m\nreplace x/y v1.0.0 => ./one\nreplace x/y => ./all\nreplace (\n x/z => x/w \
             v2.0.0\n)\n",
        )
        .unwrap();
        assert_eq!(
            m.replacement("x/y", "v1.0.0"),
            Some(&ReplaceTarget::Local("./one".into()))
        );
        assert_eq!(
            m.replacement("x/y", "v1.1.0"),
            Some(&ReplaceTarget::Local("./all".into()))
        );
        assert!(matches!(
            m.replacement("x/z", "v0"),
            Some(ReplaceTarget::Module { .. })
        ));
    }

    #[test]
    fn syntax_errors() {
        assert!(parse_go_mod("module m\nrequire x\n").is_err());
        assert!(parse_go_mod("module m\nreplace x => y\n").is_err());
        assert!(parse_go_mod("require (\n x v1\n").is_err());
        assert!(parse_go_mod("module \"m\n").is_err());
        assert!(parse_go_sum("x\n").is_err());
        // Unknown directives are fine.
        assert!(parse_go_mod("module m\nfrobnicate a b c\nignore ./x\n").is_ok());
    }

    #[test]
    fn parses_go_work() {
        let w = parse_go_work(
            "go 1.22\nuse (\n\t./a\n\t./b // comment\n)\nuse ./c\nreplace x => ../x\n",
        )
        .unwrap();
        assert_eq!(w.uses, ["./a", "./b", "./c"]);
        assert_eq!(w.replaces.len(), 1);
        assert_eq!(w.go.as_deref(), Some("1.22"));
    }

    #[test]
    fn parses_go_sum() {
        let s = parse_go_sum(SUM).unwrap();
        let e = s.get("github.com/a/b", "v1.2.3").unwrap();
        assert_eq!(e.h1.as_deref(), Some("h1:AAA="));
        assert_eq!(e.go_mod_h1.as_deref(), Some("h1:AAM="));
        assert_eq!(
            s.integrity("example.com/quoted", "v1.0.0").as_deref(),
            Some("h1:QQM=")
        );
    }

    #[test]
    fn model_mapping() {
        let lock = parse(MOD, Some(SUM)).unwrap();
        assert_eq!(lock.format(), LockfileFormat::Go);
        assert_eq!(lock.lockfile_version(), "1.22.0");
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "example.com/quoted@v1.0.0",
                "github.com/a/b@v1.2.3",
                "github.com/c/d@v0.1.0"
            ]
        );
        let d = lock.package("github.com/c/d@v0.1.0").unwrap();
        assert_eq!(d.resolution.as_deref(), Some("github.com/fork/d@v0.1.1"));
        assert_eq!(d.integrity.as_deref(), Some("h1:FFF="));
        let root = lock.importer(".").unwrap();
        assert_eq!(root.name.as_deref(), Some("example.com/app"));
        assert_eq!(
            root.dependency("example.com/local").unwrap().target,
            Target::Importer("local".into())
        );
        assert!(lock.importer("local").is_some());
    }

    #[test]
    fn diff_reports_module_bumps_as_identity_changes() {
        let old = parse(MOD, Some(SUM)).unwrap();
        let new_mod = MOD.replace("github.com/a/b v1.2.3", "github.com/a/b v1.3.0");
        let new_sum = format!("{SUM}github.com/a/b v1.3.0 h1:NEW=\n");
        let new = parse(&new_mod, Some(&new_sum)).unwrap();
        let d = diff(&old, &new);
        assert!(
            d.affected_packages()
                .identity_changed
                .contains("github.com/a/b@v1.3.0")
        );
        assert_eq!(d.affected_packages().identity_changed.len(), 1);
        // Changing a replacement target is an identity change of the replaced module.
        let new_mod = MOD.replace("github.com/fork/d v0.1.1", "github.com/fork/d v0.1.2");
        let new_sum = format!("{SUM}github.com/fork/d v0.1.2 h1:F2=\n");
        let d = diff(&old, &parse(&new_mod, Some(&new_sum)).unwrap());
        assert!(
            d.affected_packages()
                .identity_changed
                .contains("github.com/c/d@v0.1.0")
        );
        // A go.sum hash change alone (tampering, re-tag) is an identity change too.
        let d = diff(&old, &parse(MOD, Some(&SUM.replace("AAA", "BBB"))).unwrap());
        assert!(
            d.affected_packages()
                .identity_changed
                .contains("github.com/a/b@v1.2.3")
        );
        // Reformatting is not a change.
        let d = diff(&old, &parse(&MOD.replace('\t', "    "), Some(SUM)).unwrap());
        assert!(d.affected_packages().identity_changed.is_empty());
    }

    #[test]
    fn go_sum_only_model() {
        let lock = go_sum_lockfile(&parse_go_sum(SUM).unwrap());
        assert_eq!(lock.packages().len(), 4);
        assert_eq!(
            lock.package("github.com/old/x@v0.0.1")
                .unwrap()
                .integrity
                .as_deref(),
            Some("h1:XXM=")
        );
    }

    #[test]
    fn semver_order() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("v1.2.3", "v1.10.0"), Less);
        assert_eq!(compare_versions("v1.0.0-rc.1", "v1.0.0"), Less);
        assert_eq!(compare_versions("v1.0.0-rc.2", "v1.0.0-rc.10"), Less);
        assert_eq!(
            compare_versions(
                "v0.0.0-20210101000000-abcdef",
                "v0.0.0-20220101000000-abcdef"
            ),
            Less
        );
        assert_eq!(compare_versions("v2.0.0+incompatible", "v2.0.0"), Equal);
        assert_eq!(compare_versions("junk", "v0.0.1"), Less);
    }
}
