//! Port of pnpm `next_format/read/keys.rs`: each snapshot's peer-suffixed
//! key, rebuilt from its providers.

use std::collections::{HashMap, HashSet};

use super::{
    super::{key, tarjan},
    Graph, Provider, Sid,
};

impl Graph<'_, '_> {
    pub fn base_keys(&self) -> Vec<String> {
        let patches = Patches::of(self);
        (0..self.keys.len())
            .map(|sid| format!("{}{}", self.keys[sid], patches.group(self, sid)))
            .collect()
    }

    pub fn keys(&self, base_keys: &[String]) -> Vec<String> {
        let settings = &self.doc.settings;
        let max_length = settings.peers_suffix_max_length;
        if settings.dedupe_peers {
            return (0..self.keys.len())
                .map(|sid| {
                    self.key_of(sid, base_keys, max_length, |_, provider| {
                        self.keys[provider].to_string()
                    })
                })
                .collect();
        }
        self.nested_keys(base_keys, max_length)
    }

    fn nested_keys(&self, base_keys: &[String], max_length: usize) -> Vec<String> {
        let nesting = Nesting {
            order: self.providers_first(),
            cyclic_names: self.cyclic_peer_names(),
        };
        let mut keys: Vec<String> = base_keys.to_vec();
        for level in self.levels(&nesting.order) {
            let level_keys: Vec<(Sid, String)> = level
                .iter()
                .flat_map(|&component| nesting.order.components[component].iter().copied())
                .filter(|&sid| !self.providers[sid].is_empty())
                .map(|sid| {
                    let key = self.key_of(sid, base_keys, max_length, |name, provider| {
                        if nesting.collapses(sid, provider, name) {
                            self.keys[provider].to_string()
                        } else {
                            keys[provider].clone()
                        }
                    });
                    (sid, key)
                })
                .collect();
            for (sid, key) in level_keys {
                keys[sid] = key;
            }
        }
        keys
    }

    fn key_of(
        &self,
        sid: Sid,
        base_keys: &[String],
        max_length: usize,
        segment: impl Fn(&str, Sid) -> String,
    ) -> String {
        if self.providers[sid].is_empty() {
            return base_keys[sid].clone();
        }
        let segments: Vec<String> = self.providers[sid]
            .iter()
            .map(|(name, provider)| match provider {
                Provider::Link(path) => {
                    format!("{name}@{}", key::link_path_to_peer_version(path))
                }
                Provider::Missing(pair) => pair.strip_prefix('/').unwrap_or(pair).to_string(),
                Provider::Snapshot(provider) => segment(name, *provider),
            })
            .collect();
        format!(
            "{}{}",
            base_keys[sid],
            key::create_peer_dep_graph_hash(segments, max_length)
        )
    }

    fn levels(&self, order: &tarjan::Components) -> Vec<Vec<usize>> {
        let mut level_of = vec![0; order.components.len()];
        let mut levels: Vec<Vec<usize>> = Vec::new();
        for (component, members) in order.components.iter().enumerate() {
            let level = members
                .iter()
                .flat_map(|&sid| self.providers[sid].iter())
                .filter_map(|(_, provider)| match provider {
                    Provider::Snapshot(provider) => order.component_of.get(provider),
                    Provider::Link(_) | Provider::Missing(_) => None,
                })
                .filter(|&&provider_component| provider_component != component)
                .map(|&provider_component| level_of[provider_component] + 1)
                .max()
                .unwrap_or(0);
            level_of[component] = level;
            if levels.len() <= level {
                levels.resize_with(level + 1, Vec::new);
            }
            levels[level].push(component);
        }
        levels
    }

    pub fn providers_first(&self) -> tarjan::Components {
        tarjan::components(
            self.keys.len(),
            |sid| {
                self.providers[sid]
                    .iter()
                    .filter_map(|(_, provider)| match provider {
                        Provider::Snapshot(provider) => Some(*provider),
                        Provider::Link(_) | Provider::Missing(_) => None,
                    })
                    .collect()
            },
            |sid| !self.providers[sid].is_empty(),
        )
    }

    pub fn cyclic_peer_names(&self) -> HashSet<String> {
        let mut names: Vec<String> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut edges: Vec<HashSet<usize>> = Vec::new();
        let mut intern = |name: &str, edges: &mut Vec<HashSet<usize>>| {
            *index.entry(name.to_string()).or_insert_with(|| {
                names.push(name.to_string());
                edges.push(HashSet::new());
                names.len() - 1
            })
        };
        for sid in 0..self.keys.len() {
            if self.providers[sid].is_empty() {
                continue;
            }
            let from = intern(self.names[sid], &mut edges);
            for (name, _) in &self.providers[sid] {
                let to = intern(name, &mut edges);
                edges[from].insert(to);
            }
        }
        let components = tarjan::components(
            edges.len(),
            |node| edges[node].iter().copied().collect(),
            |_| true,
        );
        components
            .components
            .iter()
            .filter(|component| match component.as_slice() {
                [only] => edges[*only].contains(only),
                _ => true,
            })
            .flatten()
            .map(|&node| names[node].clone())
            .collect()
    }
}

