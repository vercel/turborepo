//! Package boundaries: where installed third-party packages live and which
//! directory is one package's root. Each language configures one
//! [`PackageBoundary`]:
//!
//! | Language | Preset | Marker directories |
//! |---|---|---|
//! | JS | [`PackageBoundary::NODE_MODULES`] | `node_modules/<name>`, `node_modules/@scope/<name>` |
//! | Python | [`PackageBoundary::PYTHON`] | `site-packages/<name>`, `dist-packages/<name>` |
//! | Go | [`PackageBoundary::GO`] | `pkg/mod/<module path>@<version>`, `vendor/` (containment only) |
//! | Rust | [`PackageBoundary::CARGO`] | `registry/src/<index>/<name>-<version>`, `git/checkouts/<name>-<hash>/<rev>`, `vendor/<name>` |
//!
//! Uses (nft's rules, generalized):
//! - [`PackageBoundary::containment_prefix`] / [`PackageBoundary::is_outside`]:
//!   a file inside a package directory tree never emits assets outside the
//!   first marker directory of its path (nft's package-boundary rule for
//!   `node_modules`).
//! - [`PackageBoundary::package_root`]: the package directory below the *last*
//!   marker of a path (nft's `getPackageBase`), for example the root a native
//!   addon's shared libraries are searched in.
//! - [`PackageBoundary::marker_of_dir`]: whether a directory is a marker
//!   directory itself, which asset globs refuse to expand as a whole (nft's
//!   `validWildcard`).
//!
//! Paths are absolute, normalized and compared as bytes.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use memchr::{memchr, memmem};

use crate::os::OsStrExt;

/// How many path segments below a marker directory name one package.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PackageName {
    /// npm: one segment, or two for `@scope/<name>` (nft's `getPackageBase`).
    Npm,
    /// A fixed number of segments (`site-packages/<name>`: 1; cargo's
    /// `registry/src/<index>/<name>-<version>`: 2). The path must continue
    /// below them.
    Segments(u8),
    /// A Go module in the module cache: segments up to and including the first
    /// one containing `@` (`github.com/!azure/sdk@v1.2.0`). The path must
    /// continue below it.
    GoModule,
}

/// One marker directory: a path segment (or `/`-separated segments such as
/// `pkg/mod`) below which packages live, and how a package is named below it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Marker {
    /// Directory name(s), without leading or trailing `/`.
    pub dir: &'static str,
    /// Package naming below the marker.
    pub name: PackageName,
}

/// A language's package layout (see the module docs).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageBoundary {
    /// Marker directories. Containment uses the first occurrence of any marker
    /// in a path, package roots the last one.
    pub markers: &'static [Marker],
}

impl PackageBoundary {
    /// JS: `node_modules`.
    pub const NODE_MODULES: PackageBoundary = PackageBoundary {
        markers: &[Marker {
            dir: "node_modules",
            name: PackageName::Npm,
        }],
    };

    /// Python: `site-packages` and Debian's `dist-packages`.
    pub const PYTHON: PackageBoundary = PackageBoundary {
        markers: &[
            Marker {
                dir: "site-packages",
                name: PackageName::Segments(1),
            },
            Marker {
                dir: "dist-packages",
                name: PackageName::Segments(1),
            },
        ],
    };

    /// Go: the module cache (`$GOMODCACHE`, by default `$GOPATH/pkg/mod`) and
    /// `vendor/`. Vendored module roots are not derivable from the path
    /// alone (they need `vendor/modules.txt`), so `vendor/` only bounds
    /// containment.
    pub const GO: PackageBoundary = PackageBoundary {
        markers: &[
            Marker {
                dir: "pkg/mod",
                name: PackageName::GoModule,
            },
            Marker {
                dir: "vendor",
                name: PackageName::GoModule,
            },
        ],
    };

    /// Rust: the cargo registry and git checkouts below `$CARGO_HOME`, and
    /// `vendor/`.
    pub const CARGO: PackageBoundary = PackageBoundary {
        markers: &[
            Marker {
                dir: "registry/src",
                name: PackageName::Segments(2),
            },
            Marker {
                dir: "git/checkouts",
                name: PackageName::Segments(2),
            },
            Marker {
                dir: "vendor",
                name: PackageName::Segments(1),
            },
        ],
    };

