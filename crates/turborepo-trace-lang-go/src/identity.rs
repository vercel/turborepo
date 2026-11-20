//! Opaque third-party module identity: a file below the module
//! cache or a `vendor/` directory belongs to a module version, identified by
//! module path, version and the `go.sum` hash of the module zip.

use std::path::{Path, PathBuf};

use turborepo_trace_core::{DirCache, os::OsStrExt};
use turborepo_trace_lockfile::gosum::{self, GoSum};

use crate::{
    BOUNDARY,
    modules::{is_prefix, parse_modules_txt, unescape},
};

/// The identity of a third-party module version.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleIdentity {
    /// Module path (unescaped, `github.com/Azure/x`).
    pub module: String,
    /// Version.
    pub version: String,
    /// The module's root directory (`…/pkg/mod/github.com/!azure/x@v1.2.3`, or
    /// `…/vendor/github.com/Azure/x`).
    pub root: PathBuf,
    /// The `h1:` hash of the module zip from `go.sum`, if known.
    pub sum: Option<String>,
}

impl ModuleIdentity {
    /// `module@version`.
    pub fn key(&self) -> String {
        format!("{}@{}", self.module, self.version)
    }
}

/// The `go.sum` hashes of the module or workspace at `root` (`go.sum` of every
/// `go.work` module plus `go.work.sum`, or the module's `go.sum`).
pub fn load_sums(fs: &DirCache, root: &Path) -> GoSum {
    let mut files = vec![root.join("go.sum"), root.join("go.work.sum")];
    if let Ok(b) = fs.read_cached(&root.join("go.work"))
        && let Ok(w) = gosum::parse_go_work(&String::from_utf8_lossy(&b))
    {
        files.extend(w.uses.iter().map(|u| root.join(u).join("go.sum")));
    }
    let mut out = GoSum::default();
    for f in files {
        if let Ok(b) = fs.read_cached(&f)
            && let Ok(s) = gosum::parse_go_sum(&String::from_utf8_lossy(&b))
        {
            out.entries.extend(s.entries);
        }
    }
    out
}

/// The module version a file belongs to, if it is third-party: below the module
/// cache (`pkg/mod/<escaped path>@<escaped version>/…`) or a `vendor/`
/// directory with a `modules.txt`. `sums` supplies the `go.sum` hash. See
/// [`module_identity_in`] for a module cache that is not below a `pkg/mod`
/// directory.
pub fn module_identity(fs: &DirCache, path: &Path, sums: Option<&GoSum>) -> Option<ModuleIdentity> {
    module_identity_in(fs, path, None, sums)
}

/// [`module_identity`] that also recognizes files below the module cache
/// `modcache` (a `GOMODCACHE` with any name).
pub fn module_identity_in(
    fs: &DirCache,
    path: &Path,
    modcache: Option<&Path>,
    sums: Option<&GoSum>,
) -> Option<ModuleIdentity> {
    let sum = |m: &str, v: &str| sums.and_then(|s| s.get(m, v)).and_then(|e| e.h1.clone());
    if let Some((root, module, version)) =
        modcache.and_then(|mc| crate::modules::modcache_module(mc, path))
    {
        return Some(ModuleIdentity {
            sum: sum(&module, &version),
            module,
            version,
            root,
        });
    }
    let bytes = path.as_os_str().as_bytes();
    if let Some((root, name)) = BOUNDARY.package_root_and_name_from(path, fs.local_offset(path))
        && let Some((m, v)) = name.rsplit_once('@')
        && memchr::memmem::find(bytes, b"/pkg/mod/").is_some()
    {
        let module = unescape(m)?;
        let version = unescape(v)?;
        return Some(ModuleIdentity {
            sum: sum(&module, &version),
            module,
            version,
            root,
        });
    }
    // vendor/: the last `/vendor/` segment whose parent has vendor/modules.txt.
    let finder = memchr::memmem::rfind_iter(bytes, b"/vendor/");
    for at in finder {
        let base = Path::new(std::ffi::OsStr::from_bytes(&bytes[..at]));
        let Ok(txt) = fs.read_cached(&base.join("vendor/modules.txt")) else {
            continue;
        };
        let rel = std::str::from_utf8(&bytes[at + b"/vendor/".len()..]).ok()?;
        let pkg = rel.rsplit_once('/').map_or("", |(d, _)| d);
        let modules = parse_modules_txt(&String::from_utf8_lossy(&txt));
        let m = modules
            .iter()
            .filter(|m| is_prefix(&m.path, pkg))
            .max_by_key(|m| m.path.len())?;
        return Some(ModuleIdentity {
            sum: sum(&m.path, &m.version),
            module: m.path.clone(),
            version: m.version.clone(),
            root: base.join("vendor").join(&m.path),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modcache_identity() {
        let fs = DirCache::new();
        let mut sums = GoSum::default();
        sums.entries.insert(
            ("github.com/Azure/sdk".into(), "v1.2.0".into()),
            gosum::GoSumEntry {
                h1: Some("h1:abc=".into()),
                go_mod_h1: None,
            },
        );
        let id = module_identity(
            &fs,
            Path::new("/home/u/go/pkg/mod/github.com/!azure/sdk@v1.2.0/x/y.go"),
            Some(&sums),
        )
        .unwrap();
        assert_eq!(id.key(), "github.com/Azure/sdk@v1.2.0");
        assert_eq!(id.sum.as_deref(), Some("h1:abc="));
        assert_eq!(
            id.root,
            Path::new("/home/u/go/pkg/mod/github.com/!azure/sdk@v1.2.0")
        );
        assert!(module_identity(&fs, Path::new("/src/app/main.go"), None).is_none());
    }

    #[test]
    fn vendor_identity() {
        let t = tempfile::tempdir().unwrap();
        let v = t.path().join("vendor");
        std::fs::create_dir_all(v.join("github.com/a/b/c")).unwrap();
        std::fs::write(
            v.join("modules.txt"),
            "# github.com/a/b v1.0.0\n## explicit\ngithub.com/a/b/c\n",
        )
        .unwrap();
        let fs = DirCache::new();
        let id = module_identity(&fs, &v.join("github.com/a/b/c/x.go"), None).unwrap();
        assert_eq!(id.key(), "github.com/a/b@v1.0.0");
        assert_eq!(id.root, v.join("github.com/a/b"));
    }
}
