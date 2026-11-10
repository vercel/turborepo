//! [`oxc_resolver::FileSystem`] on top of the core [`DirCache`], so JS
//! resolution shares the tracer's single fs cache, so `oxc_resolver` does
//! not keep a second one.
//!
//! - existence and kind queries (`metadata`, `symlink_metadata`) are answered
//!   from cached directory listings (`readdir` + `d_type`), not `stat`;
//! - `package.json` and `tsconfig.json` reads go through
//!   [`DirCache::read_cached`], so a missing manifest in an ancestor directory
//!   costs no syscall and every file is read once;
//! - `read_link` and `canonicalize` use the cache's memoized `readlink` walk.
//!
//! Lookups match listing names exactly, like Linux, even on case-insensitive
//! macOS volumes.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use oxc_resolver::{FileMetadata, FileSystem, ResolveError};
use turborepo_trace_core::{
    DirCache,
    fs::{FileKind, Stat},
};

use super::overlay::{Entry, Overlay};

/// The tracer's [`DirCache`] seen through oxc's filesystem abstraction.
#[derive(Clone, Debug)]
pub struct CachedFs {
    fs: Arc<DirCache>,
    /// Another revision's files where they differ from the disk ([`Overlay`]).
    overlay: Option<Arc<Overlay>>,
    /// The file system of a config loader ([`CachedFs::for_loader`]): a
    /// manifest is read through the loader's view.
    loader: bool,
}

impl CachedFs {
    /// Wraps a shared cache.
    pub fn new_shared(fs: Arc<DirCache>) -> Self {
        CachedFs {
            fs,
            overlay: None,
            loader: false,
        }
    }

    /// Wraps a shared cache, seen at the revision of `overlay`.
    pub fn with_overlay(fs: Arc<DirCache>, overlay: Arc<Overlay>) -> Self {
        CachedFs {
            fs,
            overlay: Some(overlay),
            loader: false,
        }
    }

    /// What the overlay has at `path`, if it answers for it.
    fn overlaid(&self, path: &Path) -> Option<Entry> {
        self.overlay.as_ref()?.entry(&self.fs, path)
    }

