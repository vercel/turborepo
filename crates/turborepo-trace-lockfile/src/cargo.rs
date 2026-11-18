//! Cargo `Cargo.lock` (lockfile `version = 3` and `4`, plus the v1/v2
//! `[metadata]` checksum table) parsed into the neutral [`Lockfile`] model.
//!
//! # Model mapping
//!
//! | Cargo.lock | Model |
//! |---|---|
//! | `[[package]]` with a `source` (registry, git, alternative registry) | [`LockedPackage`] with id `name@version` (`name@version (source)` when the same name and version come from several sources), `integrity` = `checksum`, `resolution` = `source` |
//! | `[[package]]` without a `source` (workspace members and path dependencies) | [`Importer`] whose path is the package name (`name@version` if two path packages share a name), named by the package name: Cargo.lock records no directories |
//! | `dependencies = ["name", "name version", "name version (source)"]` | [`DepEdge`]s of kind [`DepKind::Prod`] to the package or importer the entry names; Cargo.lock does not distinguish normal, dev and build dependencies |
//!
//! A git dependency's `source` ends in `#<commit>`, so a moved branch is a
//! changed resolution, and therefore a changed identity, in [`crate::diff`]
//! even though git packages have no checksum.
//!
//! The parser reads the TOML subset cargo writes (`[[package]]` tables of
//! string keys and string arrays, `version = N`, `[metadata]`); other tables
//! (`[[patch.unused]]`) are skipped.

use std::collections::BTreeMap;

use crate::{
    Error, Result,
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta, Target,
    },
};

/// One raw `[[package]]` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CargoPackage {
    /// `name`.
    pub name: String,
    /// `version`.
    pub version: String,
    /// `source`, absent for workspace members and path dependencies.
    pub source: Option<String>,
    /// `checksum` (or the `[metadata]` checksum of v1/v2 lockfiles).
    pub checksum: Option<String>,
    /// `dependencies` entries as written.
    pub dependencies: Vec<String>,
}

/// A parsed `Cargo.lock` before mapping into the model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CargoLock {
    /// `version = N`; `1` when absent (v1 and v2 lockfiles have no version key;
    /// v2 is told apart by inline checksums).
    pub version: u32,
    /// Packages in file order.
    pub packages: Vec<CargoPackage>,
}

fn syntax(line: usize, message: impl Into<String>) -> Error {
    Error::Syntax {
        format: LockfileFormat::Cargo,
        line,
        message: message.into(),
    }
}

/// Parses a TOML basic string starting at `s[0] == '"'`. Returns the value and
/// the rest.
fn toml_string(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' | 'U' => {
                    let digits: String = chars.by_ref().take(4).map(|(_, c)| c).collect();
                    out.push(
                        u32::from_str_radix(&digits, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .unwrap_or('\u{fffd}'),
                    );
                }
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
    None
}

/// Collects the quoted strings of an array body (possibly spanning lines).
fn array_items(mut s: &str, out: &mut Vec<String>) -> bool {
    loop {
        s = s.trim_start_matches([' ', '\t', ',', '\r']);
        if s.is_empty() || s.starts_with('#') {
            return false;
        }
        if s.starts_with(']') {
            return true;
        }
        match toml_string(s) {
            Some((v, rest)) => {
                out.push(v);
                s = rest;
            }
            None => return false,
        }
    }
}

