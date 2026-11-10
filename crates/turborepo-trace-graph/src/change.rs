//! The change model and `affected` report types.

use serde::Serialize;

use crate::{
    view::{NodeId, PackageId},
    vocab::{Effect, Origin, UncertaintyKind},
};

/// How a path changed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// The path did not exist in the graph's snapshot.
    Added,
    /// The file's contents changed.
    Modified,
    /// The path no longer exists.
    Deleted,
}

impl ChangeKind {
    /// `added`, `modified` or `deleted`.
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
        }
    }
}

/// One changed path, relative to the graph root.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Change {
    /// The path.
    pub path: String,
    /// How it changed.
    pub kind: ChangeKind,
}

impl Change {
    /// A change.
    pub fn new(kind: ChangeKind, path: impl Into<String>) -> Self {
        Change {
            path: crate::mem::normalize(&path.into()).to_owned(),
            kind,
        }
    }
}

/// The changes of one evaluation as a [`ChangeSource`] reports them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeList {
    /// The changed paths with their status, untracked files included (as
    /// added).
    pub changes: Vec<Change>,
    /// The changed paths that are symbolic links at the base (git stores a link
    /// as a file whose contents are its target; the base reads need to
    /// know).
    pub links_at_base: Vec<String>,
}

/// Where the changed files of an evaluation come from, with their status.
///
/// `affected` compares the working tree with a base revision. By default the
/// changes come from git (`git diff --raw --no-renames` against the merge
/// base, plus untracked files); a host with its own view of the repository
/// (Turborepo's SCM layer) supplies them through this trait instead. Renames
/// are a deletion and an addition.
pub trait ChangeSource: Send + Sync + std::fmt::Debug {
    /// The base revision the changes are relative to (a commit), from which old
    /// contents (lockfiles, manifests, configs) are read. `None`: there is
    /// no base, and every change is judged from the working tree alone.
    fn base(&self) -> Option<String>;
    /// The changes. An error makes the evaluation fail open (everything is
    /// selected).
    fn changes(&self) -> Result<ChangeList, String>;
}

/// A fixed list of changes: the default [`ChangeSource`] for callers that
/// computed them.
#[derive(Clone, Debug, Default)]
pub struct GivenChanges {
    /// The base revision, if any ([`ChangeSource::base`]).
    pub base: Option<String>,
    /// The changes.
    pub list: ChangeList,
}

impl ChangeSource for GivenChanges {
    fn base(&self) -> Option<String> {
        self.base.clone()
    }
    fn changes(&self) -> Result<ChangeList, String> {
        Ok(self.list.clone())
    }
}

/// A change set: changed paths plus changed third-party packages (from a
/// lockfile diff). Every node of a changed package counts as modified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Changed paths.
    pub files: Vec<Change>,
    /// Changed packages.
    pub packages: Vec<PackageId>,
    /// Packages changed only as seen from some importers: the files of those
    /// (first-party) packages that link the package directly count as
    /// modified, and so do the package's own file nodes, if it is traced.
    /// This is how a lockfile diff is applied when one lockfile governs
    /// only part of the repository, and how a `package.json` range edit
    /// marks a dependency changed for its importer only.
    pub scoped: Vec<ScopedPackage>,
    /// Nodes seeded directly, each with its cause. For callers that decide
    /// themselves which consumers of a changed file are affected (the
    /// consumers of the changed fields of a config file, instead of every
    /// node that read it) and leave the file out of [`ChangeSet::files`].
    pub seeds: Vec<(NodeId, Cause)>,
    /// Like [`ChangeSet::seeds`], for a change that only type resolution can
    /// observe (the `types` entry of a `package.json` an importer resolves
    /// through): not seeded in an evaluation pass that does not follow
    /// type-only edges.
    pub type_seeds: Vec<(NodeId, Cause)>,
    /// Directories that the change made (every file below is new: none was
    /// there before) or removed (none is left). No changed file names them,
    /// and a lookup that asks "is it a directory?" before it looks inside
    /// has another answer: such a directory matches the records of the
    /// lookups that asked for it by name ([`crate::Engine::match_dir`]),
    /// and nothing else.
    pub dirs: Vec<Change>,
    /// Readers of a changed file ([`ChangeSet::files`]) for which the change is
    /// none: per path, the nodes that have the file as a config input and
    /// are not seeded for it. For a caller that read both revisions and
    /// knows that what these nodes got of the file is the same at both (a
    /// manifest that a config load read through its narrow view, with every
    /// field of that view unchanged). Everything else the path matches (its
    /// node, probes, scopes, memberships, its other readers) is seeded as
    /// without this.
    pub spared: Vec<(String, Vec<NodeId>)>,
}

