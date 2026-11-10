//! Turns recorded [`Observations`] into compact, deterministic shard tables
//! (probes, directory memberships, config inputs).
//!
//! Probes:
//! - With opaque third-party packages, paths inside an installed package
//!   directory are dropped: those files only change through the lockfile, which
//!   the package identity covers. A package directory that links to a workspace
//!   package is seen through: its paths are recorded at their real, first-party
//!   location. The walk-up paths themselves (`<d>/node_modules`,
//!   `<d>/node_modules/<name>`, `<d>/node_modules/<name>.js`,
//!   `<d>/node_modules/@scope`) are kept.
//! - For each bare package name the resolution looked up, those walk-up paths
//!   collapse into [`ProbeRec::WalkUp`] records, one per run of consecutive
//!   ancestor directories, instead of about a dozen paths per level. Expanding
//!   a record yields `<d>/node_modules/<name>` and
//!   `<d>/node_modules/<name><suffix>` at every level of the run. That covers
//!   every collapsed path: at a level whose `node_modules` was missing, adding
//!   anything that matters for `<name>` creates a path at or below
//!   `<d>/node_modules/<name>`.
//! - The remaining paths group by stem (the path up to the first `.` of its
//!   file name) into [`ProbeRec::Stem`] records with their suffixes, which is
//!   lossless.
//!
//! Probes and memberships outside the repository root are dropped: change sets
//! are repo-relative, and directories such as `/tmp` change all the time, so
//! they would only churn keys. (Config reads and reached files outside the root
//! are still validated.)
//!
//! Observations below a language's environment root (Python site directories,
//! the Go module cache, the cargo registry) are dropped for the same reason:
//! language package identities cover them.
//!
//! Memberships are deduplicated; a `(d, "**")` listing subsumes every listing
//! at or below `d`. Config reads drop the manifests of opaque packages for the
//! same reason as probes.
//!
//! Path reads ([`Observations::path_reads`]) become config inputs like reads,
//! but are marked path-only ([`Tables::path_configs`]): the key hashes their
//! path, not their contents. A path that is also read for its contents in the
//! same shard is a content input.
//!
//! Every table is indexed in first-seen order over files in path order, so a
//! shard's bytes depend only on what was observed.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::OsStr,
    path::{Path, PathBuf},
};

use turborepo_trace_core::Observations;

use crate::{
    identity::Identities,
    paths::rel_to,
    shard::{MembershipRec, ProbeRec, StemRec, WalkUpRec},
};

const MARKER: &str = "node_modules";

/// What a config input ends with when it stands for the **resolution view** of
/// the manifest at that path ([`turborepo_trace_core::manifest::view`]) and not
/// for the file: the key and the per-entry state hash the view's bytes, so an
/// edit of a field outside the view (a dependency entry, a script) is no change
/// for the references that read it. The same manifest read as a file (by a
/// task, a framework adapter) is another entry of the table, without the
/// suffix. A NUL cannot be part of a path. Queries never see the suffix: the
/// composed graph names the file ([`config_path`]).
pub const VIEW_SUFFIX: &str = "\u{0}view";

/// What a config input ends with when it stands for the view that the loader of
/// a `tsconfig.json` gets of the manifest at that path
/// ([`turborepo_trace_core::manifest::loader_view`]): the fields that the
/// resolution of an `extends` entry can read. A third entry of the table for
/// the same manifest, keyed by those bytes: a config at the repository's root
/// that extends a package has the root manifest as its nearest one, and
/// `packageManager` or `pnpm.overrides` there is no change for the files below
/// it. The composed graph names the file ([`config_path`]), and says which
/// nodes read it this way and no other ([`loader_config`]).
pub const LOADER_SUFFIX: &str = "\u{0}loader";

/// The path of a config input that is a view of a manifest (the resolution view
/// or the loader's), `None` for a file.
pub fn view_config(c: &str) -> Option<&str> {
    c.strip_suffix(VIEW_SUFFIX).or_else(|| loader_config(c))
}

/// The path of a config input that is the loader's view of a manifest
/// ([`LOADER_SUFFIX`]).
pub fn loader_config(c: &str) -> Option<&str> {
    c.strip_suffix(LOADER_SUFFIX)
}

/// The path a config input names, whichever kind it is.
pub fn config_path(c: &str) -> &str {
    view_config(c).unwrap_or(c)
}

/// Context for normalizing one shard's observations.
pub struct NormCx<'a> {
    /// Repository root.
    pub repo: &'a Path,
    /// The shard's package root (absolute).
    pub pkg_root: &'a Path,
    /// Keep observations inside third-party packages (`--externals follow`).
    pub follow: bool,
    /// Package layouts.
    pub ids: &'a Identities,
    /// The fs cache, to see through package links.
    pub fs: &'a turborepo_trace_core::DirCache,
    /// Language environment roots (site-packages, module cache, cargo
    /// registry): reads below them are covered by the package identities,
    /// like reads inside `node_modules` (kept with `follow`).
    pub env_roots: &'a [PathBuf],
}

