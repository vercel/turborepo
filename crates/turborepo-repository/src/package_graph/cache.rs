//! File-based cache for the PackageGraph.
//!
//! On each `turbo run`, we compute a content-hash fingerprint of every input
//! file that determines the package graph (all workspace package.json files,
//! the lockfile, workspace config, etc.). If the fingerprint matches a
//! previously cached graph, we deserialize it instead of rebuilding from
//! scratch.
//!
//! Correctness guarantees:
//! - Content hashes (xxHash64), not mtimes — immune to git checkout, cp
//!   --preserve, NFS clock skew.
//! - Workspace discovery (globwalk) always runs — new/removed packages detected.
//! - Any error during cache operations falls back silently to a full rebuild.
//! - Cache includes format version + turbo version — no cross-version skew.
//! - Atomic writes (tempfile + rename) — no partial reads.

use std::collections::{BTreeMap, HashMap};

use petgraph::graph::{Graph, NodeIndex};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_lockfiles::Lockfile;

use super::{PackageGraph, PackageInfo, PackageName, PackageNode};
use crate::package_manager::PackageManager;

/// Bump this when the serialized format changes.
const CACHE_FORMAT_VERSION: u32 = 1;

/// Content-hash fingerprint of all input files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// Sorted map of relative_path → xxHash64 hex string.
    pub file_hashes: BTreeMap<String, String>,
    /// Sorted list of discovered workspace package.json paths (relative to
    /// repo root). This ensures added/removed packages invalidate the cache
    /// even if no existing file changed.
    pub workspace_paths: Vec<String>,
}

/// The on-disk cache format.
#[derive(Debug, Serialize, Deserialize)]
pub struct CachedPackageGraph {
    cache_version: u32,
    turbo_version: String,
    pub fingerprint: Fingerprint,
    package_manager: PackageManager,
    packages: HashMap<PackageName, PackageInfo>,
    /// Graph nodes (in index order) and edges.
    nodes: Vec<PackageNode>,
    edges: Vec<(usize, usize)>,
}

/// Compute the xxHash64 of `data` and return it as a hex string.
fn xxhash64_hex(data: &[u8]) -> String {
    format!("{:016x}", xxhash_rust::xxh64::xxh64(data, 0))
}

/// Compute the fingerprint for all input files that determine the package
/// graph. `input_files` should be absolute paths. `workspace_paths` should be
/// the sorted list of workspace package.json paths relative to repo root.
///
/// Returns `None` if any file cannot be read (caller should fall back to full
/// build).
pub fn compute_fingerprint(
    repo_root: &AbsoluteSystemPath,
    input_files: &[AbsoluteSystemPathBuf],
    workspace_paths: Vec<String>,
) -> Option<Fingerprint> {
    let mut file_hashes = BTreeMap::new();
    for path in input_files {
        let contents = match std::fs::read(path.as_std_path()) {
            Ok(c) => c,
            Err(e) => {
                debug!("package graph cache: cannot read {}: {}", path, e);
                return None;
            }
        };
        let rel = match repo_root.anchor(path) {
            Ok(rel) => rel.to_string(),
            Err(_) => {
                debug!("package graph cache: cannot anchor {}", path);
                return None;
            }
        };
        file_hashes.insert(rel, xxhash64_hex(&contents));
    }
    Some(Fingerprint {
        file_hashes,
        workspace_paths,
    })
}

