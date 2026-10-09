//! Resolving glob matches to the names actually stored on disk.
//!
//! A glob's literal segments come back from the walk spelled as written, not
//! as stored. On case-insensitive or normalization-insensitive filesystems
//! (APFS, NTFS) `Index.ts` or an NFD `café.ts` open the stored `index.ts` or
//! NFC `café.ts`. Every guard compares the stored names instead, read from
//! the parent directory listing.

use std::{collections::HashMap, rc::Rc};

use turbopath::AbsoluteSystemPath;
use unicode_normalization::UnicodeNormalization;

/// A name folded for comparison: Unicode-normalized and case-folded, so two
/// names that a case- or normalization-insensitive filesystem treats as the
/// same compare equal. Folding too much only ever protects more.
pub(crate) fn fold(name: &str) -> String {
    let decomposed: String = name.nfd().collect();
    decomposed.to_uppercase().to_lowercase().nfc().collect()
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
    /// Whether the directory holds a `.git` entry: a repository of its own.
    has_git: bool,
}

impl Listing {
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.name.as_str())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// The stored path, one name per component, and what it is.
    Found {
        components: Vec<String>,
        kind: EntryKind,
        /// A directory between the repository root and the path holds its
        /// own `.git`: the path belongs to a nested repository.
        in_nested_repository: bool,
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
            // A name that is not UTF-8 can never equal a glob match.
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let file_type = entry.file_type().ok()?;
            let kind = if file_type.is_symlink() {
                EntryKind::Symlink
            } else if file_type.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            entries.push(Entry { name, kind });
        }
        let has_git = entries.iter().any(|entry| fold(&entry.name) == ".git");
        Some(Listing { entries, has_git })
    }

    /// Resolves a repository-relative path, spelled as a glob matched it, to
    /// the names stored on disk.
    pub(crate) fn resolve(&mut self, spelled: &[String]) -> Resolution {
        let mut components: Vec<String> = Vec::with_capacity(spelled.len());
        let mut in_nested_repository = false;
        for (index, segment) in spelled.iter().enumerate() {
            let Some(listing) = self.listing(&components) else {
                return if components.is_empty() || self.exists(&components) {
                    Resolution::Unreadable
                } else {
                    Resolution::Missing
                };
            };
            if !components.is_empty() && listing.has_git {
                in_nested_repository = true;
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
                    in_nested_repository,
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
    use super::fold;

    #[test]
    fn folding_ignores_case_and_normalization() {
        assert_eq!(fold("Index.TS"), fold("index.ts"));
        assert_eq!(fold("cafe\u{301}.ts"), fold("caf\u{e9}.ts"));
        assert_eq!(fold("CAF\u{c9}.ts"), fold("cafe\u{301}.ts"));
        // `ſ` (long s) folds to `s`, as APFS treats it.
        assert_eq!(fold("node_module\u{17f}"), fold("node_modules"));
        assert_ne!(fold("index.ts"), fold("index.js"));
    }
}