/// A shard's deduplicated observation tables.
#[derive(Debug, Default)]
pub struct Tables {
    /// Probe table.
    pub probes: Vec<ProbeRec>,
    probe_ix: HashMap<ProbeRec, u32>,
    /// Membership table.
    pub memberships: Vec<MembershipRec>,
    member_ix: HashMap<MembershipRec, u32>,
    /// Config inputs.
    pub configs: Vec<String>,
    config_ix: HashMap<String, u32>,
    /// Per config input: whether its contents enter the key (any content read
    /// of it).
    config_content: Vec<bool>,
    /// [`locate`] results: the same paths recur in many groups of a shard
    /// (shared framework observations are merged into every test's group).
    located: Located,
}

/// Memoized [`locate`] results.
#[derive(Debug, Default)]
pub struct Located {
    /// Per observed path: where it really is, and the package directory it was
    /// seen through.
    map: HashMap<PathBuf, (Option<PathBuf>, Option<PathBuf>)>,
    /// The package directories the current group's observations went through
    /// ([`Normalized::seen`]), collected by [`locate_in`].
    seen: BTreeSet<PathBuf>,
}

impl Located {
    /// An empty memo.
    pub fn new() -> Self {
        Self::default()
    }
}

/// One group's observations normalized but not yet interned (so groups can be
/// normalized in parallel and interned in order).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Normalized {
    probes: Vec<ProbeRec>,
    memberships: Vec<MembershipRec>,
    configs: Vec<String>,
    path_configs: Vec<String>,
    /// The symbolic links the group's observations crossed
    /// ([`turborepo_trace_core::record::link`]), and the package directories (a
    /// `node_modules/<name>` link or directory, absolute) that
    /// observations of the group lay inside: they were recorded at the place
    /// the directory leads to (a workspace package), or dropped (an
    /// installed package). Either way what the group stores depends on
    /// where the directory leads: a lookup that failed inside the package
    /// has no edge that would say so, and a link that goes or leads elsewhere
    /// leaves every recorded path as it was.
    pub(crate) seen: Vec<PathBuf>,
}

impl Normalized {
    /// A group's observations from the records a previous shard stored for it
    /// ([`crate::filereuse`]): probes, memberships and config inputs by
    /// content.
    pub fn from_records(
        probes: Vec<ProbeRec>,
        memberships: Vec<MembershipRec>,
        configs: Vec<String>,
    ) -> Normalized {
        Normalized {
            probes,
            memberships,
            configs,
            path_configs: Vec::new(),
            seen: Vec::new(),
        }
    }

    /// The records as a value: sorted, each once, without the directories they
    /// were located through. Two sets of observations with the same records
    /// are equal.
    pub(crate) fn canonical(mut self) -> Normalized {
        fn sorted<T: Ord>(v: &mut Vec<T>) {
            v.sort_unstable();
            v.dedup();
        }
        sorted(&mut self.probes);
        sorted(&mut self.memberships);
        sorted(&mut self.configs);
        sorted(&mut self.path_configs);
        self.seen = Vec::new();
        self
    }
}

/// Normalizes one group's observations ([`Tables::add`] without interning).
pub fn normalize(cx: &NormCx<'_>, obs: &Observations, memo: &mut Located) -> Normalized {
    memo.seen.clear();
    let mut n = normalize_records(cx, obs, memo);
    // The links the observer crossed itself are places seen, like the package
    // directories.
    memo.seen.extend(obs.links.iter().cloned());
    n.seen = std::mem::take(&mut memo.seen).into_iter().collect();
    n
}

fn normalize_records(cx: &NormCx<'_>, obs: &Observations, memo: &mut Located) -> Normalized {
    Normalized {
        seen: Vec::new(),
        probes: probes(cx, obs, memo),
        memberships: memberships(cx, obs, memo),
        configs: {
            let mut v = configs(cx, &obs.reads, memo);
            // A manifest read through its resolution view is another input than the file.
            if !obs.view_reads.is_empty() {
                v.extend(
                    configs(cx, &obs.view_reads, memo)
                        .into_iter()
                        .map(|c| c + VIEW_SUFFIX),
                );
            }
            // And one read by a config loader, through its narrower view, a third.
            if !obs.loader_reads.is_empty() {
                v.extend(
                    configs(cx, &obs.loader_reads, memo)
                        .into_iter()
                        .map(|c| c + LOADER_SUFFIX),
                );
            }
            v
        },
        path_configs: if obs.path_reads.is_empty() {
            Vec::new()
        } else {
            configs(cx, &obs.path_reads, memo)
        },
    }
}