    /// The containment prefix of `path`: everything up to and including the
    /// first `/<marker>/` in it (`/app/node_modules/`), or `None` if the
    /// path is not inside a marker directory. Assets of such a file must
    /// stay below this prefix.
    pub fn containment_prefix<'p>(&self, path: &'p Path) -> Option<&'p [u8]> {
        self.containment_prefix_from(path, 0)
    }

    /// [`PackageBoundary::containment_prefix`] for the part of `path` from byte
    /// offset `from` on (`path[from..]` starts with `/`): a marker
    /// directory before it does not count. In a confined trace `from` is
    /// [`crate::DirCache::local_offset`]: the directories a checkout lives
    /// below are no part of the tree, whatever they are called (a checkout
    /// below `/ci/node_modules/app` is no installed package).
    pub fn containment_prefix_from<'p>(&self, path: &'p Path, from: usize) -> Option<&'p [u8]> {
        let b = path.as_os_str().as_bytes();
        let end = self
            .markers
            .iter()
            .filter_map(|m| first_marker(b, m.dir, from))
            .min()?;
        Some(&b[..end])
    }

    /// Whether `target` is outside the package boundary of `from` (see
    /// [`PackageBoundary::containment_prefix`]). Always `false` for files
    /// outside packages.
    pub fn is_outside(&self, from: &Path, target: &Path) -> bool {
        self.is_outside_from(from, 0, target)
    }

    /// [`PackageBoundary::is_outside`] with
    /// [`PackageBoundary::containment_prefix_from`] for `from` and its
    /// offset `offset`.
    pub fn is_outside_from(&self, from: &Path, offset: usize, target: &Path) -> bool {
        self.containment_prefix_from(from, offset)
            .is_some_and(|p| !target.as_os_str().as_bytes().starts_with(p))
    }

    /// The package root of `path`: the package directory below the last marker
    /// in it, or `None` outside packages or if the path does not reach
    /// below a package directory.
    pub fn package_root(&self, path: &Path) -> Option<PathBuf> {
        let (root, _) = self.package_root_and_name(path)?;
        Some(root)
    }

    /// [`PackageBoundary::package_root`] plus the package's name: the part of
    /// the root below the marker (`@scope/pkg`, `numpy`,
    /// `github.com/x/y@v1.0.0`).
    pub fn package_root_and_name(&self, path: &Path) -> Option<(PathBuf, String)> {
        self.package_root_and_name_from(path, 0)
    }

    /// [`PackageBoundary::package_root_and_name`] for the part of `path` from
    /// byte offset `from` on (see
    /// [`PackageBoundary::containment_prefix_from`]).
    pub fn package_root_and_name_from(
        &self,
        path: &Path,
        from: usize,
    ) -> Option<(PathBuf, String)> {
        let b = path.as_os_str().as_bytes();
        // The last marker occurrence across all markers.
        let (start, m) = self
            .markers
            .iter()
            .filter_map(|m| last_marker(b, m.dir, from).map(|end| (end, m)))
            .max_by_key(|(end, _)| *end)?;
        let rest = &b[start..];
        let len = match m.name {
            PackageName::Npm => {
                let mut end = memchr(b'/', rest)?;
                if rest.starts_with(b"@") {
                    end += 1 + memchr(b'/', &rest[end + 1..]).unwrap_or(rest.len() - end - 1);
                }
                end.min(rest.len())
            }
            PackageName::Segments(n) => {
                let mut end = 0usize;
                for i in 0..n {
                    if i > 0 {
                        end += 1;
                    }
                    end += memchr(b'/', &rest[end..])?;
                }
                end
            }
            PackageName::GoModule => {
                let mut end = 0usize;
                loop {
                    let seg_len = memchr(b'/', &rest[end..])?;
                    let seg = &rest[end..end + seg_len];
                    end += seg_len;
                    if memchr(b'@', seg).is_some() {
                        break end;
                    }
                    end += 1;
                }
            }
        };
        let root = PathBuf::from(OsStr::from_bytes(&b[..start + len]));
        let name = String::from_utf8_lossy(&rest[..len]).into_owned();
        Some((root, name))
    }

    /// If `dir` is itself a marker directory (`/app/node_modules`), that
    /// marker's name.
    pub fn marker_of_dir(&self, dir: &Path) -> Option<&'static str> {
        self.markers
            .iter()
            .find(|m| dir.ends_with(m.dir))
            .map(|m| m.dir)
    }

    /// Whether any component sequence of `path` is a marker directory.
    pub fn contains(&self, path: &Path) -> bool {
        self.containment_prefix(path).is_some()
    }

    /// [`PackageBoundary::contains`] for the part of `path` from byte offset
    /// `from` on (see [`PackageBoundary::containment_prefix_from`]).
    pub fn contains_from(&self, path: &Path, from: usize) -> bool {
        self.containment_prefix_from(path, from).is_some()
    }
}

/// Whether `dir` occurs at `b[i..]` as a whole `/<dir>/` segment run.
fn is_segment(b: &[u8], i: usize, dir: &[u8]) -> bool {
    i > 0 && b[i - 1] == b'/' && b.get(i + dir.len()) == Some(&b'/')
}

/// End offset (just past the trailing `/`) of the first `/<dir>/` in
/// `b[from..]`.
fn first_marker(b: &[u8], dir: &str, from: usize) -> Option<usize> {
    let d = dir.as_bytes();
    let mut from = from.min(b.len());
    while let Some(off) = memmem::find(&b[from..], d) {
        let i = from + off;
        if is_segment(b, i, d) {
            return Some(i + d.len() + 1);
        }
        from = i + 1;
    }
    None
}

