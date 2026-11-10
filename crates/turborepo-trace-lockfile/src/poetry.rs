//! Poetry `poetry.lock` (`lock-version` 1.x and 2.x) parsed into the neutral
//! [`Lockfile`] model.
//!
//! # Model mapping
//!
//! | poetry.lock | Model |
//! |---|---|
//! | `[[package]]` | [`LockedPackage`] with id `name@version` (canonical name, PEP 503; `name@version (source)` when several sources lock the same version), `integrity` = the sorted `files` hashes (or lock-version 1's `[metadata.files]`) joined by spaces, `resolution` = `type+url[#resolved_reference]` for git, url, file and directory sources |
//! | `optional = true` | [`PackageFlags::OPTIONAL`] |
//! | `category = "dev"` (1.x), or `groups` without `"main"` (2.x) | [`PackageFlags::DEV`] |
//! | `[package.dependencies]` (string, table or list of tables) | [`DepKind::Prod`] edges ([`DepKind::Optional`] for `optional = true`), specifier = version and markers |
//! | the project | the root [`Importer`] `"."`, whose dependencies are the packages nothing else depends on |
//!
//! poetry.lock does not record the project's own dependency declarations, so
//! the root importer's edges are derived from the graph's roots (kind from the
//! package flags): a direct dependency that is also a transitive one is not
//! listed. When a dependency matches several locked versions (marker-split
//! locks), the edge targets the version its specifier pins, else
//! it stays unresolved.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Error, Result,
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta,
        PackageFlags, Target,
    },
    pytoml::{self, Val},
    requirements::canonicalize_name,
};

fn source_resolution(v: &Val) -> Option<String> {
    let s = v.get("source")?;
    let kind = s.str("type")?;
    if matches!(kind, "legacy" | "pypi" | "") {
        return None;
    }
    let url = s.str("url").unwrap_or("");
    Some(
        match s.str("resolved_reference").or_else(|| s.str("reference")) {
            Some(r) if kind == "git" => format!("{kind}+{url}#{r}"),
            _ => format!("{kind}+{url}"),
        },
    )
}

fn hashes(files: &[Val]) -> Option<String> {
    let mut h: Vec<&str> = files.iter().filter_map(|f| f.str("hash")).collect();
    h.sort_unstable();
    h.dedup();
    (!h.is_empty()).then(|| h.join(" "))
}