/// Parses the raw `Cargo.lock` structure.
// Invariant: the package section is entered only after a package is pushed.
#[allow(clippy::expect_used)]
pub fn parse_raw(contents: &str) -> Result<CargoLock> {
    #[derive(PartialEq)]
    enum Section {
        Root,
        Package,
        Metadata,
        Other,
    }
    let mut lock = CargoLock {
        version: 1,
        packages: Vec::new(),
    };
    let mut metadata: BTreeMap<String, String> = BTreeMap::new();
    let mut section = Section::Root;
    let mut in_array: Option<usize> = None;
    let mut saw_version = false;
    for (n, raw) in contents.lines().enumerate() {
        let line_no = n + 1;
        let line = raw.trim();
        if let Some(start) = in_array {
            let Some(p) = lock.packages.last_mut() else {
                return Err(syntax(start, "array outside a package"));
            };
            if array_items(line, &mut p.dependencies) || line.starts_with(']') {
                in_array = None;
            }
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = match line {
                "[[package]]" => {
                    lock.packages.push(CargoPackage::default());
                    Section::Package
                }
                "[metadata]" => Section::Metadata,
                _ => Section::Other,
            };
            continue;
        }
        if section == Section::Other {
            continue;
        }
        // key = value
        let (key, value) = if line.starts_with('"') {
            let (k, rest) = toml_string(line).ok_or_else(|| syntax(line_no, "unterminated key"))?;
            let rest = rest.trim_start();
            let Some(v) = rest.strip_prefix('=') else {
                return Err(syntax(line_no, "expected `=`"));
            };
            (k, v.trim())
        } else {
            let Some((k, v)) = line.split_once('=') else {
                return Err(syntax(
                    line_no,
                    format!("expected `key = value`, got {line:?}"),
                ));
            };
            (k.trim().to_owned(), v.trim())
        };
        match section {
            Section::Root => {
                if key == "version" {
                    lock.version = value
                        .parse()
                        .map_err(|_| syntax(line_no, format!("bad version {value:?}")))?;
                    saw_version = true;
                }
            }
            Section::Metadata => {
                if let Some((v, _)) = toml_string(value)
                    && let Some(k) = key.strip_prefix("checksum ")
                {
                    metadata.insert(k.to_owned(), v);
                }
            }
            Section::Package => {
                let p = lock.packages.last_mut().expect("in a package");
                if let Some(body) = value.strip_prefix('[') {
                    if key == "dependencies" && !array_items(body, &mut p.dependencies) {
                        in_array = Some(line_no);
                    } else if key != "dependencies" && !body.trim_end().ends_with(']') {
                        // An unknown multi-line array: skip it.
                        in_array = None;
                    }
                    continue;
                }
                let Some((v, _)) = toml_string(value) else {
                    continue;
                };
                match key.as_str() {
                    "name" => p.name = v,
                    "version" => p.version = v,
                    "source" => p.source = Some(v),
                    "checksum" => p.checksum = Some(v),
                    _ => {}
                }
            }
            Section::Other => {}
        }
    }
    if in_array.is_some() {
        return Err(syntax(contents.lines().count(), "unterminated array"));
    }
    if !saw_version {
        lock.version = if lock.packages.iter().any(|p| p.checksum.is_some()) {
            2
        } else {
            1
        };
    }
    if lock.version > 4 {
        return Err(Error::UnsupportedVersion {
            format: LockfileFormat::Cargo,
            version: lock.version.to_string(),
        });
    }
    for p in &mut lock.packages {
        if p.name.is_empty() || p.version.is_empty() {
            return Err(syntax(0, "package without name or version"));
        }
        if p.checksum.is_none()
            && let Some(src) = &p.source
        {
            p.checksum = metadata
                .get(&format!("{} {} ({src})", p.name, p.version))
                .cloned();
        }
    }
    Ok(lock)
}

/// Parses a `Cargo.lock` into the neutral model (see the module docs).
pub fn parse(contents: &str) -> Result<Lockfile> {
    Ok(to_lockfile(&parse_raw(contents)?))
}

/// The model id of each package: `Package(id)` or `Importer(path)`.
fn ids(lock: &CargoLock) -> Vec<Target> {
    let mut count: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut path_names: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &lock.packages {
        if p.source.is_some() {
            *count.entry((&p.name, &p.version)).or_default() += 1;
        } else {
            *path_names.entry(&p.name).or_default() += 1;
        }
    }
    lock.packages
        .iter()
        .map(|p| match &p.source {
            Some(src) => {
                if count[&(p.name.as_str(), p.version.as_str())] > 1 {
                    Target::Package(format!("{}@{} ({src})", p.name, p.version))
                } else {
                    Target::Package(format!("{}@{}", p.name, p.version))
                }
            }
            None => {
                if path_names[p.name.as_str()] > 1 {
                    Target::Importer(format!("{}@{}", p.name, p.version))
                } else {
                    Target::Importer(p.name.clone())
                }
            }
        })
        .collect()
}

/// Finds the package a `dependencies` entry (`name`, `name version`,
/// `name version (source)`) refers to.
fn find_dep(lock: &CargoLock, entry: &str) -> Option<usize> {
    let (head, source) = match entry.split_once(" (") {
        Some((h, s)) => (h, Some(s.trim_end_matches(')'))),
        None => (entry, None),
    };
    let mut it = head.split_whitespace();
    let name = it.next()?;
    let version = it.next();
    lock.packages.iter().position(|p| {
        p.name == name
            && version.is_none_or(|v| p.version == v)
            && source.is_none_or(|s| p.source.as_deref() == Some(s))
    })
}

