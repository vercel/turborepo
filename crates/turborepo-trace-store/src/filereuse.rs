//! Per-file reuse inside a re-keyed package.
//!
//! A shard's key covers every input of its package, so one changed input
//! re-keys the whole package: an edited file, a manifest a few of its files
//! read, a dependency closure only its tasks use. Tracing every file of the
//! package again for that repeats work whose result cannot differ: a file's
//! records are a function of its own contents and of the state of what **its
//! own** references observed.
//!
//! So a shard stores the state of each of its observations at trace time, per
//! table entry ([`crate::shard::StatesRec`], computed by [`states`], the
//! function a refresh compares them with), and for each file what only that
//! file depends on: the files its edges reached, the package links they went
//! through, and its key-only configs ([`crate::shard::FileRec`]). When a
//! package's key no longer matches, [`plan`] finds the observations whose state
//! changed and keeps the records of every file that read none of
//! them; the build analyzes the other files and assembles the new shard from
//! both ([`pending`]). Task pseudo-files are always computed again: closures,
//! hubs, task listings and task configs reach nothing else.
//!
//! # Why a kept file's records are what a fresh trace would write
//!
//! A source file's records come from three steps, each a function of the file
//! and of observations that end up in the file's own groups:
//!
//! 1. [`turborepo_trace_core::Tracer::analyze`] resolves each file
//!    independently; every lookup is recorded in the reference's group (missing
//!    paths, listings, reads), and a memoized lookup replays its record for
//!    every caller.
//! 2. The framework pass ([`crate::fw::Fw::augment`]) adds a group with what it
//!    observed for the file, and its key-only configs. Project state it shares
//!    between files (a setup file's mocks, the config's helpers) is recorded on
//!    the tests it was computed for, not on every file it can affect, so
//!    [`plan`] treats it as shared: when a listing or a config that a framework
//!    group read has changed (other than the reading file's own source), or a
//!    key-only config has, nothing is kept and the package is traced as before.
//!    (A path that a framework group looked for is that file's own record: the
//!    lookups that decide for other files are key observations of each of
//!    them.) So is it when the set of framework config files that can govern
//!    the package changes ([`crate::fw::ConfigPaths::digest_for`]). A config is
//!    consulted for every file below it, not only for tests: whether the file
//!    is a test, whether a project contains it, what a module mapper makes of
//!    its imports. The contents of the config files are key-only configs of
//!    each such file, and everything else the load accessed is a key
//!    observation of each ([`crate::fw`], `Loaded::consulted`): every path it
//!    asked about, whatever the answer; every listing it made (the directories
//!    a `projects` glob matches, the haste map's mocks). So is what the walk
//!    that *finds* the file's config accessed (the manifests on the way, each
//!    asked for a `jest` key). The records are complete by construction: the
//!    adapters reach the file system through one module, which logs what they
//!    ask (`turborepo_trace_frameworks::fsx::with_log`). A package without a
//!    test has them all.
//! 3. Target classification and normalization ([`crate::build::assemble`])
//!    depend on the reached files (present, and owned as before:
//!    [`crate::build::found_digest`]), the package links (the identity found
//!    there, or what the lockfile resolves to, including "nothing"), and for a
//!    target inside the shard on its still being a file of the shard: all
//!    checked per file.
//!
//! What a step computed a file's records from without the records naming it is
//! on the file as well: a **key observation**
//! ([`turborepo_trace_core::record::key`]), one of the sets the shard stores
//! once and the file names ([`crate::shard::Shard::key_groups`],
//! [`FileRec::key_groups`]). The projects a `tsconfig.json` above the file
//! references are loaded for it (one that does not load is reported on the
//! file, one that includes the file governs it), and a project that loads and
//! does not include the file is no input of it. [`plan`] reads a file's sets
//! like its groups, and the composed graph never does.
//!
//! # What was there and is gone
//!
//! A trace records what it looked for and did not find, what it listed and what
//! it read. What it **found** it records by the edge the lookup ends in. A
//! lookup that found something and made no edge of it leaves nothing on the
//! file: the crate root a Rust module file belongs to, the manual mock next to
//! a module, a directory a second resolution entered. The section below lists
//! the ones that leave a record by now; the plan does not depend on that list
//! being complete. Such a lookup can only come out differently when something
//! that was there is gone (what appears is covered by the paths
//! each file looked for), so when the shard knows of something that is gone,
//! nothing is kept and the package is traced as before the early cutoff: a file
//! of the package, a file one of its files reached, the place of a package link
//! (also one that leads elsewhere now), a config ([`gone`]). The common changes
//! keep their files: edits, additions, a dependency of another version.
//!
//! # A lookup that found something leaves a record of it
//!
//! The rule above needs the shard to know of the thing that went. When the file
//! that looked is the only one of its package that had to do with it, nothing
//! of the shard moves, the package is not even re-keyed, and its shard is one a
//! fresh build does not write. So these lookups leave a record, which enters
//! the shard's key (the randomized test found each of them):
//!
//! - a resolution of the framework pass whose result is named, not linked (the
//!   module a `jest.mock` replaces, in a `mock:` marker), and the manual mock a
//!   file other than a test registers (found next to its module): the framework
//!   pass reports what it found ([`crate::fw::FileFw::reached`]), and the file
//!   reaches it without an edge ([`crate::build::TargetCx::reach`]);
//! - the symbolic links crossed on the way to a first-party file (a workspace
//!   package's `node_modules/<name>`): the file the link led to is still there
//!   when the link goes;
//! - a lookup that failed inside a package directory: its probes are recorded
//!   where the directory leads (a workspace package) or dropped (an installed
//!   one), and nothing says through which directory
//!   ([`crate::normalize::Normalized::seen`]);
//! - a walk up the `node_modules` directories that stopped where it found the
//!   name: the record holds the levels that missed, and whether the name is
//!   there one level above them enters the key like a guard
//!   ([`crate::normalize::expand_above`]);
//! - a lookup that went into a directory and found nothing there: the paths it
//!   looked for are recorded, the directory is not. A trace before the
//!   directory was there recorded the directory as missing, and no probe of
//!   either shard exists: the two had one key. Whether the directory of each
//!   probed path is there enters the key ([`crate::normalize::probe_dirs`]) and
//!   the state of each of those probes ([`crate::normalize::DIR_ABSENT`]): a
//!   lookup that asks whether a path is a directory before it looks inside (the
//!   loader of a tsconfig, for the target of an `extends` and for a project
//!   reference) asks other questions once the directory is gone, so the file
//!   that looked is traced again.
//!
//! The second and third are links without identity whose state is where the
//! place leads now ([`crate::build::SEEN_LINK`]). A place that leads into the
//! tree or nowhere is part of the shard's tree part, covered by the directory
//! of every link crossed, so a refresh of an unchanged tree does not resolve it
//! again; one that leads into an installed package is checked with the package
//! links.
//!
//! The observation tables of the new shard are interned in file order, each
//! group's records in the order of their values
//! ([`crate::normalize::Tables::intern`]), so a kept group numbers them exactly
//! as the group traced again would: the refreshed shard is the cold
//! build's shard byte for byte. The refresh-equals-cold property tests of this
//! crate run every re-keyed package through this path.
//!
//! `TURBO_TRACE_DEBUG_NO_FILE_REUSE=1` turns it off (every file of a re-keyed
//! package is traced again, as before): the graph must be the same either way.

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use rayon::prelude::*;
use xxhash_rust::xxh3::Xxh3;

