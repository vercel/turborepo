//! Resolving glob matches to the names actually stored on disk.
//!
//! A glob's literal segments come back from the walk spelled as written, not
//! as stored. On case-insensitive or normalization-insensitive filesystems
//! (APFS, NTFS) `Index.ts` or an NFD `café.ts` open the stored `index.ts` or
//! NFC `café.ts`. Every guard compares the stored names instead, read from
//! the parent directory listing.

use std::{collections::HashMap, ffi::OsString, rc::Rc};

use turbopath::AbsoluteSystemPath;
use unicode_normalization::UnicodeNormalization;

/// A name folded for comparison: Unicode-normalized and case-folded, so two
/// names that a case- or normalization-insensitive filesystem treats as the
/// same compare equal. Folding too much only ever protects more.
pub(crate) fn fold(name: &str) -> String {
    let decomposed: String = name.nfd().collect();
    decomposed.to_uppercase().to_lowercase().nfc().collect()
}

/// A coarse key under which every spelling a case- or
/// normalization-insensitive filesystem could treat as the same name
/// collides: compatibility-decomposed, lowercased, and reduced to ASCII,
/// where a character whose uppercase is ASCII (`ß`, `ı`, `ſ`) counts as that
/// and any other character is dropped. Used only to narrow down which names
/// to compare by identity, so merging names that differ costs a lookup,
/// never safety.
pub(crate) fn skeleton(path: &str) -> String {
    let lowered: String = path.nfkd().flat_map(char::to_lowercase).collect();
    let mut skeleton = String::with_capacity(lowered.len());
    for char in lowered.nfkd() {
        if char.is_ascii() {
            skeleton.push(char.to_ascii_lowercase());
            continue;
        }
        let upper: String = char.to_uppercase().collect();
        if upper.is_ascii() {
            skeleton.push_str(&upper.to_ascii_lowercase());
        }
    }
    skeleton
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug)]
struct Entry {
    name: String,
    kind: EntryKind,
}

#[derive(Debug)]
pub(crate) struct Listing {
    entries: Vec<Entry>,
    /// Entries whose names are not UTF-8. No glob match or plan entry can
    /// name them, so they are never removed and keep their directory.
    unnamed: usize,
    /// Whether the directory holds a `.git` entry: a repository of its own.
    has_git: bool,
}

impl Listing {
    pub(crate) fn from_entries(entries: impl IntoIterator<Item = (OsString, EntryKind)>) -> Self {
        let mut named = Vec::new();
        let mut unnamed = 0;
        for (name, kind) in entries {
            match name.into_string() {
                Ok(name) => named.push(Entry { name, kind }),
                Err(_) => unnamed += 1,
            }
        }
        let has_git = named.iter().any(|entry| fold(&entry.name) == ".git");
        Self {
            entries: named,
            unnamed,
            has_git,
        }
    }

    #[cfg(test)]
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }

    /// Whether removing the children for which `removed` holds leaves the
    /// directory empty. An entry the plan cannot name always remains, so a
    /// dry run never lists a directory the real removal would find full.
    pub(crate) fn emptied_by(&self, mut removed: impl FnMut(&str) -> bool) -> bool {
        self.unnamed == 0 && self.entries.iter().all(|entry| removed(&entry.name))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// The stored path, one name per component, and what it is.
    Found {
        components: Vec<String>,
        kind: EntryKind,
        /// The number of leading components naming the outermost directory
        /// below the repository root that holds its own `.git`: the path
        /// belongs to that nested repository.
        nested_repository: Option<usize>,
    },
    /// Nothing by that name (anymore).
    Missing,
    /// Several stored names match the spelling.
    Ambiguous,
    /// A directory on the way is a symlink.
    ThroughSymlink,
    /// A directory on the way could not be listed.
    Unreadable,
}

/// Directory listings under a real repository root, read once each.
pub(crate) struct DiskNames<'a> {
    root: &'a AbsoluteSystemPath,
    listings: HashMap<Vec<String>, Option<Rc<Listing>>>,
}