/// A package changed for some importers only (see [`ChangeSet::scoped`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedPackage {
    /// The changed package.
    pub package: PackageId,
    /// The importers (first-party packages) for which it changed.
    pub importers: Vec<PackageId>,
}

impl ChangeSet {
    /// An empty change set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a path change.
    pub fn push(&mut self, kind: ChangeKind, path: impl Into<String>) -> &mut Self {
        self.files.push(Change::new(kind, path));
        self
    }

    /// Adds a changed package.
    pub fn push_package(&mut self, p: PackageId) -> &mut Self {
        self.packages.push(p);
        self
    }

    /// Adds a package changed for `importers` only.
    pub fn push_scoped(&mut self, package: PackageId, importers: Vec<PackageId>) -> &mut Self {
        self.scoped.push(ScopedPackage { package, importers });
        self
    }

    /// Adds a node seeded directly ([`ChangeSet::seeds`]).
    pub fn push_seed(&mut self, node: NodeId, cause: Cause) -> &mut Self {
        self.seeds.push((node, cause));
        self
    }

    /// Appends another change set.
    pub fn extend(&mut self, other: ChangeSet) {
        self.files.extend(other.files);
        self.packages.extend(other.packages);
        self.scoped.extend(other.scoped);
        self.seeds.extend(other.seeds);
        self.type_seeds.extend(other.type_seeds);
    }

    /// Parses `git diff --name-status` output (tab- or space-separated). `A`,
    /// `M`, `D`, `T` (type change, as modified), `C` (copy, as added) and
    /// `R` (rename, as a delete plus an add) are understood; a bare path
    /// counts as modified. Empty lines are skipped.
    pub fn from_name_status(text: &str) -> Result<ChangeSet, String> {
        let mut cs = ChangeSet::new();
        for (lineno, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = if line.contains('\t') {
                line.split('\t').collect()
            } else {
                line.split_whitespace().collect()
            };
            let status = fields[0];
            let bad = || format!("line {}: cannot parse `{line}`", lineno + 1);
            let is_status = status.len() <= 4
                && status.chars().next().is_some_and(|c| "AMDTRCU".contains(c))
                && status[1..].bytes().all(|b| b.is_ascii_digit());
            if !is_status || fields.len() == 1 {
                cs.push(ChangeKind::Modified, line.trim());
                continue;
            }
            match (&status[..1], fields.len()) {
                ("A", 2) => {
                    cs.push(ChangeKind::Added, fields[1]);
                }
                ("M" | "T" | "U", 2) => {
                    cs.push(ChangeKind::Modified, fields[1]);
                }
                ("D", 2) => {
                    cs.push(ChangeKind::Deleted, fields[1]);
                }
                ("R", 3) => {
                    cs.push(ChangeKind::Deleted, fields[1]);
                    cs.push(ChangeKind::Added, fields[2]);
                }
                ("C", 3) => {
                    cs.push(ChangeKind::Added, fields[2]);
                }
                _ => return Err(bad()),
            }
        }
        Ok(cs)
    }
}

/// How entries whose closure contains first-party uncertainty are reported
/// (`--on-uncertain`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// Report them as affected (safe default).
    #[default]
    Affected,
    /// Report them as unknown.
    Unknown,
    /// Ignore the uncertainty.
    Ignore,
}

impl Policy {
    /// Parses `affected`, `unknown` or `ignore`.
    pub fn parse(s: &str) -> Option<Policy> {
        Some(match s {
            "affected" => Policy::Affected,
            "unknown" => Policy::Unknown,
            "ignore" => Policy::Ignore,
            _ => return None,
        })
    }
}

