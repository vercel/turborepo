//! Another revision of the tree, seen through the working tree: the files that
//! differ between the two are answered from the revision, everything else from
//! the disk.
//!
//! [`super::governance`] runs tsconfig discovery on the base revision of an
//! `affected` query this way. The files of a revision that differ are known
//! from the diff (added: not there; modified or deleted: there, with the
//! revision's contents, which are read on first use); what the diff does not
//! name is the same at both revisions, and what git does not track
//! (`node_modules`, generated files) is taken as it is on disk.
//!
//! Git tracks files, not directories. A directory above a file of the revision
//! exists there. A directory that holds nothing but files the diff added exists
//! on disk and did not at the revision: it is answered as on disk, and every
//! file in it is absent, so nothing loads through it at the revision.
//!
//! Entries are keyed by real paths, which is how a diff names them. A path that
//! goes through a symbolic link (a workspace package's `node_modules` link, a
//! tracked link to a shared config, a linked directory) is looked up by where
//! the link leads: a link that the diff does not name is the same at both
//! revisions, so the revision's file behind it is the one the diff has at the
//! link's target.
//!
//! A path the diff does name is the revision's, with everything below it:
//!
//! - **a link of the revision** ([`Overlay::with_links`]: git stores a link as
//!   a file whose contents are where it leads) is followed by the revision's
//!   own text, resolved like any other path here. So a linked directory that
//!   the diff made lead elsewhere leads, at the revision, where it led then;
//! - **anything else the diff names** (a path that was not there, a file) has
//!   nothing below it at the revision, whatever the working tree has there now
//!   (a link that the diff added, and the directory it leads to);
//! - **a directory of the revision at a path the diff names** (its files were
//!   deleted and a file or a link took its place) holds what the diff says it
//!   held, and nothing else.
//!
//! What the revision has at a path that the diff does not name is on disk, at
//! the place the revision's links lead to: [`Overlay::on_disk`] is the path to
//! ask the working tree for (the path as asked would go through the working
//! tree's links).

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use dashmap::DashMap;
use turborepo_trace_core::DirCache;

/// Reads a file of the revision (an absolute path of the working tree); `None`
/// when it can't be read.
pub type ReadAt = Box<dyn Fn(&Path) -> Option<Vec<u8>> + Send + Sync>;

/// What a revision has at a path that differs from the working tree.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A file.
    File,
    /// A directory (it holds a file of the revision).
    Dir,
    /// Nothing.
    Absent,
}

/// The files of a revision that differ from the working tree.
pub struct Overlay {
    /// Absolute path → whether the file is there at the revision.
    files: HashMap<PathBuf, bool>,
    /// Every directory above a file that is there.
    dirs: HashSet<PathBuf>,
    /// The files that are symbolic links at the revision
    /// ([`Overlay::with_links`]).
    links: HashSet<PathBuf>,
    read: ReadAt,
    contents: DashMap<PathBuf, Option<Arc<[u8]>>>,
    /// Per directory asked about: where its links lead, and what answers for
    /// the paths in it ([`Overlay::real_dir`]).
    real_dirs: DashMap<PathBuf, (PathBuf, Within)>,
}

/// What answers for the paths in a directory, at the revision.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Within {
    /// The entries of the revision, and the working tree for every other path.
    Disk,
    /// Only the entries of the revision: the directory is one of the revision
    /// at a path where the working tree has something else (a file, a
    /// link).
    Revision,
    /// Nothing: there is no such directory at the revision.
    Nothing,
}

/// Symbolic links followed for one path (Linux `MAXSYMLINKS`).
const MAX_LINKS: u32 = 40;

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

impl Overlay {
    /// `files`: every path that differs, absolute, with whether the revision
    /// has it. `read` gives the contents of one that it has.
    pub fn new(files: impl IntoIterator<Item = (PathBuf, bool)>, read: ReadAt) -> Self {
        let files: HashMap<PathBuf, bool> = files.into_iter().collect();
        let mut dirs = HashSet::new();
        for (p, there) in &files {
            if !*there {
                continue;
            }
            let mut cur = p.parent();
            while let Some(d) = cur {
                if !dirs.insert(d.to_path_buf()) {
                    break;
                }
                cur = d.parent();
            }
        }
        Overlay {
            files,
            dirs,
            links: HashSet::new(),
            read,
            contents: DashMap::new(),
            real_dirs: DashMap::new(),
        }
    }

    /// With the files of the revision (among those given to [`Overlay::new`])
    /// that are symbolic links there: the revision's contents of each are
    /// where it leads.
    pub fn with_links(mut self, links: impl IntoIterator<Item = PathBuf>) -> Self {
        self.links = links
            .into_iter()
            .filter(|l| self.files.get(l) == Some(&true))
            .collect();
        self
    }