struct Nesting {
    order: tarjan::Components,
    cyclic_names: HashSet<String>,
}

impl Nesting {
    fn collapses(&self, sid: Sid, provider: Sid, name: &str) -> bool {
        let component = self.order.component_of.get(&sid);
        let in_cycle = component.is_some() && component == self.order.component_of.get(&provider);
        in_cycle || self.cyclic_names.contains(name)
    }
}

/// Port of pnpm `patch_groups_from_hashes` and `get_patch_info`.
struct Patches<'a> {
    groups: Option<HashMap<&'a str, PatchGroup<'a>>>,
}

#[derive(Default)]
struct PatchGroup<'a> {
    exact: HashMap<&'a str, &'a str>,
    range: Vec<(&'a str, &'a str)>,
    all: Option<&'a str>,
}

impl<'a> Patches<'a> {
    fn of(graph: &Graph<'_, 'a>) -> Self {
        Patches {
            groups: graph
                .doc
                .settings
                .patched_dependencies
                .as_ref()
                .and_then(group_patched_dependencies),
        }
    }

    fn group(&self, graph: &Graph<'_, '_>, sid: Sid) -> String {
        let Some(groups) = self.groups.as_ref() else {
            return String::new();
        };
        let name = graph.names[sid];
        let version = recorded_version(graph, sid);
        match get_patch_info(groups, name, &version) {
            Some(hash) => format!("(patch_hash={hash})"),
            None => String::new(),
        }
    }
}

/// Port of pnpm `name_version_from_package_key`.
fn recorded_version(graph: &Graph<'_, '_>, sid: Sid) -> String {
    let (package, _) = graph.slots[sid];
    let slot = key::split(graph.keys[sid]).map_or("", |(_, suffix)| suffix);
    graph.doc.packages[package]
        .map
        .get("version")
        .and_then(serde_yaml_ng::Value::as_str)
        .unwrap_or(slot)
        .to_string()
}

/// `None` when a key's version is neither a version nor a valid range, so
/// no package is treated as patched (pnpm leaves the error to the resolver).
fn group_patched_dependencies<'a>(
    patches: &std::collections::BTreeMap<&'a str, &'a str>,
) -> Option<HashMap<&'a str, PatchGroup<'a>>> {
    let mut groups: HashMap<&'a str, PatchGroup<'a>> = HashMap::new();
    for (&patch_key, &hash) in patches {
        let separator = patch_key
            .char_indices()
            .skip(1)
            .find_map(|(at, character)| (character == '@').then_some(at));
        let parsed = separator.and_then(|at| {
            let version = &patch_key[at + 1..];
            (!version.is_empty()).then_some((&patch_key[..at], version))
        });
        match parsed {
            Some((name, version)) if node_semver::Version::parse(version).is_ok() => {
                groups.entry(name).or_default().exact.insert(version, hash);
            }
            Some((name, range)) => {
                node_semver::Range::parse(range).ok()?;
                let group = groups.entry(name).or_default();
                if range.trim() == "*" {
                    group.all = Some(hash);
                } else {
                    group.range.push((range, hash));
                }
            }
            None => groups.entry(patch_key).or_default().all = Some(hash),
        }
    }
    Some(groups)
}

/// The hash of the patch that applies to `name@version`; none when two
/// ranges match (pnpm's ERR_PNPM_PATCH_KEY_CONFLICT, left to the resolver).
fn get_patch_info<'a>(
    groups: &HashMap<&str, PatchGroup<'a>>,
    name: &str,
    version: &str,
) -> Option<&'a str> {
    let group = groups.get(name)?;
    if let Some(hash) = group.exact.get(version) {
        return Some(hash);
    }
    let Ok(parsed) = node_semver::Version::parse(version) else {
        return group.all;
    };
    let satisfied: Vec<&str> = group
        .range
        .iter()
        .filter(|(range, _)| {
            node_semver::Range::parse(range).is_ok_and(|range| range.satisfies(&parsed))
        })
        .map(|(_, hash)| *hash)
        .collect();
    match satisfied.as_slice() {
        [] => group.all,
        [only] => Some(only),
        _ => None,
    }
}