/// Adds observations to a finished shard as **key-only** table entries: probes,
/// listings and content-keyed config inputs that no file references. The shard
/// key validates them like every other entry (a probed path appears, a listing
/// changes, a file's contents change), and no node of the composed graph
/// carries them.
///
/// For what a package looked for when it found nothing to attach the
/// observation to: task discovery in a package without tasks (the
/// `.github/workflows` listing, the Makefile and task-file probes). Without
/// this the first workflow or Makefile of such a package would not re-key its
/// shard, and a refreshed graph would miss tasks a fresh build has.
///
/// Returns the places the observations went through ([`Normalized::seen`]): the
/// caller records them as places seen, since where each leads is part of what
/// was observed.
pub fn add_key_only(
    cx: &NormCx<'_>,
    obs: &Observations,
    shard: &mut crate::shard::Shard,
) -> Vec<PathBuf> {
    let n = normalize(cx, obs, &mut Located::default());
    // The records the shard has, indexed once: a project with thousands of package
    // directories has as many key-only records, each looked up here (in push order,
    // so the tables and the key are what a lookup in the vectors gives).
    let mut probes: std::collections::HashSet<ProbeRec> = shard.probes.iter().cloned().collect();
    for p in n.probes {
        if probes.insert(p.clone()) {
            shard.probes.push(p);
        }
    }
    let mut memberships: std::collections::HashSet<MembershipRec> =
        shard.memberships.iter().cloned().collect();
    for m in n.memberships {
        if memberships.insert(m.clone()) {
            shard.memberships.push(m);
        }
    }
    let mut configs: HashMap<String, usize> = HashMap::new();
    for (i, c) in shard.configs.iter().enumerate() {
        configs.entry(c.clone()).or_insert(i);
    }
    for c in n.configs {
        match configs.get(&c) {
            // Already an input by path: its contents are keyed now.
            Some(&i) => shard.path_configs.retain(|&j| j as usize != i),
            None => {
                configs.insert(c.clone(), shard.configs.len());
                shard.configs.push(c);
            }
        }
    }
    n.seen
}

/// Config inputs given as absolute paths, relative to the package root (no
/// third-party filtering; [`Tables::add_configs`] without interning).
pub fn config_rels(cx: &NormCx<'_>, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| rel_to(cx.pkg_root, cx.repo, p))
        .collect()
}

/// [`locate`], memoized in `memo`.
fn locate_in(cx: &NormCx<'_>, memo: &mut Located, p: &Path) -> Option<PathBuf> {
    let (at, through) = match memo.map.get(p) {
        Some(r) => r.clone(),
        None => {
            let r = locate(cx, p);
            memo.map.insert(p.to_path_buf(), r.clone());
            r
        }
    };
    if let Some(dir) = through {
        memo.seen.insert(dir);
    }
    at
}

/// Indexes of one group's observations into [`Tables`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GroupIx {
    /// Probe indexes.
    pub probes: Vec<u32>,
    /// Membership indexes.
    pub memberships: Vec<u32>,
    /// Config indexes.
    pub configs: Vec<u32>,
}

impl Tables {
    /// Adds config inputs given as absolute paths (no third-party filtering)
    /// and returns their indexes.
    pub fn add_configs(&mut self, cx: &NormCx<'_>, paths: &[PathBuf]) -> Vec<u32> {
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            out.push(self.config(rel_to(cx.pkg_root, cx.repo, p), true));
        }
        out
    }

    /// Interns one config input; `content`: its contents enter the key.
    fn config(&mut self, c: String, content: bool) -> u32 {
        let i = match self.config_ix.get(&c) {
            Some(&i) => i,
            None => {
                let i = self.configs.len() as u32;
                self.config_ix.insert(c.clone(), i);
                self.configs.push(c);
                self.config_content.push(false);
                i
            }
        };
        self.config_content[i as usize] |= content;
        i
    }

    /// Adds a key-only config input (contents keyed, referenced by no group).
    pub fn add_key_config(&mut self, c: String) {
        self.config(c, true);
    }

    /// Indexes of the config inputs keyed by path only, sorted.
    pub fn path_configs(&self) -> Vec<u32> {
        (0..self.configs.len() as u32)
            .filter(|&i| !self.config_content[i as usize])
            .collect()
    }

    /// Adds one group's observations.
    pub fn add(&mut self, cx: &NormCx<'_>, obs: &Observations) -> GroupIx {
        let n = normalize(cx, obs, &mut self.located);
        self.intern(n, Vec::new())
    }

    /// Interns one group's normalized observations ([`normalize`]) and its
    /// version pins (config inputs relative to the package root, by
    /// content).
    ///
    /// The records of a group enter the tables in the order of their values,
    /// whatever order they were observed or stored in: a group rebuilt from
    /// a previous shard's records ([`Normalized::from_records`]) numbers
    /// the tables as the group traced again would, so the two shards are
    /// the same bytes.
    pub fn intern(&mut self, mut n: Normalized, pins: Vec<String>) -> GroupIx {
        n.probes.sort_unstable();
        n.memberships.sort_unstable();
        let mut configs: Vec<(String, bool)> = (n.configs.into_iter().map(|c| (c, true)))
            .chain(n.path_configs.into_iter().map(|c| (c, false)))
            .chain(pins.into_iter().map(|c| (c, true)))
            .collect();
        configs.sort_unstable();
        let mut out = GroupIx::default();
        for p in n.probes {
            let next = self.probes.len() as u32;
            let i = *self.probe_ix.entry(p.clone()).or_insert_with(|| {
                self.probes.push(p);
                next
            });
            out.probes.push(i);
        }
        for m in n.memberships {
            let next = self.memberships.len() as u32;
            let i = *self.member_ix.entry(m.clone()).or_insert_with(|| {
                self.memberships.push(m);
                next
            });
            out.memberships.push(i);
        }
        for (c, content) in configs {
            out.configs.push(self.config(c, content));
        }
        out.probes.sort_unstable();
        out.probes.dedup();
        out.memberships.sort_unstable();
        out.memberships.dedup();
        out.configs.sort_unstable();
        out.configs.dedup();
        out
    }
}