use crate::{
    build::{
        Cx, LOCK_LINK, ObsAcc, PendingFile, PendingKey, PendingRef, SEEN_LINK, link_digest,
        lock_link_digest, seen_digest,
    },
    hash::{join, split},
    normalize::{ABOVE, DIR_ABSENT, Normalized, PRESENCE, nearest_dir_on, probe_dir},
    paths::abs,
    shard::{
        ArchivedFileRec, ArchivedShard, ArchivedTargetRec, FRAMEWORK_LEVEL, FileRec, LinkRec,
        MembershipRec, ProbeRec, ProbeStateRec, StatesRec, TargetRec, UncRec,
    },
    workspace::PackageDef,
};

/// The current state of every observation of a shard, per table entry.
pub(crate) struct States {
    probes: Vec<u64>,
    memberships: Vec<u64>,
    configs: Vec<u64>,
    found: Vec<u64>,
    links: Vec<u64>,
    fw: u128,
}

impl States {
    /// As stored in a shard.
    pub(crate) fn rec(self) -> StatesRec {
        StatesRec {
            probes: self
                .probes
                .iter()
                .enumerate()
                .filter(|(_, s)| **s != 0)
                .map(|(i, s)| ProbeStateRec {
                    probe: i as u32,
                    state: *s,
                })
                .collect(),
            memberships: self.memberships,
            configs: self.configs,
            found: self.found,
            links: self.links,
            fw: split(self.fw),
        }
    }
}

