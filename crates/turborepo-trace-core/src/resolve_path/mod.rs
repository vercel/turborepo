//! A generic, configurable path resolver for the language plugins.
//!
//! Many languages resolve module specifiers the same basic way: relative to the
//! referencing file, absolute, or by searching a list of roots, then probing
//! extensions and index files. [`PathResolver`] implements that once on top of
//! the core [`DirCache`], so probing is answered from cached directory listings
//! instead of `stat` calls.
//!
//! ```
//! use turborepo_trace_core::resolve_path::{PathResolver, PathResolverConfig};
//!
//! let resolver = PathResolver::new(PathResolverConfig {
//!     extensions: vec!["py".into()],
//!     index_files: vec!["__init__.py".into()],
//!     ..Default::default()
//! });
//! # let _ = resolver;
//! ```
//!
//! Shared resolution helpers every language reuses:
//!
//! - [`assets`]: runtime file references (fs reads, directory listings, assets)
//!   and module patterns, with nft's glob rules and a language-configured
//!   [`AssetRules`] (package boundary, skipped directories, file filter).
//! - [`sharedlib`]: shared libraries shipped next to native binaries
//!   ([`SharedLibs`]).
//! - [`special`]: the package special-case format ([`special::FileCase`]) and
//!   its common actions.

pub mod assets;
pub mod sharedlib;
pub mod special;

use std::path::{Path, PathBuf};

pub use assets::{AssetRules, GlobOut, MissingAsset};
pub use sharedlib::SharedLibs;

use crate::{
    DirCache, Resolution, ResolveCx, Span, Target, Uncertainty, UncertaintyKind,
    fs::{FileKind, Lookup, Stat, normalize},
};

/// Configuration for [`PathResolver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathResolverConfig {
    /// Search roots for bare specifiers (neither `./`, `../` nor `/`), tried in
    /// order.
    pub roots: Vec<PathBuf>,
    /// Extensions without the dot, appended in order when the exact path is not
    /// a file (`./a` tries `./a.js`, `./a.json`, ...).
    pub extensions: Vec<String>,
    /// File names tried inside a directory, in order (`index.js`,
    /// `__init__.py`, `mod.rs`).
    pub index_files: Vec<String>,
    /// Try bare specifiers relative to the referencing file before the roots
    /// (C-style `#include "x.h"`).
    pub bare_relative_to_file: bool,
    /// Resolve a directory without a matching index file to [`Resolved::Dir`]
    /// instead of failing.
    pub allow_dirs: bool,
    /// On failure, retry with ASCII case-insensitive matching and report
    /// [`Resolved::CaseMismatch`]. Default `true`.
    pub detect_case_mismatch: bool,
}

impl Default for PathResolverConfig {
    fn default() -> Self {
        PathResolverConfig {
            roots: Vec::new(),
            extensions: Vec::new(),
            index_files: Vec::new(),
            bare_relative_to_file: false,
            allow_dirs: false,
            detect_case_mismatch: true,
        }
    }
}

/// The outcome of [`PathResolver::resolve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// A file, spelled exactly as on disk.
    File(PathBuf),
    /// A directory without index file (only with
    /// [`PathResolverConfig::allow_dirs`]).
    Dir(PathBuf),
    /// Found only case-insensitively. `path` is the on-disk spelling.
    CaseMismatch {
        /// The on-disk spelling.
        path: PathBuf,
        /// Whether it is a directory (see [`PathResolverConfig::allow_dirs`]).
        is_dir: bool,
    },
    /// Nothing matched.
    NotFound {
        /// Every path probed, in order, for diagnostics.
        tried: Vec<PathBuf>,
    },
}

/// How a specifier is anchored.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpecKind {
    /// `./x`, `../x`, `.` or `..`.
    Relative,
    /// `/x`.
    Absolute,
    /// Anything else.
    Bare,
}

/// Classifies a specifier.
pub fn spec_kind(spec: &str) -> SpecKind {
    if spec.starts_with('/') {
        SpecKind::Absolute
    } else if spec == "." || spec == ".." || spec.starts_with("./") || spec.starts_with("../") {
        SpecKind::Relative
    } else {
        SpecKind::Bare
    }
}