/// Where an observed path really is. A path inside an installed package
/// directory that is a link to a first-party (workspace) package is translated
/// to the real path, which is what edits touch. A path inside a real
/// third-party package is `None` (dropped) unless `follow`. Anything else is
/// kept as is.
fn locate(cx: &NormCx<'_>, p: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    if cx.follow {
        return (Some(p.to_path_buf()), None);
    }
    if cx.env_roots.iter().any(|r| p.starts_with(r)) {
        return (None, None);
    }
    let Some((root, _)) = cx.ids.package_root(p).filter(|(root, _)| p != root) else {
        return (Some(p.to_path_buf()), None);
    };
    let real = cx.fs.canonicalize(&root).path;
    if cx.ids.is_third_party(&real.join("x")) || !real.starts_with(cx.repo) {
        return (None, Some(root));
    }
    let at = p.strip_prefix(&root).ok().map(|rest| real.join(rest));
    (at, Some(root))
}

fn name_of(p: &Path) -> &str {
    p.file_name().and_then(OsStr::to_str).unwrap_or("")
}

/// If `p` is a walk-up path for package `name`, the level directory and the
/// suffix (`""` for the package directory itself, `None` suffix for a missing
/// marker or scope).
fn walkup_level(p: &Path, name: &str) -> Option<(PathBuf, Option<String>)> {
    let file = name_of(p);
    if file == MARKER {
        return Some((p.parent()?.to_path_buf(), None));
    }
    let parent = p.parent()?;
    let (scope, base) = match name.split_once('/') {
        Some((s, b)) => (Some(s), b),
        None => (None, name),
    };
    let marker_dir = match scope {
        Some(s) => {
            if file == s && name_of(parent) == MARKER {
                return Some((parent.parent()?.to_path_buf(), None));
            }
            if name_of(parent) != s {
                return None;
            }
            parent.parent()?
        }
        None => parent,
    };
    if name_of(marker_dir) != MARKER {
        return None;
    }
    let suffix = file.strip_prefix(base)?;
    if !suffix.is_empty() && !suffix.starts_with('.') {
        return None;
    }
    Some((marker_dir.parent()?.to_path_buf(), Some(suffix.to_owned())))
}

/// The stem of a path: everything up to the first `.` of the file name (not
/// counting a leading dot).
fn stem_split(p: &str) -> (&str, &str) {
    let name_start = p.rfind('/').map_or(0, |i| i + 1);
    let name = &p[name_start..];
    // Skip the name's first character, which may take more than one byte.
    let first = name.chars().next().map_or(0, char::len_utf8);
    match name[first..].find('.') {
        Some(i) => p.split_at(name_start + first + i),
        None => (p, ""),
    }
}