/// Options for `affected`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffectedOptions {
    /// Uncertainty policy.
    pub policy: Policy,
    /// Bit `k` set: uncertainty kind with index `k` (in
    /// [`UncertaintyKind::ALL`] order) counts. By default everything counts
    /// except `partial_specifier`, which is treated as a directory
    /// membership rather than uncertainty, the build-task kinds
    /// `unknown_command` and `shell_unsupported`, which always come with the
    /// task's whole package directory as a membership, and
    /// `eval_like` (`eval`, `new Function`), which is ignored by default
    /// and counted with [`strict_eval`](AffectedOptions::strict_eval)
    /// (`--strict-eval`).
    ///
    /// Covered uncertainties ([`crate::Uncertainty::scope`]) never count,
    /// whatever their kind: their scope seeds the node instead. Of the
    /// kinds that count, some records are still not open by default; see
    /// [`AffectedOptions::disposition`].
    pub counted_kinds: u32,
    /// Count every record of a counted kind as open (`--strict`): unresolved
    /// specifiers, generated files, and reads of paths that come from the
    /// environment or the command
    /// line. See [`AffectedOptions::disposition`].
    pub strict: bool,
    /// The graph may reflect the state *after* the change (it was refreshed
    /// from the working tree), not only the state before it. By default the
    /// graph is the "before" state: deleted files are nodes and added files
    /// are matched through probes. A graph built after the change no longer
    /// has nodes for deleted files; the files that resolved to one now
    /// probe its path (or fall back to a later candidate after probing it), so
    /// with this set a deleted path also seeds every node whose probes it
    /// matches. Sound for either state, at the cost of those extra seeds.
    pub after_change: bool,
    /// Reads with a computed path in languages with a name scan are bounded by
    /// the names of the code loaded with them ([`Disposition::Named`],
    /// [`crate::names`]) instead of being open. Set by callers that
    /// evaluate [`crate::Engine::named_affected`] for the same change; off
    /// by default, so that a caller without a name index keeps them open.
    pub named: bool,
    /// With [`named`](AffectedOptions::named): the package assumption of
    /// [`crate::names`]. A read with a computed path is taken to read only
    /// files of the package it runs for (the package of the entry, or of
    /// the file that holds the read), so a string, the working directory
    /// and the test's directory name a changed path only there. Names
    /// that say where they go (a file's own location, a path built on it) are
    /// not bounded. On by default; off, such a name counts for a changed
    /// path anywhere in the repository. [`strict`](AffectedOptions::strict)
    /// opens these reads altogether, so it assumes nothing.
    pub named_in_package: bool,
    /// With [`named`](AffectedOptions::named): a process that an entry reaches
    /// only through a build task (a script the task runs, consumed through
    /// the task's output) takes the names that depend on who runs it (the
    /// working directory, the package of the package assumption) from that
    /// task, not from the entry ([`crate::names`], "Who runs a process").
    /// On by default; off, such a process is treated as if the entry ran it.
    pub named_by_runner: bool,
    /// Module loads with an unknown target in JS/TS files are bounded by the
    /// declared packages ([`Disposition::Declared`], [`crate::loads`])
    /// instead of being open: such a load selects an entry only when the
    /// change can reach it. This rests on how the module system resolves a
    /// specifier and, for loads by a computed path, on an assumption (see
    /// [`crate::loads`]). Set by callers that evaluate
    /// [`crate::Engine::bounded_loads`] for the same change; off by default, so
    /// that a caller without the declared packages keeps them open.
    /// [`strict`](AffectedOptions::strict) opens these loads altogether.
    pub loads_declared: bool,
    /// With [`loads_declared`](AffectedOptions::loads_declared): a load is held
    /// to what ONE resolution step reaches from the loading file's package
    /// or from the entry's package, with chains of loads followed: the rule
    /// in its direct form ([`crate::loads`]). On by default. Off, a load is
    /// held only to the declared closure of the entry's package
    /// ([`crate::loads`], "The declared closure"). The step is what
    /// the module system proves for a specifier; for a load by a computed path
    /// it is the assumption.
    pub loads_step: bool,
    /// An `unresolved_specifier` record of a Python file is covered by what its
    /// resolution recorded ([`Disposition::Probed`]) instead of being open.
    /// The resolver raises the record only for a path that does not exist
    /// (its probes are recorded) and for a module that no file of the
    /// repository can provide (a new entry of its name is a
    /// recorded membership). Such a module comes from the interpreter's
    /// environment, and this is the assumption: the environment is
    /// installed from the dependency declarations and lockfiles in the
    /// importing file's directory or a directory above it, which are inputs
    /// of the file. On by default; [`strict`](AffectedOptions::strict)
    /// opens these records.
    pub python_declared: bool,
    /// Whether a read below a temporary directory that the code writes out
    /// (`/tmp/…`) is taken to name no file of the repository
    /// ([`crate::output::TEMP_READ_NOTE`]): its record is ignored. `false`
    /// (`TURBO_TRACE_DEBUG_TEMP_READS=0`): the record is open, as
    /// under [`strict`](AffectedOptions::strict).
    pub temp_reads: bool,
    /// Whether a read whose path has a computed directory directly below a
    /// place the repository may be mounted at (`` `/app/${dir}/x.json` ``)
    /// is taken to name no directory of the repository
    /// ([`crate::output::MOUNT_READ_NOTE`]): its record is ignored. `false`
    /// (`TURBO_TRACE_DEBUG_MOUNT_READS=0`): the record is open, as under
    /// [`strict`](AffectedOptions::strict).
    pub mount_reads: bool,
}