impl<'a> DiskNames<'a> {
    pub(crate) fn new(real_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            root: real_root,
            listings: HashMap::new(),
        }
    }

    /// The listing of the directory at `components` (stored names), or
    /// `None` when it cannot be read.
    pub(crate) fn listing(&mut self, components: &[String]) -> Option<Rc<Listing>> {
        if let Some(listing) = self.listings.get(components) {
            return listing.clone();
        }
        let listing = self.read_listing(components).map(Rc::new);
        self.listings.insert(components.to_vec(), listing.clone());
        listing
    }

    fn read_listing(&self, components: &[String]) -> Option<Listing> {
        let mut path = self.root.as_std_path().to_path_buf();
        path.extend(components);
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).ok()? {
            let entry = entry.ok()?;
            let file_type = entry.file_type().ok()?;
            let kind = if file_type.is_symlink() {
                EntryKind::Symlink
            } else if file_type.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            entries.push((entry.file_name(), kind));
        }
        Some(Listing::from_entries(entries))
    }

    /// Resolves a repository-relative path, spelled as a glob matched it, to
    /// the names stored on disk.
    pub(crate) fn resolve(&mut self, spelled: &[String]) -> Resolution {
        let mut components: Vec<String> = Vec::with_capacity(spelled.len());
        let mut nested_repository = None;
        for (index, segment) in spelled.iter().enumerate() {
            let Some(listing) = self.listing(&components) else {
                return if components.is_empty() || self.exists(&components) {
                    Resolution::Unreadable
                } else {
                    Resolution::Missing
                };
            };
            if !components.is_empty() && listing.has_git && nested_repository.is_none() {
                nested_repository = Some(components.len());
            }
            let entry = match find(&listing, segment) {
                Found::One(entry) => entry,
                Found::None => return Resolution::Missing,
                Found::Many => return Resolution::Ambiguous,
            };
            components.push(entry.name.clone());
            if index + 1 == spelled.len() {
                return Resolution::Found {
                    components,
                    kind: entry.kind,
                    nested_repository,
                };
            }
            match entry.kind {
                EntryKind::Directory => {}
                EntryKind::Symlink => return Resolution::ThroughSymlink,
                EntryKind::File => return Resolution::Missing,
            }
        }
        Resolution::Missing
    }

    fn exists(&self, components: &[String]) -> bool {
        let mut path = self.root.as_std_path().to_path_buf();
        path.extend(components);
        std::fs::symlink_metadata(path).is_ok()
    }
}

enum Found<'a> {
    One(&'a Entry),
    None,
    Many,
}

/// The stored entry a spelling names: the exact name if stored, otherwise
/// the only name that folds to the same value.
fn find<'a>(listing: &'a Listing, spelled: &str) -> Found<'a> {
    if let Some(entry) = listing.entries.iter().find(|entry| entry.name == spelled) {
        return Found::One(entry);
    }
    let folded = fold(spelled);
    let mut matches = listing
        .entries
        .iter()
        .filter(|entry| fold(&entry.name) == folded);
    match (matches.next(), matches.next()) {
        (Some(entry), None) => Found::One(entry),
        (None, _) => Found::None,
        (Some(_), Some(_)) => Found::Many,
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryKind, Listing, fold, skeleton};

    #[test]
    fn folding_ignores_case_and_normalization() {
        assert_eq!(fold("Index.TS"), fold("index.ts"));
        assert_eq!(fold("cafe\u{301}.ts"), fold("caf\u{e9}.ts"));
        assert_eq!(fold("CAF\u{c9}.ts"), fold("cafe\u{301}.ts"));
        // `ſ` (long s) folds to `s`, as APFS treats it.
        assert_eq!(fold("node_module\u{17f}"), fold("node_modules"));
        assert_ne!(fold("index.ts"), fold("index.js"));
    }

    /// Security round 2 #3: `ẞ` and `ß` are one name on APFS but `fold`
    /// keeps them apart; the skeleton must never separate them.
    #[test]
    fn skeletons_never_separate_equivalent_spellings() {
        for (a, b) in [
            (
                "packages/a/gen/stra\u{df}e.ts",
                "packages/a/GEN/STRA\u{1e9e}E.ts",
            ),
            ("cafe\u{301}.ts", "CAF\u{c9}.TS"),
            ("\u{212a}elvin", "kelvin"),
            ("node_module\u{17f}", "NODE_MODULES"),
            ("\u{212b}ngstrom", "\u{e5}ngstrom"),
            ("\u{131}ndex", "INDEX"),
            ("\u{130}ndex", "index"),
        ] {
            assert_eq!(skeleton(a), skeleton(b), "{a} vs {b}");
        }
        assert_ne!(skeleton("index.ts"), skeleton("index.js"));
    }

    /// Correctness round 2 N6: a name that is not UTF-8 can never be in the
    /// plan, so it keeps its directory in the dry run and the real run alike.
    #[cfg(unix)]
    #[test]
    fn names_that_are_not_utf8_keep_their_directory() {
        use std::os::unix::ffi::OsStringExt;

        let named = Listing::from_entries([("out.js".into(), EntryKind::File)]);
        assert!(named.emptied_by(|name| name == "out.js"));

        let listing = Listing::from_entries([
            ("out.js".into(), EntryKind::File),
            (
                std::ffi::OsString::from_vec(vec![0x66, 0xff]),
                EntryKind::File,
            ),
        ]);
        assert_eq!(listing.names().collect::<Vec<_>>(), ["out.js"]);
        assert!(!listing.emptied_by(|_| true));
    }
}