/// Parses `poetry.lock`.
pub fn parse(contents: &str) -> Result<Lockfile> {
    let doc = pytoml::parse(contents, LockfileFormat::Poetry)?;
    let meta = doc.get("metadata");
    let version = meta
        .and_then(|m| m.scalar("lock-version"))
        .unwrap_or("1.0")
        .to_owned();
    if !(version.starts_with("1.") || version.starts_with("2.")) {
        return Err(Error::UnsupportedVersion {
            format: LockfileFormat::Poetry,
            version,
        });
    }
    let legacy_files = meta.and_then(|m| m.get("files"));
    struct Pkg<'a> {
        canon: String,
        version: String,
        resolution: Option<String>,
        v: &'a Val,
    }
    let pkgs: Vec<Pkg<'_>> = doc
        .arr("package")
        .iter()
        .filter_map(|v| {
            let name = v.str("name")?;
            Some(Pkg {
                canon: canonicalize_name(name),
                version: v.scalar("version").unwrap_or("").to_owned(),
                resolution: source_resolution(v),
                v,
            })
        })
        .collect();
    let mut count: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for p in &pkgs {
        *count.entry((&p.canon, &p.version)).or_default() += 1;
    }
    let ids: Vec<String> = pkgs
        .iter()
        .map(|p| {
            let base = format!("{}@{}", p.canon, p.version);
            match &p.resolution {
                Some(r) if count[&(p.canon.as_str(), p.version.as_str())] > 1 => {
                    format!("{base} ({r})")
                }
                _ => base,
            }
        })
        .collect();
    let find = |canon: &str, spec: &str| -> Target {
        let cands: Vec<usize> = (0..pkgs.len())
            .filter(|&i| pkgs[i].canon == canon)
            .collect();
        if cands.len() == 1 {
            return Target::Package(ids[cands[0]].clone());
        }
        let pinned = spec.trim().trim_start_matches("==").trim();
        let exact: Vec<usize> = cands
            .iter()
            .copied()
            .filter(|&i| pkgs[i].version == pinned)
            .collect();
        match exact.as_slice() {
            [i] => Target::Package(ids[*i].clone()),
            _ => Target::Unresolved(canon.to_owned()),
        }
    };
    let mut packages = Vec::new();
    let mut has_parent: BTreeSet<String> = BTreeSet::new();
    for (i, p) in pkgs.iter().enumerate() {
        let mut lp = LockedPackage::new(ids[i].clone(), p.canon.clone(), p.version.clone());
        let files = match p.v.get("files") {
            Some(Val::Arr(f)) => f.as_slice(),
            _ => legacy_files
                .and_then(|f| {
                    f.table()
                        .iter()
                        .find(|(k, _)| canonicalize_name(k) == p.canon)
                        .map(|(_, v)| v)
                })
                .map_or(&[][..], |v| match v {
                    Val::Arr(a) => a.as_slice(),
                    _ => &[],
                }),
        };
        lp.integrity = hashes(files);
        lp.resolution = p.resolution.clone();
        if p.v.scalar("optional") == Some("true") {
            lp.flags |= PackageFlags::OPTIONAL;
        }
        let groups: Vec<&str> =
            p.v.arr("groups")
                .iter()
                .filter_map(|g| match g {
                    Val::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
        if p.v.str("category") == Some("dev") || (!groups.is_empty() && !groups.contains(&"main")) {
            lp.flags |= PackageFlags::DEV;
        }
        if let Some(deps) = p.v.get("dependencies") {
            for (name, d) in deps.table() {
                let canon = canonicalize_name(name);
                let entries: Vec<&Val> = match d {
                    Val::Arr(a) => a.iter().collect(),
                    other => vec![other],
                };
                for e in entries {
                    let (spec, optional, markers) = match e {
                        Val::Str(s) => (s.clone(), false, None),
                        Val::Table(_) => (
                            e.scalar("version").unwrap_or("*").to_owned(),
                            e.scalar("optional") == Some("true"),
                            e.str("markers"),
                        ),
                        _ => continue,
                    };
                    let target = find(&canon, &spec);
                    if let Target::Package(id) = &target {
                        has_parent.insert(id.clone());
                    }
                    let mut specifier = spec;
                    if let Some(m) = markers {
                        specifier.push_str(&format!("; {m}"));
                    }
                    lp.dependencies.push(DepEdge {
                        name: canon.clone(),
                        kind: if optional {
                            DepKind::Optional
                        } else {
                            DepKind::Prod
                        },
                        target,
                        specifier: Some(specifier),
                    });
                }
            }
        }
        packages.push(lp);
    }
    let mut root = Importer::new(".");
    for p in packages.iter().filter(|p| !has_parent.contains(&p.id)) {
        root.dependencies.push(DepEdge {
            name: p.name.clone(),
            kind: if p.flags.contains(PackageFlags::DEV) {
                DepKind::Dev
            } else if p.flags.contains(PackageFlags::OPTIONAL) {
                DepKind::Optional
            } else {
                DepKind::Prod
            },
            target: Target::Package(p.id.clone()),
            specifier: None,
        });
    }
    Ok(Lockfile::from_parts(
        LockfileFormat::Poetry,
        version,
        packages,
        vec![root],
        LockfileMeta::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Resolved, diff};

    const V2: &str = r#"# This file is automatically @generated by Poetry 2.0.1 and should not be changed by hand.

[[package]]
name = "certifi"
version = "2024.8.30"
description = "Python package for providing Mozilla's CA Bundle."
optional = false
python-versions = ">=3.6"
groups = ["main"]
files = [
    {file = "certifi-2024.8.30-py3-none-any.whl", hash = "sha256:b"},
    {file = "certifi-2024.8.30.tar.gz", hash = "sha256:a"},
]

[[package]]
name = "numpy"
version = "1.26.4"
description = ""
optional = false
python-versions = ">=3.9"
groups = ["main"]
files = []

[[package]]
name = "numpy"
version = "2.1.2"
description = ""
optional = false
python-versions = ">=3.10"
groups = ["main"]
files = []

[[package]]
name = "pytest"
version = "8.3.3"
description = "pytest: simple powerful testing with Python"
optional = false
python-versions = ">=3.8"
groups = ["dev"]
files = [
    {file = "pytest-8.3.3-py3-none-any.whl", hash = "sha256:p"},
]

[[package]]
name = "requests"
version = "2.32.3"
description = "Python HTTP for Humans."
optional = false
python-versions = ">=3.8"
groups = ["main"]
files = [
    {file = "requests-2.32.3.tar.gz", hash = "sha256:r"},
]

[package.dependencies]
certifi = ">=2017.4.17"
numpy = [
    {version = "1.26.4", markers = "python_version < \"3.10\""},
    {version = "2.1.2", markers = "python_version >= \"3.10\""},
]
PySocks = {version = ">=1.5.6,<1.5.7 || >1.5.7", optional = true}

[package.extras]
socks = ["PySocks (>=1.5.6,!=1.5.7)"]

[[package]]
name = "mylib"
version = "0.3.0"
description = ""
optional = false
python-versions = "*"
groups = ["main"]
files = []

[package.source]
type = "git"
url = "https://github.com/x/mylib.git"
reference = "main"
resolved_reference = "0123abc"

[metadata]
lock-version = "2.1"
python-versions = "^3.12"
content-hash = "deadbeef"
"#;

    const V1: &str = r#"[[package]]
name = "six"
version = "1.16.0"
description = ""
category = "dev"
optional = false
python-versions = "*"

[metadata]
lock-version = "1.1"
python-versions = "^3.8"
content-hash = "x"

[metadata.files]
six = [
    {file = "six-1.16.0.tar.gz", hash = "sha256:s"},
]
"#;

    #[test]
    fn parses_v2() {
        let lock = parse(V2).unwrap();
        assert_eq!(lock.format(), LockfileFormat::Poetry);
        assert_eq!(lock.lockfile_version(), "2.1");
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "certifi@2024.8.30",
                "mylib@0.3.0",
                "numpy@1.26.4",
                "numpy@2.1.2",
                "pytest@8.3.3",
                "requests@2.32.3"
            ]
        );
        let c = lock.package("certifi@2024.8.30").unwrap();
        assert_eq!(c.integrity.as_deref(), Some("sha256:a sha256:b"));
        assert_eq!(
            lock.package("mylib@0.3.0").unwrap().resolution.as_deref(),
            Some("git+https://github.com/x/mylib.git#0123abc")
        );
        assert!(
            lock.package("pytest@8.3.3")
                .unwrap()
                .flags
                .contains(PackageFlags::DEV)
        );
        let req = lock.package("requests@2.32.3").unwrap();
        let numpy: Vec<_> = req
            .dependencies
            .iter()
            .filter(|e| e.name == "numpy")
            .map(|e| e.target.clone())
            .collect();
        assert_eq!(
            numpy,
            [
                Target::Package("numpy@1.26.4".into()),
                Target::Package("numpy@2.1.2".into())
            ]
        );
        let socks = req
            .dependencies
            .iter()
            .find(|e| e.name == "pysocks")
            .unwrap();
        assert_eq!(socks.kind, DepKind::Optional);
        assert!(matches!(socks.target, Target::Unresolved(_)));
        // Root importer: graph roots.
        let root: Vec<_> = lock
            .importer(".")
            .unwrap()
            .dependencies
            .iter()
            .map(|e| (e.name.as_str(), e.kind))
            .collect();
        assert_eq!(
            root,
            [
                ("mylib", DepKind::Prod),
                ("pytest", DepKind::Dev),
                ("requests", DepKind::Prod)
            ]
        );
        assert!(matches!(
            lock.resolve(".", "requests"),
            Some(Resolved::Package(_))
        ));
    }

    #[test]
    fn parses_v1_metadata_files() {
        let lock = parse(V1).unwrap();
        let six = lock.package("six@1.16.0").unwrap();
        assert_eq!(six.integrity.as_deref(), Some("sha256:s"));
        assert!(six.flags.contains(PackageFlags::DEV));
    }

    #[test]
    fn diff_and_errors() {
        let old = parse(V2).unwrap();
        let new = parse(&V2.replace("0123abc", "4567def")).unwrap();
        let d = diff(&old, &new);
        let changed: Vec<_> = d.changed.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(changed, ["mylib@0.3.0"]);
        assert!(matches!(
            parse("[metadata]\nlock-version = \"3.0\"\n"),
            Err(Error::UnsupportedVersion { .. })
        ));
        assert!(parse("[[package]\n").is_err());
    }
}