/// Maps a raw lockfile into the model.
pub fn to_lockfile(lock: &CargoLock) -> Lockfile {
    let ids = ids(lock);
    let mut packages = Vec::new();
    let mut importers = Vec::new();
    for (i, p) in lock.packages.iter().enumerate() {
        let deps: Vec<DepEdge> = p
            .dependencies
            .iter()
            .map(|e| {
                let target = match find_dep(lock, e) {
                    Some(j) => ids[j].clone(),
                    None => Target::Unresolved(e.clone()),
                };
                let name = e.split_whitespace().next().unwrap_or(e).to_owned();
                DepEdge {
                    name,
                    kind: DepKind::Prod,
                    target,
                    specifier: None,
                }
            })
            .collect();
        match &ids[i] {
            Target::Package(id) => {
                let mut lp = LockedPackage::new(id.clone(), p.name.clone(), p.version.clone());
                lp.integrity = p.checksum.clone();
                lp.resolution = p.source.clone();
                lp.dependencies = deps;
                packages.push(lp);
            }
            Target::Importer(path) => {
                let mut imp = Importer::new(path);
                imp.name = Some(p.name.clone());
                imp.dependencies = deps;
                importers.push(imp);
            }
            Target::Unresolved(_) => {}
        }
    }
    Lockfile::from_parts(
        LockfileFormat::Cargo,
        lock.version.to_string(),
        packages,
        importers,
        LockfileMeta::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Resolved;

    const V4: &str = r#"# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "app"
version = "0.1.0"
dependencies = [
 "memchr 2.7.4",
 "memchr 2.8.0",
 "util",
 "gitdep",
]

[[package]]
name = "gitdep"
version = "0.2.0"
source = "git+https://github.com/x/gitdep?branch=main#0123456789abcdef"

[[package]]
name = "memchr"
version = "2.7.4"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaa"

[[package]]
name = "memchr"
version = "2.8.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bbb"

[[package]]
name = "util"
version = "0.1.0"
dependencies = ["memchr 2.8.0"]

[[patch.unused]]
name = "ignored"
version = "1.0.0"
"#;

    #[test]
    fn parses_v4() {
        let lock = parse(V4).unwrap();
        assert_eq!(lock.format(), LockfileFormat::Cargo);
        assert_eq!(lock.lockfile_version(), "4");
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["gitdep@0.2.0", "memchr@2.7.4", "memchr@2.8.0"]);
        let m = lock.package("memchr@2.8.0").unwrap();
        assert_eq!(m.integrity.as_deref(), Some("bbb"));
        assert!(m.resolution.as_deref().unwrap().starts_with("registry+"));
        let imps: Vec<_> = lock.importers().iter().map(|i| i.path.as_str()).collect();
        assert_eq!(imps, ["app", "util"]);
        match lock.resolve("app", "util") {
            Some(Resolved::Importer(i)) => assert_eq!(i.path, "util"),
            other => panic!("{other:?}"),
        }
        match lock.resolve("util", "memchr") {
            Some(Resolved::Package(p)) => assert_eq!(p.version, "2.8.0"),
            other => panic!("{other:?}"),
        }
        let closure: Vec<_> = lock
            .importer_closure("app", false)
            .iter()
            .map(|p| p.id.clone())
            .collect();
        assert_eq!(closure, ["gitdep@0.2.0", "memchr@2.7.4", "memchr@2.8.0"]);
    }

    #[test]
    fn v1_metadata_checksums_and_duplicate_sources() {
        let src = r#"[[package]]
name = "a"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "a"
version = "1.0.0"
source = "registry+https://example.com/index"

[[package]]
name = "root"
version = "0.0.1"
dependencies = [
 "a 1.0.0 (registry+https://example.com/index)",
]

[metadata]
"checksum a 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)" = "c1"
"#;
        let raw = parse_raw(src).unwrap();
        assert_eq!(raw.version, 1);
        assert_eq!(raw.packages[0].checksum.as_deref(), Some("c1"));
        let lock = to_lockfile(&raw);
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.clone()).collect();
        assert_eq!(
            ids,
            [
                "a@1.0.0 (registry+https://example.com/index)",
                "a@1.0.0 (registry+https://github.com/rust-lang/crates.io-index)"
            ]
        );
        match lock.resolve("root", "a") {
            Some(Resolved::Package(p)) => {
                assert_eq!(
                    p.resolution.as_deref(),
                    Some("registry+https://example.com/index")
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn errors() {
        assert!(parse("version = 9\n").is_err());
        assert!(parse("[[package]]\nname = \"a\"\n").is_err());
        assert!(
            parse("[[package]]\nname = \"a\"\nversion = \"1\"\ndependencies = [\n \"b\",\n")
                .is_err()
        );
        assert!(parse("garbage line\n").is_err());
    }

    #[test]
    fn diff_reports_bumps_and_git_moves() {
        let new = V4
            .replace("checksum = \"bbb\"", "checksum = \"ccc\"")
            .replace("#0123456789abcdef", "#fedcba9876543210");
        let d = crate::diff(&parse(V4).unwrap(), &parse(&new).unwrap());
        let changed: Vec<_> = d.changed.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(changed, ["gitdep@0.2.0", "memchr@2.8.0"]);
    }
}
