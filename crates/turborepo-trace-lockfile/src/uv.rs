//! uv `uv.lock` (lockfile `version = 1`, any `revision`) parsed into the
//! neutral [`Lockfile`] model.
//!
//! # Model mapping
//!
//! | uv.lock | Model |
//! |---|---|
//! | `[[package]]` with a `registry`, `git`, `url` or `path` (archive) source | [`LockedPackage`] with id `name@version` (`name@version (source)` when the same name and version come from several sources), `integrity` = the sdist hash (else the first wheel hash), `resolution` = the source for non-registry packages |
//! | `[[package]]` with an `editable`, `virtual` or `directory` source (the project, workspace members, local path dependencies) | [`Importer`] at that path, named by the package |
//! | `dependencies` | [`DepKind::Prod`] edges |
//! | `optional-dependencies.<extra>` | [`DepKind::Optional`] edges (named `name`, specifier `extra:<extra>`) |
//! | `dev-dependencies.<group>` | [`DepKind::Dev`] edges (specifier `group:<group>`) |
//!
//! A dependency entry names a package and, when several versions are locked,
//! its `version` and `source`; entries are matched on canonical names (PEP
//! 503). Markers are recorded in the edge specifier (`; marker`). Packages
//! reachable only through optional or dev edges are not flagged: uv records no
//! such flags, and extras and groups are install-time choices.

use std::collections::BTreeMap;

use crate::{
    Error, Result,
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta, Target,
    },
    pytoml::{self, Val},
    requirements::canonicalize_name,
};

/// Source kinds that make a package first-party (an importer).
const LOCAL_SOURCES: &[&str] = &["editable", "virtual", "directory"];

struct Pkg<'a> {
    name: String,
    canon: String,
    version: String,
    /// `(kind, value)` of the `source` table.
    source: Option<(&'a str, &'a str)>,
    v: &'a Val,
}

fn source_of(v: &Val) -> Option<(&str, &str)> {
    let t = v.get("source")?.table();
    let (k, val) = t.first()?;
    match val {
        Val::Str(s) => Some((k.as_str(), s.as_str())),
        _ => None,
    }
}