/// Try to load a cached PackageGraph from disk. Returns `None` on any error
/// or fingerprint mismatch.
pub fn try_load(
    cache_path: &AbsoluteSystemPath,
    current_fingerprint: &Fingerprint,
    turbo_version: &str,
    repo_root: &AbsoluteSystemPath,
    lockfile: Option<Box<dyn Lockfile>>,
) -> Option<PackageGraph> {
    let bytes = match std::fs::read(cache_path.as_std_path()) {
        Ok(b) => b,
        Err(e) => {
            debug!("package graph cache: cannot read cache file: {}", e);
            return None;
        }
    };

    let cached: CachedPackageGraph = match serde_json::from_slice(&bytes) {
        Ok(c) => c,
        Err(e) => {
            warn!("package graph cache: corrupt cache file, rebuilding: {}", e);
            // Best-effort delete the corrupt file
            let _ = std::fs::remove_file(cache_path.as_std_path());
            return None;
        }
    };

    // Validate version
    if cached.cache_version != CACHE_FORMAT_VERSION {
        debug!(
            "package graph cache: version mismatch (cached={}, current={})",
            cached.cache_version, CACHE_FORMAT_VERSION
        );
        return None;
    }
    if cached.turbo_version != turbo_version {
        debug!(
            "package graph cache: turbo version mismatch (cached={}, current={})",
            cached.turbo_version, turbo_version
        );
        return None;
    }

    // Validate fingerprint
    if &cached.fingerprint != current_fingerprint {
        debug!("package graph cache: fingerprint mismatch, rebuilding");
        return None;
    }

    // Reconstruct the petgraph
    reconstruct(cached, repo_root, lockfile)
}

/// Reconstruct a PackageGraph from cached data.
fn reconstruct(
    cached: CachedPackageGraph,
    repo_root: &AbsoluteSystemPath,
    lockfile: Option<Box<dyn Lockfile>>,
) -> Option<PackageGraph> {
    let mut graph = Graph::new();
    let mut node_lookup = HashMap::new();

    // Add nodes in order
    for node in &cached.nodes {
        let idx = graph.add_node(node.clone());
        node_lookup.insert(node.clone(), idx);
    }

    // Add edges
    for &(from_idx, to_idx) in &cached.edges {
        let from = NodeIndex::new(from_idx);
        let to = NodeIndex::new(to_idx);
        if from.index() >= cached.nodes.len() || to.index() >= cached.nodes.len() {
            warn!("package graph cache: invalid edge indices, rebuilding");
            return None;
        }
        graph.add_edge(from, to, ());
    }

    Some(PackageGraph {
        graph,
        node_lookup,
        packages: cached.packages,
        package_manager: cached.package_manager,
        lockfile,
        repo_root: repo_root.to_owned(),
        external_dep_to_internal_dependents: std::sync::OnceLock::new(),
    })
}

/// Save the PackageGraph to disk as a cache file. Errors are logged and
/// swallowed — cache writes are best-effort.
pub fn save(
    cache_path: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    fingerprint: Fingerprint,
    turbo_version: &str,
) {
    let cached = match serialize_graph(pkg_graph, fingerprint, turbo_version) {
        Some(c) => c,
        None => return,
    };

    let bytes = match serde_json::to_vec(&cached) {
        Ok(b) => b,
        Err(e) => {
            warn!("package graph cache: failed to serialize: {}", e);
            return;
        }
    };

    // Ensure parent directory exists
    if let Some(parent) = cache_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent.as_std_path()) {
            warn!(
                "package graph cache: failed to create cache directory: {}",
                e
            );
            return;
        }
    }

    // Atomic write: write to tempfile then rename
    let parent_dir = match cache_path.parent() {
        Some(p) => p,
        None => {
            warn!("package graph cache: cache path has no parent directory");
            return;
        }
    };

    let temp_path = parent_dir.join_component(&format!(
        ".package-graph-cache-{}.tmp",
        std::process::id()
    ));

    if let Err(e) = std::fs::write(temp_path.as_std_path(), &bytes) {
        warn!("package graph cache: failed to write temp file: {}", e);
        let _ = std::fs::remove_file(temp_path.as_std_path());
        return;
    }

    if let Err(e) = std::fs::rename(temp_path.as_std_path(), cache_path.as_std_path()) {
        warn!("package graph cache: failed to rename temp file: {}", e);
        let _ = std::fs::remove_file(temp_path.as_std_path());
    }
}