impl Default for AffectedOptions {
    fn default() -> Self {
        AffectedOptions {
            policy: Policy::Affected,
            counted_kinds: !((1 << UncertaintyKind::PartialSpecifier as u32)
                | (1 << UncertaintyKind::UnknownCommand as u32)
                | (1 << UncertaintyKind::ShellUnsupported as u32)
                | (1 << UncertaintyKind::EvalLike as u32)),
            strict: false,
            after_change: false,
            named: false,
            named_in_package: true,
            named_by_runner: true,
            loads_declared: false,
            loads_step: true,
            python_declared: true,
            temp_reads: true,
            mount_reads: true,
        }
    }
}

impl AffectedOptions {
    /// Sets the policy.
    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    /// Sets [`after_change`](AffectedOptions::after_change).
    pub fn after_change(mut self, on: bool) -> Self {
        self.after_change = on;
        self
    }

    /// Sets [`named`](AffectedOptions::named).
    pub fn named(mut self, on: bool) -> Self {
        self.named = on;
        self
    }

    /// Sets [`loads_step`](AffectedOptions::loads_step).
    pub fn loads_step(mut self, on: bool) -> Self {
        self.loads_step = on;
        self
    }

    /// Sets [`loads_declared`](AffectedOptions::loads_declared).
    pub fn loads_declared(mut self, on: bool) -> Self {
        self.loads_declared = on;
        self
    }

    /// Sets [`temp_reads`](AffectedOptions::temp_reads).
    pub fn temp_reads(mut self, on: bool) -> Self {
        self.temp_reads = on;
        self
    }

    /// Sets [`mount_reads`](AffectedOptions::mount_reads).
    pub fn mount_reads(mut self, on: bool) -> Self {
        self.mount_reads = on;
        self
    }

    /// Sets [`python_declared`](AffectedOptions::python_declared).
    pub fn python_declared(mut self, on: bool) -> Self {
        self.python_declared = on;
        self
    }

    /// Sets [`named_in_package`](AffectedOptions::named_in_package).
    pub fn named_in_package(mut self, on: bool) -> Self {
        self.named_in_package = on;
        self
    }

    /// Sets [`named_by_runner`](AffectedOptions::named_by_runner).
    pub fn named_by_runner(mut self, on: bool) -> Self {
        self.named_by_runner = on;
        self
    }

    /// Counts `eval_like` uncertainty (`--strict-eval`); by default it is
    /// ignored.
    pub fn strict_eval(self, on: bool) -> Self {
        self.count_kind(UncertaintyKind::EvalLike, on)
    }

    /// Whether uncertainty of `kind` counts.
    pub fn counts(&self, kind: UncertaintyKind) -> bool {
        self.counted_kinds & (1 << kind as u32) != 0
    }

    /// Counts everything [`AffectedOptions::disposition`] would otherwise treat
    /// as probed or ignored (`--strict`), `eval_like` included.
    pub fn strict(mut self, on: bool) -> Self {
        self.strict = on;
        if on {
            self = self.count_kind(UncertaintyKind::EvalLike, true);
        }
        self
    }