fn digest(parts: &[&str]) -> u64 {
    let mut h = Xxh3::new();
    for p in parts {
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    h.digest()
}

/// The state of everything `s` observed, in the current tree: per probe which
/// of its expanded paths and guard directories exist and as what, and whether
/// the directory of its paths does (0 when none of the paths exists and the
/// directory does: [`crate::normalize::DIR_ABSENT`]), per membership its
/// listing, per config its contents, per reached file its presence and owner,
/// per link the identity found there. The shard key hashes the same facts
/// ([`crate::build::obs_digest_keyed`]); here they are kept apart, so that two
/// states can be compared entry by entry.
pub(crate) fn states(cx: &Cx, s: &ArchivedShard) -> States {
    let fs = cx.tracer.fs();
    let root = cx.pkg_root_abs(s.package.root.as_str());
    let probes: Vec<u64> = s
        .probes
        .as_slice()
        .par_chunks(512)
        .flat_map_iter(|chunk| {
            let mut paths = Vec::new();
            // Neighbours in the table mostly share a directory.
            let mut last_dir: Option<(&str, Option<u64>)> = None;
            chunk
                .iter()
                .map(|p| {
                    let mut h: Option<Xxh3> = None;
                    for (tag, expand) in PRESENCE {
                        paths.clear();
                        expand(p, &mut paths);
                        for rel in &paths {
                            let a = abs(&root, rel);
                            if tag == ABOVE && !a.starts_with(&cx.repo) {
                                continue;
                            }
                            if let Some(kind) = crate::normalize::presence(fs, &a) {
                                let h = h.get_or_insert_with(Xxh3::new);
                                h.update(tag.as_bytes());
                                h.update(&(rel.len() as u64).to_le_bytes());
                                h.update(rel.as_bytes());
                                h.update(kind.as_bytes());
                            }
                        }
                    }
                    // The directory the paths lie in: when it is gone (or no directory),
                    // every answer the record holds is another one (`DIR_ABSENT`).
                    // (A path that is there has its directory: only a probe that found
                    // nothing can be one whose directory is gone.)
                    if h.is_none()
                        && let Some(dir) = probe_dir(p)
                    {
                        if !matches!(&last_dir, Some((d, _)) if *d == dir) {
                            // (Memoized per directory by the cache: one lookup.)
                            let there = fs.dir_exact(&abs(&root, dir));
                            let nearest = (!there).then(|| nearest_dir_on(fs, &root, dir));
                            // The state of every probe of the run: computed once.
                            let absent = nearest.map(|n| digest(&[DIR_ABSENT, n.as_str()]).max(1));
                            last_dir = Some((dir, absent));
                        }
                        if let Some((_, Some(absent))) = last_dir {
                            return absent;
                        }
                    }
                    h.map_or(0, |h| h.digest().max(1))
                })
                .collect::<Vec<u64>>()
        })
        .collect();
    let memberships: Vec<u64> = s
        .memberships
        .as_slice()
        .par_iter()
        .enumerate()
        .map(|(i, m)| {
            // A listing only hubs reference: no file's records depend on it.
            if s.hub_members.binary_search(&(i as u32).into()).is_ok() {
                return 0;
            }
            cx.membership_digest(&abs(&root, m.dir.as_str()), m.pattern.as_str())
                .0 as u64
        })
        .collect();
    let path_only = |i: usize| s.path_configs.binary_search(&(i as u32).into()).is_ok();
    let view = |c: &str| crate::normalize::view_config(c).map(|p| abs(&root, p));
    let content: Vec<PathBuf> = s
        .configs
        .iter()
        .enumerate()
        .filter(|(i, c)| !path_only(*i) && view(c.as_str()).is_none())
        .map(|(_, c)| abs(&root, c.as_str()))
        .collect();
    let mut hashes = cx.hashes.hash_many(&content).into_iter();
    let configs: Vec<u64> = s
        .configs
        .iter()
        .enumerate()
        .map(|(i, c)| {
            // A manifest read through its resolution view: the view's bytes.
            if let Some(p) = view(c.as_str()) {
                return match crate::build::view_digest(cx, c.as_str(), &p) {
                    Some(x) => (x as u64) ^ ((x >> 64) as u64),
                    None => digest(&["missing"]),
                };
            }
            if path_only(i) {
                let p = abs(&root, c.as_str());
                let is_file = match cx.is_file_memo.get(&p) {
                    Some(v) => *v,
                    None => {
                        let v = fs.is_file(&p);
                        cx.is_file_memo.insert(p, v);
                        v
                    }
                };
                return digest(&["path", if is_file { "file" } else { "missing" }]);
            }
            match hashes.next().flatten() {
                Some(x) => (x as u64) ^ ((x >> 64) as u64),
                None => digest(&["missing"]),
            }
        })
        .collect();
    let found: Vec<u64> = s
        .obs
        .found
        .as_slice()
        .par_iter()
        .map(|f| {
            let d = crate::build::found_digest(cx, &abs(&root, f.as_str()));
            (d as u64) ^ ((d >> 64) as u64)
        })
        .collect();
    let links: Vec<u64> = s
        .obs
        .links
        .iter()
        .map(|l| {
            let d = if let Some(rest) = l.location.strip_prefix(LOCK_LINK) {
                lock_link_digest(cx, rest)
            } else if let Some(rest) = l.location.strip_prefix(SEEN_LINK) {
                seen_digest(cx, &abs(&root, rest))
            } else {
                link_digest(cx, &abs(&root, l.location.as_str()))
            };
            (d as u64) ^ ((d >> 64) as u64)
        })
        .collect();
    States {
        probes,
        memberships,
        configs,
        found,
        links,
        fw: {
            // The framework config paths, and the place of the checkout for a shard whose
            // files it decides ([`crate::fw::place_state`]).
            let d = cx.fw_configs.digest_for(s.package.root.as_str());
            match crate::fw::place_state(cx, s) {
                Some(below) => {
                    let mut h = Xxh3::new();
                    h.update(crate::fw::place_word(below).as_bytes());
                    h.update(&d.to_le_bytes());
                    h.digest128()
                }
                None => d,
            }
        },
    }
}

/// Which entries of a re-keyed package keep their records.
pub(crate) struct Plan {
    /// Per entry of the package (in entry order): the index of its file in the
    /// previous shard when its records are kept.
    pub keep: Vec<Option<u32>>,
    /// How many are kept.
    pub kept: usize,
}

fn disabled() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| turborepo_trace_core::debug::var_os("NO_FILE_REUSE").is_some())
}