/// A configurable resolver. Cheap to share across threads.
#[derive(Clone, Debug, Default)]
pub struct PathResolver {
    cfg: PathResolverConfig,
}

/// Probe mode: exact listing match, or case-insensitive fallback.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Mode {
    Exact,
    CaseInsensitive,
}

impl PathResolver {
    /// Creates a resolver.
    pub fn new(cfg: PathResolverConfig) -> Self {
        PathResolver { cfg }
    }

    /// The configuration.
    pub fn config(&self) -> &PathResolverConfig {
        &self.cfg
    }

    /// Candidate base paths for `spec`, normalized, in probing order.
    pub fn candidates(&self, spec: &str, from_dir: &Path) -> Vec<PathBuf> {
        match spec_kind(spec) {
            SpecKind::Relative => vec![normalize(&from_dir.join(spec))],
            SpecKind::Absolute => vec![normalize(Path::new(spec))],
            SpecKind::Bare => {
                let mut v = Vec::with_capacity(self.cfg.roots.len() + 1);
                if self.cfg.bare_relative_to_file {
                    v.push(normalize(&from_dir.join(spec)));
                }
                v.extend(self.cfg.roots.iter().map(|r| normalize(&r.join(spec))));
                v
            }
        }
    }

    /// Resolves `spec` from a file in `from_dir`.
    ///
    /// For each candidate: the exact path if it is a file; then the path with
    /// each extension appended; then, if it is a directory, each index
    /// file; then the directory itself if allowed. A trailing `/` restricts
    /// the specifier to directories. All candidates are tried exactly
    /// before any case-insensitive retry.
    pub fn resolve(&self, spec: &str, from_dir: &Path, fs: &DirCache) -> Resolved {
        let mut tried = Vec::new();
        if spec.is_empty() {
            return Resolved::NotFound { tried };
        }
        let dir_only = spec.ends_with('/');
        let candidates = self.candidates(spec, from_dir);
        for &mode in &[Mode::Exact, Mode::CaseInsensitive] {
            if mode == Mode::CaseInsensitive && !self.cfg.detect_case_mismatch {
                break;
            }
            for base in &candidates {
                let record = mode == Mode::Exact;
                if let Some(r) = self.probe(base, dir_only, fs, mode, record.then_some(&mut tried))
                {
                    return r;
                }
            }
        }
        Resolved::NotFound { tried }
    }

    fn probe(
        &self,
        base: &Path,
        dir_only: bool,
        fs: &DirCache,
        mode: Mode,
        mut tried: Option<&mut Vec<PathBuf>>,
    ) -> Option<Resolved> {
        let mut check = |p: PathBuf, want: FileKind| -> Option<Resolved> {
            let found = match mode {
                Mode::Exact => match fs.stat(&p) {
                    Some(Stat { kind, .. }) if kind == want => Some(None),
                    _ => None,
                },
                Mode::CaseInsensitive => match fs.lookup(&p) {
                    Lookup::CaseMismatch { actual, stat } if stat.kind == want => {
                        Some(Some(actual))
                    }
                    _ => None,
                },
            };
            match found {
                None => {
                    if let Some(t) = tried.as_deref_mut() {
                        t.push(p);
                    }
                    None
                }
                Some(None) => Some(if want == FileKind::Dir {
                    Resolved::Dir(p)
                } else {
                    Resolved::File(p)
                }),
                Some(Some(actual)) => Some(Resolved::CaseMismatch {
                    path: actual,
                    is_dir: want == FileKind::Dir,
                }),
            }
        };

        if !dir_only {
            if let Some(r) = check(base.to_path_buf(), FileKind::File) {
                return Some(r);
            }
            for ext in &self.cfg.extensions {
                let mut s = base.as_os_str().to_owned();
                s.push(".");
                s.push(ext);
                if let Some(r) = check(PathBuf::from(s), FileKind::File) {
                    return Some(r);
                }
            }
        }
        // Directory: index files, then the directory itself.
        let dir = match mode {
            Mode::Exact => fs.is_dir(base).then(|| base.to_path_buf()),
            Mode::CaseInsensitive => match fs.lookup(base) {
                Lookup::Found(Stat {
                    kind: FileKind::Dir,
                    ..
                }) => Some(base.to_path_buf()),
                Lookup::CaseMismatch {
                    actual,
                    stat:
                        Stat {
                            kind: FileKind::Dir,
                            ..
                        },
                } => Some(actual),
                _ => None,
            },
        };
        let dir = dir?;
        let dir_mismatch = dir != base;
        for idx in &self.cfg.index_files {
            let p = dir.join(idx);
            let r = check(p.clone(), FileKind::File);
            match r {
                Some(Resolved::File(f)) if dir_mismatch => {
                    return Some(Resolved::CaseMismatch {
                        path: f,
                        is_dir: false,
                    });
                }
                Some(r) => return Some(r),
                None => {
                    // In case-insensitive mode an exactly spelled index file inside a
                    // mismatched directory is still a mismatch.
                    if mode == Mode::CaseInsensitive && dir_mismatch && fs.is_file(&p) {
                        return Some(Resolved::CaseMismatch {
                            path: p,
                            is_dir: false,
                        });
                    }
                }
            }
        }
        if self.cfg.allow_dirs {
            return Some(if dir_mismatch {
                Resolved::CaseMismatch {
                    path: dir,
                    is_dir: true,
                }
            } else {
                Resolved::Dir(dir)
            });
        }
        None
    }