    /// The path to ask the working tree for `path` when the overlay has no
    /// entry for it: `path` itself, or with an overlay the place the
    /// revision's links lead to ([`Overlay::on_disk`]).
    fn on_disk<'a>(&self, path: &'a Path) -> std::borrow::Cow<'a, Path> {
        match &self.overlay {
            Some(o) => o.on_disk(&self.fs, path),
            None => std::borrow::Cow::Borrowed(path),
        }
    }

    /// The bytes of `path`: the overlay's, else the cached read (a manifest's
    /// resolution view when `path` is a `package.json`; the loader's view
    /// for a config loader).
    fn bytes(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        if let Some(o) = &self.overlay
            && let Some(r) = o.read(&self.fs, path)
        {
            // A file of the revision is read like one of the working tree: an observation
            // of whoever asks (recorded where its links lead, as a diff names it).
            if r.is_ok() && self.fs.records() {
                turborepo_trace_core::record::read(&o.on_disk(&self.fs, path));
            }
            return r;
        }
        let path = self.on_disk(path);
        if !is_manifest(&path) {
            self.fs.read_cached(&path)
        } else if self.loader {
            self.fs.read_manifest_loader(&path)
        } else {
            self.fs.read_manifest(&path)
        }
    }

    /// This file system as the loader of a `tsconfig.json` gets it: a
    /// `package.json` is
    /// its [`turborepo_trace_core::manifest::loader_view`], the fields the
    /// resolution of an `extends` entry can read, so that no load can
    /// depend on anything else of a manifest (and its record says as much:
    /// [`turborepo_trace_core::record::loader_read`]).
    pub(super) fn for_loader(&self) -> Self {
        CachedFs {
            loader: true,
            ..self.clone()
        }
    }

    /// The underlying cache.
    pub fn cache(&self) -> &Arc<DirCache> {
        &self.fs
    }

    /// The revision this file system shows, if not the working tree.
    pub(super) fn overlay(&self) -> Option<&Arc<Overlay>> {
        self.overlay.as_ref()
    }

    /// Records the answer "`path` is a directory".
    ///
    /// An answer is an observation, whichever answer it is. A miss is recorded
    /// by the cache and a file by its read; a yes to "is it a directory?"
    /// had no record, and the loader of a tsconfig asks that first (the
    /// target of an `extends`, a project reference: `<path>/tsconfig.json`
    /// when it is one, `<path>.json` when it is not). The record is the
    /// directory's membership of its parent, under its own name: its
    /// state is what is there (nothing, a file, a directory), and a path that
    /// comes to be one or ceases to be one matches it and nothing else
    /// does. The loader drops the ones that something it read lies in
    /// (`Tsconfigs::load`): the file says as much.
    ///
    /// Only for whoever records what the file system answers (a config load): a
    /// module resolution reports what it looked for itself, with recording
    /// paused. Not for a `node_modules` directory, a scope directory or a
    /// package directory in one: whether each level of a walk up has one,
    /// and the package in it, is the walk's record.
    ///
    /// A directory **inside** a package directory is recorded like any other.
    /// The loader names a config by the path it was asked for, so a config
    /// of a workspace package that was reached through the package's
    /// `node_modules` link asks about its own relative `extends` target by
    /// the `node_modules` path (`app/node_modules/@acme/tsconfig/base`),
    /// and the answer is one about a directory of the repository
    /// (`tooling/tsconfig/base`). The record is made at the path as
    /// asked, like a read or a miss there, and the store puts it where the
    /// package directory leads (`turborepo_trace_store`'s
    /// `normalize::locate`): at the real directory for a link to a
    /// workspace package, nowhere for a path inside a package that is
    /// installed (the package's identity covers what is in it).
    fn is_a_directory(&self, path: &Path) {
        if !self.fs.records() || !turborepo_trace_core::record::is_recording() {
            return;
        }
        if super::in_node_modules(&self.fs, path) && !inside_installed_package(path) {
            return;
        }
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            turborepo_trace_core::record::listing(
                parent,
                &super::literal_glob(&name.to_string_lossy()),
            );
        }
    }

    /// The kind of `path` in the working tree (at the place the overlay's
    /// revision has it).
    fn stat(&self, path: &Path) -> io::Result<Stat> {
        self.fs
            .stat(&self.on_disk(path))
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

/// The metadata of a path the overlay answers for: a plain file or directory,
/// or nothing.
fn overlaid_metadata(e: Entry) -> io::Result<FileMetadata> {
    match e {
        Entry::File => Ok(FileMetadata::new(true, false, false)),
        Entry::Dir => Ok(FileMetadata::new(false, true, false)),
        Entry::Absent => Err(io::Error::from(io::ErrorKind::NotFound)),
    }
}

/// Whether `path` lies below the package directory of its innermost
/// `node_modules` directory (`node_modules/react/lib`,
/// `node_modules/@scope/pkg/x`): not the `node_modules` directory itself, a
/// scope directory or the package directory.
fn inside_installed_package(path: &Path) -> bool {
    let mut after: Option<Vec<&std::ffi::OsStr>> = None;
    for c in path.components() {
        let c = c.as_os_str();
        if c == "node_modules" {
            after = Some(Vec::new());
        } else if let Some(a) = &mut after {
            a.push(c);
        }
    }
    let Some(after) = after else {
        return false;
    };
    let name_len = match after.first() {
        Some(first) if first.as_encoded_bytes().starts_with(b"@") => 2,
        Some(_) => 1,
        None => return false,
    };
    after.len() > name_len
}

/// Whether `path` names a `package.json`.
fn is_manifest(path: &Path) -> bool {
    path.file_name() == Some(std::ffi::OsStr::new("package.json"))
}

impl FileSystem for CachedFs {
    /// Only used by `ResolverGeneric::new`; the tracer always passes its shared
    /// cache through `new_with_file_system`.
    fn new() -> Self {
        CachedFs {
            fs: Arc::new(DirCache::new()),
            overlay: None,
            loader: false,
        }
    }

    /// oxc reads `package.json` files through this: it gets their resolution
    /// view ([`turborepo_trace_core::manifest`]), so nothing it resolves
    /// can depend on a field outside the view.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.bytes(path).map(|b| b.to_vec())
    }

    /// oxc reads only tsconfig files through this (`package.json` goes through
    /// `read`), so this is where a tsconfig's `extends` entries that name a
    /// package nobody installed are redirected to an empty config
    /// ([`MISSING_BASE`], see [`redirect_missing_bases`]).
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        if path == Path::new(MISSING_BASE) {
            return Ok("{}".to_owned());
        }
        let bytes = self.bytes(path)?;
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            )
        })?;
        Ok(match redirect_missing_bases(&self.fs, path, &text) {
            Some(t) => t,
            None => text,
        })
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if path == Path::new(MISSING_BASE) {
            return Ok(FileMetadata::new(true, false, false));
        }
        if let Some(e) = self.overlaid(path) {
            return overlaid_metadata(e);
        }
        let s = self.stat(path)?;
        if s.kind == FileKind::Dir {
            self.is_a_directory(path);
        }
        Ok(FileMetadata::new(
            s.kind == FileKind::File,
            s.kind == FileKind::Dir,
            false,
        ))
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if path == Path::new(MISSING_BASE) {
            return Ok(FileMetadata::new(true, false, false));
        }
        if let Some(e) = self.overlaid(path) {
            return overlaid_metadata(e);
        }
        let s = self.stat(path)?;
        Ok(if s.is_symlink {
            // (Where it leads is asked next, with `metadata`.)
            FileMetadata::new(false, false, true)
        } else {
            if s.kind == FileKind::Dir {
                self.is_a_directory(path);
            }
            FileMetadata::new(s.kind == FileKind::File, s.kind == FileKind::Dir, false)
        })
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        match self.fs.read_link(path) {
            Some(t) => Ok(t.to_path_buf()),
            None => Err(io::Error::new(io::ErrorKind::InvalidInput, "not a symlink").into()),
        }
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        if path == Path::new(MISSING_BASE) {
            return Ok(path.to_path_buf());
        }
        let real = self.fs.canonicalize(&self.on_disk(path)).path;
        match self.overlaid(path) {
            Some(Entry::Absent) => return Err(io::Error::from(io::ErrorKind::NotFound)),
            Some(_) => return Ok(real),
            None => {}
        }
        if self.fs.exists(&real) {
            Ok(real)
        } else {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }
    }
}