fn debug() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| turborepo_trace_core::debug::var_os("DEBUG_REUSE").is_some())
}

/// Whether `f` is a task, hub or task-part pseudo-file.
fn pseudo(f: &ArchivedFileRec) -> bool {
    crate::tasks::is_pseudo_lang(f.lang.as_ref().map(|l| l.as_str()))
}

/// What the previous shard `old` knew to be there and is gone now, if anything:
/// a file of the package (`entry`: the package's files now), a file one of its
/// files reached, the place of a package link (also one that leads elsewhere
/// now), a config. `found`, `links` and `configs`: which table entries changed
/// their state.
fn gone(
    cx: &Cx,
    old: &ArchivedShard,
    entry: &HashMap<&str, usize>,
    found: &[bool],
    links: &[bool],
    configs: &[bool],
) -> Option<String> {
    let fs = cx.tracer.fs();
    let root = cx.pkg_root_abs(old.package.root.as_str());
    if let Some(f) = old
        .files
        .iter()
        .find(|f| !pseudo(f) && !entry.contains_key(f.path.as_str()))
    {
        return Some(format!("the package's file {}", f.path.as_str()));
    }
    if let Some(f) = old
        .obs
        .found
        .iter()
        .zip(found)
        .find(|(f, changed)| **changed && !fs.is_file(&abs(&root, f.as_str())))
    {
        return Some(format!("the reached file {}", f.0.as_str()));
    }
    let link_gone = |l: &crate::shard::ArchivedLinkRec| {
        let location = l.location.as_str();
        if let Some(rest) = location.strip_prefix(LOCK_LINK) {
            let (importer, name) = rest.split_once(':').unwrap_or((rest, ""));
            cx.ids.lock_resolve(importer, name).is_none()
        } else if let Some(rest) = location.strip_prefix(SEEN_LINK) {
            // A place that led into an installed package and still leads into one is a
            // package link by another name (a dependency of another version). Any other
            // place has no state but where it leads: another one is another place, or none.
            l.name.is_empty()
                || crate::build::seen_state(cx, &abs(&root, rest))
                    .package
                    .is_none()
        } else {
            !fs.is_dir(&abs(&root, location))
        }
    };
    if let Some(l) = old
        .obs
        .links
        .iter()
        .zip(links)
        .find(|(l, changed)| **changed && link_gone(l))
    {
        return Some(format!("the place of the link {}", l.0.location.as_str()));
    }
    if let Some(c) = old.configs.iter().zip(configs).find(|(c, changed)| {
        **changed && !fs.is_file(&abs(&root, crate::normalize::config_path(c.as_str())))
    }) {
        return Some(format!("the config {}", c.0.as_str()));
    }
    None
}

