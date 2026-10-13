//! Shared transitive-closure computation over the global package graph.
//!
//! The legacy walk in `lib.rs` recomputes every workspace's closure
//! independently: each workspace re-resolves and re-visits every edge of its
//! closure, so shared dependencies (the overwhelming majority in a monorepo)
//! are traversed once per workspace that reaches them. This module instead
//! computes closures once, globally:
//!
//! 1. resolve each workspace's *direct* dependencies exactly as the legacy walk
//!    does (direct resolution is inherently workspace-scoped),
//! 2. build the global package graph, resolving each distinct transitive edge
//!    once through a [`TransitiveEdgeResolver`],
//! 3. condense strongly connected components (npm graphs have cycles) with an
//!    iterative Tarjan pass, which emits SCCs successors-first,
//! 4. run a bottom-up closure DP over the condensation using bitsets, so set
//!    unions are word-parallel, then
//! 5. materialize each workspace's closure as the union of its direct
//!    dependencies' SCC closures.
//!
//! Soundness: transitive resolution is not workspace-independent for every
//! lockfile format (pnpm importers can shadow a transitive dependency's
//! name; Bun resolves through the workspace). The resolver therefore
//! *proves* uniformity per edge and reports [`TransitiveEdgeResolution::
//! WorkspaceSensitive`] when any workspace could resolve an edge
//! differently. A single sensitive edge aborts the DP (`Ok(None)`) and the
//! caller falls back to the legacy walk, so behavior is preserved
//! unconditionally — the DP is only ever a faster path to the identical
//! answer.
//!
//! Memory: closure bitsets are computed in fixed-size id-range chunks
//! (blocked transitive closure), so peak usage is bounded by
//! `SCC count x chunk size` regardless of graph size, at no asymptotic
//! cost: each chunk pass touches every condensation edge once.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::{
    Error, Lockfile, Package, SortedClosures, TransitiveEdgeResolution, TransitiveEdgeResolver,
};

/// Upper bound for one chunk's closure bitsets. 64MiB of transient memory
/// keeps large graphs to a handful of passes without noticeable pressure.
const MAX_CHUNK_BYTES: usize = 64 * 1024 * 1024;
/// Below this chunk width the per-pass graph traversal overhead dominates;
/// prefer the legacy walk for absurdly large graphs instead.
const MIN_CHUNK_BITS: usize = 1024;
/// Minimum items per rayon task when resolution runs in parallel. Levels
/// smaller than this run on the calling thread.
const PARALLEL_MIN_LEN: usize = 32;

/// Maps `items` in order, on rayon workers when `parallel` is set. Resolution
/// for formats with cheap lookups (pnpm, npm, yarn1) is faster on the calling
/// thread than the pool round-trips a parallel level costs.
fn par_map<T: Sync, R: Send>(
    items: &[T],
    parallel: bool,
    f: impl Fn(&T) -> R + Sync + Send,
) -> Vec<R> {
    if parallel {
        items
            .par_iter()
            .with_min_len(PARALLEL_MIN_LEN)
            .map(f)
            .collect()
    } else {
        items.iter().map(f).collect()
    }
}

