//! Bun's text lockfile `bun.lock` (JSONC, lockfileVersion 0–2). The binary
//! `bun.lockb` is not supported.
//!
//! `packages` is keyed by install path in package-name segments (`send/ms` is
//! `ms` nested under `send`; workspace-local installs are prefixed with the
//! workspace name). Each value is an array `[ident, registry, info, integrity]`
//! for registry packages (shorter for workspace, git, file and tarball
//! packages). Dependencies resolve like Node: the nearest enclosing path first.

use std::collections::HashMap;

use serde_json::Value;

use crate::{
    Error, Result,
    json::strip_jsonc,
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta,
        Platform, Target, normalize_importer_path,
    },
    util::split_name_at,
};

const DEP_SECTIONS: [(&str, DepKind); 4] = [
    ("dependencies", DepKind::Prod),
    ("optionalDependencies", DepKind::Optional),
    ("peerDependencies", DepKind::Peer),
    ("devDependencies", DepKind::Dev),
];

/// Splits a bun install path into package-name segments (`@a/b/c` → `["@a/b",
/// "c"]`).
fn segments(key: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = key;
    while !rest.is_empty() {
        let end = if rest.starts_with('@') {
            match rest.find('/') {
                Some(s) => rest[s + 1..].find('/').map_or(rest.len(), |e| s + 1 + e),
                None => rest.len(),
            }
        } else {
            rest.find('/').unwrap_or(rest.len())
        };
        out.push(&rest[..end]);
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() && s != "none" => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Parses a `bun.lock`.
pub fn parse(src: &str) -> Result<Lockfile> {
    let json = strip_jsonc(src);
    let root: Value = serde_json::from_str(&json).map_err(|e| Error::Json {
        format: LockfileFormat::Bun,
        source: e,
    })?;
    let version = match root.get("lockfileVersion") {
        Some(Value::Number(n)) => n.to_string(),
        _ => {
            return Err(Error::Syntax {
                format: LockfileFormat::Bun,
                line: 1,
                message: "missing lockfileVersion".into(),
            });
        }
    };
    let empty = serde_json::Map::new();
    let pkgs = root
        .get("packages")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let workspaces = root
        .get("workspaces")
        .and_then(Value::as_object)
        .unwrap_or(&empty);

    // Workspace name → path, for key prefixes and `workspace:` idents.
    let ws_prefix: HashMap<&str, &str> = workspaces
        .iter()
        .filter_map(|(path, w)| Some((path.as_str(), w.get("name")?.as_str()?)))
        .collect();

    let target_of = |key: &str| -> Option<Target> {
        let entry = pkgs.get(key)?.as_array()?;
        let ident = entry.first()?.as_str()?;
        let (_, rest) = split_name_at(ident)?;
        if let Some(path) = rest.strip_prefix("workspace:") {
            return Some(Target::Importer(normalize_importer_path(path)));
        }
        Some(Target::Package(key.to_string()))
    };
    // Nearest enclosing install path first: `a/b` + `n` tries `a/b/n`, `a/n`, `n`.
    let resolve = |from: &[&str], name: &str| -> Option<Target> {
        for depth in (0..=from.len()).rev() {
            let mut key = from[..depth].join("/");
            if !key.is_empty() {
                key.push('/');
            }
            key.push_str(name);
            if let Some(t) = target_of(&key) {
                return Some(t);
            }
        }
        None
    };
    let edges_of = |info: &Value, from: &[&str], include_dev: bool| -> Vec<DepEdge> {
        let optional_peers = str_list(info.get("optionalPeers"));
        let mut edges = Vec::new();
        for (section, kind) in DEP_SECTIONS {
            if kind == DepKind::Dev && !include_dev {
                continue;
            }
            let Some(deps) = info.get(section).and_then(Value::as_object) else {
                continue;
            };
            for (name, range) in deps {
                let range = range.as_str().unwrap_or("").to_string();
                let target = match resolve(from, name) {
                    Some(t) => t,
                    None => Target::Unresolved(range.clone()),
                };
                // Unmet optional peers (`optionalPeers`) are expected; drop them instead of
                // recording noise. Unmet required peers stay as unresolved edges.
                if kind == DepKind::Peer
                    && target.package_id().is_none()
                    && optional_peers.contains(name)
                {
                    continue;
                }
                edges.push(DepEdge {
                    name: name.clone(),
                    kind,
                    target,
                    specifier: Some(range),
                });
            }
        }
        edges
    };

    let mut importers = Vec::new();
    for (path, w) in workspaces {
        let prefix: Vec<&str> = match ws_prefix.get(path.as_str()) {
            Some(name) if !path.is_empty() => vec![name],
            _ => Vec::new(),
        };
        importers.push(Importer {
            path: normalize_importer_path(path),
            name: w.get("name").and_then(Value::as_str).map(str::to_string),
            dependencies: edges_of(w, &prefix, true),
        });
    }

    let mut packages = Vec::new();
    for (key, entry) in pkgs {
        let Some(arr) = entry.as_array() else {
            continue;
        };
        let Some(ident) = arr.first().and_then(Value::as_str) else {
            continue;
        };
        let (name, rest) = split_name_at(ident).unwrap_or((ident, ""));
        if rest.starts_with("workspace:") {
            continue;
        }
        let info = arr
            .iter()
            .skip(1)
            .find(|v| v.is_object())
            .cloned()
            .unwrap_or(Value::Null);
        let integrity = arr
            .iter()
            .skip(2)
            .filter_map(Value::as_str)
            .find(|s| s.starts_with("sha"))
            .map(str::to_string);
        let registry = arr.get(1).and_then(Value::as_str).filter(|s| !s.is_empty());
        let is_registry_version = !rest.contains(':') && !rest.contains('/');
        let mut pkg = LockedPackage::new(
            key.clone(),
            name,
            if is_registry_version { rest } else { "" },
        );
        if !is_registry_version {
            pkg.resolution = Some(rest.to_string());
            if let Some(v) = info.get("version").and_then(Value::as_str) {
                pkg.version = v.to_string();
            }
        } else if let Some(r) = registry {
            pkg.resolution = Some(r.to_string());
        }
        pkg.integrity = integrity;
        let from = segments(key);
        pkg.dependencies = edges_of(&info, &from, false);
        pkg.platform = Platform {
            os: str_list(info.get("os")),
            cpu: str_list(info.get("cpu")),
            libc: str_list(info.get("libc")),
        };
        packages.push(pkg);
    }

    let mut meta = LockfileMeta::default();
    for section in ["overrides", "resolutions"] {
        if let Some(o) = root.get(section).and_then(Value::as_object) {
            for (k, v) in o {
                meta.overrides
                    .insert(k.clone(), v.as_str().unwrap_or("").to_string());
            }
        }
    }
    if let Some(o) = root.get("patchedDependencies").and_then(Value::as_object) {
        for (k, v) in o {
            meta.patched_dependencies
                .insert(k.clone(), v.as_str().unwrap_or("").to_string());
        }
    }
    if let Some(cats) = root.get("catalogs").and_then(Value::as_object) {
        for (cat, entries) in cats {
            let m = entries.as_object().map(|o| {
                o.iter()
                    .map(|(k, v)| {
                        let spec = v.as_str().unwrap_or("").to_string();
                        (
                            k.clone(),
                            crate::model::CatalogEntry {
                                specifier: spec.clone(),
                                version: spec,
                            },
                        )
                    })
                    .collect()
            });
            meta.catalogs.insert(cat.clone(), m.unwrap_or_default());
        }
    }
    if let Some(o) = root.get("catalog").and_then(Value::as_object) {
        let m = o
            .iter()
            .map(|(k, v)| {
                let spec = v.as_str().unwrap_or("").to_string();
                (
                    k.clone(),
                    crate::model::CatalogEntry {
                        specifier: spec.clone(),
                        version: spec,
                    },
                )
            })
            .collect();
        meta.catalogs.insert("default".into(), m);
    }
    Ok(Lockfile::from_parts(
        LockfileFormat::Bun,
        version,
        packages,
        importers,
        meta,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_segments() {
        assert_eq!(segments("send/ms"), vec!["send", "ms"]);
        assert_eq!(segments("@a/b/c/@d/e"), vec!["@a/b", "c", "@d/e"]);
        assert_eq!(segments("@a/b"), vec!["@a/b"]);
    }
}