    /// How a change query treats an uncertainty record of a file in language
    /// `lang` (the node's [`crate::GraphView::lang`]).
    ///
    /// | Record | Disposition | Why |
    /// |---|---|---|
    /// | has a scope | [`Disposition::Scoped`] | changes inside the scope seed its node |
    /// | `partial_specifier`, `unknown_command`, `shell_unsupported` | [`Disposition::Membership`] | they come with a directory membership |
    /// | `unresolved_specifier` in a JS/TS file | [`Disposition::Probed`] | the literal specifier is known; only a path its resolution probed, a config it read or a lockfile entry can make it resolve, and those are tracked: a JS import that doesn't resolve on disk is linked to its package through the importer's lockfile entry |
    /// | `unresolved_specifier` in a Python file, with [`python_declared`](AffectedOptions::python_declared) | [`Disposition::Probed`] | the record is raised for a path that does not exist (probed), or for a module that no file of the repository can provide whatever the search path is: a new file or directory of its name is a recorded membership, and the dependency declarations and lockfiles next to the importing file or above it are its inputs (the environment is assumed to be installed from them) |
    /// | `unresolved_specifier` of a reference in a shell file | [`Disposition::Probed`] | the script names a literal path that is not in the tree (a build output, a file made at run time): only a file appearing at the path its resolution probed can make it resolve. A path with a part the shell computes is a `computed_specifier` and stays open, and so does a path a `source` takes relative to the working directory (origin `cwd`): the file may be where the script is started |
    /// | `eval_like` | [`Disposition::Ignored`] | `--strict-eval` counts it |
    /// | `generated_file` | [`Disposition::Ignored`] | the committed file is what runs; its generator's inputs matter only when a task regenerates it |
    /// | a *read* (not a module load) whose path comes from the environment or the command line | [`Disposition::Ignored`] | the path is an input from outside the repository |
    /// | with [`named`](AffectedOptions::named): a *read* in a JS/TS or Go file whose path is computed (`computed_specifier`), depends on the working directory (`cwd_dependent`) or is a pattern too broad to expand (`pattern_too_broad`) | [`Disposition::Named`] | a change to a path that the code loaded with it names selects the entry ([`crate::names`]) |
    /// | with [`loads_declared`](AffectedOptions::loads_declared): any other *module load* in a JS/TS file whose target is not known | [`Disposition::Declared`] | a change that affects a package the load can reach (its file's package or the entry's, and what they declare) selects the entries that reach it ([`crate::loads`]) |
    /// | anything else of a counted kind | [`Disposition::Open`] | triggers `--on-uncertain` |
    ///
    /// A Python import of a module that exists in the repository but not on the
    /// modelled search path is not such a record: the resolver reports it
    /// as `conditional_resolution`, which stays open (which file loads
    /// depends on `sys.path`). An `unresolved_specifier` in another
    /// language stays open. Go imports of a module that `go.mod` requires
    /// never get here: the graph store links them to the module version (no
    /// record), and what is left in a Go file is an import that no requirement
    /// provides. A read whose path is a function parameter stays open too,
    /// until its callers are bound.
    ///
    /// With [`strict`](AffectedOptions::strict), probed and ignored records are
    /// open.
    pub fn disposition(&self, u: &crate::view::Uncertainty<'_>, lang: Option<&str>) -> Disposition {
        if u.is_covered() {
            return Disposition::Scoped;
        }
        if !self.counts(u.kind) {
            return if u.kind == UncertaintyKind::EvalLike {
                Disposition::Ignored
            } else {
                Disposition::Membership
            };
        }
        if self.strict {
            return Disposition::Open;
        }
        let read = u.effect == Effect::Read;
        match u.kind {
            UncertaintyKind::UnresolvedSpecifier if lang == Some("js") => Disposition::Probed,
            UncertaintyKind::UnresolvedSpecifier
                if lang == Some("python") && self.python_declared =>
            {
                Disposition::Probed
            }
            // A shell reference to a literal path that is not there: its resolution
            // recorded the probe. A diagnostic of a command (no reference, no probe) stays
            // open.
            UncertaintyKind::UnresolvedSpecifier
                if lang == Some("shell") && u.effect != Effect::None && u.origin != Origin::Cwd =>
            {
                Disposition::Probed
            }
            // A read below a temporary directory, with the assumption about it off.
            UncertaintyKind::GeneratedFile
                if !self.temp_reads && u.detail.ends_with(crate::output::TEMP_READ_NOTE) =>
            {
                Disposition::Open
            }
            // A read with a computed directory directly below a mount point, with the
            // assumption about it off.
            UncertaintyKind::GeneratedFile
                if !self.mount_reads && u.detail.ends_with(crate::output::MOUNT_READ_NOTE) =>
            {
                Disposition::Open
            }
            UncertaintyKind::GeneratedFile => Disposition::Ignored,
            UncertaintyKind::EnvDependent if read => Disposition::Ignored,
            UncertaintyKind::ComputedSpecifier
                if read && matches!(u.origin, Origin::Env | Origin::Argv) =>
            {
                Disposition::Ignored
            }
            // A read (not a module load) whose path is computed, depends on the working
            // directory, or is a pattern too broad to expand: bounded by the names of the
            // code loaded with it.
            UncertaintyKind::ComputedSpecifier
            | UncertaintyKind::CwdDependent
            | UncertaintyKind::PatternTooBroad
                if read && self.named && matches!(lang, Some("js" | "go")) =>
            {
                Disposition::Named
            }
            // A module load whose target is not known: the module system resolves it from
            // the loading file's package, so the declared packages bound it.
            _ if u.effect == Effect::Load && self.loads_declared && lang == Some("js") => {
                Disposition::Declared
            }
            _ => Disposition::Open,
        }
    }