/// Compute all workspace closures via the shared DP. Returns `Ok(None)`
/// when the graph contains a workspace-sensitive edge (caller must use the
/// legacy walk).
pub(crate) fn all_transitive_closures_dp<L: Lockfile + ?Sized>(
    lockfile: &L,
    resolver: &dyn TransitiveEdgeResolver,
    workspaces: &HashMap<String, BTreeMap<String, String>>,
    ignore_missing_packages: bool,
) -> Result<Option<SortedClosures>, Error> {
    // Interned package table. Ids index `packages` and bitset positions.
    let mut ids: FxHashMap<Package, u32> = FxHashMap::default();
    let mut packages: Vec<Arc<Package>> = Vec::new();
    let mut intern = |pkg: Package, packages: &mut Vec<Arc<Package>>| -> u32 {
        *ids.entry(pkg).or_insert_with_key(|key| {
            packages.push(Arc::new(key.clone()));
            (packages.len() - 1) as u32
        })
    };

    // 1. Direct dependencies stay workspace-resolved, mirroring the legacy walk's
    //    root handling (including `ignore_missing_packages`). Workspaces may
    //    resolve in parallel; interning stays sequential.
    let parallel = resolver.parallel();
    let workspace_list: Vec<(&String, &BTreeMap<String, String>)> = workspaces.iter().collect();
    let resolved_roots = par_map(&workspace_list, parallel, |&(workspace, unresolved)| {
        let mut resolved = Vec::with_capacity(unresolved.len());
        for (name, specifier) in unresolved {
            match lockfile.resolve_package(workspace, name, specifier) {
                Ok(Some(pkg)) => resolved.push(pkg),
                Ok(None) => {}
                Err(Error::MissingWorkspace(_)) if ignore_missing_packages => {}
                Err(e) => return Err(e),
            }
        }
        Ok((workspace, resolved))
    })
    .into_iter()
    .collect::<Result<Vec<_>, Error>>()?;
    let roots: Vec<(&String, Vec<u32>)> = resolved_roots
        .into_iter()
        .map(|(workspace, resolved)| {
            let root_ids = resolved
                .into_iter()
                .map(|pkg| intern(pkg, &mut packages))
                .collect();
            (workspace, root_ids)
        })
        .collect();

    // 2. Global graph: level-synchronous BFS from all roots, resolving each
    //    distinct (name, version) edge once. Each level fetches the frontier's
    //    dependency maps and resolves its not-yet-seen edges (in parallel when the
    //    resolver asks for it), then interns results and records adjacency
    //    sequentially.
    let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); packages.len()];
    // `name\0version` -> target, probed through a reused key buffer so a
    // lookup never allocates.
    let mut edge_memo: FxHashMap<String, Option<u32>> = FxHashMap::default();
    let mut key_buf = String::new();
    // Nodes already expanded or queued for expansion.
    let mut scheduled = vec![false; packages.len()];
    let mut frontier: Vec<u32> = Vec::new();
    for (_, root_ids) in &roots {
        for &id in root_ids {
            if !scheduled[id as usize] {
                scheduled[id as usize] = true;
                frontier.push(id);
            }
        }
    }
    // Per-edge outcome of the memo probe: a known target, or an index into
    // this level's pending (first-seen) edges.
    enum EdgeRef {
        Known(Option<u32>),
        Pending(u32),
    }
    while !frontier.is_empty() {
        let level_deps = par_map(&frontier, parallel, |&id| {
            lockfile.all_dependencies(&packages[id as usize].key)
        })
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?;

        let mut edge_refs: Vec<EdgeRef> = Vec::new();
        let mut pending: Vec<(&str, &str)> = Vec::new();
        {
            let mut pending_index: FxHashMap<(&str, &str), u32> = FxHashMap::default();
            for deps in level_deps.iter().flatten() {
                for (name, version) in deps.as_ref() {
                    key_buf.clear();
                    key_buf.push_str(name);
                    key_buf.push('\0');
                    key_buf.push_str(version);
                    let edge_ref = match edge_memo.get(key_buf.as_str()) {
                        Some(&target) => EdgeRef::Known(target),
                        None => {
                            let edge = (name.as_str(), version.as_str());
                            EdgeRef::Pending(*pending_index.entry(edge).or_insert_with(|| {
                                pending.push(edge);
                                (pending.len() - 1) as u32
                            }))
                        }
                    };
                    edge_refs.push(edge_ref);
                }
            }
        }

        let outcomes = par_map(&pending, parallel, |&(name, version)| {
            resolver.resolve_edge(name, version)
        });
        // A single workspace-sensitive edge invalidates the DP; prefer the
        // fallback over any error so the legacy walk reports exactly what it
        // always has.
        if outcomes
            .iter()
            .any(|outcome| matches!(outcome, Ok(TransitiveEdgeResolution::WorkspaceSensitive)))
        {
            return Ok(None);
        }
        let mut pending_targets: Vec<Option<u32>> = Vec::with_capacity(pending.len());
        for (&(name, version), outcome) in pending.iter().zip(outcomes) {
            let target = match outcome? {
                TransitiveEdgeResolution::Global(resolved) => {
                    resolved.map(|pkg| intern(pkg, &mut packages))
                }
                TransitiveEdgeResolution::WorkspaceSensitive => return Ok(None),
            };
            let mut key = String::with_capacity(name.len() + version.len() + 1);
            key.push_str(name);
            key.push('\0');
            key.push_str(version);
            edge_memo.insert(key, target);
            pending_targets.push(target);
        }
        if packages.len() > adjacency.len() {
            adjacency.resize(packages.len(), Vec::new());
            scheduled.resize(packages.len(), false);
        }

        let mut next = Vec::new();
        let mut edge_refs = edge_refs.into_iter();
        for (&id, deps) in frontier.iter().zip(&level_deps) {
            let Some(deps) = deps else {
                continue;
            };
            // `edge_refs` holds one entry per edge, in this same order.
            for edge_ref in edge_refs.by_ref().take(deps.len()) {
                let target = match edge_ref {
                    EdgeRef::Known(target) => target,
                    EdgeRef::Pending(index) => pending_targets[index as usize],
                };
                let Some(target) = target else {
                    continue;
                };
                adjacency[id as usize].push(target);
                if !scheduled[target as usize] {
                    scheduled[target as usize] = true;
                    next.push(target);
                }
            }
        }
        frontier = next;
    }

    let node_count = packages.len();
    if node_count == 0 {
        return Ok(Some(
            roots
                .into_iter()
                .map(|(ws, _)| (ws.clone(), Vec::new()))
                .collect(),
        ));
    }

    // Rank permutation: position of each package id under (key, version)
    // ordering (`Package`'s derived `Ord`). Bitset positions are ranks, not
    // ids, so enumerating a closure's set bits (chunks ascend, bits ascend
    // within a chunk) yields its members already in canonical sorted order:
    // no per-workspace sort, and no string comparisons per element.
    let (order, rank): (Vec<u32>, Vec<u32>) = {
        let mut order: Vec<u32> = (0..node_count as u32).collect();
        order.sort_unstable_by(|&a, &b| packages[a as usize].cmp(&packages[b as usize]));
        let mut rank = vec![0u32; node_count];
        for (pos, &id) in order.iter().enumerate() {
            rank[id as usize] = pos as u32;
        }
        (order, rank)
    };

    // 3. SCC condensation. Tarjan emits SCCs successors-first, so a later SCC's
    //    closure only references already-computed rows.
    let sccs = tarjan_scc(&adjacency);
    let scc_count = sccs.components.len();

    // Condensation edges, deduplicated per source SCC.
    let mut scc_edges: Vec<Vec<u32>> = vec![Vec::new(); scc_count];
    {
        let mut last_seen = vec![u32::MAX; scc_count];
        for (scc_idx, members) in sccs.components.iter().enumerate() {
            for &node in members {
                for &succ in &adjacency[node as usize] {
                    let succ_scc = sccs.scc_of[succ as usize];
                    if succ_scc != scc_idx as u32 && last_seen[succ_scc as usize] != scc_idx as u32
                    {
                        last_seen[succ_scc as usize] = scc_idx as u32;
                        scc_edges[scc_idx].push(succ_scc);
                    }
                }
            }
        }
    }

    // Workspace roots as deduplicated SCC indices.
    let workspace_root_sccs: Vec<(&String, Vec<u32>)> = roots
        .iter()
        .map(|(ws, root_ids)| {
            let mut root_sccs: Vec<u32> = root_ids
                .iter()
                .map(|&id| sccs.scc_of[id as usize])
                .collect();
            root_sccs.sort_unstable();
            root_sccs.dedup();
            (*ws, root_sccs)
        })
        .collect();

    // 4 + 5. Blocked bitset DP with per-chunk materialization.
    let chunk_bits = {
        let by_memory = (MAX_CHUNK_BYTES / scc_count.max(1)) * 8;
        let wanted = node_count.next_multiple_of(64);
        wanted.min(by_memory).max(64) / 64 * 64
    };
    if chunk_bits < MIN_CHUNK_BITS && node_count > chunk_bits {
        // Graph so large the blocked DP would need excessive passes.
        return Ok(None);
    }

    let mut result: SortedClosures = workspace_root_sccs
        .iter()
        .map(|(ws, _)| ((*ws).clone(), Vec::new()))
        .collect();

    let words_per_chunk = chunk_bits / 64;
    let mut closures = vec![0u64; scc_count * words_per_chunk];
    let mut accumulator = vec![0u64; words_per_chunk];

    for chunk_start in (0..node_count).step_by(chunk_bits) {
        let chunk_end = (chunk_start + chunk_bits).min(node_count);
        if chunk_start > 0 {
            closures.fill(0);
        }

        for (scc_idx, (members, edges)) in sccs.components.iter().zip(&scc_edges).enumerate() {
            // Successor rows live strictly before this SCC's row (Tarjan
            // emission order), so split the buffer to union them in.
            let (done, current) = closures.split_at_mut(scc_idx * words_per_chunk);
            let row = &mut current[..words_per_chunk];
            for &member in members {
                let position = rank[member as usize] as usize;
                if (chunk_start..chunk_end).contains(&position) {
                    let bit = position - chunk_start;
                    row[bit / 64] |= 1u64 << (bit % 64);
                }
            }
            for &succ in edges {
                let succ_row = &done[succ as usize * words_per_chunk..][..words_per_chunk];
                for (dst, src) in row.iter_mut().zip(succ_row) {
                    *dst |= src;
                }
            }
        }

        for (workspace, root_sccs) in &workspace_root_sccs {
            if root_sccs.is_empty() {
                continue;
            }
            accumulator.fill(0);
            for &scc_idx in root_sccs {
                let row = &closures[scc_idx as usize * words_per_chunk..][..words_per_chunk];
                for (dst, src) in accumulator.iter_mut().zip(row) {
                    *dst |= src;
                }
            }
            // Inserted for every workspace when `result` was built; a miss is
            // unreachable but must not panic under the workspace lint policy.
            let Some(closure) = result.get_mut(*workspace) else {
                continue;
            };
            let members: u32 = accumulator.iter().map(|word| word.count_ones()).sum();
            closure.reserve(members as usize);
            // Bits are rank positions and chunks ascend, so members are
            // appended in canonical (key, version) order.
            for (word_idx, &word) in accumulator.iter().enumerate() {
                let mut word = word;
                while word != 0 {
                    let bit = word.trailing_zeros() as usize;
                    word &= word - 1;
                    let position = chunk_start + word_idx * 64 + bit;
                    let id = order[position];
                    closure.push(Arc::clone(&packages[id as usize]));
                }
            }
        }
    }

    Ok(Some(result))
}