    /// Resolves `spec` and converts the outcome into a core [`Resolution`]:
    /// - a file or directory becomes [`Target::File`] / [`Target::Dir`];
    /// - a case mismatch becomes the on-disk target plus a
    ///   [`UncertaintyKind::CaseMismatch`] record (it works on macOS, breaks on
    ///   Linux);
    /// - failure becomes [`Target::Unresolved`] with
    ///   [`UncertaintyKind::UnresolvedSpecifier`].
    pub fn resolve_to(
        &self,
        spec: &str,
        from_dir: &Path,
        cx: &ResolveCx<'_>,
        span: Span,
    ) -> Resolution {
        match self.resolve(spec, from_dir, cx.fs) {
            Resolved::File(p) => Resolution::file(cx.intern(&p)),
            Resolved::Dir(p) => Resolution::target(Target::Dir(cx.intern(&p))),
            Resolved::CaseMismatch { path, is_dir } => {
                let id = cx.intern(&path);
                let t = if is_dir {
                    Target::Dir(id)
                } else {
                    Target::File(id)
                };
                Resolution::target(t).with_uncertainty(Uncertainty::new(
                    UncertaintyKind::CaseMismatch,
                    span,
                    format!("{spec} matches {} only case-insensitively", path.display()),
                ))
            }
            Resolved::NotFound { tried } => Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::UnresolvedSpecifier,
                span,
                format!("cannot resolve {spec} ({} paths tried)", tried.len()),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn tree() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("src/pkg")).unwrap();
        fs::create_dir_all(r.join("src/empty")).unwrap();
        fs::create_dir_all(r.join("lib/Util")).unwrap();
        fs::write(r.join("src/a.js"), "").unwrap();
        fs::write(r.join("src/b.json"), "").unwrap();
        fs::write(r.join("src/pkg/index.js"), "").unwrap();
        fs::write(r.join("lib/mod.py"), "").unwrap();
        fs::write(r.join("lib/Util/__init__.py"), "").unwrap();
        fs::write(r.join("src/Readme.MD"), "").unwrap();
        t
    }

    fn js() -> PathResolver {
        PathResolver::new(PathResolverConfig {
            extensions: vec!["js".into(), "json".into()],
            index_files: vec!["index.js".into()],
            ..Default::default()
        })
    }

    #[test]
    fn spec_kinds() {
        assert_eq!(spec_kind("./a"), SpecKind::Relative);
        assert_eq!(spec_kind("../a"), SpecKind::Relative);
        assert_eq!(spec_kind(".."), SpecKind::Relative);
        assert_eq!(spec_kind("/a"), SpecKind::Absolute);
        assert_eq!(spec_kind("a"), SpecKind::Bare);
        assert_eq!(spec_kind(".a"), SpecKind::Bare);
    }