fn probes(cx: &NormCx<'_>, obs: &Observations, memo: &mut Located) -> Vec<ProbeRec> {
    let mut missing: BTreeSet<PathBuf> = obs
        .missing
        .iter()
        .filter_map(|p| locate_in(cx, memo, p))
        .filter(|p| p.starts_with(cx.repo))
        .collect();
    let mut out = Vec::new();
    let mut names: Vec<&str> = obs.packages.iter().map(String::as_str).collect();
    names.sort_unstable();
    names.dedup();
    // A missing marker directory is shared by every name looked up through it, so
    // every name sees the full set; claimed paths are removed once all names
    // are done.
    let mut claimed = BTreeSet::new();
    // Only paths at most two levels below a `node_modules` directory (or one
    // itself) can be walk-up paths of any name; test the names against those
    // alone.
    let walkish: Vec<&PathBuf> = if names.is_empty() {
        Vec::new()
    } else {
        missing
            .iter()
            .filter(|p| p.ancestors().take(3).any(|a| name_of(a) == MARKER))
            .collect()
    };
    for name in names {
        let mut levels: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
        for &p in &walkish {
            if let Some((dir, suffix)) = walkup_level(p, name) {
                let set = levels.entry(dir).or_default();
                if let Some(s) = suffix.filter(|s| !s.is_empty()) {
                    set.insert(s);
                }
                claimed.insert(p.clone());
            }
        }
        if levels.is_empty() {
            continue;
        }
        // Runs of consecutive ancestors, deepest first.
        let mut dirs: Vec<(PathBuf, BTreeSet<String>)> = levels.into_iter().collect();
        dirs.sort_by(|a, b| {
            b.0.components()
                .count()
                .cmp(&a.0.components().count())
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut used = vec![false; dirs.len()];
        let at: HashMap<PathBuf, usize> = dirs
            .iter()
            .enumerate()
            .map(|(j, (d, _))| (d.clone(), j))
            .collect();
        for i in 0..dirs.len() {
            if used[i] {
                continue;
            }
            used[i] = true;
            let start = dirs[i].0.clone();
            let mut suffixes = dirs[i].1.clone();
            let mut levels = 1u32;
            let mut cur = start.clone();
            while let Some(parent) = cur.parent().map(Path::to_path_buf) {
                let Some(&j) = at.get(&parent).filter(|&&j| !used[j]) else {
                    break;
                };
                used[j] = true;
                suffixes.extend(dirs[j].1.iter().cloned());
                levels += 1;
                cur = parent;
            }
            out.push(ProbeRec::WalkUp(WalkUpRec {
                dir: rel_to(cx.pkg_root, cx.repo, &start),
                levels,
                marker: MARKER.to_owned(),
                name: name.to_owned(),
                suffixes: suffixes.into_iter().collect(),
            }));
        }
    }
    for p in &claimed {
        missing.remove(p);
    }
    let mut stems: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in &missing {
        let rel = rel_to(cx.pkg_root, cx.repo, p);
        let (stem, suffix) = stem_split(&rel);
        stems
            .entry(stem.to_owned())
            .or_default()
            .insert(suffix.to_owned());
    }
    for (stem, suffixes) in stems {
        if suffixes.len() == 1 {
            let s = suffixes.into_iter().next().unwrap_or_default();
            out.push(ProbeRec::Path(format!("{stem}{s}")));
        } else {
            out.push(ProbeRec::Stem(StemRec {
                path: stem,
                suffixes: suffixes.into_iter().collect(),
            }));
        }
    }
    out
}

fn memberships(cx: &NormCx<'_>, obs: &Observations, memo: &mut Located) -> Vec<MembershipRec> {
    let mut list: BTreeSet<(PathBuf, String)> = obs
        .listings
        .iter()
        .filter_map(|(d, pat)| {
            let real = locate_in(cx, memo, &d.join("x"))?;
            let dir = real.parent()?;
            dir.starts_with(cx.repo)
                .then(|| (dir.to_path_buf(), pat.clone()))
        })
        .collect();
    // A listing is subsumed by a `**` listing of itself (unless it is that listing)
    // or of any ancestor: one set lookup per ancestor instead of a scan over
    // every `**` listing.
    let whole: std::collections::HashSet<PathBuf> = list
        .iter()
        .filter(|(_, p)| p == "**")
        .map(|(d, _)| d.clone())
        .collect();
    if !whole.is_empty() {
        list.retain(|(d, p)| {
            !d.ancestors()
                .any(|w| whole.contains(w) && !(w == d.as_path() && p == "**"))
        });
    }
    list.into_iter()
        .map(|(d, pattern)| MembershipRec {
            dir: rel_to(cx.pkg_root, cx.repo, &d),
            pattern,
        })
        .collect()
}

fn configs(cx: &NormCx<'_>, reads: &[PathBuf], memo: &mut Located) -> Vec<String> {
    // A read inside a real third-party package (its manifest, a config it ships) is
    // covered by the package identity; through a workspace link it is a
    // first-party file.
    let set: BTreeSet<PathBuf> = reads
        .iter()
        .filter_map(|p| locate_in(cx, memo, p))
        .collect();
    set.iter()
        .map(|p| rel_to(cx.pkg_root, cx.repo, p))
        .collect()
}

/// Expands a probe record into package-relative (or absolute) paths.
pub fn expand_probe(p: &crate::shard::ArchivedProbeRec, out: &mut Vec<String>) {
    use crate::shard::ArchivedProbeRec as P;
    match p {
        P::Path(s) => out.push(s.to_string()),
        P::Stem(st) => {
            for s in st.suffixes.iter() {
                out.push(format!("{}{s}", st.path));
            }
        }
        P::WalkUp(w) => {
            let (marker, name, suffixes) = (&w.marker, &w.name, &w.suffixes);
            let mut d = w.dir.to_string();
            for _ in 0..w.levels.to_native() {
                let base = if d.is_empty() || d == "/" {
                    format!("{}{marker}/{name}", if d == "/" { "/" } else { "" })
                } else {
                    format!("{d}/{marker}/{name}")
                };
                for s in suffixes.iter() {
                    out.push(format!("{base}{s}"));
                }
                out.push(base);
                d = parent_rel(&d);
                if d == "\0" {
                    break;
                }
            }
        }
    }
}

/// The directories a probe record's observations were conditional on without
/// being probes themselves, as package-relative (or absolute) paths: for a
/// walk-up, the marker directory `<d>/node_modules` at every level and, for a
/// scoped name, the scope directory `<d>/node_modules/@scope`.
///
/// The resolver looks for `<name><suffix>` files at a level only where those
/// directories exist, so one that appears changes what a trace records there
/// (the record's suffixes) although nothing the record expands to exists yet. A
/// shard's key hashes which of them exist ([`crate::build`], the tree part), so
/// that a shard's bytes stay a function of its key: a store kept across such a
/// change, or an artifact packed before it, would otherwise hold a shard that a
/// fresh build does not write. They are not probes: no query matches
/// them and no export lists them.
pub fn expand_guards(p: &crate::shard::ArchivedProbeRec, out: &mut Vec<String>) {
    let crate::shard::ArchivedProbeRec::WalkUp(w) = p else {
        return;
    };
    let marker = w.marker.as_str();
    // As `walkup_level` reads a name: everything before the first `/` is its scope.
    let scope = w.name.split_once('/').map(|(s, _)| s);
    let mut d = w.dir.to_string();
    for _ in 0..w.levels.to_native() {
        let base = if d.is_empty() || d == "/" {
            format!("{}{marker}", if d == "/" { "/" } else { "" })
        } else {
            format!("{d}/{marker}")
        };
        if let Some(s) = scope {
            out.push(format!("{base}/{s}"));
        }
        out.push(base);
        d = parent_rel(&d);
        if d == "\0" {
            break;
        }
    }
}

/// The directories the probed paths of `probes` lie in (of a walk-up record
/// none: its levels have guards, [`expand_guards`]), as package-relative (or
/// absolute) paths, sorted and deduplicated. `""` is the package root.
pub fn probe_dirs(probes: &[crate::shard::ArchivedProbeRec]) -> Vec<String> {
    let dirs: BTreeSet<&str> = probes.iter().filter_map(probe_dir).collect();
    dirs.into_iter().map(str::to_owned).collect()
}

/// The directory the probed paths of `p` lie in ([`probe_dirs`]); none for a
/// walk-up record.
pub(crate) fn probe_dir(p: &crate::shard::ArchivedProbeRec) -> Option<&str> {
    use crate::shard::ArchivedProbeRec as P;
    let path = match p {
        P::Path(s) => s.as_str(),
        P::Stem(st) => st.path.as_str(),
        P::WalkUp(_) => return None,
    };
    Some(match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    })
}

/// The tag under which a probe's state says that the directory of its paths is
/// not there ([`probe_dir`]; the key says which of those directories are,
/// [`probe_dirs`]).
///
/// An answer is an observation, whichever answer it is. A lookup that asks "is
/// `d` a directory?" and gets a yes goes on inside `d`, and what it asks there
/// is what it records. The yes is held by those records: each of them is the
/// answer "`d/x` is not there, and `d` is". When `d` goes (or a file takes its
/// place) every such answer is another one, although no path of the record
/// exists before or after, and the lookup asks other questions (`d.json`
/// instead of `d/tsconfig.json`). So the state of a probe is the whole answer
/// the file system gave: which of its paths are there and as what
/// ([`presence`]), and whether their directory is.
///
/// Without the directory, the answer is "not there, from [`nearest_dir`] on":
/// the path stops at the first name that is missing, and anything that appears
/// at that name (a directory, or a link to one: a link leads the rest of the
/// path elsewhere, where a trace records it) makes it another answer.
pub(crate) const DIR_ABSENT: &str = "probe-dir-absent";

/// The nearest directory at or above `dir` (as [`probe_dir`] gives it) that
/// `is_dir` says is there; `"\0"` when none is.
pub(crate) fn nearest_dir(dir: &str, is_dir: impl Fn(&str) -> bool) -> String {
    let mut d = dir.to_owned();
    while d != "\0" && !is_dir(&d) {
        d = parent_rel(&d);
    }
    d
}

/// [`nearest_dir`] of the directory `dir` of the package at `root` on the file
/// system `fs`, where a directory is one that is there under exactly that
/// spelling ([`turborepo_trace_core::DirCache::dir_exact`]). The one function
/// for a shard's key, a probe's state and the key dump: the three say the same
/// thing or the dump explains nothing.
pub(crate) fn nearest_dir_on(
    fs: &turborepo_trace_core::DirCache,
    root: &Path,
    dir: &str,
) -> String {
    nearest_dir(dir, |d| fs.dir_exact(&crate::paths::abs(root, d)))
}

/// Where a walk-up went on: the name's path one level above the record's last
/// level (`<d>/node_modules/<name>`), as a package-relative (or absolute) path.
/// Nothing for a record that ends at the filesystem root.
///
/// A walk-up record holds the levels that missed. The walk stopped above them
/// because it found the name there, or it went on outside the repository, where
/// nothing is recorded. Found there: no record of the lookup says so (what was
/// found may not even be the target of an edge: a second resolution of a
/// specifier that fails inside the package), and when the name goes from that
/// level, a fresh trace records one level more. So whether that path exists
/// enters a shard's key like a guard's ([`expand_guards`]), for the levels
/// inside the repository ([`ABOVE`]).
pub fn expand_above(p: &crate::shard::ArchivedProbeRec, out: &mut Vec<String>) {
    let crate::shard::ArchivedProbeRec::WalkUp(w) = p else {
        return;
    };
    let (marker, name) = (w.marker.as_str(), w.name.as_str());
    let mut d = w.dir.to_string();
    for _ in 0..w.levels.to_native() {
        d = parent_rel(&d);
        if d == "\0" {
            return;
        }
    }
    out.push(if d.is_empty() {
        format!("{marker}/{name}")
    } else if d == "/" {
        format!("/{marker}/{name}")
    } else {
        format!("{d}/{marker}/{name}")
    });
}

/// An expansion of a probe record into paths.
pub(crate) type Expand = fn(&crate::shard::ArchivedProbeRec, &mut Vec<String>);

/// The tag of [`expand_above`] in [`PRESENCE`]. Its paths count only inside the
/// repository: probes outside it are not recorded, so a walk-up that reached
/// the repository's root has its next level outside.
pub(crate) const ABOVE: &str = "above-present";

/// What a shard's key and a probe's state hash of a probe record: which of the
/// paths of each expansion exist, and as what ([`presence`]), under the
/// expansion's tag.
pub(crate) const PRESENCE: [(&str, Expand); 3] = [
    ("probe-present", expand_probe),
    ("guard-present", expand_guards),
    (ABOVE, expand_above),
];

/// What is at the probed path `p`: `None` for nothing, else its kind. A lookup
/// asks more than whether a path is there (is it a file, is it a directory), so
/// a path that was a file and is a directory now is another answer: a shard's
/// key and a probe's state hash the kind of what is there.
///
/// A symbolic link that leads to nothing is another answer than no entry at
/// all, although a lookup finds nothing at either: it follows the link and
/// looks where the link leads, which is another place than the one it was asked
/// for. Where it leads is not part of this state: a trace that follows the link
/// records it as a place seen, whose state says where a place that leads
/// nowhere would lead ([`crate::build::seen_state`]); a trace that
/// does not records nothing of the place, and its records are the same wherever
/// the link leads. (The probe's state carried the link's text until the place
/// seen did; no case told the two apart after that.)
///
/// A link that leads to something says so too (a link to a file, to a
/// directory): what it leads to answers "is it a file" like the file would, and
/// a lookup that asks where a path really is (a config's `realpathSync`,
/// [`turborepo_trace_frameworks::fsx::realpath`]) gets another answer when a
/// directory or a file on the way comes to be a link. Where such a link leads
/// is the link's own record (a link that was crossed is a place seen).
pub(crate) fn presence(
    fs: &turborepo_trace_core::DirCache,
    p: &Path,
) -> Option<std::borrow::Cow<'static, str>> {
    use turborepo_trace_core::fs::FileKind;
    Some(match fs.stat_or_dangling(p) {
        Ok(s) => std::borrow::Cow::Borrowed(match (s.kind, s.is_symlink) {
            (FileKind::File, false) => "file",
            (FileKind::Dir, false) => "dir",
            (FileKind::Other, false) => "other",
            (FileKind::File, true) => "link to a file",
            (FileKind::Dir, true) => "link to a directory",
            (FileKind::Other, true) => "link to another",
        }),
        Err(true) => std::borrow::Cow::Borrowed("link to nothing"),
        Err(false) => return None,
    })
}