    /// Whether an uncertainty of a file in language `lang` triggers the policy
    /// ([`Disposition::Open`]).
    pub fn triggers(&self, u: &crate::view::Uncertainty<'_>, lang: Option<&str>) -> bool {
        self.disposition(u, lang) == Disposition::Open
    }

    /// Whether an uncertainty of a file in language `lang` is reported as
    /// ignored by default ([`Disposition::Ignored`]).
    pub fn ignores(&self, u: &crate::view::Uncertainty<'_>, lang: Option<&str>) -> bool {
        self.disposition(u, lang) == Disposition::Ignored
    }

    /// Includes or excludes a kind.
    pub fn count_kind(mut self, kind: UncertaintyKind, on: bool) -> Self {
        if on {
            self.counted_kinds |= 1 << kind as u32;
        } else {
            self.counted_kinds &= !(1 << kind as u32);
        }
        self
    }
}

/// Why a node was seeded as changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The node's file was modified (or re-added).
    Modified {
        /// The changed path.
        path: String,
    },
    /// The node's file was deleted.
    Deleted {
        /// The changed path.
        path: String,
    },
    /// An added path matched a probe recorded on the node.
    Probe {
        /// The added path.
        added: String,
        /// The probe it matched: the path itself, a missing ancestor directory,
        /// or a path that was looked for below it (the added path is a
        /// link or a new directory).
        probe: String,
    },
    /// An added or deleted path matched a directory membership recorded on the
    /// node.
    Membership {
        /// The changed path.
        path: String,
        /// Added or deleted.
        kind: ChangeKind,
        /// Membership directory.
        dir: String,
        /// Membership pattern.
        pattern: String,
    },
    /// The node belongs to a changed package.
    Package {
        /// The package.
        package: PackageId,
    },
    /// An added, modified or deleted path lies inside the scope of a covered
    /// uncertainty recorded on the node ([`crate::Uncertainty::scope`]).
    Scope {
        /// The changed path.
        path: String,
        /// How it changed.
        kind: ChangeKind,
        /// Scope directory.
        dir: String,
        /// Scope pattern.
        pattern: String,
    },
    /// An added, modified or deleted path is named by a string or a file
    /// location of code that is loaded together with a read whose path is
    /// computed ([`crate::names`]). The first witness node is the file with
    /// the name.
    Named {
        /// The changed path.
        path: String,
        /// What names it, readable: `the string 'fixtures/a.json'`.
        by: String,
        /// 1-based line of the name in the first witness node; 0 if unknown.
        line: u32,
        /// The file with the read.
        site: String,
        /// 1-based line of the read; 0 if unknown.
        site_line: u32,
    },
}

/// The verdict for one entry.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Affected: by a change, or by uncertainty under [`Policy::Affected`].
    Affected,
    /// Its closure has first-party uncertainty and the policy is
    /// [`Policy::Unknown`].
    Unknown,
    /// Not affected.
    Unaffected,
}

/// Uncertainty that determined a verdict: the nearest uncertain first-party
/// node in the entry's closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncertainSite {
    /// The uncertain node.
    pub node: NodeId,
    /// Its counted uncertainties: kind, line, detail.
    pub reasons: Vec<(UncertaintyKind, u32, String)>,
    /// For a load that the declared packages bound ([`crate::loads`]): why this
    /// change selects the entry through it, as text (`the load can reach
    /// packages/a/ (packages/a/x.ts changed)`). `None` for an open site,
    /// which selects on every change.
    pub bound: Option<String>,
}

