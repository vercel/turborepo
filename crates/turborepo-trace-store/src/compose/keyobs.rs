//! The key observations of a composed graph, for a query that compares two
//! revisions.
//!
//! A key observation ([`turborepo_trace_core::record::key`],
//! [`crate::shard::Shard::key_groups`]) is something a file's records were
//! computed from that is no input of the file: no probe, input or membership of
//! the composed tables names it, so no query selects through it. That is right
//! as long as the observation only *kept* the file's records as they
//! are (a project a `tsconfig.json` references loads and does not include the
//! file). It is not when the observation was different at the base of an
//! `affected` query: the file's records at the base may then have been others.
//! A caller that can tell (it reads both revisions) asks here which files
//! observed a changed path.
//!
//! The sets of the framework pass ([`crate::shard::KeyGroupRec::framework`]:
//! what the load of a runner config accessed) are not asked about here. They
//! say when a file is traced again; what a runner config contributed to a file
//! at the base is not compared by any query yet, with or without them.
//!
//! The shards are read in place; nothing is added to the tables or their cache.
//! A changed path that no probe or config record of any shard expands to (the
//! tables' string table) is in no key set either, which answers most changes
//! without looking at a shard.

use std::collections::HashMap;

use rayon::prelude::*;
use turborepo_trace_graph::NodeId;

use super::{Graph, NONE};
use crate::{
    normalize::{config_path, expand_probe},
    paths::join_repo,
};

impl Graph {
    /// The files whose key observations name one of `paths`
    /// (repository-relative), as `(file, index into paths)`, sorted, each
    /// pair once. A path is named by a config read at it, and by a probe of
    /// it or of a directory above it (as for the probes of a query: a
    /// missing directory stands for everything below it).
    pub fn key_observers(&self, paths: &[&str]) -> Vec<(NodeId, u32)> {
        if paths.is_empty() || !self.has_key_observations() {
            return Vec::new();
        }
        // What a record must expand to: a path itself (a read or a probe), or a
        // directory above one (a probe).
        let mut exact: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut above: HashMap<&str, Vec<u32>> = HashMap::new();
        for (i, p) in paths.iter().enumerate() {
            exact.entry(p).or_default().push(i as u32);
            let mut dir = *p;
            while let Some(at) = dir.rfind('/') {
                dir = &dir[..at];
                above.entry(dir).or_default().push(i as u32);
            }
        }
        // Every record of every shard is in the string table, key sets included.
        let strings = &self.tables().strings;
        if !exact
            .keys()
            .chain(above.keys())
            .any(|s| strings.find(s).is_some())
        {
            return Vec::new();
        }
        // Per shard and set: the paths it names.
        let sets: Vec<Vec<Vec<u32>>> = self
            .shards
            .par_iter()
            .map(|s| {
                let a = s.get();
                let root = a.package.root.as_str();
                let mut scratch = Vec::new();
                a.key_groups
                    .iter()
                    .map(|g| {
                        let mut hit: Vec<u32> = Vec::new();
                        // A set of the framework pass is no part of the question.
                        if g.framework {
                            return hit;
                        }
                        for c in g.configs.iter() {
                            let c = a.configs[c.to_native() as usize].as_str();
                            if let Some(p) = join_repo(root, config_path(c))
                                && let Some(ix) = exact.get(p.as_str())
                            {
                                hit.extend_from_slice(ix);
                            }
                        }
                        for p in g.probes.iter() {
                            scratch.clear();
                            expand_probe(&a.probes[p.to_native() as usize], &mut scratch);
                            for rel in &scratch {
                                let Some(p) = join_repo(root, rel) else {
                                    continue;
                                };
                                for ix in [exact.get(p.as_str()), above.get(p.as_str())]
                                    .into_iter()
                                    .flatten()
                                {
                                    hit.extend_from_slice(ix);
                                }
                            }
                        }
                        hit.sort_unstable();
                        hit.dedup();
                        hit
                    })
                    .collect()
            })
            .collect();
        self.files_of_sets(&sets)
    }

    /// The files that name a set with a hit, each with the hits:
    /// `sets[shard][set]` is what the set matched.
    fn files_of_sets(&self, sets: &[Vec<Vec<u32>>]) -> Vec<(NodeId, u32)> {
        let any: Vec<bool> = sets
            .iter()
            .map(|of| of.iter().any(|hit| !hit.is_empty()))
            .collect();
        let mut out: Vec<(NodeId, u32)> = Vec::new();
        for (n, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE || !any[node.shard as usize] {
                continue;
            }
            let shard = node.shard as usize;
            let f = &self.shards[shard].get().files[node.file as usize];
            for g in f.key_groups.iter() {
                let hit = &sets[shard][g.to_native() as usize];
                out.extend(hit.iter().map(|&i| (NodeId(n as u32), i)));
            }
        }
        out.sort_unstable_by_key(|&(n, i)| (n.0, i));
        out.dedup();
        out
    }