/// The entries of the package `def` (`rels`: their paths relative to the
/// package root, in entry order; `hashes`: their content hashes) whose records
/// in the previous shard `old` are what a trace in the current tree would write
/// (see the module docs). `None` when nothing can be kept.
pub(crate) fn plan(
    cx: &Cx,
    def: &PackageDef,
    rels: &[String],
    hashes: Option<&[Option<u128>]>,
    old: &ArchivedShard,
) -> Option<Plan> {
    if disabled() || old.package.name.as_str() != def.name {
        return None;
    }
    let hashes = hashes?;
    let then = &old.obs.states;
    let now = states(cx, old);
    // `TURBO_TRACE_DEBUG_DEBUG_REUSE`: why a package keeps what it keeps, on
    // stderr.
    let why = |what: &str| {
        if debug() {
            eprintln!("reuse {:?}: nothing kept: {what}", def.root);
        }
    };
    // A shard without states (none were computed for it), or other framework
    // configs for the package: nothing is kept.
    if then.memberships.len() != now.memberships.len()
        || then.configs.len() != now.configs.len()
        || then.found.len() != now.found.len()
        || then.links.len() != now.links.len()
    {
        why("the previous shard has no observation states");
        return None;
    }
    if join([then.fw[0].to_native(), then.fw[1].to_native()]) != now.fw {
        why(
            "the framework config files that can govern the package changed, or the checkout \
             moved across a node_modules directory",
        );
        return None;
    }
    // The place of the checkout decides other tests than it did (or decides now, or
    // no longer): which files are tests is an answer for every file of a
    // project.
    if old
        .located
        .iter()
        .any(|l| *cx.places.state(l.spec.as_str()) != *l.root.as_str())
    {
        why("the place of the checkout decides which files are tests");
        return None;
    }
    let mut probe_then = vec![0u64; now.probes.len()];
    for e in then.probes.iter() {
        *probe_then.get_mut(e.probe.to_native() as usize)? = e.state.to_native();
    }
    let differs = |a: &[u64], b: &[rkyv::Archived<u64>]| -> Vec<bool> {
        a.iter().zip(b).map(|(x, y)| *x != y.to_native()).collect()
    };
    let probes: Vec<bool> = now
        .probes
        .iter()
        .zip(&probe_then)
        .map(|(x, y)| x != y)
        .collect();
    let memberships = differs(&now.memberships, &then.memberships);
    let configs = differs(&now.configs, &then.configs);
    let found = differs(&now.found, &then.found);
    let links = differs(&now.links, &then.links);
    let any = |ix: &rkyv::vec::ArchivedVec<rkyv::Archived<u32>>, changed: &[bool]| {
        ix.iter().any(|i| changed[i.to_native() as usize])
    };
    let path_only = |i: u32| old.path_configs.binary_search(&i.into()).is_ok();
    // Per set of key observations: whether one of its entries changed its state.
    let key_changed: Vec<bool> = old
        .key_groups
        .iter()
        .map(|g| {
            any(&g.probes, &probes)
                || any(&g.memberships, &memberships)
                || any(&g.configs, &configs)
                || any(&g.links, &links)
        })
        .collect();
    let entry: HashMap<&str, usize> = rels
        .iter()
        .enumerate()
        .map(|(i, r)| (r.as_str(), i))
        .collect();

    // Something the shard knew to be there is gone (see the module docs): nothing
    // is kept.
    if let Some(what) = gone(cx, old, &entry, &found, &links, &configs) {
        why(&format!("{what}: what was there is gone"));
        return None;
    }

    // Shared framework state: something a framework group read has changed and it
    // is not only the reading file's own source, or a key-only config has. It
    // can decide the framework inputs of any file of the package, also of one
    // that has none so far (a project root that now contains it): nothing is
    // kept.
    let shared = old.files.as_slice().par_iter().find_map_any(|f| {
        if pseudo(f) {
            return None;
        }
        let config = |c: &rkyv::Archived<u32>| old.configs[c.to_native() as usize].as_str();
        if let Some(c) = f
            .key_configs
            .iter()
            .find(|c| configs[c.to_native() as usize])
        {
            return Some(format!(
                "key-only config {} of {}",
                config(c),
                f.path.as_str()
            ));
        }
        for r in f.refs.iter() {
            if r.index.to_native() != FRAMEWORK_LEVEL {
                continue;
            }
            // (A path that a framework group looked for is the file's own: the walk for a
            // config, which decides for other files too, is a key observation of each.)
            if let Some(m) = r
                .memberships
                .iter()
                .find(|m| memberships[m.to_native() as usize])
            {
                let m = &old.memberships[m.to_native() as usize];
                return Some(format!(
                    "listing {} :: {} of the framework group of {}",
                    m.dir.as_str(),
                    m.pattern.as_str(),
                    f.path.as_str()
                ));
            }
            if let Some(c) = r
                .configs
                .iter()
                .find(|c| configs[c.to_native() as usize] && config(c) != f.path.as_str())
            {
                return Some(format!(
                    "config {} of the framework group of {}",
                    config(c),
                    f.path.as_str()
                ));
            }
        }
        None
    });
    if let Some(what) = shared {
        why(&format!("shared framework state changed ({what})"));
        return None;
    }

    // The entry whose records `f` keeps, or why it is traced again.
    let keep_one = |f: &ArchivedFileRec| -> Result<usize, &'static str> {
        if pseudo(f) {
            return Err("pseudo-file");
        }
        if f.extraction.is_none() {
            return Err("no extraction");
        }
        let ei = *entry.get(f.path.as_str()).ok_or("no longer an entry")?;
        let same = f
            .hash
            .as_ref()
            .is_some_and(|h| hashes[ei] == Some(join([h[0].to_native(), h[1].to_native()])));
        if !same {
            return Err("contents changed");
        }
        if any(&f.found, &found) {
            return Err("a file it reached");
        }
        if any(&f.links, &links) {
            return Err("a package link");
        }
        if any(&f.key_groups, &key_changed) {
            return Err("a key observation");
        }
        for r in f.refs.iter() {
            if any(&r.probes, &probes) {
                return Err("a path it looked for");
            }
            if any(&r.memberships, &memberships) {
                return Err("a listing");
            }
            if any(&r.configs, &configs) {
                return Err("a config it read");
            }
            // A source file reads configs by content; anything else is traced again.
            if r.configs.iter().any(|c| path_only(c.to_native())) {
                return Err("a path-only config");
            }
            for e in r.edges.iter() {
                if let ArchivedTargetRec::File(t) = &e.target {
                    // Still a file of the shard: an entry of the package now.
                    let still = old
                        .files
                        .get(t.to_native() as usize)
                        .is_some_and(|t| !pseudo(t) && entry.contains_key(t.path.as_str()));
                    if !still {
                        return Err("a file of the package it reached");
                    }
                }
            }
        }
        Ok(ei)
    };
    let verdicts: Vec<Result<usize, &'static str>> =
        old.files.as_slice().par_iter().map(keep_one).collect();
    let kept: Vec<(usize, u32)> = verdicts
        .iter()
        .enumerate()
        .filter_map(|(fi, v)| v.ok().map(|ei| (ei, fi as u32)))
        .collect();
    if debug() {
        let mut reasons: std::collections::BTreeMap<&str, usize> = Default::default();
        for v in &verdicts {
            if let Err(r) = v {
                *reasons.entry(r).or_default() += 1;
            }
        }
        let count = |c: &[bool]| c.iter().filter(|x| **x).count();
        eprintln!(
            "reuse {:?}: {} of {} entries kept; changed: {} probes, {} listings, {} configs, {} \
             reached files, {} links; traced again: {:?}",
            def.root,
            kept.len(),
            rels.len(),
            count(&probes),
            count(&memberships),
            count(&configs),
            count(&found),
            count(&links),
            reasons
        );
    }
    if kept.is_empty() {
        return None;
    }
    let mut keep = vec![None; rels.len()];
    for (ei, fi) in &kept {
        keep[*ei] = Some(*fi);
    }
    Some(Plan {
        keep,
        kept: kept.len(),
    })
}

