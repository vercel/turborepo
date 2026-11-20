//! Shared libraries shipped next to native binaries (nft's `sharedlibEmit`),
//! for any language: a native binary inside a package (`.node` addon, Python
//! extension module, ...) pulls in every shared library of the platform below
//! its package root (`*.so`, `*.so.*`, and `*.dylib` on macOS, `*.dll` on
//! Windows; see [`turborepo_trace_core::platform::is_sharedlib_for`]), ignoring
//! nested [`AssetRules::skip_dirs`]. The edge has kind [`RefKind::SharedLib`]
//! and a spec `special-case sharedlib: <package>`.
//!
//! ```ignore
//! let libs = SharedLibs::new(&cx.options.platform(), RULES);
//! // After resolving a reference:
//! libs.add_for_targets(&res.targets.to_vec(), &mut res.extra, is_native_binary, cx);
//! // Or for an explicit root (Python wheels' `<dist>.libs/`):
//! libs.edge_in(&root, "numpy", cx);
//! ```

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dashmap::DashMap;

use crate::{
    ExtraEdge, FileId, RefKind, ResolveCx, Span, Target, Uncertainty, UncertaintyKind,
    platform::{Os, Platform, is_sharedlib_for},
    resolve_path::assets::{AssetRules, GlobOut, special_spec},
};

/// Whether `path` is a native binary whose package may ship shared libraries: a
/// Node addon (`.node`), a Python extension (`.so`, `.pyd`) or a shared library
/// (`.so.*`, `.dylib`, `.dll`). Languages may use a narrower test (JS uses
/// `.node` only, like nft).
pub fn is_native_binary(path: &Path) -> bool {
    use crate::os::OsStrExt;
    let name = path.file_name().map(|n| n.as_bytes()).unwrap_or_default();
    [&b".node"[..], b".pyd", b".dylib", b".dll"]
        .iter()
        .any(|e| name.ends_with(e))
        || is_sharedlib_for(Os::Linux, name)
}

/// Shared-library emission for one resolver, cached per package root.
#[derive(Debug)]
pub struct SharedLibs {
    os: Os,
    rules: AssetRules,
    cache: DashMap<PathBuf, Arc<GlobOut>>,
}

fn file_name_bytes(p: &Path) -> &[u8] {
    p.file_name()
        .map(|n| n.as_encoded_bytes())
        .unwrap_or_default()
}

fn keep_linux(p: &Path) -> bool {
    is_sharedlib_for(Os::Linux, file_name_bytes(p))
}

fn keep_darwin(p: &Path) -> bool {
    is_sharedlib_for(Os::Darwin, file_name_bytes(p))
}

fn keep_windows(p: &Path) -> bool {
    is_sharedlib_for(Os::Windows, file_name_bytes(p))
}

impl SharedLibs {
    /// Shared libraries for `platform`, searched with `rules` (package boundary
    /// and skipped directories; the rules' file filter is not used).
    pub fn new(platform: &Platform, rules: AssetRules) -> Self {
        SharedLibs {
            os: platform.os,
            rules,
            cache: DashMap::new(),
        }
    }

    /// Drops the per-package cache.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// The shared-library edge for native binary `file`, searched in its
    /// package root
    /// ([`turborepo_trace_core::PackageBoundary::package_root`]). `None`
    /// outside packages or when the package ships none.
    pub fn edge(&self, file: &Path, cx: &ResolveCx<'_>) -> Option<ExtraEdge> {
        let (root, name) = self
            .rules
            .boundary
            .package_root_and_name_from(file, cx.fs.local_offset(file))?;
        self.edge_in(&root, &name, cx)
    }

    /// The shared-library edge for every library below `root`, labelled with
    /// package `name`. `None` when there are none.
    pub fn edge_in(&self, root: &Path, name: &str, cx: &ResolveCx<'_>) -> Option<ExtraEdge> {
        let g = match self.cache.get(root) {
            Some(g) => Arc::clone(&g),
            None => {
                let keep: fn(&Path) -> bool = match self.os {
                    Os::Darwin => keep_darwin,
                    Os::Windows => keep_windows,
                    _ => keep_linux,
                };
                let mut g = self.rules.tree_glob(root, cx, keep);
                g.pattern = format!("{}/**/*.{{so,so.*,dylib}}", root.display());
                let g = Arc::new(g);
                self.cache.insert(root.to_path_buf(), Arc::clone(&g));
                g
            }
        };
        if g.matches.is_empty() {
            return None;
        }
        let mut e = ExtraEdge::new(
            RefKind::SharedLib,
            Target::Glob {
                pattern: g.pattern.clone().into(),
                matches: g.matches.iter().map(|m| cx.intern(m)).collect(),
                truncated: g.truncated,
            },
            special_spec(format!("sharedlib: {name}")),
        );
        if g.truncated {
            e = e.with_uncertainty(Uncertainty::new(
                UncertaintyKind::PatternTooBroad,
                Span::DUMMY,
                format!(
                    "shared libraries of {name}: more than {} files",
                    cx.options.max_glob
                ),
            ));
        }
        Some(e)
    }

    /// Adds one shared-library edge per package for every native binary
    /// (`is_native`) among `targets` and the targets of `extra` (files and
    /// glob matches).
    pub fn add_for_targets(
        &self,
        targets: &[Target],
        extra: &mut Vec<ExtraEdge>,
        is_native: fn(&Path) -> bool,
        cx: &ResolveCx<'_>,
    ) {
        let mut seen: Vec<PathBuf> = Vec::new();
        let extra_targets: Vec<Target> = extra.iter().map(|e| e.target.clone()).collect();
        for t in targets.iter().chain(&extra_targets) {
            let ids: &[FileId] = match t {
                Target::File(id) => std::slice::from_ref(id),
                Target::Glob { matches, .. } => matches,
                _ => &[],
            };
            for id in ids {
                let Some(p) = cx.interner.try_path(*id) else {
                    continue;
                };
                if !is_native(&p) {
                    continue;
                }
                let Some((pkg, _)) = self
                    .rules
                    .boundary
                    .package_root_and_name_from(&p, cx.fs.local_offset(&p))
                else {
                    continue;
                };
                if seen.contains(&pkg) {
                    continue;
                }
                seen.push(pkg);
                if let Some(e) = self.edge(&p, cx) {
                    extra.push(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_binaries() {
        for n in [
            "a.node",
            "x.cpython-312-x86_64-linux-gnu.so",
            "x.pyd",
            "libz.so.1",
            "a.dylib",
        ] {
            assert!(is_native_binary(Path::new(n)), "{n}");
        }
        for n in ["a.js", "a.py", "also.something"] {
            assert!(!is_native_binary(Path::new(n)), "{n}");
        }
    }
}