/// The virtual, empty tsconfig that stands in for an `extends` base from a
/// package that is not installed.
pub const MISSING_BASE: &str = "/__turbotrace__/missing-tsconfig-base.json";

/// Rewrites the `extends` entries of the tsconfig text `text` (at `path`) that
/// name a package which is installed nowhere above the config: each becomes
/// [`MISSING_BASE`]. `None` when nothing changes.
///
/// Without this, one missing base (`"extends":
/// "@tsconfig/node24/tsconfig.json"` in a checkout without `node_modules`)
/// makes oxc reject the whole config, so its own `paths` and `baseUrl` (and
/// those of the bases that do exist) would be lost for every file below
/// it. A base that lives in a third-party package can't define first-party path
/// mappings: its `paths` and `baseUrl` would be relative to the package's own
/// directory. So the config without that base resolves first-party imports
/// exactly as the full one. The lookups for the package directory go through
/// the shared cache, so an install that adds it is seen like any other probed
/// path.
///
/// Relative, absolute and `#import` bases are left alone (a missing one is a
/// real error), and so is a bare specifier that names the package the config
/// lies in (a self-reference: the nearest `package.json` has it as its name).
pub fn redirect_missing_bases(fs: &DirCache, path: &Path, text: &str) -> Option<String> {
    let dir = path.parent()?;
    let spans = extends_strings(text)?;
    let mut out: Option<String> = None;
    // Back to front, so earlier spans stay valid.
    for (start, end) in spans.into_iter().rev() {
        let spec = &text[start + 1..end - 1];
        if spec.is_empty() || spec.starts_with(['.', '/', '#']) || spec.contains('\\') {
            continue;
        }
        let mut parts = spec.split('/');
        let name = match (parts.next(), spec.starts_with('@')) {
            (Some(scope), true) => match parts.next() {
                Some(n) => format!("{scope}/{n}"),
                None => continue,
            },
            (Some(n), false) => n.to_owned(),
            (None, _) => continue,
        };
        if package_reachable(fs, dir, &name) {
            continue;
        }
        let t = out.get_or_insert_with(|| text.to_owned());
        t.replace_range(start + 1..end - 1, MISSING_BASE);
    }
    out
}