/// The records of file `fi` of the previous shard `old`, ready to be interned
/// into the new shard: its groups with their observations by value, its edges
/// with targets inside the shard renumbered (`file_ix`: the new shard's files
/// by path), what it reached.
// Invariant: the archive was validated when the shard was opened.
#[allow(clippy::expect_used)]
pub(crate) fn pending(old: &ArchivedShard, fi: u32, file_ix: &HashMap<&str, u32>) -> PendingFile {
    let mut rec: FileRec =
        rkyv::deserialize::<FileRec, rkyv::rancor::Error>(&old.files[fi as usize])
            .expect("validated shard");
    let own = |a: &crate::shard::ArchivedProbeRec| -> ProbeRec {
        rkyv::deserialize::<ProbeRec, rkyv::rancor::Error>(a).expect("validated shard")
    };
    let refs = std::mem::take(&mut rec.refs)
        .into_iter()
        .map(|mut r| {
            for e in &mut r.edges {
                if let TargetRec::File(t) = &mut e.target {
                    *t = file_ix[old.files[*t as usize].path.as_str()];
                }
            }
            let norm = Normalized::from_records(
                r.probes
                    .iter()
                    .map(|&i| own(&old.probes[i as usize]))
                    .collect(),
                r.memberships
                    .iter()
                    .map(|&i| {
                        let m = &old.memberships[i as usize];
                        MembershipRec {
                            dir: m.dir.to_string(),
                            pattern: m.pattern.to_string(),
                        }
                    })
                    .collect(),
                r.configs
                    .iter()
                    .map(|&i| old.configs[i as usize].to_string())
                    .collect(),
            );
            r.probes = Vec::new();
            r.memberships = Vec::new();
            r.configs = Vec::new();
            PendingRef {
                rec: r,
                norm,
                pins: Vec::new(),
            }
        })
        .collect();
    let uncs = std::mem::take(&mut rec.uncertainties)
        .into_iter()
        .map(|u| {
            (
                UncRec {
                    kind: u.kind,
                    line: u.line,
                    span: u.span,
                    detail: old.details[u.detail as usize].to_string(),
                    origin: u.origin,
                },
                u.scope,
                u.effect,
            )
        })
        .collect();
    let found: BTreeSet<String> = rec
        .found
        .iter()
        .map(|&i| old.obs.found[i as usize].to_string())
        .collect();
    let links: BTreeSet<LinkRec> = rec
        .links
        .iter()
        .map(|&i| {
            rkyv::deserialize::<LinkRec, rkyv::rancor::Error>(&old.obs.links[i as usize])
                .expect("validated shard")
        })
        .collect();
    // Its sets of key observations, by their index in `old`: the new shard takes
    // each from there once ([`key_set`]).
    let keys = std::mem::take(&mut rec.key_groups)
        .into_iter()
        .map(PendingKey::Kept)
        .collect();
    rec.key_configs = Vec::new();
    rec.found = Vec::new();
    rec.links = Vec::new();
    PendingFile {
        rec,
        refs,
        keys,
        uncs,
        acc: ObsAcc {
            found,
            links,
            ..Default::default()
        },
    }
}