/// End offset (just past the trailing `/`) of the last `/<dir>/` in
/// `b[from..]`.
fn last_marker(b: &[u8], dir: &str, from: usize) -> Option<usize> {
    let d = dir.as_bytes();
    let mut to = b.len();
    while let Some(i) = memmem::rfind(&b[..to], d) {
        // The segment's leading `/` is at `i - 1`: before the part asked about.
        if i <= from {
            return None;
        }
        if is_segment(b, i, d) {
            return Some(i + d.len() + 1);
        }
        // Next candidates end before this one does.
        to = i + d.len() - 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(b: &PackageBoundary, s: &str) -> Option<String> {
        b.package_root(Path::new(s))
            .map(|p| p.display().to_string())
    }

    #[test]
    fn npm_package_roots() {
        let nm = PackageBoundary::NODE_MODULES;
        assert_eq!(root(&nm, "/a/src/x.js"), None);
        assert_eq!(
            root(&nm, "/a/node_modules/foo/lib/x.js").as_deref(),
            Some("/a/node_modules/foo")
        );
        assert_eq!(
            root(&nm, "/a/node_modules/@s/foo/lib/x.js").as_deref(),
            Some("/a/node_modules/@s/foo")
        );
        assert_eq!(
            root(&nm, "/a/node_modules/.pnpm/x@1/node_modules/foo/x.js").as_deref(),
            Some("/a/node_modules/.pnpm/x@1/node_modules/foo")
        );
        assert_eq!(root(&nm, "/a/node_modules/foo"), None);
        // nft quirk kept: a scoped path without a file below still yields a root.
        assert_eq!(
            root(&nm, "/a/node_modules/@s/foo").as_deref(),
            Some("/a/node_modules/@s/foo")
        );
        let (_, name) = nm
            .package_root_and_name(Path::new("/a/node_modules/@s/foo/x.node"))
            .unwrap();
        assert_eq!(name, "@s/foo");
    }

    #[test]
    fn containment() {
        let nm = PackageBoundary::NODE_MODULES;
        let from = Path::new("/a/node_modules/foo/lib/x.js");
        assert!(!nm.is_outside(from, Path::new("/a/node_modules/bar/x")));
        assert!(nm.is_outside(from, Path::new("/a/data/x")));
        assert!(!nm.is_outside(Path::new("/a/src/x.js"), Path::new("/etc/x")));
        assert_eq!(
            nm.containment_prefix(Path::new("/a/node_modules/x/node_modules/y/z.js")),
            Some(&b"/a/node_modules/"[..])
        );
        assert_eq!(
            nm.marker_of_dir(Path::new("/a/node_modules")),
            Some("node_modules")
        );
        assert_eq!(nm.marker_of_dir(Path::new("/a/node_modules/x")), None);
    }

    #[test]
    fn other_layouts() {
        let py = PackageBoundary::PYTHON;
        let f = "/v/lib/python3.12/site-packages/numpy/core/_m.cpython-312-x86_64-linux-gnu.so";
        assert_eq!(
            root(&py, f).as_deref(),
            Some("/v/lib/python3.12/site-packages/numpy")
        );
        assert_eq!(root(&py, "/v/lib/python3.12/site-packages/six.py"), None);
        assert_eq!(
            root(&py, "/usr/lib/python3/dist-packages/yaml/x.py").as_deref(),
            Some("/usr/lib/python3/dist-packages/yaml")
        );
        assert!(py.is_outside(
            Path::new("/v/lib/python3.12/site-packages/a/x.py"),
            Path::new("/srv/app/data")
        ));
        assert_eq!(
            py.marker_of_dir(Path::new("/v/site-packages")),
            Some("site-packages")
        );

        let go = PackageBoundary::GO;
        let (r, name) = go
            .package_root_and_name(Path::new(
                "/home/u/go/pkg/mod/github.com/!azure/sdk@v1.2.0/x/y.go",
            ))
            .unwrap();
        assert_eq!(
            r,
            Path::new("/home/u/go/pkg/mod/github.com/!azure/sdk@v1.2.0")
        );
        assert_eq!(name, "github.com/!azure/sdk@v1.2.0");
        assert_eq!(root(&go, "/app/vendor/github.com/a/b/c.go"), None);
        assert!(go.contains(Path::new("/app/vendor/github.com/a/b/c.go")));
        assert_eq!(
            go.marker_of_dir(Path::new("/home/u/go/pkg/mod")),
            Some("pkg/mod")
        );

        let cargo = PackageBoundary::CARGO;
        assert_eq!(
            root(
                &cargo,
                "/h/.cargo/registry/src/index.crates.io-6f17d22bba15001f/serde-1.0.0/src/lib.rs"
            )
            .as_deref(),
            Some("/h/.cargo/registry/src/index.crates.io-6f17d22bba15001f/serde-1.0.0")
        );
        assert_eq!(
            root(&cargo, "/w/vendor/serde/src/lib.rs").as_deref(),
            Some("/w/vendor/serde")
        );
    }
}
