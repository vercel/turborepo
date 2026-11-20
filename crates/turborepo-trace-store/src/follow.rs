//! `--externals follow`: traced third-party shards, one per `name@version`.
//!
//! Starting from the packages first-party shards link to, each package's code
//! files (its real root, without nested `node_modules`) are analyzed as
//! entries, and the packages its shard links to are added, until no new package
//! appears. A `name@version` installed at several real paths (pnpm
//! peer-dependency variants) is traced once, at the smallest path.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use rayon::prelude::*;
use turborepo_trace_core::{AnalyzeInput, DirCache, FileAnalysis};

use crate::{
    Error,
    build::{Cx, TargetCx, assemble, obs_digest, third_party_key, write_shard},
    manifest::{Manifest, ShardEntry},
    shard::{self, LoadedShard, PackageRec, kind},
};

type Id = (String, String);

/// `(reused, traced, written, analyzed files)`.
pub(crate) type Stats = (usize, usize, usize, usize);

/// Builds every reachable third-party shard. Returns the manifest entries and
/// `(reused, traced, written, analyzed files)`.
// Invariant: it decodes what this function just encoded.
#[allow(clippy::expect_used)]
pub(crate) fn build_third_party(
    cx: &Cx,
    old: Option<&Manifest>,
    roots: BTreeMap<Id, String>,
) -> Result<(Vec<ShardEntry>, Stats), Error> {
    let old_keys: HashMap<Id, &str> = old
        .iter()
        .flat_map(|m| &m.shards)
        .filter(|s| s.kind == "third-party")
        .filter_map(|s| Some(((s.name.clone(), s.version.clone()?), s.key.as_str())))
        .collect();
    let mut done: BTreeMap<Id, ShardEntry> = BTreeMap::new();
    let mut queue = roots;
    let mut stats = (0, 0, 0, 0);
    while !queue.is_empty() {
        let batch: Vec<(Id, String)> = std::mem::take(&mut queue)
            .into_iter()
            .filter(|(id, _)| !done.contains_key(id))
            .collect();
        if batch.is_empty() {
            break;
        }
        // Validate previous shards.
        let states: Vec<(Id, PackageRec, Option<LoadedShard>)> = batch
            .into_par_iter()
            .map(|(id, root)| {
                let pkg = PackageRec {
                    kind: kind::THIRD_PARTY,
                    name: id.0.clone(),
                    version: Some(id.1.clone()),
                    integrity: cx.ids.integrity(&id.0, &id.1),
                    root,
                };
                let reused = old_keys.get(&id).and_then(|k| {
                    let s = shard::load_from(&cx.store, None, k).ok()?;
                    (s.get().package.root.as_str() == pkg.root
                        && third_party_key(cx, &pkg, obs_digest(cx, s.get())) == *k)
                        .then_some(s)
                });
                (id, pkg, reused)
            })
            .collect();
        // Analyze the rest.
        let mut inputs = Vec::new();
        let mut owners = Vec::new();
        let mut rels_of: Vec<Vec<String>> = Vec::new();
        for (si, (_, pkg, reused)) in states.iter().enumerate() {
            let rels = if reused.is_none() {
                package_files(cx, &cx_root(cx, &pkg.root))
            } else {
                Vec::new()
            };
            for (ei, r) in rels.iter().enumerate() {
                inputs.push(AnalyzeInput {
                    path: crate::paths::abs(&cx_root(cx, &pkg.root), r),
                    cached: None,
                });
                owners.push((si, ei));
            }
            rels_of.push(rels);
        }
        stats.3 += inputs.len();
        let analyses = cx.tracer.analyze(inputs);
        let mut per: Vec<Vec<(usize, FileAnalysis)>> =
            (0..states.len()).map(|_| Vec::new()).collect();
        for ((si, ei), a) in owners.into_iter().zip(analyses) {
            per[si].push((ei, a));
        }
        for (((id, pkg, reused), files), rels) in states.into_iter().zip(per).zip(rels_of) {
            let (key, links): (String, Vec<(Id, String)>) = match reused {
                Some(s) => {
                    stats.0 += 1;
                    let links = s
                        .get()
                        .obs
                        .links
                        .iter()
                        .map(|l| {
                            (
                                (l.name.to_string(), l.version.to_string()),
                                l.root.to_string(),
                            )
                        })
                        .collect();
                    (s.get().key.to_string(), links)
                }
                None => {
                    stats.1 += 1;
                    let root_abs = cx_root(cx, &pkg.root);
                    let file_ix: HashMap<&str, u32> = rels
                        .iter()
                        .enumerate()
                        .map(|(i, r)| (r.as_str(), i as u32))
                        .collect();
                    let tcx = TargetCx {
                        cx,
                        root_abs: &root_abs,
                        own: None,
                        file_ix: &file_ix,
                        follow: true,
                    };
                    let files = files
                        .into_iter()
                        .map(|(i, a)| (i, crate::build::FileSource::Fresh(a)))
                        .collect();
                    let mut s = assemble(
                        cx,
                        pkg.clone(),
                        &root_abs,
                        &rels,
                        files,
                        None,
                        &HashMap::new(),
                        |_| None,
                        &tcx,
                    );
                    let loaded = shard::decode(&shard::encode(&s), "new").expect("own encoding");
                    s.key = third_party_key(cx, &pkg, obs_digest(cx, loaded.get()));
                    stats.2 += write_shard(cx, &s)? as usize;
                    let links = s
                        .obs
                        .links
                        .iter()
                        .map(|l| ((l.name.clone(), l.version.clone()), l.root.clone()))
                        .collect();
                    (s.key, links)
                }
            };
            for (lid, lroot) in links {
                // Workspace links and lockfile-only links (nothing installed to trace).
                if lid.1.is_empty() || lroot.is_empty() || done.contains_key(&lid) || lid == id {
                    continue;
                }
                let e = queue.entry(lid).or_insert_with(|| lroot.clone());
                if lroot < *e {
                    *e = lroot;
                }
            }
            done.insert(
                id.clone(),
                ShardEntry {
                    key,
                    kind: "third-party".to_owned(),
                    name: id.0,
                    version: Some(id.1),
                    root: pkg.root,
                },
            );
        }
    }
    Ok((done.into_values().collect(), stats))
}

fn cx_root(cx: &Cx, root: &str) -> PathBuf {
    if root.starts_with('/') {
        PathBuf::from(root)
    } else {
        cx.repo.join(root)
    }
}

/// Code files of an installed package, relative to its root, sorted; nested
/// `node_modules` and symlinked directories are skipped.
fn package_files(cx: &Cx, root: &Path) -> Vec<String> {
    let fs: &DirCache = cx.tracer.fs();
    let langs = cx.tracer.languages();
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((d, prefix)) = stack.pop() {
        let Some(listing) = fs.list(&d) else {
            continue;
        };
        for e in listing.iter() {
            let name = e.name().to_string_lossy();
            let rel = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            match e.kind() {
                turborepo_trace_core::fs::EntryKind::Dir if name != "node_modules" => {
                    stack.push((d.join(e.name()), rel));
                }
                turborepo_trace_core::fs::EntryKind::File
                    if langs.by_extension(Path::new(&rel)).is_some() =>
                {
                    out.push(rel);
                }
                _ => {}
            }
        }
    }
    out.sort_unstable();
    out
}