    /// Gives the overlay the revision's contents of `path` (one of its files),
    /// which the caller has read already.
    pub fn preload(&self, path: &Path, bytes: Vec<u8>) {
        self.contents
            .insert(path.to_path_buf(), Some(Arc::from(bytes)));
    }

    /// The revision's contents of the entry at the real path `real`, read once.
    fn contents(&self, real: &Path) -> Option<Arc<[u8]>> {
        if let Some(b) = self.contents.get(real) {
            return b.clone();
        }
        let b: Option<Arc<[u8]>> = (self.read)(real).map(Arc::from);
        self.contents.insert(real.to_path_buf(), b.clone());
        b
    }

    /// `path` as the revision's entries name it: with the symbolic links on the
    /// way resolved, the revision's own where the diff names a link and the
    /// working tree's otherwise (module docs), and what answers for it
    /// there. Asked of the disk with one `readlink` per directory (once)
    /// and per path, not of the directory cache's listings: most paths
    /// asked about lie in directories that nothing else looks at.
    fn locate(&self, fs: &DirCache, path: &Path, depth: u32) -> (PathBuf, Within) {
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return (path.to_path_buf(), Within::Disk);
        };
        let (real_dir, within) = self.real_dir(fs, dir, depth);
        let at = real_dir.join(name);
        if within == Within::Nothing {
            return (at, within);
        }
        self.follow(fs, at, within, depth)
    }

    /// The directory `dir` with its links resolved, and what answers for the
    /// paths in it; once.
    fn real_dir(&self, fs: &DirCache, dir: &Path, depth: u32) -> (PathBuf, Within) {
        if let Some(r) = self.real_dirs.get(dir) {
            return r.clone();
        }
        let (at, above) = self.locate(fs, dir, depth);
        let within = match (above, self.files.contains_key(&at)) {
            (Within::Nothing, _) => Within::Nothing,
            // A path the diff names: in the working tree it is a file, a link or nothing,
            // so only what the revision has below it is there.
            (_, true) | (Within::Revision, false) => {
                if self.dirs.contains(&at) {
                    Within::Revision
                } else {
                    Within::Nothing
                }
            }
            (Within::Disk, false) => Within::Disk,
        };
        self.real_dirs
            .insert(dir.to_path_buf(), (at.clone(), within));
        (at, within)
    }

    /// Where `at` (its directory is real, and `within` answers for the paths in
    /// it) leads when it is a symbolic link: of the revision when the diff
    /// names it, of the working tree otherwise. `at` itself when it is no
    /// link (a file or a directory of the revision, a path that was not
    /// there, a path of the working tree that is none).
    fn follow(&self, fs: &DirCache, at: PathBuf, within: Within, depth: u32) -> (PathBuf, Within) {
        if depth >= MAX_LINKS {
            return (at, within);
        }
        let target: PathBuf = if self.links.contains(&at) {
            // The revision's link: its contents are where it leads.
            match self.contents(&at) {
                Some(text) => {
                    use turborepo_trace_core::os::OsStrExt as _;
                    PathBuf::from(std::ffi::OsStr::from_bytes(&text))
                }
                None => return (at, within),
            }
        } else if self.files.contains_key(&at)
            || self.dirs.contains(&at)
            || within == Within::Revision
        {
            return (at, within);
        } else {
            match fs.read_link(&at) {
                Some(t) => t.to_path_buf(),
                None => return (at, within),
            }
        };
        let Some(dir) = at.parent() else {
            return (at, within);
        };
        // The target is relative to the link's directory, and can go through links
        // itself.
        let next = turborepo_trace_core::fs::normalize(&dir.join(target));
        self.locate(fs, &next, depth + 1)
    }

    /// What the revision has at `path`; `None` when the working tree answers,
    /// at [`Overlay::on_disk`].
    pub fn entry(&self, fs: &DirCache, path: &Path) -> Option<Entry> {
        let (real, within) = self.locate(fs, path, 0);
        self.entry_at(&real, within)
    }

    /// [`Overlay::entry`] of a located path.
    fn entry_at(&self, real: &Path, within: Within) -> Option<Entry> {
        if within == Within::Nothing {
            return Some(Entry::Absent);
        }
        // A path with a file of the revision below it is a directory there, whatever
        // the working tree has at it.
        let dir = || self.dirs.contains(real);
        match self.files.get(real) {
            Some(true) => Some(Entry::File),
            Some(false) if dir() => Some(Entry::Dir),
            Some(false) => Some(Entry::Absent),
            None if dir() => Some(Entry::Dir),
            None if within == Within::Revision => Some(Entry::Absent),
            None => None,
        }
    }

    /// The path to ask the working tree for what the revision has at `path`,
    /// when the revision has no entry there ([`Overlay::entry`] is `None`):
    /// `path` with the links resolved as the revision has them. A link that
    /// the diff made lead elsewhere led, at the revision, to a place that
    /// is on disk as it was.
    pub fn on_disk<'a>(&self, fs: &DirCache, path: &'a Path) -> Cow<'a, Path> {
        let (real, _) = self.locate(fs, path, 0);
        if real == path {
            Cow::Borrowed(path)
        } else {
            Cow::Owned(real)
        }
    }

    /// The revision's contents of `path`; `None` when the working tree answers,
    /// at [`Overlay::on_disk`].
    pub fn read(&self, fs: &DirCache, path: &Path) -> Option<io::Result<Arc<[u8]>>> {
        let (real, within) = self.locate(fs, path, 0);
        match self.entry_at(&real, within)? {
            Entry::File => {}
            Entry::Dir => return Some(Err(io::Error::other("is a directory"))),
            Entry::Absent => return Some(Err(io::Error::from(io::ErrorKind::NotFound))),
        }
        Some(
            self.contents(&real)
                .ok_or_else(|| io::Error::other("the file can't be read at the revision")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file of the revision is found behind the links of the working tree: a
    /// link to the file, a linked directory, a link to a link. A link that
    /// the diff names is an entry of its own.
    #[test]
    fn a_file_of_the_revision_behind_a_link() {
        let d = tempfile::tempdir().unwrap();
        let r = turborepo_trace_core::fs::normalize(&std::fs::canonicalize(d.path()).unwrap());
        std::fs::create_dir_all(r.join("shared/configs")).unwrap();
        std::fs::create_dir_all(r.join("app")).unwrap();
        std::fs::write(r.join("shared/configs/lib.json"), "now").unwrap();
        let link = |target: &str, at: &str| {
            turborepo_trace_core::os::symlink(target, r.join(at)).unwrap();
        };
        link("../shared/configs/lib.json", "app/lib.base.json");
        link("../shared/configs", "app/configs");
        link("lib.base.json", "app/again.json");
        // Its target was deleted by the diff.
        link("../shared/configs/gone.json", "app/dangling.json");
        // Retargeted by the diff: at the revision it led elsewhere.
        link("../shared/configs/lib.json", "app/moved.json");
        let fs = DirCache::new();
        let o = Overlay::new(
            [
                (r.join("shared/configs/lib.json"), true),
                (r.join("shared/configs/gone.json"), true),
                (r.join("app/moved.json"), true),
            ],
            Box::new(|p| Some(format!("then {}", p.file_name()?.to_string_lossy()).into_bytes())),
        );
        let read = |p: &str| o.read(&fs, &r.join(p)).map(|b| b.unwrap().to_vec());
        for through in [
            "app/lib.base.json",
            "app/configs/lib.json",
            "app/again.json",
            "shared/configs/lib.json",
        ] {
            assert_eq!(
                o.entry(&fs, &r.join(through)),
                Some(Entry::File),
                "{through}"
            );
            assert_eq!(read(through).unwrap(), b"then lib.json", "{through}");
        }
        assert_eq!(read("app/dangling.json").unwrap(), b"then gone.json");
        // The link the diff names: the revision's entry at the link itself.
        assert_eq!(read("app/moved.json").unwrap(), b"then moved.json");
        // A directory of the revision behind the linked directory.
        assert_eq!(o.entry(&fs, &r.join("app/configs")), Some(Entry::Dir));
        // What the diff does not name is the working tree's.
        assert!(o.entry(&fs, &r.join("app/other.json")).is_none());
    }

    /// Below a link that the diff names, the revision's tree: a linked
    /// directory that led elsewhere, one that was not there, one that was a
    /// file, and one whose place a directory of the revision had.
    #[test]
    fn below_a_link_that_the_diff_names() {
        let d = tempfile::tempdir().unwrap();
        let r = turborepo_trace_core::fs::normalize(&std::fs::canonicalize(d.path()).unwrap());
        for dir in ["shared/x", "shared/y", "app"] {
            std::fs::create_dir_all(r.join(dir)).unwrap();
        }
        std::fs::write(r.join("shared/x/lib.json"), "x").unwrap();
        std::fs::write(r.join("shared/y/lib.json"), "y").unwrap();
        std::fs::write(r.join("shared/y/only-y.json"), "only y").unwrap();
        let link = |target: &str, at: &str| {
            turborepo_trace_core::os::symlink(target, r.join(at)).unwrap();
        };
        // Now they all lead to `shared/x`.
        for at in ["moved", "added", "was-file", "was-dir", "same", "to-file"] {
            link("../shared/x", &format!("app/{at}"));
        }
        std::fs::remove_file(r.join("app/to-file")).unwrap();
        link("../shared/x/lib.json", "app/to-file");
        let fs = DirCache::new();
        let o = Overlay::new(
            [
                // At the revision: a link to `shared/y`, nothing, a file, a directory with
                // one file, and a link to another file.
                (r.join("app/moved"), true),
                (r.join("app/added"), false),
                (r.join("app/was-file"), true),
                (r.join("app/was-dir"), false),
                (r.join("app/was-dir/lib.json"), true),
                (r.join("app/to-file"), true),
                // A file behind the revision's link that the diff changed too.
                (r.join("shared/y/lib.json"), true),
            ],
            Box::new(|p| {
                let name = p.file_name()?.to_string_lossy().into_owned();
                Some(
                    match name.as_str() {
                        "moved" => "../shared/y".to_owned(),
                        "to-file" => "../shared/y/only-y.json".to_owned(),
                        other => format!("then {other}"),
                    }
                    .into_bytes(),
                )
            }),
        )
        .with_links([
            r.join("app/moved"),
            r.join("app/to-file"),
            // (Not a file of the revision: ignored.)
            r.join("app/added"),
        ]);
        let entry = |p: &str| o.entry(&fs, &r.join(p));
        let read = |p: &str| o.read(&fs, &r.join(p)).map(|b| b.unwrap().to_vec());
        let on_disk = |p: &str| o.on_disk(&fs, &r.join(p)).into_owned();
        // The link led to `shared/y`: the diff's file there, and the others on disk
        // there.
        assert_eq!(entry("app/moved/lib.json"), Some(Entry::File));
        assert_eq!(read("app/moved/lib.json").unwrap(), b"then lib.json");
        assert_eq!(entry("app/moved/only-y.json"), None);
        assert_eq!(
            on_disk("app/moved/only-y.json"),
            r.join("shared/y/only-y.json")
        );
        assert_eq!(
            on_disk("app/moved/nothing.json"),
            r.join("shared/y/nothing.json")
        );
        assert_eq!(on_disk("app/moved"), r.join("shared/y"));
        // A link to a file leads where it led.
        assert_eq!(entry("app/to-file"), None);
        assert_eq!(on_disk("app/to-file"), r.join("shared/y/only-y.json"));
        // Not there at the revision: nothing below it, whatever the working tree's link
        // leads to.
        assert_eq!(entry("app/added"), Some(Entry::Absent));
        assert_eq!(entry("app/added/lib.json"), Some(Entry::Absent));
        assert_eq!(entry("app/added/deep/x.json"), Some(Entry::Absent));
        // A file at the revision: nothing below it.
        assert_eq!(entry("app/was-file"), Some(Entry::File));
        assert_eq!(read("app/was-file").unwrap(), b"then was-file");
        assert_eq!(entry("app/was-file/lib.json"), Some(Entry::Absent));
        // A directory of the revision: what the diff says it held, and nothing else.
        assert_eq!(entry("app/was-dir"), Some(Entry::Dir));
        assert_eq!(read("app/was-dir/lib.json").unwrap(), b"then lib.json");
        assert_eq!(entry("app/was-dir/other.json"), Some(Entry::Absent));
        // A link the diff does not name is the working tree's.
        assert_eq!(entry("app/same/lib.json"), None);
        assert_eq!(on_disk("app/same/lib.json"), r.join("shared/x/lib.json"));
    }

    #[test]
    fn files_of_the_revision_and_the_directories_above_them() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("a")).unwrap();
        std::fs::write(r.join("a/added.json"), "{}").unwrap();
        std::fs::write(r.join("a/kept.json"), "{}").unwrap();
        let fs = DirCache::new();
        let o = Overlay::new(
            [
                (r.join("a/added.json"), false),
                (r.join("a/modified.json"), true),
                (r.join("gone/deep/deleted.json"), true),
            ],
            Box::new(|p| {
                Some(format!("// {}", p.file_name().unwrap().to_string_lossy()).into_bytes())
            }),
        );
        assert_eq!(o.entry(&fs, &r.join("a/added.json")), Some(Entry::Absent));
        assert_eq!(o.entry(&fs, &r.join("a/modified.json")), Some(Entry::File));
        assert_eq!(o.entry(&fs, &r.join("a/kept.json")), None);
        assert_eq!(o.entry(&fs, &r.join("gone/deep")), Some(Entry::Dir));
        assert_eq!(o.entry(&fs, &r.join("gone")), Some(Entry::Dir));
        assert_eq!(o.entry(&fs, &r.join("other")), None);
        assert!(o.read(&fs, &r.join("a/kept.json")).is_none());
        assert_eq!(
            &*o.read(&fs, &r.join("gone/deep/deleted.json"))
                .unwrap()
                .unwrap(),
            b"// deleted.json"
        );
        assert_eq!(
            o.read(&fs, &r.join("a/added.json"))
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }
}