    /// The files whose key observations were located through a package named by
    /// `names` (a config that a referenced project extends, shipped by a
    /// package), as `(file, index into names)`, sorted, each pair once.
    /// Each name comes with whether a link to a first-party package counts
    /// too: the files of a workspace package are paths of
    /// the tree ([`Graph::key_observers`]), so its name only matters when the
    /// link itself was made to lead elsewhere.
    pub fn key_package_observers(&self, names: &[(&str, bool)]) -> Vec<(NodeId, u32)> {
        if names.is_empty() || !self.has_key_observations() {
            return Vec::new();
        }
        let sets: Vec<Vec<Vec<u32>>> = self
            .shards
            .iter()
            .map(|s| {
                let a = s.get();
                a.key_groups
                    .iter()
                    .map(|g| {
                        let mut hit: Vec<u32> = g
                            .links
                            .iter()
                            .filter(|_| !g.framework)
                            .filter_map(|l| {
                                let l = &a.obs.links[l.to_native() as usize];
                                let root = l.root.as_str();
                                let third_party = root.starts_with('/')
                                    || root.split('/').any(|s| s == "node_modules");
                                names
                                    .iter()
                                    .position(|(n, any)| {
                                        *n == l.name.as_str() && (*any || third_party)
                                    })
                                    .map(|i| i as u32)
                            })
                            .collect();
                        hit.sort_unstable();
                        hit.dedup();
                        hit
                    })
                    .collect()
            })
            .collect();
        self.files_of_sets(&sets)
    }

    /// The files that have the manifest at `path` (repository-relative) as an
    /// input because the load of a `tsconfig.json` read it, through the
    /// loader's view ([`crate::normalize::LOADER_SUFFIX`]: `name`,
    /// `exports`, `imports`, `main`, `tsconfig`, `typesVersions`), each
    /// with whether that is the only way the file's records have it (`true`: no
    /// reference of the file read the manifest as a file or through its
    /// resolution view). Sorted by node.
    ///
    /// The composed tables name the file for every kind of read, so a changed
    /// manifest matches all of its readers. A caller that reads both
    /// revisions can tell more: when the loader's view is the same at both,
    /// the change is none for a file that has the manifest this way alone,
    /// and when it differs, it is one for every file that has it
    /// this way, whatever else the caller concluded from the fields. The shards
    /// are read in place, as for [`Graph::key_observers`]. (Inputs that a
    /// derived layer adds to a node are not in the shards: the caller asks
    /// its view for those.)
    pub fn loader_readers(&self, path: &str) -> Vec<(NodeId, bool)> {
        if self.tables().strings.find(path).is_none() {
            return Vec::new();
        }
        // Per shard: the config entries that are the loader's view of the manifest, and
        // the ones that are the manifest read any other way.
        let entries: Vec<(Vec<u32>, Vec<u32>)> = self
            .shards
            .par_iter()
            .map(|s| {
                let a = s.get();
                let root = a.package.root.as_str();
                let (mut loader, mut other) = (Vec::new(), Vec::new());
                for (i, c) in a.configs.iter().enumerate() {
                    let c = c.as_str();
                    if join_repo(root, config_path(c)).as_deref() != Some(path) {
                        continue;
                    }
                    if crate::normalize::loader_config(c).is_some() {
                        loader.push(i as u32);
                    } else {
                        other.push(i as u32);
                    }
                }
                (loader, other)
            })
            .collect();
        let mut out: Vec<(NodeId, bool)> = Vec::new();
        for (n, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE {
                continue;
            }
            let (loader, other) = &entries[node.shard as usize];
            if loader.is_empty() {
                continue;
            }
            let f = &self.shards[node.shard as usize].get().files[node.file as usize];
            let reads = |of: &[u32]| {
                f.refs
                    .iter()
                    .any(|r| r.configs.iter().any(|c| of.contains(&c.to_native())))
            };
            if reads(loader) {
                out.push((NodeId(n as u32), !reads(other)));
            }
        }
        out
    }

    /// Whether any file of the graph has key observations that a query asks
    /// about (not only those of the framework pass,
    /// [`crate::shard::KeyGroupRec::framework`]).
    pub fn has_key_observations(&self) -> bool {
        self.shards
            .iter()
            .any(|s| s.get().key_groups.iter().any(|g| !g.framework))
    }
}