/// Parses `uv.lock`.
pub fn parse(contents: &str) -> Result<Lockfile> {
    let doc = pytoml::parse(contents, LockfileFormat::Uv)?;
    let version = doc.scalar("version").unwrap_or("1").to_owned();
    if version != "1" {
        return Err(Error::UnsupportedVersion {
            format: LockfileFormat::Uv,
            version,
        });
    }
    let pkgs: Vec<Pkg<'_>> = doc
        .arr("package")
        .iter()
        .filter_map(|v| {
            let name = v.str("name")?.to_owned();
            Some(Pkg {
                canon: canonicalize_name(&name),
                version: v.scalar("version").unwrap_or("").to_owned(),
                source: source_of(v),
                name,
                v,
            })
        })
        .collect();
    // Ids: name@version, disambiguated by source on collisions.
    let mut count: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for p in &pkgs {
        *count.entry((&p.canon, &p.version)).or_default() += 1;
    }
    let targets: Vec<Target> = pkgs
        .iter()
        .map(|p| match p.source {
            Some((k, path)) if LOCAL_SOURCES.contains(&k) => {
                Target::Importer(crate::model::normalize_importer_path(path))
            }
            _ => {
                let base = format!("{}@{}", p.canon, p.version);
                if count[&(p.canon.as_str(), p.version.as_str())] > 1
                    && let Some((k, s)) = p.source
                {
                    Target::Package(format!("{base} ({k}+{s})"))
                } else {
                    Target::Package(base)
                }
            }
        })
        .collect();
    let find = |dep: &Val| -> Target {
        let name = dep.str("name").unwrap_or("");
        let canon = canonicalize_name(name);
        let version = dep.scalar("version");
        let source = dep.get("source").and_then(|s| {
            let (k, v) = s.table().first()?;
            match v {
                Val::Str(v) => Some((k.as_str(), v.as_str())),
                _ => None,
            }
        });
        let mut found = pkgs.iter().enumerate().filter(|(_, p)| {
            p.canon == canon
                && version.is_none_or(|v| v == p.version)
                && source.is_none_or(|s| Some(s) == p.source)
        });
        match (found.next(), found.next()) {
            (Some((i, _)), None) => targets[i].clone(),
            _ => Target::Unresolved(name.to_owned()),
        }
    };
    let edge = |dep: &Val, kind: DepKind, group: Option<String>| -> DepEdge {
        let marker = dep.str("marker");
        let extra: Vec<String> = dep
            .arr("extra")
            .iter()
            .filter_map(|e| match e {
                Val::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        let mut spec = group.unwrap_or_default();
        if !extra.is_empty() {
            spec.push_str(&format!("[{}]", extra.join(",")));
        }
        if let Some(m) = marker {
            spec.push_str(&format!("; {m}"));
        }
        DepEdge {
            name: canonicalize_name(dep.str("name").unwrap_or("")),
            kind,
            target: find(dep),
            specifier: (!spec.is_empty()).then_some(spec),
        }
    };
    let mut packages = Vec::new();
    let mut importers = Vec::new();
    for (i, p) in pkgs.iter().enumerate() {
        let mut deps: Vec<DepEdge> =
            p.v.arr("dependencies")
                .iter()
                .map(|d| edge(d, DepKind::Prod, None))
                .collect();
        for (key, kind, prefix) in [
            ("optional-dependencies", DepKind::Optional, "extra"),
            ("dev-dependencies", DepKind::Dev, "group"),
        ] {
            if let Some(t) = p.v.get(key) {
                for (group, list) in t.table() {
                    if let Val::Arr(items) = list {
                        for d in items {
                            deps.push(edge(d, kind, Some(format!("{prefix}:{group}"))));
                        }
                    }
                }
            }
        }
        match &targets[i] {
            Target::Package(id) => {
                let mut lp = LockedPackage::new(id.clone(), p.canon.clone(), p.version.clone());
                lp.integrity =
                    p.v.get("sdist")
                        .and_then(|s| s.str("hash"))
                        .or_else(|| p.v.arr("wheels").first().and_then(|w| w.str("hash")))
                        .map(str::to_owned);
                lp.resolution = match p.source {
                    Some(("registry", _)) | None => None,
                    Some((k, s)) => Some(format!("{k}+{s}")),
                };
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
    let revision = doc.scalar("revision");
    Ok(Lockfile::from_parts(
        LockfileFormat::Uv,
        match revision {
            Some(r) => format!("{version}.{r}"),
            None => version,
        },
        packages,
        importers,
        LockfileMeta::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Resolved, diff};

    const LOCK: &str = r#"version = 1
revision = 2
requires-python = ">=3.12"

[manifest]
members = ["app", "lib"]

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "." }
dependencies = [
    { name = "lib" },
    { name = "requests" },
    { name = "Typing_Extensions", marker = "python_full_version < '3.13'" },
]

[package.optional-dependencies]
fast = [
    { name = "orjson" },
]

[package.dev-dependencies]
dev = [
    { name = "pytest" },
]

[[package]]
name = "lib"
version = "0.1.0"
source = { editable = "packages/lib" }

[[package]]
name = "orjson"
version = "3.10.0"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://files/orjson-3.10.0.tar.gz", hash = "sha256:o", size = 1 }

[[package]]
name = "pytest"
version = "8.3.0"
source = { registry = "https://pypi.org/simple" }
wheels = [
    { url = "https://files/pytest.whl", hash = "sha256:p", size = 2 },
]

[[package]]
name = "requests"
version = "2.32.3"
source = { registry = "https://pypi.org/simple" }
dependencies = [
    { name = "urllib3" },
]
sdist = { url = "https://files/requests.tar.gz", hash = "sha256:r", size = 3 }

[[package]]
name = "typing-extensions"
version = "4.12.2"
source = { git = "https://github.com/python/typing_extensions?rev=main#abc" }

[[package]]
name = "urllib3"
version = "2.2.3"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://files/urllib3.tar.gz", hash = "sha256:u", size = 4 }
"#;

    #[test]
    fn parses_workspace() {
        let lock = parse(LOCK).unwrap();
        assert_eq!(lock.format(), LockfileFormat::Uv);
        assert_eq!(lock.lockfile_version(), "1.2");
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "orjson@3.10.0",
                "pytest@8.3.0",
                "requests@2.32.3",
                "typing-extensions@4.12.2",
                "urllib3@2.2.3"
            ]
        );
        let imps: Vec<_> = lock.importers().iter().map(|i| i.path.as_str()).collect();
        assert_eq!(imps, [".", "packages/lib"]);
        assert_eq!(
            lock.package("pytest@8.3.0").unwrap().integrity.as_deref(),
            Some("sha256:p")
        );
        assert!(
            lock.package("typing-extensions@4.12.2")
                .unwrap()
                .resolution
                .as_deref()
                .unwrap()
                .starts_with("git+")
        );
        match lock.resolve(".", "lib") {
            Some(Resolved::Importer(i)) => assert_eq!(i.path, "packages/lib"),
            other => panic!("{other:?}"),
        }
        match lock.resolve(".", "typing-extensions") {
            Some(Resolved::Package(p)) => assert_eq!(p.version, "4.12.2"),
            other => panic!("{other:?}"),
        }
        let runtime: Vec<_> = lock
            .importer_closure(".", false)
            .iter()
            .map(|p| p.id.clone())
            .collect();
        assert_eq!(
            runtime,
            [
                "orjson@3.10.0",
                "requests@2.32.3",
                "typing-extensions@4.12.2",
                "urllib3@2.2.3"
            ]
        );
    }

    #[test]
    fn diff_detects_version_and_hash_changes() {
        let old = parse(LOCK).unwrap();
        let new = parse(
            &LOCK
                .replace("version = \"2.2.3\"", "version = \"2.2.4\"")
                .replace("sha256:r", "sha256:r2"),
        )
        .unwrap();
        let d = diff(&old, &new);
        assert_eq!(d.added, ["urllib3@2.2.4"]);
        assert_eq!(d.removed, ["urllib3@2.2.3"]);
        let changed: Vec<_> = d.changed.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(changed, ["requests@2.32.3"]);
        let affected = d.affected_packages();
        assert!(affected.identity_changed.contains("requests@2.32.3"));
        assert!(affected.identity_changed.contains("urllib3@2.2.4"));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            parse("version = 2\n"),
            Err(Error::UnsupportedVersion { .. })
        ));
        assert!(parse("version = 1\n[[package]\n").is_err());
    }
}
