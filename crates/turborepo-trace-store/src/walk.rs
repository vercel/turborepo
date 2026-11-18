//! The repository walk: every regular file below the root, with `.gitignore`,
//! `.ignore` and `.git/info/exclude` respected (also without a `.git`
//! directory), in parallel.
//!
//! Always skipped: `.git`, `node_modules` (third-party code is reached through
//! resolution), the store directory itself and every directory with the default
//! name of one (`.turbotrace`: a graph another build keeps in the tree). Hidden
//! files are included (`.github/actions` can be a workspace package).
//!
//! [`walk_cached`] keeps the result in `<cache dir>/walk` with the mtime of
//! every directory the walk entered and the stat of every ignore file it
//! obeyed. When none of them changed, the file list cannot have changed
//! (adding, removing or renaming an entry updates its directory's mtime), so
//! the cached list is returned after one parallel `stat` per directory.
//! Directories modified within two seconds of the walk are not trusted (coarse
//! timestamps), so such a walk is not cached.
//!
//! When the cached list is stale, the fresh walk is compared with it:
//! [`Walk::changed`] lists the walked directories whose mtime differs (or that
//! appeared or disappeared), so callers can keep what only depends on the other
//! directories ([`Walk::prev`] names the cache the comparison was made
//! against). An ignore file that changed makes the comparison unusable.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

use ignore::{WalkBuilder, WalkState};
use rayon::prelude::*;
use turborepo_trace_core::os::MetadataExt;

/// Repo-relative, `/`-separated paths of every file, sorted.
pub fn walk(root: &Path, out_dir: &Path, threads: usize) -> Vec<String> {
    split(walk_stamped(root, out_dir, threads).0).0
}

/// (2: the list holds the symbolic links too.)
const MAGIC: &[u8; 8] = b"DTWALK2\n";

/// What a symbolic link's path starts with in the list a walk makes and its
/// cache holds: a link is no file of the walk (it is not followed, and what it
/// leads to is reached through resolution), and it is an entry of the tree
/// ([`Walk::links`]). No path has a NUL.
const LINK: char = '\0';

/// The files and the symbolic links of a walk's list (both sorted).
fn split(entries: Vec<String>) -> (Vec<String>, Vec<String>) {
    let (links, files): (Vec<String>, Vec<String>) =
        entries.into_iter().partition(|e| e.starts_with(LINK));
    let links = links.into_iter().map(|l| l[1..].to_owned()).collect();
    (files, links)
}
/// Ignore files whose stat is part of the walk stamp.
const IGNORE_FILES: &[&str] = &[".gitignore", ".ignore"];

fn mtime_ns(m: &std::fs::Metadata) -> i128 {
    m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128
}

/// Directory mtimes and ignore-file stamps of one walk.
#[derive(Default)]
struct Stamp {
    dirs: Vec<(String, i128)>,
    /// `(path, mtime_ns, size)`; `size` -1 for a missing file.
    files: Vec<(String, i128, i64)>,
}

/// The result of [`walk_cached`].
pub struct Walk {
    /// Repo-relative files, sorted.
    pub files: Vec<String>,
    /// Repo-relative symbolic links the walk passed (to a file, to a directory,
    /// to nothing; ignore rules respected like for files), sorted. A link
    /// is not followed.
    pub links: Vec<String>,
    /// Repo-relative directories the walk entered (`""` is the root), sorted.
    pub dirs: Vec<String>,
    /// Some(digest of the walk cache) when the cached walk was valid: the
    /// walked tree is unchanged since that cache was written.
    pub unchanged: Option<u128>,
    /// When the cached walk was stale but comparable: the digest of that cache.
    pub prev: Option<u128>,
    /// With [`Walk::prev`]: the repo-relative directories whose listing may
    /// differ from that walk (mtime changed, added or removed), sorted.
    /// Adding, removing or renaming an entry changes its directory's mtime,
    /// so every other walked directory lists the same names.
    pub changed: Vec<String>,
}