    #[test]
    fn relative_exact_extension_and_index() {
        let t = tree();
        let src = t.path().join("src");
        let c = DirCache::new();
        let r = js();
        assert_eq!(
            r.resolve("./a.js", &src, &c),
            Resolved::File(src.join("a.js"))
        );
        assert_eq!(r.resolve("./a", &src, &c), Resolved::File(src.join("a.js")));
        assert_eq!(
            r.resolve("./b", &src, &c),
            Resolved::File(src.join("b.json"))
        );
        assert_eq!(
            r.resolve("./pkg", &src, &c),
            Resolved::File(src.join("pkg/index.js"))
        );
        assert_eq!(
            r.resolve("./pkg/", &src, &c),
            Resolved::File(src.join("pkg/index.js"))
        );
        assert_eq!(
            r.resolve("../src/./a", &src, &c),
            Resolved::File(src.join("a.js"))
        );
        match r.resolve("./zzz", &src, &c) {
            Resolved::NotFound { tried } => assert_eq!(tried.len(), 3),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            r.resolve("", &src, &c),
            Resolved::NotFound { tried: vec![] }
        );
    }

    #[test]
    fn dirs() {
        let t = tree();
        let src = t.path().join("src");
        let c = DirCache::new();
        assert!(matches!(
            js().resolve("./empty", &src, &c),
            Resolved::NotFound { .. }
        ));
        let r = PathResolver::new(PathResolverConfig {
            allow_dirs: true,
            ..Default::default()
        });
        assert_eq!(
            r.resolve("./empty", &src, &c),
            Resolved::Dir(src.join("empty"))
        );
        // Trailing slash: directories only.
        assert!(matches!(
            r.resolve("./a.js/", &src, &c),
            Resolved::NotFound { .. }
        ));
    }

    #[test]
    fn bare_with_roots() {
        let t = tree();
        let root = t.path();
        let c = DirCache::new();
        let r = PathResolver::new(PathResolverConfig {
            roots: vec![root.join("src"), root.join("lib")],
            extensions: vec!["py".into()],
            index_files: vec!["__init__.py".into()],
            ..Default::default()
        });
        assert_eq!(
            r.resolve("mod", Path::new("/elsewhere"), &c),
            Resolved::File(root.join("lib/mod.py"))
        );
        assert_eq!(
            r.resolve("Util", Path::new("/elsewhere"), &c),
            Resolved::File(root.join("lib/Util/__init__.py"))
        );
        let abs = root.join("src/a.js");
        assert_eq!(
            r.resolve(abs.to_str().unwrap(), Path::new("/x"), &c),
            Resolved::File(abs.clone())
        );

        let r2 = PathResolver::new(PathResolverConfig {
            bare_relative_to_file: true,
            ..Default::default()
        });
        assert_eq!(
            r2.resolve("a.js", &root.join("src"), &c),
            Resolved::File(abs)
        );
    }

    #[test]
    fn case_mismatch() {
        let t = tree();
        let root = t.path();
        let c = DirCache::new();
        let r = js();
        assert_eq!(
            r.resolve("./README.md", &root.join("src"), &c),
            Resolved::CaseMismatch {
                path: root.join("src/Readme.MD"),
                is_dir: false
            }
        );
        assert_eq!(
            r.resolve("./A", &root.join("src"), &c),
            Resolved::CaseMismatch {
                path: root.join("src/a.js"),
                is_dir: false
            }
        );
        // Directory component mismatch with an index file.
        let py = PathResolver::new(PathResolverConfig {
            roots: vec![root.join("lib")],
            index_files: vec!["__init__.py".into()],
            ..Default::default()
        });
        assert_eq!(
            py.resolve("util", Path::new("/x"), &c),
            Resolved::CaseMismatch {
                path: root.join("lib/Util/__init__.py"),
                is_dir: false
            }
        );
        let strict = PathResolver::new(PathResolverConfig {
            detect_case_mismatch: false,
            ..r.config().clone()
        });
        assert!(matches!(
            strict.resolve("./A", &root.join("src"), &c),
            Resolved::NotFound { .. }
        ));
    }
}
