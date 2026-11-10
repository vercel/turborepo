//! Pipenv `Pipfile.lock` (`_meta.pipfile-spec` 6) parsed into the neutral
//! [`Lockfile`] model.
//!
//! # Model mapping
//!
//! | Pipfile.lock | Model |
//! |---|---|
//! | `default.<name>` / `develop.<name>` with `"version": "==V"` | [`LockedPackage`] `name@V` (canonical name, PEP 503); `integrity` = the sorted `hashes` joined by spaces; `resolution` = `git+url#ref`, `file` or `path` when present |
//! | `editable: true` with a `path` | [`Importer`] at the path |
//! | the project | the root [`Importer`] `"."`: `default` entries are [`DepKind::Prod`] edges, `develop` entries [`DepKind::Dev`] edges (specifier = `markers`) |
//! | only in `develop` | [`PackageFlags::DEV`] |
//!
//! Pipfile.lock is flat: it records no package-to-package edges, so every
//! locked package is a direct dependency of `"."`.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    Error, Result,
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta,
        PackageFlags, Target,
    },
    requirements::canonicalize_name,
};

/// Parses `Pipfile.lock`.
pub fn parse(contents: &str) -> Result<Lockfile> {
    let root: Value = serde_json::from_str(contents).map_err(|source| Error::Json {
        format: LockfileFormat::Pipfile,
        source,
    })?;
    let spec = root
        .pointer("/_meta/pipfile-spec")
        .and_then(Value::as_u64)
        .unwrap_or(6);
    if spec != 6 {
        return Err(Error::UnsupportedVersion {
            format: LockfileFormat::Pipfile,
            version: spec.to_string(),
        });
    }
    let mut packages: BTreeMap<String, LockedPackage> = BTreeMap::new();
    let mut project = Importer::new(".");
    let mut importers = Vec::new();
    for (section, kind) in [("default", DepKind::Prod), ("develop", DepKind::Dev)] {
        let Some(entries) = root.get(section).and_then(Value::as_object) else {
            continue;
        };
        for (name, e) in entries {
            let canon = canonicalize_name(name);
            let s = |k: &str| e.get(k).and_then(Value::as_str);
            let markers = s("markers").map(str::to_owned);
            if e.get("editable").and_then(Value::as_bool) == Some(true)
                && let Some(path) = s("path")
            {
                let mut imp = Importer::new(path);
                imp.name = Some(canon.clone());
                importers.push(imp);
                project.dependencies.push(DepEdge {
                    name: canon,
                    kind,
                    target: Target::Importer(crate::model::normalize_importer_path(path)),
                    specifier: markers,
                });
                continue;
            }
            let version = s("version")
                .map(|v| {
                    v.trim_start_matches("===")
                        .trim_start_matches("==")
                        .to_owned()
                })
                .unwrap_or_default();
            let resolution = match (s("git"), s("file"), s("path")) {
                (Some(g), _, _) => Some(match s("ref") {
                    Some(r) => format!("git+{g}#{r}"),
                    None => format!("git+{g}"),
                }),
                (None, Some(f), _) => Some(f.to_owned()),
                (None, None, Some(p)) => Some(p.to_owned()),
                _ => None,
            };
            let id = if version.is_empty() {
                match &resolution {
                    Some(r) => format!("{canon}@{r}"),
                    None => canon.clone(),
                }
            } else {
                format!("{canon}@{version}")
            };
            let mut hashes: Vec<&str> = e
                .get("hashes")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            hashes.sort_unstable();
            hashes.dedup();
            let p = packages.entry(id.clone()).or_insert_with(|| {
                let mut p = LockedPackage::new(id.clone(), canon.clone(), version.clone());
                p.flags |= PackageFlags::DEV;
                p
            });
            if kind == DepKind::Prod {
                p.flags.remove(PackageFlags::DEV);
            }
            if p.integrity.is_none() && !hashes.is_empty() {
                p.integrity = Some(hashes.join(" "));
            }
            if p.resolution.is_none() {
                p.resolution = resolution;
            }
            project.dependencies.push(DepEdge {
                name: canon,
                kind,
                target: Target::Package(id),
                specifier: markers,
            });
        }
    }
    importers.push(project);
    Ok(Lockfile::from_parts(
        LockfileFormat::Pipfile,
        spec.to_string(),
        packages.into_values().collect(),
        importers,
        LockfileMeta::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff;

    const LOCK: &str = r#"{
    "_meta": {
        "hash": {"sha256": "abc"},
        "pipfile-spec": 6,
        "requires": {"python_version": "3.12"},
        "sources": [{"name": "pypi", "url": "https://pypi.org/simple", "verify_ssl": true}]
    },
    "default": {
        "Requests": {
            "hashes": ["sha256:b", "sha256:a"],
            "index": "pypi",
            "markers": "python_version >= '3.8'",
            "version": "==2.32.3"
        },
        "mylib": {"editable": true, "path": "./libs/mylib"},
        "gitdep": {"git": "https://github.com/x/gitdep.git", "ref": "0123"}
    },
    "develop": {
        "pytest": {"hashes": ["sha256:p"], "version": "==8.3.3"},
        "requests": {"hashes": ["sha256:a", "sha256:b"], "version": "==2.32.3"}
    }
}"#;

    #[test]
    fn parses() {
        let lock = parse(LOCK).unwrap();
        assert_eq!(lock.format(), LockfileFormat::Pipfile);
        let ids: Vec<_> = lock.packages().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "gitdep@git+https://github.com/x/gitdep.git#0123",
                "pytest@8.3.3",
                "requests@2.32.3"
            ]
        );
        let r = lock.package("requests@2.32.3").unwrap();
        assert_eq!(r.integrity.as_deref(), Some("sha256:a sha256:b"));
        assert!(!r.flags.contains(PackageFlags::DEV));
        assert!(
            lock.package("pytest@8.3.3")
                .unwrap()
                .flags
                .contains(PackageFlags::DEV)
        );
        let imps: Vec<_> = lock.importers().iter().map(|i| i.path.as_str()).collect();
        assert_eq!(imps, [".", "libs/mylib"]);
        let prod: Vec<_> = lock
            .importer_closure(".", false)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(prod, ["gitdep", "requests"]);
    }

    #[test]
    fn diff_and_errors() {
        let old = parse(LOCK).unwrap();
        let new = parse(&LOCK.replace("==8.3.3", "==8.3.4")).unwrap();
        let d = diff(&old, &new);
        assert_eq!(d.added, ["pytest@8.3.4"]);
        assert_eq!(d.removed, ["pytest@8.3.3"]);
        assert!(parse("{").is_err());
        assert!(matches!(
            parse(r#"{"_meta": {"pipfile-spec": 5}}"#),
            Err(Error::UnsupportedVersion { .. })
        ));
    }
}
