//! Yarn berry (v2+) `yarn.lock`: YAML with `__metadata` (lockfile versions 4–8
//! written by yarn 2, 3 and 4).
//!
//! Ids are resolution locators (`ms@npm:2.1.3`, `resolve@patch:…`). Entries
//! whose resolution uses the `workspace:` protocol become importers.
//! Dependencies are descriptors resolved through the entry keys;
//! `dependenciesMeta.<name>.optional` marks optional edges. Peer dependencies
//! are not resolved in the lockfile (yarn resolves them at link time), so they
//! are recorded as unresolved peer edges.

use std::collections::BTreeMap;

use crate::{
    Error, Result,
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta,
        Platform, Target,
    },
    util::{split_descriptors, split_name_at},
    yaml::{Node, Parser, YamlError},
};

struct Raw {
    resolution: String,
    version: String,
    checksum: Option<String>,
    deps: Vec<(String, String, DepKind)>,
    conditions: Option<String>,
}

/// Parses a yarn berry `yarn.lock`.
pub fn parse(src: &str) -> Result<Lockfile> {
    let syntax = |e: YamlError| Error::Syntax {
        format: LockfileFormat::YarnBerry,
        line: e.line,
        message: e.message,
    };
    let mut p = Parser::new(src);
    let mut version: Option<String> = None;
    let mut descriptors: BTreeMap<String, String> = BTreeMap::new();
    let mut raws: Vec<Raw> = Vec::new();
    p.entries_top(|key, node| {
        if key == "__metadata" {
            version = node.get_str("version").map(str::to_string);
            return Ok(());
        }
        let Some(resolution) = node.get_str("resolution") else {
            return Ok(());
        };
        for d in split_descriptors(&key) {
            descriptors.insert(d, resolution.to_string());
        }
        let meta = node.get("dependenciesMeta");
        let mut deps = Vec::new();
        for (name, range) in node
            .get("dependencies")
            .map(Node::entries)
            .unwrap_or_default()
        {
            let optional = meta
                .and_then(|m| m.get(name))
                .and_then(|m| m.get("optional"))
                .is_some_and(Node::is_true);
            let kind = if optional {
                DepKind::Optional
            } else {
                DepKind::Prod
            };
            deps.push((
                name.to_string(),
                range.as_str().unwrap_or("").to_string(),
                kind,
            ));
        }
        for (name, range) in node
            .get("optionalDependencies")
            .map(Node::entries)
            .unwrap_or_default()
        {
            deps.push((
                name.to_string(),
                range.as_str().unwrap_or("").to_string(),
                DepKind::Optional,
            ));
        }
        for (name, range) in node
            .get("peerDependencies")
            .map(Node::entries)
            .unwrap_or_default()
        {
            deps.push((
                name.to_string(),
                range.as_str().unwrap_or("").to_string(),
                DepKind::Peer,
            ));
        }
        raws.push(Raw {
            resolution: resolution.to_string(),
            version: node.get_str("version").unwrap_or("").to_string(),
            checksum: node.get_str("checksum").map(str::to_string),
            deps,
            conditions: node.get_str("conditions").map(str::to_string),
        });
        Ok(())
    })
    .map_err(syntax)?;
    let Some(version) = version else {
        return Err(Error::Syntax {
            format: LockfileFormat::YarnBerry,
            line: 1,
            message: "missing __metadata.version".into(),
        });
    };

    // `resolutions` in package.json rewrite descriptors at resolve time, so a
    // dependent's descriptor (`agent-base@npm:^7.0.2`) may have no lockfile
    // key. Fall back to the only locked package with that name; with several
    // candidates the edge stays unresolved.
    let mut by_name: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for raw in &raws {
        if let Some((name, rest)) = split_name_at(&raw.resolution)
            && !rest.starts_with("workspace:")
        {
            by_name.entry(name).or_default().push(&raw.resolution);
        }
    }
    let resolve = |name: &str, range: &str| -> Option<String> {
        descriptors
            .get(&format!("{name}@{range}"))
            .or_else(|| {
                if range.contains(':') {
                    None
                } else {
                    descriptors.get(&format!("{name}@npm:{range}"))
                }
            })
            .cloned()
            .or_else(|| match by_name.get(name).map(Vec::as_slice) {
                Some([only]) => Some(only.to_string()),
                _ => None,
            })
    };
    let workspace_path = |locator: &str| -> Option<String> {
        let (_, rest) = split_name_at(locator)?;
        rest.strip_prefix("workspace:")
            .map(crate::model::normalize_importer_path)
    };

    let mut packages = Vec::new();
    let mut importers = Vec::new();
    for raw in &raws {
        let mut edges = Vec::new();
        for (name, range, kind) in &raw.deps {
            let target = if *kind == DepKind::Peer {
                Target::Unresolved(range.clone())
            } else {
                match resolve(name, range) {
                    Some(loc) => match workspace_path(&loc) {
                        Some(path) => Target::Importer(path),
                        None => Target::Package(loc),
                    },
                    None => Target::Unresolved(range.clone()),
                }
            };
            edges.push(DepEdge {
                name: name.clone(),
                kind: *kind,
                target,
                specifier: Some(range.clone()),
            });
        }
        let (name, _) = split_name_at(&raw.resolution).unwrap_or((&raw.resolution, ""));
        if let Some(path) = workspace_path(&raw.resolution) {
            importers.push(Importer {
                path,
                name: Some(name.to_string()),
                dependencies: edges,
            });
            continue;
        }
        let mut pkg = LockedPackage::new(raw.resolution.clone(), name, raw.version.clone());
        pkg.integrity = raw.checksum.clone();
        pkg.dependencies = edges;
        if raw.resolution.contains("@patch:") {
            pkg.patch = patch_hash(&raw.resolution).map(str::to_string);
        }
        // Registry packages are fully identified by name and version; keep other
        // sources.
        if !raw.resolution.contains("@npm:") || raw.resolution.contains("@patch:") {
            pkg.resolution = Some(raw.resolution.clone());
        }
        if let Some(c) = &raw.conditions {
            pkg.platform = parse_conditions(c);
        }
        packages.push(pkg);
    }
    let mut lock = Lockfile::from_parts(
        LockfileFormat::YarnBerry,
        version,
        packages,
        importers,
        LockfileMeta::default(),
    );
    lock.set_descriptors(descriptors);
    Ok(lock)
}

