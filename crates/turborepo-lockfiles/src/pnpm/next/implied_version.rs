//! Port of pnpm `implied_version.rs` and `next_format/linked.rs`: the
//! version a project dependency written as its specifier alone resolves to.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::{
    key,
    wire::{CatalogEntry, Package, Project},
};

pub(super) struct Implied<'a> {
    pub catalogs: Option<&'a BTreeMap<String, BTreeMap<String, CatalogEntry>>>,
    pub injected: HashMap<&'a str, &'a str>,
    pub linked: HashMap<&'a str, &'a str>,
}

impl<'a> Implied<'a> {
    pub fn of(
        catalogs: Option<&'a BTreeMap<String, BTreeMap<String, CatalogEntry>>>,
        packages: &[Package<'a>],
        projects: &[Project<'a>],
    ) -> Self {
        Implied {
            catalogs,
            injected: injected_paths(packages),
            linked: named_importers(projects),
        }
    }

    pub fn version(&self, id: &str, alias: &str, specifier: &str) -> Option<String> {
        implied_version(
            alias,
            specifier,
            |catalog, alias| {
                let entry = self.catalogs?.get(catalog)?.get(alias)?;
                Some((entry.specifier.as_str(), entry.version.as_str()))
            },
            |name| self.injected.get(name).copied(),
            |name| self.linked.get(name).map(|target| link_path(id, target)),
        )
    }
}

fn injected_paths<'a>(packages: &[Package<'a>]) -> HashMap<&'a str, &'a str> {
    let mut paths: HashMap<&'a str, Option<&'a str>> = HashMap::new();
    for package in packages {
        let Some((_, suffix)) = key::split(package.key) else {
            continue;
        };
        let Some(path) = suffix.strip_prefix("file:") else {
            continue;
        };
        paths
            .entry(package.name)
            .and_modify(|entry| *entry = None)
            .or_insert(Some(path));
    }
    paths
        .into_iter()
        .filter_map(|(name, path)| Some((name, path?)))
        .collect()
}

fn named_importers<'a>(projects: &[Project<'a>]) -> HashMap<&'a str, &'a str> {
    let mut named: HashMap<&'a str, Option<&'a str>> = HashMap::new();
    for project in projects {
        if let Some(name) = project.name {
            named
                .entry(name)
                .and_modify(|claim| {
                    if *claim != Some(project.id) {
                        *claim = None;
                    }
                })
                .or_insert(Some(project.id));
        }
    }
    let by_name: HashMap<&'a str, &'a str> = named
        .into_iter()
        .filter_map(|(name, id)| Some((name, id?)))
        .collect();
    let mut seen = HashSet::new();
    let mut shared = HashSet::new();
    for id in by_name.values() {
        if !seen.insert(*id) {
            shared.insert(*id);
        }
    }
    by_name
        .into_iter()
        .filter(|(_, id)| !shared.contains(id))
        .collect()
}

fn implied_version<'p>(
    alias: &str,
    specifier: &str,
    catalog_entry: impl FnOnce(&str, &str) -> Option<(&'p str, &'p str)>,
    injected_path: impl FnOnce(&str) -> Option<&'p str>,
    link_path: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    if let Some(target) = specifier.strip_prefix("workspace:") {
        return workspace_version(alias, target, injected_path, link_path);
    }
    if let Some(catalog) = specifier.strip_prefix("catalog:") {
        let catalog = if catalog.is_empty() {
            "default"
        } else {
            catalog
        };
        let (catalog_specifier, version) = catalog_entry(catalog, alias)?;
        return Some(match catalog_specifier.strip_prefix("npm:") {
            Some(target) => format!("{}@{version}", alias_name(target)),
            None => version.to_string(),
        });
    }
    Some(
        specifier
            .strip_prefix("npm:")
            .unwrap_or(specifier)
            .to_string(),
    )
}

pub(super) fn catalog_implied_version(specifier: &str) -> Option<&str> {
    let Some(target) = specifier.strip_prefix("npm:") else {
        return Some(specifier);
    };
    target.get(alias_name(target).len() + 1..)
}

fn alias_name(target: &str) -> &str {
    let Some(rest) = target.get(1..) else {
        return target;
    };
    match rest.find('@') {
        Some(at) => &target[..=at],
        None => target,
    }
}

fn workspace_version<'p>(
    alias: &str,
    target: &str,
    injected_path: impl FnOnce(&str) -> Option<&'p str>,
    link_path: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    let name = workspace_name(target);
    let package = name.unwrap_or(alias);
    match (injected_path(package), link_path(package)) {
        (Some(path), None) => Some(match name {
            Some(name) if name != alias => format!("{name}@file:{path}"),
            _ => format!("file:{path}"),
        }),
        (None, Some(path)) => Some(format!("link:{path}")),
        _ => None,
    }
}

fn workspace_name(target: &str) -> Option<&str> {
    let at = target.get(1..)?.find('@')? + 1;
    Some(&target[..at])
}

fn link_path(id: &str, target: &str) -> String {
    let from: Vec<&str> = segments(id).collect();
    let to: Vec<&str> = segments(target).collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(from, to)| from == to)
        .count();
    let path: Vec<&str> = std::iter::repeat_n("..", from.len() - common)
        .chain(to[common..].iter().copied())
        .collect();
    if path.is_empty() {
        ".".to_string()
    } else {
        path.join("/")
    }
}

fn segments(id: &str) -> impl Iterator<Item = &str> {
    id.split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implies_versions() {
        let none = |_: &str, _: &str| None;
        assert_eq!(
            implied_version("odd", "npm:is-odd@3.0.1", none, |_| None, |_| None).as_deref(),
            Some("is-odd@3.0.1")
        );
        assert_eq!(
            implied_version("a", "workspace:*", none, |_| None, |_| Some("../a".into())).as_deref(),
            Some("link:../a")
        );
        assert_eq!(
            implied_version("a", "workspace:*", none, |_| Some("packages/a"), |_| None).as_deref(),
            Some("file:packages/a")
        );
        assert_eq!(
            implied_version(
                "a",
                "workspace:*",
                none,
                |_| Some("packages/a"),
                |_| Some("x".into())
            ),
            None
        );
        assert_eq!(
            implied_version(
                "lodash",
                "catalog:",
                |_, _| Some(("^4", "4.18.1")),
                |_| None,
                |_| None
            )
            .as_deref(),
            Some("4.18.1")
        );
        assert_eq!(catalog_implied_version("npm:@a/b@1.2.3"), Some("1.2.3"));
        assert_eq!(link_path("packages/d", "packages/c"), "../c");
        assert_eq!(link_path(".", "packages/loc"), "packages/loc");
    }
}