/// Whether package `name` is installed in a `node_modules` at or above `dir`,
/// or is the name of the package `dir` lies in (a self-reference: the nearest
/// `package.json` at or above `dir`, which is the one the loader asks). A
/// manifest that merely lists `name` among its dependencies doesn't make it
/// reachable: that is the usual shape of a checkout without `node_modules`.
///
/// Every lookup here is one of the config's load ([`super::tsconfig`]: what a
/// load is computed from), so it reads no more than the answer needs: of the
/// manifests above `dir`, the nearest. With every ancestor's manifest read, the
/// repository's root manifest would be an input of every config that extends a
/// package nobody installed.
fn package_reachable(fs: &DirCache, dir: &Path, name: &str) -> bool {
    let mut cur = Some(dir);
    let mut scope_found = false;
    while let Some(d) = cur {
        if fs.exists(&d.join("node_modules").join(name)) {
            return true;
        }
        let manifest = d.join("package.json");
        if !scope_found && fs.is_file(&manifest) {
            scope_found = true;
            // (Through the loader's view of a manifest: the name is in it.)
            if fs.read_manifest_loader(&manifest).is_ok_and(|b| {
                serde_json::from_slice::<serde_json::Value>(&b)
                    // An unreadable manifest: leave the base alone.
                    .map_or(true, |v| {
                        v.get("name").and_then(|n| n.as_str()) == Some(name)
                    })
            }) {
                return true;
            }
        }
        cur = d.parent();
    }
    false
}