/// `…::version=1.22.8&hash=c3c19d` → `c3c19d`.
fn patch_hash(locator: &str) -> Option<&str> {
    let i = locator.rfind("hash=")?;
    let rest = &locator[i + 5..];
    Some(&rest[..rest.find('&').unwrap_or(rest.len())])
}

/// `os=darwin & cpu=arm64` (also `|`-separated alternatives) → platform lists.
fn parse_conditions(c: &str) -> Platform {
    let mut p = Platform::default();
    for term in c.split(['&', '|', '(', ')']) {
        let Some((k, v)) = term.trim().split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "os" => p.os.push(v),
            "cpu" => p.cpu.push(v),
            "libc" => p.libc.push(v),
            _ => {}
        }
    }
    p
}

impl<'a> Parser<'a> {
    /// Streams the top-level mapping, parsing each value fully.
    pub(crate) fn entries_top(
        &mut self,
        mut f: impl FnMut(std::borrow::Cow<'a, str>, Node<'a>) -> crate::yaml::Result<()>,
    ) -> crate::yaml::Result<()> {
        self.top_level(|p, k| {
            let v = p.value(0)?;
            f(k, v)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_and_patch() {
        let p = parse_conditions("os=darwin & cpu=arm64");
        assert_eq!(p.os, vec!["darwin"]);
        assert_eq!(p.cpu, vec!["arm64"]);
        assert_eq!(
            patch_hash(
                "resolve@patch:resolve@npm%3A1.22.8#~builtin<compat/resolve>::version=1.22.8&\
                 hash=c3c19d"
            ),
            Some("c3c19d")
        );
    }
}