/// The parent of a package-relative (or absolute) directory: `a/b` → `a`, `a` →
/// ``, `` → `..`, `../x` → `..`, `..` → `../..`, `/a` → `/`, `/` → stop
/// (`"\0"`).
fn parent_rel(d: &str) -> String {
    if d == "/" {
        return "\0".to_owned();
    }
    if d.is_empty() {
        return "..".to_owned();
    }
    let last = d.rsplit('/').next().unwrap_or(d);
    if last == ".." {
        return format!("{d}/..");
    }
    match d.rfind('/') {
        Some(0) => "/".to_owned(),
        Some(i) => d[..i].to_owned(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use turborepo_trace_core::PackageBoundary;

    use super::*;

    fn cx<'a>(ids: &'a Identities) -> NormCx<'a> {
        static FS: std::sync::OnceLock<turborepo_trace_core::DirCache> = std::sync::OnceLock::new();
        NormCx {
            repo: Path::new("/r"),
            pkg_root: Path::new("/r/p"),
            follow: false,
            ids,
            fs: FS.get_or_init(turborepo_trace_core::DirCache::new),
            env_roots: &[],
        }
    }

    #[test]
    fn walkup_and_stems() {
        let ids = Identities::new(vec![PackageBoundary::NODE_MODULES], None);
        let mut obs = Observations::default();
        for p in [
            "/r/p/src/node_modules",
            "/r/p/node_modules/react",
            "/r/p/node_modules/react.js",
            "/r/p/node_modules/react.ts",
            "/r/node_modules/react",
            "/node_modules",
            "/r/p/src/b",
            "/r/p/src/b.ts",
            "/r/p/src/b.tsx",
            "/r/p/src/c.json",
            "/r/node_modules/.pnpm/x@1/node_modules/y/index.js",
        ] {
            obs.missing.push(PathBuf::from(p));
        }
        obs.packages.push("react".into());
        let mut t = Tables::default();
        let g = t.add(&cx(&ids), &obs);
        assert_eq!(g.probes.len(), t.probes.len());
        let walk: Vec<_> = t
            .probes
            .iter()
            .filter(|p| matches!(p, ProbeRec::WalkUp(_)))
            .collect();
        // `/r/p/src`, `/r/p` and `/r` are consecutive ancestors: one record. `/` is
        // outside the repository and dropped.
        assert_eq!(
            walk,
            [&ProbeRec::WalkUp(WalkUpRec {
                dir: "src".into(),
                levels: 3,
                marker: "node_modules".into(),
                name: "react".into(),
                suffixes: vec![".js".into(), ".ts".into()],
            })]
        );
        assert!(t.probes.contains(&ProbeRec::Stem(StemRec {
            path: "src/b".into(),
            suffixes: vec!["".into(), ".ts".into(), ".tsx".into()],
        })));
        assert!(t.probes.contains(&ProbeRec::Path("src/c.json".into())));
        // Inside an installed package: dropped.
        assert_eq!(t.probes.len(), 3);

        // Expansion covers every collapsed path.
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&t.probes).unwrap();
        let arch =
            rkyv::access::<rkyv::Archived<Vec<ProbeRec>>, rkyv::rancor::Error>(&bytes).unwrap();
        let mut all = Vec::new();
        for p in arch.iter() {
            expand_probe(p, &mut all);
        }
        for want in [
            "src/node_modules/react",
            "node_modules/react.js",
            "../node_modules/react",
            "../node_modules/react.ts",
            "src/b.tsx",
        ] {
            assert!(all.iter().any(|a| a == want), "{want} in {all:?}");
        }
        // What the walk-up was conditional on: the marker directory of each level.
        // Other records have no guards.
        let mut guards = Vec::new();
        for p in arch.iter() {
            expand_guards(p, &mut guards);
        }
        assert_eq!(
            guards,
            ["src/node_modules", "node_modules", "../node_modules"]
        );
        // Where the walk went on: one level above the last (here outside the
        // repository, which the key leaves out).
        let mut above = Vec::new();
        for p in arch.iter() {
            expand_above(p, &mut above);
        }
        assert_eq!(above, ["../../node_modules/react"]);
    }

    #[test]
    fn the_level_above_a_walk_up() {
        let above = |dir: &str, levels: u32| {
            let rec = vec![ProbeRec::WalkUp(WalkUpRec {
                dir: dir.into(),
                levels,
                marker: "node_modules".into(),
                name: "@s/n".into(),
                suffixes: Vec::new(),
            })];
            let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&rec).unwrap();
            let arch =
                rkyv::access::<rkyv::Archived<Vec<ProbeRec>>, rkyv::rancor::Error>(&bytes).unwrap();
            let mut out = Vec::new();
            expand_above(&arch[0], &mut out);
            out
        };
        assert_eq!(above("src/a", 1), ["src/node_modules/@s/n"]);
        assert_eq!(above("src/a", 2), ["node_modules/@s/n"]);
        assert_eq!(above("src/a", 3), ["../node_modules/@s/n"]);
        assert_eq!(above("", 1), ["../node_modules/@s/n"]);
        assert_eq!(above("/x/y", 1), ["/x/node_modules/@s/n"]);
        assert_eq!(above("/x/y", 2), ["/node_modules/@s/n"]);
        // The record ends at the filesystem root: no level above.
        assert!(above("/x/y", 3).is_empty());
    }

    #[test]
    fn a_scoped_walk_up_is_guarded_by_its_scope_directories() {
        let rec = vec![ProbeRec::WalkUp(WalkUpRec {
            dir: "src/matchers".into(),
            levels: 4,
            marker: "node_modules".into(),
            name: "@types/absent".into(),
            suffixes: Vec::new(),
        })];
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&rec).unwrap();
        let arch =
            rkyv::access::<rkyv::Archived<Vec<ProbeRec>>, rkyv::rancor::Error>(&bytes).unwrap();
        let mut guards = Vec::new();
        expand_guards(&arch[0], &mut guards);
        assert_eq!(
            guards,
            [
                "src/matchers/node_modules/@types",
                "src/matchers/node_modules",
                "src/node_modules/@types",
                "src/node_modules",
                "node_modules/@types",
                "node_modules",
                "../node_modules/@types",
                "../node_modules",
            ]
        );
        // The probes themselves are unchanged by the guards.
        let mut probes = Vec::new();
        expand_probe(&arch[0], &mut probes);
        assert_eq!(probes.len(), 4);
        assert!(probes.iter().all(|p| p.ends_with("/@types/absent")));
    }

    #[test]
    fn memberships_and_configs() {
        let ids = Identities::new(vec![PackageBoundary::NODE_MODULES], None);
        let mut obs = Observations::default();
        obs.listings.push(("/r/p/a".into(), "**".into()));
        obs.listings.push(("/r/p/a/b".into(), "**".into()));
        obs.listings.push(("/r/p/a".into(), "*.json".into()));
        obs.listings.push(("/r/p/c".into(), "*.json".into()));
        obs.reads.push("/r/p/package.json".into());
        obs.reads.push("/r/node_modules/x/package.json".into());
        obs.reads.push("/r/tsconfig.json".into());
        let mut t = Tables::default();
        t.add(&cx(&ids), &obs);
        assert_eq!(
            t.memberships,
            [
                MembershipRec {
                    dir: "a".into(),
                    pattern: "**".into()
                },
                MembershipRec {
                    dir: "c".into(),
                    pattern: "*.json".into()
                }
            ]
        );
        // Package-relative, interned in the order of their values.
        assert_eq!(t.configs, ["../tsconfig.json", "package.json"]);
    }

    #[test]
    fn parents_and_stems() {
        assert_eq!(parent_rel("a/b"), "a");
        assert_eq!(parent_rel("a"), "");
        assert_eq!(parent_rel(""), "..");
        assert_eq!(parent_rel(".."), "../..");
        assert_eq!(parent_rel("/a"), "/");
        assert_eq!(stem_split("src/a.test.ts"), ("src/a", ".test.ts"));
        assert_eq!(stem_split("src/.env.x"), ("src/.env", ".x"));
        assert_eq!(stem_split("src/a"), ("src/a", ""));
        assert_eq!(stem_split("src/é.js"), ("src/é", ".js"));
        assert_eq!(stem_split("é"), ("é", ""));
        assert_eq!(stem_split("src/"), ("src/", ""));
    }
}
