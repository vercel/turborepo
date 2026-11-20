//! Measurement tooling (not a product feature): [`Engine::named_affected`]
//! reports one name per entry; this reports, per name, every entry it selects,
//! so that the names of a selection can be ranked by the entries only they
//! hold.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::{EntryBase, NameIndex, NamedBy, ancestor, below};
use crate::{
    change::{AffectedOptions, Change, Disposition},
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId},
    vocab::UncertaintyKind,
};

/// One name (a string shape, a file location, the working directory, the test's
/// directory) and the entries it selects.
#[derive(Clone, Debug)]
pub struct NamedGroup {
    /// `shape`, `location`, `path`, `everything`, `cwd`, `testdir` or
    /// `cwdpath`.
    pub kind: &'static str,
    /// The shape, the location directory, the entry-relative directory, or for
    /// `cwdpath` the place as the cause words it (`the working directory
    /// (the repository root)`).
    pub label: String,
    /// Changed paths it names.
    pub paths: usize,
    /// One changed path it names.
    pub example: String,
    /// The files that hold the name.
    pub files: Vec<u32>,
    /// The entries selected through it.
    pub entries: Vec<u32>,
}

impl<G: GraphView> Engine<'_, G> {
    /// Per name that names a changed path: the entries
    /// [`Engine::named_affected`] would select through that name alone
    /// (under the package assumption, when it is on). Same arguments. The
    /// paths a file builds on the working directory are groups of kind
    /// `cwdpath`, one per place.
    #[allow(clippy::too_many_arguments)]
    pub fn named_groups(
        &self,
        index: &NameIndex,
        changes: &[Change],
        entries: &NodeSet,
        runtime_entries: Option<&NodeSet>,
        opts: &AffectedOptions,
        cwds: &dyn Fn(NodeId) -> Vec<String>,
        task_dirs: &dyn Fn(NodeId) -> Option<Vec<String>>,
    ) -> Vec<NamedGroup> {
        let g = self.graph();
        let mut out = Vec::new();
        let Some(p) = self.pairing(changes, entries, runtime_entries, opts, task_dirs) else {
            return out;
        };
        // The directories the runner of a process gives it (an entry, or the task that
        // runs the process for it).
        let dirs_of = |runner: NodeId| -> Vec<String> {
            match p.runners.get(&runner.0) {
                Some(r) => r.dirs.clone(),
                None => cwds(runner),
            }
        };

        // 1. Strings and file locations, one group per name.
        struct Acc<'c> {
            files: Vec<u32>,
            paths: HashSet<&'c str>,
            example: &'c str,
            /// Under the package assumption, for a string: the naming files by
            /// the package that holds the path they name, and the
            /// files whose string climbs (not held to a package).
            scoped: BTreeMap<&'c str, Vec<u32>>,
            free: Vec<u32>,
        }
        let mut groups: BTreeMap<(&'static str, String), Acc<'_>> = BTreeMap::new();
        for c in changes {
            let holders = p.packages.as_ref().map(|pk| pk.holders(&c.path));
            index.named(&c.path, &mut |r, by| {
                let key = match by {
                    NamedBy::Shape(s) => ("shape", s.to_owned()),
                    NamedBy::Location(d) => ("location", d.to_owned()),
                    NamedBy::Path(p) => ("path", p.to_owned()),
                    NamedBy::Everything => ("everything", String::new()),
                };
                let located = matches!(key.0, "location" | "path");
                let a = groups.entry(key).or_insert_with(|| Acc {
                    files: Vec::new(),
                    paths: HashSet::new(),
                    example: &c.path,
                    scoped: BTreeMap::new(),
                    free: Vec::new(),
                });
                a.files.push(r.node.0);
                a.paths.insert(&c.path);
                match &holders {
                    Some(_) if located => {}
                    Some(_) if r.climbs => a.free.push(r.node.0),
                    Some(holders) => {
                        for k in holders {
                            a.scoped.entry(*k).or_default().push(r.node.0);
                        }
                    }
                    None => {}
                }
            });
        }
        for ((kind, label), mut a) in groups {
            a.files.sort_unstable();
            a.files.dedup();
            let ancestors = |files: &[u32]| {
                self.code_ancestors(&p.code, files.iter().map(|&n| NodeId(n)), Some(index))
            };
            let mut hit: Vec<u32> = Vec::new();
            if matches!(kind, "location" | "path") || p.packages.is_none() {
                let to_name = ancestors(&a.files);
                hit.extend(p.reach(&to_name, &p.to_site, None).iter().map(|r| r.0.0));
            } else {
                if !a.free.is_empty() {
                    a.free.sort_unstable();
                    a.free.dedup();
                    let to_name = ancestors(&a.free);
                    hit.extend(p.reach(&to_name, &p.to_site, None).iter().map(|r| r.0.0));
                }
                for (pkg, files) in &mut a.scoped {
                    let sites = p.sites_in(&[*pkg]);
                    if !p.has_runners(pkg) && sites.is_empty() {
                        continue;
                    }
                    files.sort_unstable();
                    files.dedup();
                    let to_name = ancestors(files);
                    let mut tasks_hit = Vec::new();
                    let direct = p.reach_by_runner(
                        &to_name,
                        &p.to_site,
                        Some(&[*pkg]),
                        &|_| false,
                        &|_, _| true,
                        &mut |_, _, _| Some(()),
                        &mut tasks_hit,
                    );
                    hit.extend(direct.iter().map(|r| r.entry.0));
                    hit.extend(p.above(&tasks_hit, &|_| false).iter().map(|r| r.entry.0));
                    if !sites.is_empty() {
                        let to_site = self.code_ancestors(&p.code, sites.iter().copied(), None);
                        hit.extend(p.reach(&to_name, &to_site, None).iter().map(|r| r.0.0));
                    }
                }
            }
            hit.sort_unstable();
            hit.dedup();
            out.push(NamedGroup {
                kind,
                label,
                paths: a.paths.len(),
                example: a.example.to_owned(),
                files: a.files,
                entries: hit,
            });
        }

        // 2. Names that depend on the entry, one group per directory that matched.
        let mut cwd_users: HashMap<u32, u32> = HashMap::new();
        for r in &index.cwd_users {
            cwd_users.entry(r.node.0).or_insert(r.line);
        }
        let mut record_only: HashSet<u32> = HashSet::new();
        for &n in &p.sites {
            if let Some(u) = g.uncertainties(n).find(|u| {
                u.kind == UncertaintyKind::CwdDependent
                    && self.disposition_of(n, u, opts) == Disposition::Named
            }) && !cwd_users.contains_key(&n.0)
            {
                cwd_users.insert(n.0, u.line);
                record_only.insert(n.0);
            }
        }
        let mut test_users: HashMap<u32, u32> = HashMap::new();
        for r in &index.test_path_users {
            test_users.entry(r.node.0).or_insert(r.line);
        }
        for (users, base) in [
            (cwd_users, EntryBase::Cwd),
            (test_users, EntryBase::TestDir),
        ] {
            if users.is_empty() {
                continue;
            }
            let kind = match base {
                EntryBase::Cwd => "cwd",
                EntryBase::TestDir => "testdir",
            };
            // One search per user file: an entry may be reached from several, with
            // different `ups`.
            let mut by_dir: BTreeMap<String, (HashSet<u32>, HashSet<u32>, String)> =
                BTreeMap::new();
            let mut seeds: Vec<u32> = users.keys().copied().collect();
            seeds.sort_unstable();
            for &user in &seeds {
                let to_user = self.code_ancestors(&p.code, std::iter::once(NodeId(user)), None);
                let ups = index.ups.get(&user).copied().unwrap_or(0);
                for (scope, to_site) in p.rounds() {
                    let mut tasks_hit = Vec::new();
                    let mut found = p.reach_by_runner(
                        &to_user,
                        to_site,
                        None,
                        &|_| false,
                        &|_, _| true,
                        &mut |runner, _, _| {
                            let own = [p.home(runner)];
                            let pkgs = scope.unwrap_or(&own);
                            let is_task = p.runners.contains_key(&runner.0);
                            let dirs: Vec<String> =
                                if base == EntryBase::Cwd && record_only.contains(&user) {
                                    dirs_of(runner)
                                        .into_iter()
                                        .filter(|d| !d.is_empty())
                                        .collect()
                                } else if base == EntryBase::Cwd || is_task {
                                    let d = dirs_of(runner);
                                    if d.is_empty() { vec![String::new()] } else { d }
                                } else {
                                    vec![
                                        g.path(runner)
                                            .rsplit_once('/')
                                            .map_or("", |x| x.0)
                                            .to_owned(),
                                    ]
                                };
                            changes
                                .iter()
                                .filter(|c| p.inside(pkgs, &c.path))
                                .find_map(|c| {
                                    dirs.iter()
                                        .map(|d| ancestor(d, ups))
                                        .find(|d| below(&c.path, d))
                                        .map(|d| (c.path.clone(), d.to_owned()))
                                })
                        },
                        &mut tasks_hit,
                    );
                    found.extend(p.above(&tasks_hit, &|_| false));
                    for s in found {
                        let e = s.entry;
                        let (path, dir) = s.hit;
                        let slot = by_dir
                            .entry(dir)
                            .or_insert_with(|| (HashSet::new(), HashSet::new(), path));
                        slot.0.insert(user);
                        slot.1.insert(e.0);
                    }
                }
            }
            for (dir, (files, hit, example)) in by_dir {
                let mut files: Vec<u32> = files.into_iter().collect();
                files.sort_unstable();
                let mut hit: Vec<u32> = hit.into_iter().collect();
                hit.sort_unstable();
                out.push(NamedGroup {
                    kind,
                    label: dir,
                    paths: 0,
                    example,
                    files,
                    entries: hit,
                });
            }
        }

        // 3. Paths a file builds on the working directory (`join(process.cwd(), '..',
        // name)`): one group per place they lead to, as the cause says it (`the working
        // directory (the repository root)`, `… of task:x#build (services/a/)`). The
        // entries are what `named_affected` selects with nothing but those paths in the
        // index, so an entry counts under the first place that names a changed path for
        // it.
        if !index.cwd_dirs.is_empty() {
            let only = NameIndex {
                cwd_dirs: index.cwd_dirs.clone(),
                ups: index.ups.clone(),
                data: index.data.clone(),
                count: index.cwd_dirs.len(),
                ..NameIndex::default()
            };
            let mut by_place: BTreeMap<String, (HashSet<u32>, Vec<u32>, String)> = BTreeMap::new();
            for r in self.named_affected(
                &only,
                changes,
                entries,
                runtime_entries,
                opts,
                cwds,
                task_dirs,
            ) {
                let crate::change::Cause::Named { path, by, .. } = &r.cause else {
                    continue;
                };
                let slot = by_place
                    .entry(by.clone())
                    .or_insert_with(|| (HashSet::new(), Vec::new(), path.clone()));
                slot.0.extend(r.witness.first().map(|n| n.0));
                slot.1.push(r.entry.0);
            }
            for (place, (files, mut hit, example)) in by_place {
                let mut files: Vec<u32> = files.into_iter().collect();
                files.sort_unstable();
                hit.sort_unstable();
                out.push(NamedGroup {
                    kind: "cwdpath",
                    label: place,
                    paths: 0,
                    example,
                    files,
                    entries: hit,
                });
            }
        }
        out
    }
}