/// The set `i` of key observations of the previous shard `old`, by value: its
/// records and the places they were located through, ready to be interned into
/// the new shard.
// Invariant: the archive was validated when the shard was opened.
#[allow(clippy::expect_used)]
pub(crate) fn key_set(old: &ArchivedShard, i: u32) -> crate::build::KeyRecords {
    let g = &old.key_groups[i as usize];
    let norm = Normalized::from_records(
        g.probes
            .iter()
            .map(|i| {
                rkyv::deserialize::<ProbeRec, rkyv::rancor::Error>(
                    &old.probes[i.to_native() as usize],
                )
                .expect("validated shard")
            })
            .collect(),
        g.memberships
            .iter()
            .map(|i| {
                let m = &old.memberships[i.to_native() as usize];
                MembershipRec {
                    dir: m.dir.to_string(),
                    pattern: m.pattern.to_string(),
                }
            })
            .collect(),
        g.configs
            .iter()
            .map(|i| old.configs[i.to_native() as usize].to_string())
            .collect(),
    );
    let links = g
        .links
        .iter()
        .map(|i| {
            rkyv::deserialize::<LinkRec, rkyv::rancor::Error>(
                &old.obs.links[i.to_native() as usize],
            )
            .expect("validated shard")
        })
        .collect();
    (norm, links, g.framework)
}