/// A digest identifying the current walk cache file, if any.
pub fn cache_token(out_dir: &Path) -> Option<u128> {
    std::fs::read(crate::cache_dir(out_dir).join("walk"))
        .ok()
        .map(|b| xxhash_rust::xxh3::xxh3_128(&b))
}

/// [`walk`], reusing the previous result when no directory changed (see the
/// module docs). When some did (and no ignore file changed), only the changed
/// directories and their ancestors are listed again; every other directory's
/// subtree comes from the cache.
pub fn walk_cached(root: &Path, out_dir: &Path, threads: usize) -> Walk {
    let cache = crate::cache_dir(out_dir).join("walk");
    let old = std::fs::read(&cache).ok();
    let mut incremental = None;
    if let Some(bytes) = &old
        && let Some((st, files)) = load_cache(bytes, root)
    {
        match check(&st, root) {
            Some(changed) if changed.is_empty() => {
                let (files, links) = split(files);
                return Walk {
                    files,
                    links,
                    dirs: st.dirs.into_iter().map(|(d, _)| d).collect(),
                    unchanged: Some(xxhash_rust::xxh3::xxh3_128(bytes)),
                    prev: None,
                    changed: Vec::new(),
                };
            }
            Some(changed) if turborepo_trace_core::debug::var_os("FULL_WALK").is_none() => {
                incremental = Some((st, files, changed));
            }
            _ => {}
        }
    }
    let start = now_ns();
    let (files, stamp) = match &incremental {
        Some((st, files, changed)) => {
            let w = walk_incremental(root, out_dir, threads, st, files, changed);
            if turborepo_trace_core::debug::var_os("CHECK_WALK").is_some() {
                let full = walk_stamped(root, out_dir, threads);
                let dirs = |s: &Stamp| s.dirs.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>();
                eprintln!(
                    "walk check: incremental {} files, full {} files: {}",
                    w.0.len(),
                    full.0.len(),
                    if w.0 == full.0 && dirs(&w.1) == dirs(&full.1) {
                        "same"
                    } else {
                        "DIFFERENT"
                    }
                );
            }
            w
        }
        None => walk_stamped(root, out_dir, threads),
    };
    let racy = start - 2_000_000_000;
    let trusted =
        stamp.dirs.iter().all(|(_, m)| *m < racy) && stamp.files.iter().all(|(_, m, _)| *m < racy);
    if turborepo_trace_core::debug::var_os("DEBUG_WALK").is_some() && !trusted {
        let recent: Vec<&String> = stamp
            .dirs
            .iter()
            .filter(|(_, m)| *m >= racy)
            .map(|(d, _)| d)
            .take(5)
            .collect();
        eprintln!("walk cache: not trusted (recent directories {recent:?})");
    }
    if trusted {
        crate::create_cache_dir(out_dir);
        let _ = crate::write_atomic(&cache, &encode(root, &stamp, &files));
    } else {
        let _ = std::fs::remove_file(&cache);
    }
    let (prev, changed) = match old.as_deref().and_then(|b| Some((b, load_stamp(b, root)?))) {
        Some((b, old)) => match changed_dirs(&old, &stamp) {
            Some(c) => (Some(xxhash_rust::xxh3::xxh3_128(b)), c),
            None => (None, Vec::new()),
        },
        None => (None, Vec::new()),
    };
    let (files, links) = split(files);
    Walk {
        files,
        links,
        dirs: stamp.dirs.into_iter().map(|(d, _)| d).collect(),
        unchanged: None,
        prev,
        changed,
    }
}

