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
    //    root handling (including `ignore_missing_packages`).
    let mut roots: Vec<(&String, Vec<u32>)> = Vec::with_capacity(workspaces.len());
    for (workspace, unresolved) in workspaces {
        let mut root_ids = Vec::with_capacity(unresolved.len());
        for (name, specifier) in unresolved {
            match lockfile.resolve_package(workspace, name, specifier) {
                Ok(Some(pkg)) => root_ids.push(intern(pkg, &mut packages)),
                Ok(None) => {}
                Err(Error::MissingWorkspace(_)) if ignore_missing_packages => {}
                Err(e) => return Err(e),
            }
        }
        roots.push((workspace, root_ids));
    }

    // 2. Global graph: BFS from all roots, resolving each distinct (name, version)
    //    edge once.
    let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); packages.len()];
    let mut edge_memo: FxHashMap<(String, String), Option<u32>> = FxHashMap::default();
    let mut stack: Vec<u32> = {
        let mut seen = vec![false; packages.len()];
        let mut stack = Vec::new();
        for (_, root_ids) in &roots {
            for &id in root_ids {
                if !seen[id as usize] {
                    seen[id as usize] = true;
                    stack.push(id);
                }
            }
        }
        stack
    };
    let mut expanded = vec![false; packages.len()];
    while let Some(id) = stack.pop() {
        if expanded[id as usize] {
            continue;
        }
        expanded[id as usize] = true;

        let Some(deps) = lockfile.all_dependencies(&packages[id as usize].key)? else {
            continue;
        };
        for (name, version) in deps.as_ref() {
            let target = match edge_memo.get(&(name.clone(), version.clone())) {
                Some(cached) => *cached,
                None => {
                    let resolved = match resolver.resolve_edge(name, version)? {
                        TransitiveEdgeResolution::Global(resolved) => resolved,
                        TransitiveEdgeResolution::WorkspaceSensitive => return Ok(None),
                    };
                    let target = resolved.map(|pkg| {
                        let id = intern(pkg, &mut packages);
                        if id as usize >= adjacency.len() {
                            adjacency.resize(id as usize + 1, Vec::new());
                            expanded.resize(id as usize + 1, false);
                        }
                        id
                    });
                    edge_memo.insert((name.clone(), version.clone()), target);
                    target
                }
            };
            if let Some(target) = target {
                adjacency[id as usize].push(target);
                if !expanded[target as usize] {
                    stack.push(target);
                }
            }
        }
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
    // ordering (`Package`'s derived `Ord`). Closure members are emitted in id
    // order per chunk; a final u32 sort by rank yields the canonical sorted
    // closure without comparing strings per element.
    let rank: Vec<u32> = {
        let mut order: Vec<u32> = (0..node_count as u32).collect();
        order.sort_unstable_by(|&a, &b| packages[a as usize].cmp(&packages[b as usize]));
        let mut rank = vec![0u32; node_count];
        for (pos, &id) in order.iter().enumerate() {
            rank[id as usize] = pos as u32;
        }
        rank
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

    let mut member_ids: HashMap<String, Vec<u32>> = workspace_root_sccs
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
                let member = member as usize;
                if (chunk_start..chunk_end).contains(&member) {
                    let bit = member - chunk_start;
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
            // Inserted for every workspace when `member_ids` was built; a
            // miss is unreachable but must not panic under the workspace
            // lint policy.
            let Some(ids) = member_ids.get_mut(*workspace) else {
                continue;
            };
            for (word_idx, &word) in accumulator.iter().enumerate() {
                let mut word = word;
                while word != 0 {
                    let bit = word.trailing_zeros() as usize;
                    word &= word - 1;
                    let id = chunk_start + word_idx * 64 + bit;
                    ids.push(id as u32);
                }
            }
        }
    }

    let result: SortedClosures = member_ids
        .into_iter()
        .map(|(ws, mut ids)| {
            ids.sort_unstable_by_key(|&id| rank[id as usize]);
            let closure = ids
                .into_iter()
                .map(|id| Arc::clone(&packages[id as usize]))
                .collect();
            (ws, closure)
        })
        .collect();

    Ok(Some(result))
}

struct SccResult {
    /// SCCs in emission order: every SCC's successors appear earlier.
    components: Vec<Vec<u32>>,
    /// Node id -> index into `components`.
    scc_of: Vec<u32>,
}

/// Iterative Tarjan SCC. Recursion-free: lockfile graphs can chain
/// thousands of packages deep.
fn tarjan_scc(adjacency: &[Vec<u32>]) -> SccResult {
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
    fn assert_dp_matches_legacy(
        lockfile: &PnpmLockfile,
        workspaces: HashMap<String, BTreeMap<String, String>>,
    ) {
        let via_dp = {
            let resolver = crate::Lockfile::transitive_edge_resolver(lockfile)
                .expect("pnpm supports edge resolution");
            all_transitive_closures_dp(lockfile, resolver.as_ref(), &workspaces, false)
                .expect("dp closure")
        };
        let legacy: HashMap<String, HashSet<Package>> = workspaces
            .iter()
            .map(|(ws, deps)| {
                let closure =
                    crate::transitive_closure(lockfile, ws, deps.clone(), false).expect("legacy");
                (ws.clone(), closure)
            })
            .collect();
        match via_dp {
            Some(dp) => {
                let dp_as_sets: HashMap<String, HashSet<Package>> = dp
                    .iter()
                    .map(|(ws, closure)| {
                        assert!(
                            closure.is_sorted_by(|a, b| a <= b),
                            "dp closure for {ws} must be sorted by (key, version)"
                        );
                        (
                            ws.clone(),
                            closure.iter().map(|pkg| (**pkg).clone()).collect(),
                        )
                    })
                    .collect();
                assert_eq!(dp_as_sets, legacy, "dp result must match the legacy walk");
            }
            None => panic!("dp unexpectedly fell back for a uniform lockfile"),
        }
    }

    fn workspaces_of(lockfile: &PnpmLockfile) -> HashMap<String, BTreeMap<String, String>> {
        lockfile.test_workspaces()
    }

    #[test]
    fn test_dp_matches_legacy_on_repo_lockfile() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let lockfile_path = std::path::Path::new(manifest_dir)
            .join("../..")
            .join("pnpm-lock.yaml");
        let bytes = std::fs::read(&lockfile_path).expect("repo lockfile readable");
        let lockfile = PnpmLockfile::from_bytes(&bytes).expect("parse");
        assert_dp_matches_legacy(&lockfile, workspaces_of(&lockfile));
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