/// Convert a PackageGraph into the serializable CachedPackageGraph.
fn serialize_graph(
    pkg_graph: &PackageGraph,
    fingerprint: Fingerprint,
    turbo_version: &str,
) -> Option<CachedPackageGraph> {
    // Collect nodes in index order
    let node_count = pkg_graph.graph.node_count();
    let mut nodes = Vec::with_capacity(node_count);
    for idx in pkg_graph.graph.node_indices() {
        nodes.push(pkg_graph.graph[idx].clone());
    }

    // Collect edges as (from_index, to_index)
    let edges: Vec<(usize, usize)> = pkg_graph
        .graph
        .raw_edges()
        .iter()
        .map(|e| (e.source().index(), e.target().index()))
        .collect();

    Some(CachedPackageGraph {
        cache_version: CACHE_FORMAT_VERSION,
        turbo_version: turbo_version.to_string(),
        fingerprint,
        package_manager: pkg_graph.package_manager.clone(),
        packages: pkg_graph.packages.clone(),
        nodes,
        edges,
    })
}

/// The default cache file name within the `.turbo/cache/` directory.
pub const CACHE_FILE_NAME: &str = "package-graph-v1.json";

/// Collect all input file paths that determine the package graph.
///
/// `workspace_package_json_paths` should be the discovered workspace
/// package.json absolute paths.
pub fn collect_input_files(
    repo_root: &AbsoluteSystemPath,
    package_manager: &PackageManager,
    workspace_package_json_paths: &[AbsoluteSystemPathBuf],
    turbo_json_paths: &[AbsoluteSystemPathBuf],
) -> Vec<AbsoluteSystemPathBuf> {
    let mut files = Vec::new();

    // Root package.json
    files.push(repo_root.join_component("package.json"));

    // Lockfile
    let lockfile_path = package_manager.lockfile_path(repo_root);
    files.push(lockfile_path);

    // Workspace config (pnpm-workspace.yaml)
    if let Some(ws_config) = package_manager.workspace_configuration_path() {
        let config_path = repo_root.join_component(ws_config);
        if config_path.exists() {
            files.push(config_path);
        }
    }

    // .npmrc (affects link-workspace-packages for pnpm)
    let npmrc_path = repo_root.join_component(".npmrc");
    if npmrc_path.exists() {
        files.push(npmrc_path);
    }

    // .yarnrc.yml (for Berry)
    let yarnrc_path = repo_root.join_component(".yarnrc.yml");
    if yarnrc_path.exists() {
        files.push(yarnrc_path);
    }

    // All workspace package.json files
    files.extend(workspace_package_json_paths.iter().cloned());

    // Turbo.json files
    files.extend(turbo_json_paths.iter().cloned());

    files
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_xxhash64_hex() {
        let hash = xxhash64_hex(b"hello world");
        assert_eq!(hash.len(), 16); // 64-bit hex = 16 chars
        // Verify deterministic
        assert_eq!(hash, xxhash64_hex(b"hello world"));
        // Different content produces different hash
        assert_ne!(hash, xxhash64_hex(b"hello world!"));
    }

    #[test]
    fn test_fingerprint_equality() {
        let fp1 = Fingerprint {
            file_hashes: BTreeMap::from([
                ("a.json".to_string(), "abc123".to_string()),
                ("b.json".to_string(), "def456".to_string()),
            ]),
            workspace_paths: vec!["packages/a".to_string()],
        };
        let fp2 = fp1.clone();
        assert_eq!(fp1, fp2);

        let fp3 = Fingerprint {
            file_hashes: BTreeMap::from([
                ("a.json".to_string(), "abc123".to_string()),
                ("b.json".to_string(), "CHANGED".to_string()),
            ]),
            workspace_paths: vec!["packages/a".to_string()],
        };
        assert_ne!(fp1, fp3);
    }

    #[test]
    fn test_fingerprint_workspace_path_change() {
        let fp1 = Fingerprint {
            file_hashes: BTreeMap::from([("a.json".to_string(), "abc123".to_string())]),
            workspace_paths: vec!["packages/a".to_string()],
        };
        let fp2 = Fingerprint {
            file_hashes: BTreeMap::from([("a.json".to_string(), "abc123".to_string())]),
            workspace_paths: vec!["packages/a".to_string(), "packages/b".to_string()],
        };
        assert_ne!(fp1, fp2);
    }
}