/// Directories whose listing may differ between two walks (sorted), or `None`
/// if an ignore file both walks obeyed changed (the walked set itself may
/// differ in unstamped ways).
fn changed_dirs(old: &Stamp, new: &Stamp) -> Option<Vec<String>> {
    let old_files: std::collections::HashMap<&str, (i128, i64)> = old
        .files
        .iter()
        .map(|(f, m, z)| (f.as_str(), (*m, *z)))
        .collect();
    for (f, m, z) in &new.files {
        if let Some(&o) = old_files.get(f.as_str())
            && o != (*m, *z)
        {
            return None;
        }
    }
    // Both lists are sorted by path.
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.dirs.len() || j < new.dirs.len() {
        match (old.dirs.get(i), new.dirs.get(j)) {
            (Some(a), Some(b)) if a.0 == b.0 => {
                if a.1 != b.1 {
                    out.push(b.0.clone());
                }
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a.0 < b.0 => {
                out.push(a.0.clone());
                i += 1;
            }
            (Some(_), Some(b)) => {
                out.push(b.0.clone());
                j += 1;
            }
            (Some(a), None) => {
                out.push(a.0.clone());
                i += 1;
            }
            (None, Some(b)) => {
                out.push(b.0.clone());
                j += 1;
            }
            (None, None) => break,
        }
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// The stamps of a walk cache written for `root` (not checked against the
/// tree).
fn load_stamp(bytes: &[u8], root: &Path) -> Option<Stamp> {
    let (_, mut r) = open_cache(bytes, root)?;
    read_stamp(&mut r)
}

/// Checks a walk cache's magic, checksum and root; a reader after the root.
fn open_cache<'a>(bytes: &'a [u8], root: &Path) -> Option<(&'a [u8], Reader<'a>)> {
    if bytes.len() < 16 || &bytes[..8] != MAGIC {
        return None;
    }
    let (body, sum) = bytes.split_at(bytes.len() - 8);
    if xxhash_rust::xxh3::xxh3_64(body).to_le_bytes() != sum {
        return None;
    }
    let mut r = Reader(&body[8..]);
    if r.str()? != root.to_string_lossy() {
        return None;
    }
    Some((body, r))
}

fn read_stamp(r: &mut Reader<'_>) -> Option<Stamp> {
    let mut st = Stamp::default();
    for _ in 0..r.u64()? {
        st.dirs.push((r.str()?, r.i128()?));
    }
    for _ in 0..r.u64()? {
        st.files.push((r.str()?, r.i128()?, r.i64()?));
    }
    Some(st)
}

fn now_ns() -> i128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn encode(root: &Path, st: &Stamp, files: &[String]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    put_str(&mut out, &root.to_string_lossy());
    out.extend_from_slice(&(st.dirs.len() as u64).to_le_bytes());
    for (d, m) in &st.dirs {
        put_str(&mut out, d);
        out.extend_from_slice(&m.to_le_bytes());
    }
    out.extend_from_slice(&(st.files.len() as u64).to_le_bytes());
    for (f, m, z) in &st.files {
        put_str(&mut out, f);
        out.extend_from_slice(&m.to_le_bytes());
        out.extend_from_slice(&z.to_le_bytes());
    }
    out.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for f in files {
        put_str(&mut out, f);
    }
    let sum = xxhash_rust::xxh3::xxh3_64(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    out
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn i128(&mut self) -> Option<i128> {
        Some(i128::from_le_bytes(self.take(16)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

/// The stamps and files of a walk cache written for `root` (not checked against
/// the tree).
fn load_cache(bytes: &[u8], root: &Path) -> Option<(Stamp, Vec<String>)> {
    let (_, mut r) = open_cache(bytes, root)?;
    let st = read_stamp(&mut r)?;
    let n = r.u64()? as usize;
    let mut files = Vec::with_capacity(n);
    for _ in 0..n {
        files.push(r.str()?);
    }
    Some((st, files))
}

/// The cached walk's directories whose mtime changed (or that are gone),
/// sorted; `None` if an ignore file changed. A missing ignore file in a walked
/// directory needs no `stat` while that directory keeps its mtime (adding a
/// file changes it); in a changed directory it is checked.
fn check(st: &Stamp, root: &Path) -> Option<Vec<String>> {
    let abs = |p: &str| -> PathBuf {
        if p.is_empty() {
            root.to_path_buf()
        } else {
            root.join(p)
        }
    };
    let debug = turborepo_trace_core::debug::var_os("DEBUG_WALK").is_some();
    let changed: Vec<String> = st
        .dirs
        .par_iter()
        .filter(|(d, m)| {
            let ok = std::fs::symlink_metadata(abs(d))
                .is_ok_and(|md| md.is_dir() && mtime_ns(&md) == *m);
            if !ok && debug {
                eprintln!("walk cache: directory {d:?} changed");
            }
            !ok
        })
        .map(|(d, _)| d.clone())
        .collect();
    let walked: std::collections::HashSet<&str> = st.dirs.iter().map(|(d, _)| d.as_str()).collect();
    let changed_set: std::collections::HashSet<&str> = changed.iter().map(String::as_str).collect();
    let files_ok = st.files.par_iter().all(|(f, m, z)| {
        let dir = f.rsplit_once('/').map_or("", |(d, _)| d);
        if *z == -1 && walked.contains(dir) && !changed_set.contains(dir) {
            return true;
        }
        let ok = match std::fs::metadata(abs(f)) {
            Ok(md) => mtime_ns(&md) == *m && md.len() as i64 == *z,
            Err(_) => *z == -1,
        };
        if !ok && debug {
            eprintln!("walk cache: ignore file {f:?} changed");
        }
        ok
    });
    files_ok.then_some(changed)
}

/// The walk's builder, shared by full and incremental walks.
fn builder(root: &Path, threads: usize) -> WalkBuilder {
    let mut b = WalkBuilder::new(root);
    b.hidden(false)
        .parents(false)
        .git_global(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .ignore(true)
        .follow_links(false)
        .threads(threads);
    b
}

/// Stamps of the ignore files the walk could have obeyed in `dirs`: present
/// ones and the missing candidates in each directory, plus `.git/info/exclude`
/// (with `exclude`).
fn ignore_stamps<'a>(
    root: &Path,
    dirs: impl Iterator<Item = &'a str>,
    exclude: bool,
) -> Vec<(String, i128, i64)> {
    let mut ign: Vec<String> = Vec::new();
    for d in dirs {
        for n in IGNORE_FILES {
            ign.push(if d.is_empty() {
                (*n).to_owned()
            } else {
                format!("{d}/{n}")
            });
        }
    }
    if exclude {
        ign.push(".git/info/exclude".to_owned());
    }
    ign.into_par_iter()
        .map(|f| match std::fs::metadata(root.join(&f)) {
            Ok(md) => {
                let m = mtime_ns(&md);
                (f, m, md.len() as i64)
            }
            Err(_) => (f, 0, -1),
        })
        .collect()
}

/// The walk after `changed` directories of the cached walk `old` changed (no
/// ignore file did): directories that changed, their ancestors and new
/// directories are listed again; any other directory the walk reaches keeps its
/// cached subtree (it and every directory below it kept their mtimes, so they
/// list the same entries, and the ignore rules that apply to them are
/// unchanged).
fn walk_incremental(
    root: &Path,
    out_dir: &Path,
    threads: usize,
    old: &Stamp,
    old_files: &[String],
    changed: &[String],
) -> (Vec<String>, Stamp) {
    let mut descend: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in changed {
        let mut d = c.as_str();
        loop {
            if !descend.insert(d.to_owned()) {
                break;
            }
            match d.rfind('/') {
                Some(i) => d = &d[..i],
                None if !d.is_empty() => d = "",
                None => break,
            }
        }
    }
    let files = Mutex::new(Vec::new());
    let dirs = Mutex::new(Vec::new());
    let pruned_out = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    let mut b = builder(root, threads);
    {
        // `filter_entry` takes a 'static closure: it owns its copies.
        let out_dir = out_dir.to_path_buf();
        let root = root.to_path_buf();
        let old_dirs: std::collections::HashSet<String> =
            old.dirs.iter().map(|(d, _)| d.clone()).collect();
        let sink = std::sync::Arc::clone(&pruned_out);
        b.filter_entry(move |e| {
            let name = e.file_name();
            if !e.file_type().is_some_and(|t| t.is_dir()) {
                return true;
            }
            if name == ".git"
                || name == "node_modules"
                || name == crate::GRAPH_DIR
                || e.path() == out_dir
            {
                return false;
            }
            let Ok(rel) = e.path().strip_prefix(&root) else {
                return true;
            };
            let rel = rel.to_string_lossy();
            if descend.contains(rel.as_ref()) || !old_dirs.contains(rel.as_ref()) {
                return true;
            }
            sink.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(rel.into_owned());
            false
        });
    }
    b.build_parallel().run(|| {
        let files = &files;
        let dirs = &dirs;
        Box::new(move |res| {
            if let Ok(e) = res
                && let Ok(rel) = e.path().strip_prefix(root)
            {
                let rel = rel.to_string_lossy().into_owned();
                if e.file_type().is_some_and(|t| t.is_dir()) {
                    if let Ok(md) = std::fs::symlink_metadata(e.path()) {
                        dirs.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((rel, mtime_ns(&md)));
                    }
                } else if e.file_type().is_some_and(|t| t.is_file()) && !rel.is_empty() {
                    files.lock().unwrap_or_else(|e| e.into_inner()).push(rel);
                } else if e.file_type().is_some_and(|t| t.is_symlink()) && !rel.is_empty() {
                    let link = format!("{LINK}{rel}");
                    files.lock().unwrap_or_else(|e| e.into_inner()).push(link);
                }
            }
            WalkState::Continue
        })
    });
    let pruned = std::mem::take(&mut *pruned_out.lock().unwrap_or_else(|e| e.into_inner()));
    let pruned: std::collections::HashSet<String> = pruned.into_iter().collect();
    // Whether a repo-relative path is a kept directory or lies below one.
    let kept = |p: &str| -> bool {
        let mut q = p;
        loop {
            if pruned.contains(q) {
                return true;
            }
            match q.rfind('/') {
                Some(i) => q = &q[..i],
                None => return false,
            }
        }
    };
    let mut files = files.into_inner().unwrap_or_else(|e| e.into_inner());
    files.extend(
        old_files
            .iter()
            .filter(|f| kept(f.trim_start_matches(LINK)))
            .cloned(),
    );
    files.sort_unstable();
    let mut st = Stamp {
        dirs: dirs.into_inner().unwrap_or_else(|e| e.into_inner()),
        files: Vec::new(),
    };
    let fresh: std::collections::HashSet<String> = st.dirs.iter().map(|(d, _)| d.clone()).collect();
    st.dirs
        .extend(old.dirs.iter().filter(|(d, _)| kept(d)).cloned());
    st.dirs.sort_unstable();
    // Ignore stamps: fresh for the listed directories, cached for the kept ones.
    st.files = ignore_stamps(root, fresh.iter().map(String::as_str), true);
    st.files.extend(
        old.files
            .iter()
            .filter(|(f, _, _)| {
                f.rsplit_once('/')
                    .is_some_and(|(d, _)| kept(d) && !fresh.contains(d))
            })
            .cloned(),
    );
    st.files.sort_unstable();
    (files, st)
}

fn walk_stamped(root: &Path, out_dir: &Path, threads: usize) -> (Vec<String>, Stamp) {
    let files = Mutex::new(Vec::new());
    let dirs = Mutex::new(Vec::new());
    let out_dir = out_dir.to_path_buf();
    let mut b = builder(root, threads);
    b.filter_entry(move |e| {
        let name = e.file_name();
        if e.file_type().is_some_and(|t| t.is_dir()) {
            return name != ".git"
                && name != "node_modules"
                && name != crate::GRAPH_DIR
                && e.path() != out_dir;
        }
        true
    });
    b.build_parallel().run(|| {
        let files = &files;
        let dirs = &dirs;
        Box::new(move |res| {
            if let Ok(e) = res
                && let Ok(rel) = e.path().strip_prefix(root)
            {
                let rel = rel.to_string_lossy().into_owned();
                if e.file_type().is_some_and(|t| t.is_dir()) {
                    if let Ok(md) = std::fs::symlink_metadata(e.path()) {
                        dirs.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((rel, mtime_ns(&md)));
                    }
                } else if e.file_type().is_some_and(|t| t.is_file()) && !rel.is_empty() {
                    files.lock().unwrap_or_else(|e| e.into_inner()).push(rel);
                } else if e.file_type().is_some_and(|t| t.is_symlink()) && !rel.is_empty() {
                    let link = format!("{LINK}{rel}");
                    files.lock().unwrap_or_else(|e| e.into_inner()).push(link);
                }
            }
            WalkState::Continue
        })
    });
    let mut files = files.into_inner().unwrap_or_else(|e| e.into_inner());
    files.sort_unstable();
    let mut st = Stamp {
        dirs: dirs.into_inner().unwrap_or_else(|e| e.into_inner()),
        files: Vec::new(),
    };
    st.dirs.sort_unstable();
    // Every ignore file the walk could have obeyed: present ones (in the list or
    // not) and the missing candidates in each walked directory, plus
    // `.git/info/exclude`.
    st.files = ignore_stamps(root, st.dirs.iter().map(|(d, _)| d.as_str()), true);
    st.files.sort_unstable();
    (files, st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_gitignore_without_git() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        for f in [
            "a.ts",
            "dist/x.js",
            "src/b.ts",
            "node_modules/x/index.js",
            ".github/actions/a.js",
            ".turbotrace/store/k",
            // A graph directory of another build, deeper in the tree.
            "src/.turbotrace/graph.json",
        ] {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), "").unwrap();
        }
        std::fs::write(r.join(".gitignore"), "dist/\n").unwrap();
        // Symbolic links are no files of the walk, and it does not follow them: to a
        // directory, to a file, to nothing, and one that an ignore rule covers.
        for (target, link) in [
            ("src", "linked"),
            ("../a.ts", "src/link.ts"),
            ("nothing", "src/dangling"),
            ("../a.ts", "dist/ignored.ts"),
        ] {
            turborepo_trace_core::os::symlink(target, r.join(link)).unwrap();
        }
        let files = walk(r, &r.join(".turbotrace"), 2);
        assert_eq!(
            files,
            [".github/actions/a.js", ".gitignore", "a.ts", "src/b.ts"]
        );
        // They are entries of the tree, kept with the cached walk (the second walk, and
        // the third that lists `src` again and keeps the rest).
        let out = r.join(".turbotrace");
        for round in 0..3 {
            if round == 2 {
                std::fs::write(r.join("src/c.ts"), "").unwrap();
            }
            let w = walk_cached(r, &out, 2);
            assert_eq!(
                w.links,
                ["linked", "src/dangling", "src/link.ts"],
                "{round}"
            );
            assert_eq!(w.files.len(), if round == 2 { 5 } else { 4 }, "{round}");
        }
    }

    #[test]
    fn cached_walk_sees_adds_and_ignore_edits() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let out = r.join(".turbotrace");
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::write(r.join("src/a.ts"), "").unwrap();
        // Make the directories old enough to be trusted.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let set_old = |p: &Path| {
            std::fs::File::open(p).unwrap().set_modified(old).unwrap();
        };
        set_old(r);
        set_old(&r.join("src"));
        let w = walk_cached(r, &out, 2);
        assert!(w.unchanged.is_none());
        assert_eq!(w.files, ["src/a.ts"]);
        assert_eq!(w.dirs, ["", "src"]);
        // `.turbotrace/cache` was created inside the root: refresh the root's mtime.
        set_old(r);
        let _ = walk_cached(r, &out, 2);
        let w = walk_cached(r, &out, 2);
        assert!(w.unchanged.is_some(), "nothing changed");
        assert_eq!(w.unchanged, cache_token(&out));
        assert_eq!(w.files, ["src/a.ts"]);
        assert_eq!(w.dirs, ["", "src"]);
        let token = cache_token(&out);
        std::fs::write(r.join("src/b.ts"), "").unwrap();
        let w = walk_cached(r, &out, 2);
        assert!(w.unchanged.is_none());
        assert_eq!(w.files, ["src/a.ts", "src/b.ts"]);
        // Compared with the previous cache: only `src` lists differently.
        assert_eq!(w.prev, token);
        assert_eq!(w.changed, ["src"]);
        set_old(&r.join("src"));
        let _ = walk_cached(r, &out, 2);
        std::fs::write(r.join(".gitignore"), "src/b.ts\n").unwrap();
        let w = walk_cached(r, &out, 2);
        assert_eq!(w.files, [".gitignore", "src/a.ts"]);
        // A new ignore file: `src` lists differently without an mtime change.
        assert!(w.prev.is_none());
    }

    #[test]
    fn incremental_walk_equals_a_full_walk() {
        let t = tempfile::tempdir().unwrap();
        let r = std::fs::canonicalize(t.path()).unwrap();
        let out = r.join(".turbotrace");
        let put = |f: &str| {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), "").unwrap();
        };
        for f in [
            "a/x.ts",
            // A sibling that sorts between `a/b` and `a/b/...` (`-` < `/`).
            "a/b-x/v.ts",
            "a/b.d/u.ts",
            "a/b/y.ts",
            "a/b/c/z.ts",
            "a/b/c/d/w.ts",
            "e/f/g.ts",
            "e/h/i.ts",
            "k/l.ts",
            ".gitignore",
        ] {
            put(f);
        }
        std::fs::write(r.join(".gitignore"), ".turbotrace\n").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let age = |r: &Path| {
            fn rec(p: &Path, t: std::time::SystemTime) {
                if p.is_dir() {
                    for e in std::fs::read_dir(p).unwrap() {
                        rec(&e.unwrap().path(), t);
                    }
                }
                std::fs::File::open(p).unwrap().set_modified(t).unwrap();
            }
            rec(r, old);
        };
        let full = |r: &Path| walk(r, &out, 2);
        let step = |change: &dyn Fn()| {
            age(&r);
            let _ = walk_cached(&r, &out, 2);
            age(&r);
            let w = walk_cached(&r, &out, 2);
            assert!(w.unchanged.is_some(), "settled");
            change();
            let w = walk_cached(&r, &out, 2);
            assert!(w.unchanged.is_none());
            assert_eq!(w.files, full(&r));
            let mut dirs = w.dirs.clone();
            dirs.sort();
            assert_eq!(w.dirs, dirs);
            w
        };
        // A file added next to `a/b`, `a/b-x` and `a/b.d`: all three subtrees are kept.
        let w = step(&|| put("a/new.ts"));
        assert_eq!(w.changed, ["a"]);
        assert!(w.files.contains(&"a/b/c/d/w.ts".to_string()));
        // A file added deep down: only that directory and its ancestors are listed
        // again.
        let w = step(&|| put("a/b/c/new.ts"));
        assert_eq!(w.changed, ["a/b/c"]);
        // A directory removed, another added with nested content.
        let w = step(&|| {
            std::fs::remove_dir_all(r.join("e/f")).unwrap();
            put("e/n/o/p.ts");
            put("q/r/s.ts");
        });
        assert!(w.dirs.contains(&"e/n/o".to_string()) && !w.dirs.contains(&"e/f".to_string()));
        // A new ignore file in a changed directory hides an unchanged subdirectory.
        let w = step(&|| std::fs::write(r.join("a/.gitignore"), "b/\n").unwrap());
        assert!(
            !w.files.iter().any(|f| f.starts_with("a/b/")),
            "{:?}",
            w.files
        );
        // An edited ignore file brings it back.
        step(&|| std::fs::write(r.join("a/.gitignore"), "nothing\n").unwrap());
    }
}