pub(crate) struct SccResult {
    /// SCCs in emission order: every SCC's successors appear earlier.
    pub(crate) components: Vec<Vec<u32>>,
    /// Node id -> index into `components`.
    pub(crate) scc_of: Vec<u32>,
}

/// Iterative Tarjan SCC. Recursion-free: lockfile graphs can chain
/// thousands of packages deep.
pub(crate) fn tarjan_scc(adjacency: &[Vec<u32>]) -> SccResult {
    let n = adjacency.len();
    const UNVISITED: u32 = u32::MAX;
    let mut index = vec![UNVISITED; n];
    let mut lowlink = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut scc_of = vec![0u32; n];
    let mut components = Vec::new();
    let mut next_index = 0u32;
    let mut tarjan_stack: Vec<u32> = Vec::new();
    // (node, next edge offset)
    let mut call_stack: Vec<(u32, usize)> = Vec::new();

    for start in 0..n as u32 {
        if index[start as usize] != UNVISITED {
            continue;
        }
        call_stack.push((start, 0));
        while let Some(&mut (node, ref mut edge_pos)) = call_stack.last_mut() {
            let node_usize = node as usize;
            if *edge_pos == 0 {
                index[node_usize] = next_index;
                lowlink[node_usize] = next_index;
                next_index += 1;
                tarjan_stack.push(node);
                on_stack[node_usize] = true;
            }

            if let Some(&succ) = adjacency[node_usize].get(*edge_pos) {
                *edge_pos += 1;
                let succ_usize = succ as usize;
                if index[succ_usize] == UNVISITED {
                    call_stack.push((succ, 0));
                } else if on_stack[succ_usize] {
                    lowlink[node_usize] = lowlink[node_usize].min(index[succ_usize]);
                }
                continue;
            }

            // Node finished.
            call_stack.pop();
            if let Some(&mut (parent, _)) = call_stack.last_mut() {
                let parent = parent as usize;
                lowlink[parent] = lowlink[parent].min(lowlink[node_usize]);
            }
            if lowlink[node_usize] == index[node_usize] {
                let scc_idx = components.len() as u32;
                let mut members = Vec::new();
                // `node` is always on the Tarjan stack here, so this drains
                // at most to it; an empty stack is unreachable.
                while let Some(member) = tarjan_stack.pop() {
                    on_stack[member as usize] = false;
                    scc_of[member as usize] = scc_idx;
                    members.push(member);
                    if member == node {
                        break;
                    }
                }
                components.push(members);
            }
        }
    }

    SccResult { components, scc_of }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::PnpmLockfile;

    /// The DP must agree with the legacy per-workspace walk. The legacy
    /// walk is invoked directly (single-workspace entry point never uses
    /// the DP), making it an independent oracle.
    fn assert_dp_matches_legacy<L: Lockfile>(
        lockfile: &L,
        workspaces: HashMap<String, BTreeMap<String, String>>,
    ) -> SortedClosures {
        let resolver = crate::Lockfile::transitive_edge_resolver(lockfile)
            .expect("lockfile supports edge resolution");
        let dp = all_transitive_closures_dp(lockfile, resolver.as_ref(), &workspaces, false)
            .expect("dp closure")
            .expect("dp unexpectedly fell back for a uniform lockfile");
        assert_matches_legacy(lockfile, &workspaces, &dp);
        dp
    }

    fn assert_matches_legacy<L: Lockfile>(
        lockfile: &L,
        workspaces: &HashMap<String, BTreeMap<String, String>>,
        closures: &SortedClosures,
    ) {
        let legacy: HashMap<String, HashSet<Package>> = workspaces
            .iter()
            .map(|(ws, deps)| {
                let closure =
                    crate::transitive_closure(lockfile, ws, deps.clone(), false).expect("legacy");
                (ws.clone(), closure)
            })
            .collect();
        let as_sets: HashMap<String, HashSet<Package>> = closures
            .iter()
            .map(|(ws, closure)| {
                assert!(
                    closure.windows(2).all(|pair| pair[0] < pair[1]),
                    "closure for {ws} must be sorted and duplicate-free"
                );
                (
                    ws.clone(),
                    closure.iter().map(|pkg| (**pkg).clone()).collect(),
                )
            })
            .collect();
        assert_eq!(
            as_sets, legacy,
            "result must match the independent legacy walks"
        );
    }

    fn workspaces_of(lockfile: &PnpmLockfile) -> HashMap<String, BTreeMap<String, String>> {
        lockfile.test_workspaces()
    }

    #[test]
    fn test_public_closures_match_legacy_on_repo_lockfile() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let lockfile_path = std::path::Path::new(manifest_dir)
            .join("../..")
            .join("pnpm-lock.yaml");
        let bytes = std::fs::read(&lockfile_path).expect("repo lockfile readable");
        let lockfile = PnpmLockfile::from_bytes(&bytes).expect("parse");
        let workspaces = workspaces_of(&lockfile);
        // Repository updates can introduce workspace-sensitive edges. Either
        // path must retain exact parity; uniform fixtures below require the DP.
        let closures = crate::all_transitive_closures_sorted(&lockfile, workspaces.clone(), false)
            .expect("public closures");
        assert_matches_legacy(&lockfile, &workspaces, &closures);
    }

    #[test]
    fn test_pnpm_dp_matches_legacy_and_shares_packages_on_uniform_fixture() {
        // Shared dependencies and an SCC exercise the fast path independently
        // of dependency updates to the repository's own lockfile.
        let yaml = r#"lockfileVersion: '9.0'
importers:
  apps/web:
    dependencies:
      carrier:
        specifier: ^1.0.0
        version: 1.0.0
  apps/docs:
    dependencies:
      shared:
        specifier: ^1.0.0
        version: 1.0.0
packages:
  carrier@1.0.0:
    resolution: {integrity: sha512-carrier}
  shared@1.0.0:
    resolution: {integrity: sha512-shared}
  cycle-a@1.0.0:
    resolution: {integrity: sha512-a}
  cycle-b@1.0.0:
    resolution: {integrity: sha512-b}
snapshots:
  carrier@1.0.0:
    dependencies:
      shared: 1.0.0
  shared@1.0.0:
    dependencies:
      cycle-a: 1.0.0
  cycle-a@1.0.0:
    dependencies:
      cycle-b: 1.0.0
  cycle-b@1.0.0:
    dependencies:
      cycle-a: 1.0.0
"#;
        let lockfile = PnpmLockfile::from_bytes(yaml.as_bytes()).expect("parse");
        let dp = assert_dp_matches_legacy(&lockfile, workspaces_of(&lockfile));
        let mut seen = HashMap::new();
        let mut shared_count = 0;
        for closure in dp.values() {
            for package in closure {
                if let Some(previous) = seen.insert(package.as_ref(), package) {
                    assert!(
                        Arc::ptr_eq(previous, package),
                        "shared identities must reuse Arc"
                    );
                    shared_count += 1;
                }
            }
        }
        assert_eq!(
            shared_count, 3,
            "both workspaces must share the dependency SCC"
        );
    }

    #[test]
    fn test_npm_dp_matches_legacy_with_workspace_nesting() {
        // `apps/web` has a workspace-nested foo@2.0.0 while everything else
        // hoists foo@1.0.0. Direct deps are resolved per workspace (step 1),
        // and every transitive edge references an exact lockfile key, so the
        // DP must reproduce the legacy result including the nested variation
        // and the deeply nested `nested-only` package.
        let lockfile_json = serde_json::json!({
            "name": "test",
            "lockfileVersion": 3,
            "packages": {
                "": { "name": "test" },
                "apps/web": {
                    "name": "web",
                    "dependencies": { "foo": "^2.0.0", "shared": "^1.0.0" }
                },
                "apps/docs": {
                    "name": "docs",
                    "dependencies": { "foo": "^1.0.0", "shared": "^1.0.0" }
                },
                "node_modules/foo": {
                    "version": "1.0.0",
                    "dependencies": { "shared": "^1.0.0" }
                },
                "apps/web/node_modules/foo": {
                    "version": "2.0.0",
                    "dependencies": { "shared": "^1.0.0", "nested-only": "^1.0.0" }
                },
                "apps/web/node_modules/foo/node_modules/nested-only": {
                    "version": "1.0.0"
                },
                "node_modules/shared": {
                    "version": "1.0.0",
                    "dependencies": { "chain-a": "*" }
                },
                "node_modules/chain-a": {
                    "version": "1.0.0",
                    "dependencies": { "chain-b": "*" }
                },
                "node_modules/chain-b": { "version": "1.0.0" }
            }
        });
        let lockfile =
            crate::NpmLockfile::load(lockfile_json.to_string().as_bytes()).expect("parse");

        let workspaces: HashMap<String, BTreeMap<String, String>> = [
            (
                "apps/web".to_string(),
                BTreeMap::from([
                    ("foo".to_string(), "^2.0.0".to_string()),
                    ("shared".to_string(), "^1.0.0".to_string()),
                ]),
            ),
            (
                "apps/docs".to_string(),
                BTreeMap::from([
                    ("foo".to_string(), "^1.0.0".to_string()),
                    ("shared".to_string(), "^1.0.0".to_string()),
                ]),
            ),
        ]
        .into_iter()
        .collect();

        assert_dp_matches_legacy(&lockfile, workspaces);
    }

    #[test]
    fn test_yarn1_dp_matches_legacy() {
        let yarn_lock = r#"# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
# yarn lockfile v1


bar@^2.0.0:
  version "2.1.0"

foo@^1.0.0:
  version "1.0.0"
  dependencies:
    bar "^2.0.0"

lodash@^4.0.0, lodash@^4.17.0:
  version "4.17.21"
"#;
        let lockfile = crate::Yarn1Lockfile::from_bytes(yarn_lock.as_bytes()).expect("parse");

        let workspaces: HashMap<String, BTreeMap<String, String>> = [
            (
                "apps/web".to_string(),
                BTreeMap::from([
                    ("foo".to_string(), "^1.0.0".to_string()),
                    ("lodash".to_string(), "^4.17.0".to_string()),
                ]),
            ),
            (
                "apps/docs".to_string(),
                BTreeMap::from([("lodash".to_string(), "^4.0.0".to_string())]),
            ),
        ]
        .into_iter()
        .collect();

        assert_dp_matches_legacy(&lockfile, workspaces);
    }

    fn berry_workspaces(
        entries: &[(&str, &[(&str, &str)])],
    ) -> HashMap<String, BTreeMap<String, String>> {
        entries
            .iter()
            .map(|(workspace, deps)| {
                (
                    workspace.to_string(),
                    deps.iter()
                        .map(|(name, range)| (name.to_string(), range.to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn test_berry_dp_matches_legacy_with_overrides_and_cycle() {
        // Exercises an SCC, an unscoped override applied to a transitive edge
        // (workspace-independent), and a parent-scoped override whose parent
        // is an external package (never matches a workspace locator).
        let yaml = r#"__metadata:
  version: 8
  cacheKey: 10c0

"a@workspace:packages/a":
  version: 0.0.0-use.local
  resolution: "a@workspace:packages/a"
  dependencies:
    carrier: "npm:^1.0.0"
    lodash: "npm:^4.0.0"
  languageName: unknown
  linkType: soft

"b@workspace:packages/b":
  version: 0.0.0-use.local
  resolution: "b@workspace:packages/b"
  dependencies:
    lodash: "npm:^4.17.0"
  languageName: unknown
  linkType: soft

"carrier@npm:^1.0.0":
  version: 1.0.0
  resolution: "carrier@npm:1.0.0"
  dependencies:
    leaf: "npm:^1.0.0"
    shared: "npm:^1.0.0"
  checksum: 10c0-carrier
  languageName: node
  linkType: hard

"cycle-a@npm:^1.0.0":
  version: 1.0.0
  resolution: "cycle-a@npm:1.0.0"
  dependencies:
    cycle-b: "npm:^1.0.0"
  checksum: 10c0-cycle-a
  languageName: node
  linkType: hard

"cycle-b@npm:^1.0.0":
  version: 1.0.0
  resolution: "cycle-b@npm:1.0.0"
  dependencies:
    cycle-a: "npm:^1.0.0"
    shared: "npm:^1.0.0"
  checksum: 10c0-cycle-b
  languageName: node
  linkType: hard

"leaf@npm:2.0.0":
  version: 2.0.0
  resolution: "leaf@npm:2.0.0"
  checksum: 10c0-leaf2
  languageName: node
  linkType: hard

"leaf@npm:^1.0.0":
  version: 1.0.0
  resolution: "leaf@npm:1.0.0"
  checksum: 10c0-leaf1
  languageName: node
  linkType: hard

"lodash@npm:^4.0.0, lodash@npm:^4.17.0":
  version: 4.17.21
  resolution: "lodash@npm:4.17.21"
  dependencies:
    shared: "npm:^1.0.0"
  checksum: 10c0-lodash
  languageName: node
  linkType: hard

"root@workspace:.":
  version: 0.0.0-use.local
  resolution: "root@workspace:."
  languageName: unknown
  linkType: soft

"shared@npm:1.0.0, shared@npm:^1.0.0":
  version: 1.0.0
  resolution: "shared@npm:1.0.0"
  dependencies:
    cycle-a: "npm:^1.0.0"
  checksum: 10c0-shared
  languageName: node
  linkType: hard
"#;
        let manifest = crate::BerryManifest::with_resolutions([
            ("carrier/leaf".to_string(), "2.0.0".to_string()),
            ("shared".to_string(), "1.0.0".to_string()),
        ]);
        let lockfile = crate::BerryLockfile::load(yaml.as_bytes(), Some(manifest)).expect("parse");
        let workspaces = berry_workspaces(&[
            ("packages/a", &[("carrier", "^1.0.0"), ("lodash", "^4.0.0")]),
            ("packages/b", &[("lodash", "^4.17.0")]),
        ]);

        let dp = assert_dp_matches_legacy(&lockfile, workspaces);
        let a: Vec<&str> = dp["packages/a"]
            .iter()
            .map(|pkg| pkg.key.as_str())
            .collect();
        assert!(
            a.contains(&"leaf@npm:2.0.0") && !a.contains(&"leaf@npm:1.0.0"),
            "scoped override from an external parent must apply: {a:?}"
        );
    }

    #[test]
    fn test_berry_dp_falls_back_on_workspace_scoped_override() {
        // `a/shared` overrides `shared` only when the depending package is
        // workspace `a`. The legacy walk resolves transitive edges with the
        // workspace's locator, so `carrier`'s `shared` edge resolves to 2.0.0
        // for `a` but 1.0.0 for `b`. The DP must detect this and fall back.
        let yaml = r#"__metadata:
  version: 8
  cacheKey: 10c0

"a@workspace:packages/a":
  version: 0.0.0-use.local
  resolution: "a@workspace:packages/a"
  dependencies:
    carrier: "npm:^1.0.0"
  languageName: unknown
  linkType: soft

"b@workspace:packages/b":
  version: 0.0.0-use.local
  resolution: "b@workspace:packages/b"
  dependencies:
    carrier: "npm:^1.0.0"
  languageName: unknown
  linkType: soft

"carrier@npm:^1.0.0":
  version: 1.0.0
  resolution: "carrier@npm:1.0.0"
  dependencies:
    shared: "npm:^1.0.0"
  checksum: 10c0-carrier
  languageName: node
  linkType: hard

"root@workspace:.":
  version: 0.0.0-use.local
  resolution: "root@workspace:."
  languageName: unknown
  linkType: soft

"shared@npm:2.0.0":
  version: 2.0.0
  resolution: "shared@npm:2.0.0"
  checksum: 10c0-shared2
  languageName: node
  linkType: hard

"shared@npm:^1.0.0":
  version: 1.0.0
  resolution: "shared@npm:1.0.0"
  checksum: 10c0-shared1
  languageName: node
  linkType: hard
"#;
        let manifest =
            crate::BerryManifest::with_resolutions([("a/shared".to_string(), "2.0.0".to_string())]);
        let lockfile = crate::BerryLockfile::load(yaml.as_bytes(), Some(manifest)).expect("parse");
        let workspaces = berry_workspaces(&[
            ("packages/a", &[("carrier", "^1.0.0")]),
            ("packages/b", &[("carrier", "^1.0.0")]),
        ]);

        let resolver = crate::Lockfile::transitive_edge_resolver(&lockfile)
            .expect("berry supports edge resolution");
        let dp = all_transitive_closures_dp(&lockfile, resolver.as_ref(), &workspaces, false)
            .expect("dp run");
        assert!(
            dp.is_none(),
            "workspace-scoped override must force fallback"
        );

        let via_public =
            crate::all_transitive_closures_sorted(&lockfile, workspaces.clone(), false)
                .expect("public");
        assert_matches_legacy(&lockfile, &workspaces, &via_public);
        let keys =
            |ws: &str| -> Vec<&str> { via_public[ws].iter().map(|pkg| pkg.key.as_str()).collect() };
        assert!(keys("packages/a").contains(&"shared@npm:2.0.0"));
        assert!(keys("packages/b").contains(&"shared@npm:1.0.0"));
    }

    #[test]
    fn test_dp_falls_back_on_divergent_edge() {
        // Workspace `.` pins `shadowed` with an exact specifier equal to
        // the transitive version string but resolving to a peer-suffixed
        // snapshot. The transitive edge `shadowed@1.0.0` then resolves
        // differently for importers with and without that entry: the
        // ladder's `resolved_specifier == override_specifier` arm returns
        // the peer-suffixed version for `.`, while the agnostic path keeps
        // the plain one. The DP must detect this and fall back.
        let yaml = r#"lockfileVersion: '6.0'

importers:

  .:
    dependencies:
      shadowed:
        specifier: 1.0.0
        version: 1.0.0(peer@2.0.0)
  packages/a:
    dependencies:
      carrier:
        specifier: ^1.0.0
        version: 1.0.0

packages:

  /carrier@1.0.0:
    resolution: {integrity: sha512-carrier}
    dependencies:
      shadowed: 1.0.0

  /shadowed@1.0.0:
    resolution: {integrity: sha512-shadowed}

  /shadowed@1.0.0(peer@2.0.0):
    resolution: {integrity: sha512-shadowedpeer}

  /peer@2.0.0:
    resolution: {integrity: sha512-peer}
"#;
        let lockfile = PnpmLockfile::from_bytes(yaml.as_bytes()).expect("parse");
        let workspaces = workspaces_of(&lockfile);

        let resolver = crate::Lockfile::transitive_edge_resolver(&lockfile)
            .expect("pnpm supports edge resolution");
        let dp = all_transitive_closures_dp(&lockfile, resolver.as_ref(), &workspaces, false)
            .expect("dp run");
        assert!(dp.is_none(), "divergent edge must force fallback");

        // And the public entry point must still produce the legacy result.
        let via_public =
            crate::all_transitive_closures(&lockfile, workspaces.clone(), false).expect("public");
        for (ws, deps) in workspaces {
            let legacy = crate::transitive_closure(&lockfile, &ws, deps, false).expect("legacy");
            assert_eq!(via_public[&ws], legacy);
        }
    }
}