/// What the framework pass recorded for the kept file `fi` of `old` besides its
/// group: the frameworks that run it as a test with their working directories
/// (absolute), its key-only configs (absolute), and nothing more to reach (what
/// its resolutions found is among the file's kept reached files and links).
/// `rel`: the file's path relative to the package root.
#[allow(clippy::type_complexity)]
pub(crate) fn kept_framework(
    cx: &Cx,
    old: &ArchivedShard,
    fi: u32,
    root_abs: &Path,
    rel: String,
) -> (
    Vec<(String, &'static str, Vec<PathBuf>)>,
    Vec<PathBuf>,
    Vec<crate::fw::Reached>,
) {
    let tests = old.tests.as_slice();
    let lo = tests.partition_point(|t| t.file.to_native() < fi);
    let marks = tests[lo..]
        .iter()
        .take_while(|t| t.file.to_native() == fi)
        .filter_map(|t| {
            let (id, _) = crate::fw::GRAPH_FRAMEWORKS
                .iter()
                .find(|(id, _)| *id == t.framework.as_str())?;
            let cwds = t
                .cwds
                .iter()
                .map(|c| {
                    if c.is_empty() {
                        cx.repo.clone()
                    } else {
                        cx.repo.join(c.as_str())
                    }
                })
                .collect();
            Some((rel.clone(), *id, cwds))
        })
        .collect();
    let keys = old.files[fi as usize]
        .key_configs
        .iter()
        .map(|i| abs(root_abs, old.configs[i.to_native() as usize].as_str()))
        .collect();
    // What the framework pass found for the file is in its kept reached files and
    // links.
    (marks, keys, Vec::new())
}
