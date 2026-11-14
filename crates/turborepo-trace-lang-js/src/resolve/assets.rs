//! JS configuration of the shared asset resolution
//! ([`turborepo_trace_core::resolve_path::assets`]): nft's rules for Node.
//!
//! - Package boundary: `node_modules` ([`PackageBoundary::NODE_MODULES`]). A
//!   file inside `node_modules` never emits assets outside its `node_modules`
//!   tree; `node_modules` directories themselves are never expanded as assets.
//! - Globs never enter nested `node_modules` directories below their static
//!   root (nft adds `ignore: <root>/**/node_modules/**` to every glob).
//! - Asset globs skip C sources and build files (`.h`, `.c`, `.cpp`, `.cmake`)
//!   and `README.md`/`CHANGELOG.md`, as nft does ([`keep_asset`]).
//!
//! Node's package lookup ([`find_package_dir`]) also lives here.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use turborepo_trace_core::{PackageBoundary, ResolveCx, os::OsStrExt, resolve_path::AssetRules};

/// Directory names never entered by globs.
pub(crate) const SKIP_DIRS: &[&str] = &["node_modules"];

/// nft's `excludeAssetExtensions` and `excludeAssetFiles`.
pub fn keep_asset(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.as_bytes()).unwrap_or_default();
    if matches!(
        name,
        b"CHANGELOG.md" | b"README.md" | b"readme.md" | b"changelog.md"
    ) {
        return false;
    }
    !matches!(
        p.extension().map(|e| e.as_bytes()),
        Some(b"h" | b"c" | b"cpp" | b"cmake")
    )
}

/// The JS asset rules (see the module docs).
pub const JS_ASSET_RULES: AssetRules = AssetRules {
    boundary: PackageBoundary::NODE_MODULES,
    skip_dirs: SKIP_DIRS,
    keep: keep_asset,
};

/// The directory of package `name` as Node would find it from `from_dir`: the
/// first `<ancestor>/node_modules/<name>` that exists.
pub(crate) fn find_package_dir(cx: &ResolveCx<'_>, from_dir: &Path, name: &str) -> Option<PathBuf> {
    let mut cur = Some(from_dir);
    while let Some(dir) = cur {
        if dir.file_name() != Some(OsStr::new("node_modules")) {
            let cand = dir.join("node_modules").join(name);
            if cx.fs.is_dir(&cand) {
                return Some(cand);
            }
        }
        cur = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions() {
        assert!(!keep_asset(Path::new("/x/README.md")));
        assert!(!keep_asset(Path::new("/x/a.cpp")));
        assert!(keep_asset(Path::new("/x/a.json")));
    }
}