/// How a change query treats one uncertainty record
/// ([`AffectedOptions::disposition`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Nothing bounds it: entries that reach it get the `--on-uncertain`
    /// verdict.
    Open,
    /// Covered by its scope: a change inside the scope seeds its node.
    Scoped,
    /// Covered by what its resolution recorded (probes, config inputs, lockfile
    /// links).
    Probed,
    /// Reported, but not counted by default.
    Ignored,
    /// Not an uncertainty for change queries: a directory membership stands for
    /// it.
    Membership,
    /// A read with a computed path, bounded by the path-like strings and file
    /// locations of the code loaded with it ([`crate::names`]): a change to
    /// a path they name selects the entries that load both.
    Named,
    /// A module load with an unknown target, bounded by the declared packages
    /// ([`crate::loads`]): it selects the entries that reach it only when the
    /// change affects a package the load can reach.
    Declared,
}

/// A first-party uncertainty in an entry's closure that does not trigger the
/// policy (see [`crate::Engine::uncertainty_notes`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncertaintyNote {
    /// The node it is recorded on.
    pub node: NodeId,
    /// Kind.
    pub kind: UncertaintyKind,
    /// 1-based line; 0 if unknown.
    pub line: u32,
    /// Detail.
    pub detail: String,
    /// `(dir, pattern)` for a record covered by a scope; `None` otherwise.
    pub scope: Option<(String, String)>,
    /// How the policy treats it: scoped, probed or ignored.
    pub disposition: Disposition,
}

/// Graph-wide counts of first-party uncertainty records that don't trigger the
/// policy.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct UncertaintyCounts {
    /// Open records that trigger the policy.
    pub open: usize,
    /// Covered records: by a scope (changes inside it seed their node) or by
    /// what their resolution recorded ([`UncertaintyCounts::probed`],
    /// included here).
    pub covered: usize,
    /// Of the covered records, unresolved specifiers covered by probes, config
    /// inputs and lockfile links.
    pub probed: usize,
    /// Records ignored by default (`--strict` counts them; `--strict-eval`
    /// counts `eval_like`).
    pub ignored: usize,
}

/// The result for one affected or unknown entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryResult {
    /// The entry.
    pub entry: NodeId,
    /// Verdict.
    pub verdict: Verdict,
    /// Shortest chain from the seed (a changed node or the uncertain node) to
    /// the entry, both inclusive: `witness[i]` depends on nothing here,
    /// `witness[i + 1]` has an edge to `witness[i]`.
    pub witness: Vec<NodeId>,
    /// Why the first witness node changed, when the verdict comes from a
    /// change.
    pub cause: Option<Cause>,
    /// The uncertainty, when the verdict comes from it.
    pub uncertain: Option<UncertainSite>,
}

/// The full `affected` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffectedReport {
    /// Policy used.
    pub policy: Policy,
    /// Number of entries evaluated.
    pub entry_count: usize,
    /// Affected and unknown entries, sorted by path. Unaffected entries are
    /// omitted.
    pub entries: Vec<EntryResult>,
    /// Number of nodes seeded as changed.
    pub seed_count: usize,
    /// Number of nodes in the reverse closure of the seeds.
    pub reached_count: usize,
    /// Changes that matched no node, probe or membership.
    pub unmatched: Vec<Change>,
}

impl AffectedReport {
    /// Entries with the given verdict.
    pub fn with_verdict(&self, v: Verdict) -> impl Iterator<Item = &EntryResult> {
        self.entries.iter().filter(move |e| e.verdict == v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status() {
        let cs = ChangeSet::from_name_status(
            "M\tsrc/a.ts\nA  src/new.ts\nD\told.ts\nR100\tx.ts\ty.ts\n\nplain/path.ts\n",
        )
        .unwrap();
        let got: Vec<_> = cs
            .files
            .iter()
            .map(|c| (c.kind.as_str(), c.path.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("modified", "src/a.ts"),
                ("added", "src/new.ts"),
                ("deleted", "old.ts"),
                ("deleted", "x.ts"),
                ("added", "y.ts"),
                ("modified", "plain/path.ts"),
            ]
        );
        assert!(ChangeSet::from_name_status("R100\tonly-one").is_err());
    }
}