/// The byte spans (quotes included) of the string values of the top-level
/// `extends` key of a JSON-with-comments document: one string or an array of
/// strings. `None` if there is no such key or the document can't be scanned.
fn extends_strings(text: &str) -> Option<Vec<(usize, usize)>> {
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut depth = 0i32;
    // Skips whitespace and comments from `i`.
    let skip = |mut i: usize| -> usize {
        loop {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if b[i..].starts_with(b"//") {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            } else if b[i..].starts_with(b"/*") {
                i = memchr::memmem::find(&b[i + 2..], b"*/").map_or(b.len(), |e| i + 2 + e + 2);
            } else {
                return i;
            }
        }
    };
    // The end (exclusive) of the string starting at the quote at `i`.
    let string_end = |i: usize| -> Option<usize> {
        let mut j = i + 1;
        while j < b.len() {
            match b[j] {
                b'\\' => j += 2,
                b'"' => return Some(j + 1),
                _ => j += 1,
            }
        }
        None
    };
    while i < b.len() {
        i = skip(i);
        if i >= b.len() {
            break;
        }
        match b[i] {
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                i += 1;
            }
            b'"' => {
                let end = string_end(i)?;
                let is_key = depth == 1 && &b[i..end] == b"\"extends\"";
                i = end;
                if !is_key {
                    continue;
                }
                let mut j = skip(i);
                if b.get(j) != Some(&b':') {
                    continue;
                }
                j = skip(j + 1);
                let mut spans = Vec::new();
                match b.get(j) {
                    Some(b'"') => spans.push((j, string_end(j)?)),
                    Some(b'[') => {
                        j = skip(j + 1);
                        while b.get(j) == Some(&b'"') {
                            let e = string_end(j)?;
                            spans.push((j, e));
                            j = skip(e);
                            if b.get(j) == Some(&b',') {
                                j = skip(j + 1);
                            }
                        }
                    }
                    _ => {}
                }
                return Some(spans);
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn missing_package_bases_become_the_empty_config() {
        let t = tempfile::tempdir().unwrap();
        let r = turborepo_trace_core::fs::normalize(t.path());
        fs::create_dir_all(r.join("app/node_modules/@installed/base")).unwrap();
        // The root package is `@me/root` and lists the missing bases as dependencies,
        // as a checkout without `node_modules` does.
        fs::write(
            r.join("package.json"),
            "{\"name\":\"@me/root\",\"devDependencies\":{\"@tsconfig/node24\":\"24.0.3\",\"\
             missing-pkg\":\"1.0.0\"}}",
        )
        .unwrap();
        let cache = DirCache::new();
        let at = r.join("app/tsconfig.json");
        let redirect = |text: &str| redirect_missing_bases(&cache, &at, text);
        // A package that is installed nowhere: redirected, the rest of the text
        // untouched.
        assert_eq!(
            redirect("{ // c\n \"extends\": \"@tsconfig/node24/tsconfig.json\", \"x\": 1 }")
                .unwrap(),
            format!("{{ // c\n \"extends\": \"{MISSING_BASE}\", \"x\": 1 }}")
        );
        // Arrays: only the missing entries.
        assert_eq!(
            redirect(
                "{\"extends\": [\"./base.json\", \"missing-pkg\", \"@installed/base/x.json\"]}"
            )
            .unwrap(),
            format!(
                "{{\"extends\": [\"./base.json\", \"{MISSING_BASE}\", \
                 \"@installed/base/x.json\"]}}"
            )
        );
        // Installed, relative, absolute, subpath imports and self references stay.
        for keep in [
            "@installed/base/tsconfig.json",
            "./base.json",
            "../base.json",
            "/abs/base.json",
            "#config",
            "@me/root/tsconfig.base.json",
        ] {
            assert_eq!(
                redirect(&format!("{{\"extends\": \"{keep}\"}}")),
                None,
                "{keep}"
            );
        }
        // Only the top-level key counts, and strings that merely contain it don't.
        assert_eq!(
            redirect("{\"compilerOptions\": {\"extends\": \"missing-pkg\"}, \"a\": \"extends\"}"),
            None
        );
        assert_eq!(redirect("{}"), None);
        assert_eq!(redirect("not json \"extends"), None);

        // Through the filesystem adapter, oxc sees the redirected text and the empty
        // base.
        fs::write(&at, "{\"extends\": \"missing-pkg/tsconfig.json\"}").unwrap();
        let cfs = CachedFs::new_shared(Arc::new(DirCache::new()));
        assert!(cfs.read_to_string(&at).unwrap().contains(MISSING_BASE));
        assert_eq!(cfs.read_to_string(Path::new(MISSING_BASE)).unwrap(), "{}");
        assert!(cfs.metadata(Path::new(MISSING_BASE)).unwrap().is_file());
    }

    #[test]
    fn answers_from_the_shared_cache() {
        let t = tempfile::tempdir().unwrap();
        let r = turborepo_trace_core::fs::normalize(t.path());
        fs::create_dir(r.join("d")).unwrap();
        fs::write(r.join("d/package.json"), "{\"name\":\"x\"}").unwrap();
        turborepo_trace_core::os::symlink("d", r.join("link")).unwrap();
        let cache = Arc::new(DirCache::new());
        let cfs = CachedFs::new_shared(Arc::clone(&cache));

        assert!(cfs.metadata(&r.join("d")).unwrap().is_dir());
        assert!(cfs.metadata(&r.join("d/package.json")).unwrap().is_file());
        assert!(cfs.metadata(&r.join("link")).unwrap().is_dir());
        assert!(cfs.symlink_metadata(&r.join("link")).unwrap().is_symlink());
        assert!(!cfs.symlink_metadata(&r.join("d")).unwrap().is_symlink());
        assert_eq!(
            cfs.metadata(&r.join("missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            cfs.read_to_string(&r.join("link/package.json")).unwrap(),
            "{\"name\":\"x\"}"
        );
        assert_eq!(cfs.read_link(&r.join("link")).unwrap(), PathBuf::from("d"));
        assert!(cfs.read_link(&r.join("d")).is_err());
        assert_eq!(
            cfs.canonicalize(&r.join("link/package.json")).unwrap(),
            turborepo_trace_core::fs::normalize(
                &fs::canonicalize(r.join("d/package.json")).unwrap()
            )
        );
        // Everything went through the shared cache.
        assert!(cache.stats().dirs_read > 0);
    }
}
