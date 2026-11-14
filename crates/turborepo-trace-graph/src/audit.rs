//! The unlikely-path audit of a selection: which selected entries have
//! no plausible relation to the change, and which few graph nodes (the
//! *sources*) put them in the selection.
//!
//! [`Engine::affected`] selects every entry with a dependency path from a
//! changed file, and every entry that reaches an open uncertainty. Many such
//! paths are recognizable a priori as implausible: a TypeScript edit selecting
//! a Go test, a test selected through a whole-package build task, a test
//! selected only by the uncertainty floor. [`Engine::audit`] flags them with
//! cheap [`Signal`]s and attributes them to their sources, ranked by how many
//! selections each one triggers.
//!
//! # The audit graph
//!
//! The audit works on the *selection graph*: the reverse dependency graph, in
//! the direction "is selected because of", with a virtual root and virtual
//! nodes on everything that is not an ordinary import:
//!
//! ```text
//! ROOT → change(i) → the changed file's node → … → entry          (a plain path)
//! ROOT → change(i) → [input | probe | listing | scope](i) → node  (the change is read, probed or listed)
//! ROOT → floor → [open uncertainty](site) → site → … → entry
//! n → [type-only](n) → d        d imports n type-only
//! n → [framework role](n) → d   d is a test, n its setup file, config, environment, …
//! n → [read](d) → d             d reads source file n as data (fs read, asset, embed)
//! n → [undeclared](n, P) → d    d (package P) imports n, and P does not depend on n's package
//! n → [cross-language](d) → d   d and n are source files of different languages
//! n → [via test](n) → d         n is itself an entry (a test another test depends on)
//! ```
//!
//! Task nodes (`task:…`) and membership hubs (`members:…`) are nodes of the
//! graph already. Every virtual node and every task or hub node is a *bridge*;
//! bridges with the same [source id](AuditSource::id) form one source. A path
//! that crosses no bridge is *plain*: ordinary, declared, runtime imports from
//! a changed file.
//!
//! # Signals
//!
//! For a selected entry, with "reaching change" meaning a changed file with a
//! path to it (for an entry only the floor reaches: every changed file):
//!
//! | Signal | Fires when |
//! |---|---|
//! | `non_import` | no plain path reaches the entry. Every other signal implies it, and an entry without it can't be unselected by removing sources |
//! | `floor` | no change reaches it: only the uncertainty floor does |
//! | `cross_ecosystem` | no reaching change is in the entry's language ecosystem or neutral (data, docs, shell), and no known bridge connects them (a read, embed or cross-language reference of that very file, or a task that has the file as a direct input and generates a file the entry depends on) |
//! | `no_package_dep` | no reaching change lies in a package the entry's package depends on (declared, transitively), in the entry's package, or in the root package |
//! | `steering` | it is reached only from repository-steering files (`.github/**`, `CODEOWNERS`, …) and not through an explicit reference to such a file |
//! | `type_only` | every path crosses a type-only import |
//! | `via_test` | every path passes through another entry |
//!
//! # Attribution
//!
//! A witness chain is one shortest path, so counting chains through a node says
//! little: the next path appears when the first is cut. The audit counts
//! entries that *drop out*:
//!
//! - **exclusive**: entries no longer reachable from the root when this source
//!   alone is removed. Exact (one search per reported source).
//! - **inclusive**: entries with at least one path through the source. Exact.
//! - **freed** / **left**: the greedy order. Each step computes the dominator
//!   tree of the current graph (semi-NCA), picks the source that dominates the
//!   most still-selected implausible entries (if none dominates any: the one
//!   that the most reach, estimated on a sample), removes it and counts the
//!   entries that drop out. `freed` is exact given the sources above it are
//!   removed too.
//!
//! Cost: building the audit graph is one pass over the reached part of the
//! reverse adjacency; each signal is one search; each greedy step is one
//! dominator computation (`O((V + E) log V)`), and each reported source three
//! searches.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{self, Write},
    time::Instant,
};

use serde::Serialize;

use crate::{
    change::{AffectedOptions, Cause, Change, ChangeKind, ChangeSet},
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId, PackageId, PackageVersion},
    vocab::{EdgeFilter, RefFlags, RefKind},
};

const NONE: u32 = u32::MAX;

/// Language id of build-task nodes in the composed graph.
pub const TASK_LANG: &str = "task";
/// Language id of membership-hub nodes in the composed graph.
pub const HUB_LANG: &str = "task-members";

/// A language ecosystem, for the `cross_ecosystem` signal.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    /// JavaScript and TypeScript.
    Js,
    /// Python.
    Python,
    /// Go.
    Go,
    /// Rust.
    Rust,
}

impl Ecosystem {
    /// The ecosystem's bit in a mask ([`AuditOptions::package_ecosystems`]).
    pub const fn bit(self) -> u8 {
        1 << self as u8
    }

    /// `js`, `python`, `go` or `rust`.
    pub fn as_str(self) -> &'static str {
        match self {
            Ecosystem::Js => "js",
            Ecosystem::Python => "python",
            Ecosystem::Go => "go",
            Ecosystem::Rust => "rust",
        }
    }

    /// The ecosystem of a graph language id.
    pub fn of_lang(lang: &str) -> Option<Ecosystem> {
        Some(match lang {
            "js" | "ts" => Ecosystem::Js,
            "py" | "python" => Ecosystem::Python,
            "go" => Ecosystem::Go,
            "rust" | "rs" => Ecosystem::Rust,
            _ => return None,
        })
    }

    /// The ecosystem a changed path belongs to, from its name: source files,
    /// manifests, lockfiles and tool configs. `None` is neutral: data,
    /// documentation, shell, CI and anything any ecosystem may read.
    pub fn of_path(path: &str) -> Option<Ecosystem> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        let ext = lower.rsplit_once('.').map_or("", |x| x.1);
        match lower.as_str() {
            "package.json"
            | "pnpm-lock.yaml"
            | "pnpm-workspace.yaml"
            | "package-lock.json"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
            | ".npmrc"
            | ".nvmrc"
            | "turbo.json"
            | "turbo.jsonc"
            | "jsconfig.json"
            | ".eslintrc"
            | ".eslintignore"
            | ".prettierrc"
            | ".prettierignore"
            | "nx.json"
            | "lerna.json"
            | "deno.json" => {
                return Some(Ecosystem::Js);
            }
            "go.mod" | "go.sum" | "go.work" | "go.work.sum" | ".golangci.yml"
            | ".golangci.yaml" => return Some(Ecosystem::Go),
            "pyproject.toml" | "setup.py" | "setup.cfg" | "pipfile" | "pipfile.lock"
            | "poetry.lock" | "uv.lock" | "tox.ini" | "pytest.ini" | "noxfile.py"
            | ".python-version" | "manifest.in" => return Some(Ecosystem::Python),
            "cargo.toml"
            | "cargo.lock"
            | "rust-toolchain"
            | "rust-toolchain.toml"
            | "rustfmt.toml"
            | "clippy.toml" => return Some(Ecosystem::Rust),
            _ => {}
        }
        if lower.starts_with("tsconfig") && ext == "json" {
            return Some(Ecosystem::Js);
        }
        if lower.starts_with("requirements") && ext == "txt" {
            return Some(Ecosystem::Python);
        }
        if lower.starts_with(".eslintrc.") || lower.starts_with(".prettierrc.") {
            return Some(Ecosystem::Js);
        }
        Some(match ext {
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte"
            | "astro" | "mdx" | "snap" | "css" | "scss" | "sass" | "less" => Ecosystem::Js,
            "py" | "pyi" | "pyx" => Ecosystem::Python,
            "go" => Ecosystem::Go,
            "rs" => Ecosystem::Rust,
            _ => return None,
        })
    }
}

/// The class of a repository-steering file: one that configures the repository
/// host, code review or editors and can't change what an application does.
/// `None` for other paths.
///
/// Classes: `.github/**`, `CODEOWNERS`, `.vercel.approvers`, `.gitattributes`,
/// `.git-blame-ignore-revs`, `.editorconfig`, `renovate` and `mergify` configs.
pub fn steering_class(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "CODEOWNERS" => return Some("CODEOWNERS"),
        ".vercel.approvers" => return Some(".vercel.approvers"),
        ".gitattributes" => return Some(".gitattributes"),
        ".git-blame-ignore-revs" => return Some(".git-blame-ignore-revs"),
        ".editorconfig" => return Some(".editorconfig"),
        "renovate.json" | "renovate.json5" | ".renovaterc" | ".renovaterc.json"
        | ".renovaterc.json5" => return Some("renovate"),
        ".mergify.yml" | ".mergify.yaml" | "mergify.yml" => return Some("mergify"),
        _ => {}
    }
    if path.starts_with(".github/") || path.contains("/.github/") {
        return Some(".github/**");
    }
    None
}

/// The framework role of an edge specifier written by the graph store for
/// test-framework inputs (`jest:setup_file tests/setup.ts`), when the role is
/// shared by the tests of a project: configs, setup files, environments,
/// transformers, presets, plugins, conftests. Per-test inputs (snapshots,
/// mocks, test data, files compiled with a test) are ordinary edges and return
/// `None`.
pub fn framework_role(specifier: &str) -> Option<&str> {
    let label = specifier.split(' ').next()?;
    let (fw, role) = label.split_once(':')?;
    if !matches!(fw, "jest" | "vitest" | "pytest" | "gotest" | "cargo-test") {
        return None;
    }
    matches!(
        role,
        "config"
            | "setup_file"
            | "setup_file_after_env"
            | "global_setup"
            | "global_teardown"
            | "test_environment"
            | "transform"
            | "snapshot_resolver"
            | "preset"
            | "resolver"
            | "test_runner"
            | "conftest"
            | "plugin"
    )
    .then_some(label)
}

macro_rules! bit_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident { $( $(#[$vmeta:meta])* $variant:ident = $s:literal, )* }
    ) => {
        $(#[$meta])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $( $(#[$vmeta])* $variant, )* }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            /// Stable snake_case name.
            pub const fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s,)* }
            }

            /// Parses a stable name.
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($name::$variant),)* _ => None }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

bit_enum! {
    /// A plausibility signal of a selected entry (see the [module documentation](self)).
    pub enum Signal {
        /// No plain path (ordinary declared runtime imports from a changed file).
        NonImport = "non_import",
        /// Only the uncertainty floor reaches the entry.
        Floor = "floor",
        /// No reaching change is in the entry's ecosystem.
        CrossEcosystem = "cross_ecosystem",
        /// No reaching change is in the entry's declared package dependency closure.
        NoPackageDep = "no_package_dep",
        /// Reached only from repository-steering files.
        Steering = "steering",
        /// Every path crosses a type-only import.
        TypeOnly = "type_only",
        /// Every path passes through another entry.
        ViaTest = "via_test",
    }
}

impl Signal {
    /// The signal's bit in a signal mask.
    pub const fn bit(self) -> u32 {
        1 << self as u32
    }

    /// The signals set in `mask`.
    pub fn of_mask(mask: u32) -> impl Iterator<Item = Signal> {
        Signal::ALL
            .iter()
            .copied()
            .filter(move |s| mask & s.bit() != 0)
    }
}

bit_enum! {
    /// What kind of bridge a source is.
    pub enum SourceKind {
        /// The uncertainty floor as a whole (`--on-uncertain ignore` removes it).
        Floor = "floor",
        /// One file with open uncertainty.
        Uncertain = "uncertain",
        /// A build task node.
        Task = "task",
        /// A membership hub: every file of a directory that matches a pattern.
        Members = "members",
        /// A changed file that nodes read or probed (a config input).
        Input = "input",
        /// An added path that matched resolution probes.
        Probe = "probe",
        /// An added or deleted path that matched a directory listing.
        Listing = "listing",
        /// A change inside the scope of a covered uncertainty.
        Scope = "scope",
        /// Repository-steering files reaching through tasks, hubs, listings or inputs.
        Steering = "steering",
        /// A framework input shared by the tests of a project (setup file, config, …).
        Framework = "framework",
        /// A file that reads source files as data.
        Read = "read",
        /// A module imported type-only.
        TypeOnly = "type_only",
        /// An import between packages without a declared dependency.
        Undeclared = "undeclared",
        /// A reference between source files of different languages.
        CrossLang = "cross_lang",
        /// An entry that other entries depend on.
        ViaTest = "via_test",
    }
}

/// Options of [`Engine::audit`].
#[derive(Clone, Debug)]
pub struct AuditOptions<'a> {
    /// Greedy picks, and sources reported beyond them.
    pub top: usize,
    /// The declared dependencies of every package (indexed by [`PackageId`]):
    /// workspace packages it names in its manifest. `None` turns
    /// `no_package_dep` and the `undeclared` bridge off.
    pub package_deps: Option<&'a [Vec<PackageId>]>,
    /// Per package, the ecosystems its manifests declare dependencies for, as a
    /// mask of [`Ecosystem::bit`]s; 0 or `None`: any. A file whose language
    /// is not covered by its package's manifests (a Go file in a directory
    /// that only has a `package.json`) has no declared dependencies either
    /// way, so the package signal skips it.
    pub package_ecosystems: Option<&'a [u8]>,
    /// Witness chains per entry in the verdicts.
    pub chains: bool,
    /// The traversal filter of the pass that selected `entries` (`None`: the
    /// engine's). `affected` evaluates tests whose runner transpiles file
    /// by file without type-only
    /// edges ([`crate::barrel::PruneReport::runtime_entries`]): audit those
    /// entries with that filter, the others with the engine's, and
    /// [`AuditReport::merge`] the two.
    pub filter: Option<EdgeFilter>,
    /// Changed repository-steering files that are not part of the change set,
    /// because the caller counts them only through explicit references.
    /// They are reported in [`SteeringStats`] and reach nothing.
    pub ignored_steering: &'a [String],
}

impl Default for AuditOptions<'_> {
    fn default() -> Self {
        AuditOptions {
            top: 15,
            package_deps: None,
            package_ecosystems: None,
            chains: true,
            filter: None,
            ignored_steering: &[],
        }
    }
}

/// One source of implausible selections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditSource {
    /// Stable id: a task or hub node (`task:.#build`, `members:apps/web/**`),
    /// `floor`, `uncertain:<file>`, `input:<changed path>`, `probe:<added
    /// path>`, `listing:<dir>/<pattern>`, `scope:<dir>/<pattern>`,
    /// `steering:<class>`, `<framework>:<role> <file>`, `read-by:<file>`,
    /// `type-only:<file>`, `undeclared:<package root> -> <package root>`,
    /// `cross-lang:<file>`, `via-test:<file>`.
    pub id: String,
    /// The kind of bridge.
    pub kind: SourceKind,
    /// Bridge nodes with this id.
    pub members: usize,
    /// Implausible entries that drop out of the selection when only this source
    /// is removed.
    pub exclusive: usize,
    /// The same without the uncertainty floor (as under `--on-uncertain
    /// ignore`): of the implausible entries a change reaches, those no
    /// change reaches without this source. Where the floor also covers a
    /// source's entries, this is what fixing the source buys once the floor
    /// is gone.
    pub exclusive_no_floor: usize,
    /// Implausible entries with at least one path through the source.
    pub inclusive: usize,
    /// 1-based position in the greedy order, if picked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<usize>,
    /// Entries that dropped out when it was removed after the sources ranked
    /// above it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freed: Option<usize>,
    /// Implausible entries still selected after this pick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<usize>,
    /// Signals of the entries attributed to the source (`freed` and `exclusive`
    /// entries; the `inclusive` ones if there are none), by signal name.
    pub signals: BTreeMap<&'static str, usize>,
    /// One chain from a change (or the floor) through the source to an entry.
    pub example: Vec<String>,
    /// Changed files that enter through the source (at most five).
    pub changes: Vec<String>,
    /// Number of changed files that enter through the source.
    pub changes_total: usize,
}

/// Every source of one kind taken together: what removing the whole mechanism
/// would do.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KindTotal {
    /// The kind.
    pub kind: SourceKind,
    /// Sources of the kind in the audit graph.
    pub sources: usize,
    /// Implausible entries that drop out when every source of the kind is
    /// removed.
    pub exclusive: usize,
    /// The same without the uncertainty floor.
    pub exclusive_no_floor: usize,
    /// Implausible entries with a path through a source of the kind.
    pub inclusive: usize,
}

/// The audit verdict of one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryVerdict {
    /// The entry.
    pub entry: NodeId,
    /// Whether the entry is selected.
    pub selected: bool,
    /// Signals that fired ([`Signal::bit`]); 0 for unselected entries.
    pub signals: u32,
    /// The a-priori relation of the entry to the whole diff, selected or not:
    /// bits of [`Signal::CrossEcosystem`] (no changed file in its ecosystem
    /// or neutral), [`Signal::NoPackageDep`] (no changed file in its
    /// package dependency closure) and [`Signal::Steering`] (every changed
    /// file is a steering file).
    pub a_priori: u32,
    /// The entry's ecosystem.
    pub ecosystem: Option<Ecosystem>,
    /// The first bridge on the witness chain: kind and source id.
    pub bridge: Option<(SourceKind, String)>,
    /// Index into [`AuditReport::sources`] of the greedy pick that unselected
    /// the entry.
    pub source: Option<usize>,
    /// The shortest chain from a change (or the floor) to the entry, bridges in
    /// brackets.
    pub chain: Vec<String>,
}

/// Steering files in the change set and what they select.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SteeringStats {
    /// Changed steering files, by class.
    pub classes: BTreeMap<&'static str, usize>,
    /// Changed files that are steering files.
    pub changes: usize,
    /// Selected entries a steering file reaches other than through an explicit
    /// reference.
    pub inclusive: usize,
    /// Selected entries that only steering files reach (the `steering` signal).
    pub exclusive: usize,
    /// Changed steering files the selection ignores (the caller counts them
    /// only through explicit references;
    /// [`AuditOptions::ignored_steering`]). They are part of `changes` and
    /// `classes` and reach nothing here.
    pub ignored: usize,
}

/// Phase timings of an audit, in milliseconds.
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize)]
pub struct AuditTiming {
    /// Building the audit graph.
    pub graph_ms: f64,
    /// The signal searches.
    pub signals_ms: f64,
    /// The greedy order (dominator trees).
    pub greedy_ms: f64,
    /// Exclusive and inclusive counts, examples and changes of the reported
    /// sources.
    pub sources_ms: f64,
}

/// The result of [`Engine::audit`].
#[derive(Clone, Debug, PartialEq)]
pub struct AuditReport {
    /// Entries evaluated.
    pub entries: usize,
    /// Selected entries.
    pub selected: usize,
    /// Selected entries with at least one signal: no plain path reaches them.
    pub implausible: usize,
    /// Selected entries the audit graph does not reach (selected by something
    /// the audit doesn't model); they have no verdict signals.
    pub unexplained: usize,
    /// Selected entries only the uncertainty floor reaches.
    pub floor_only: usize,
    /// Selected entries only a change reaches.
    pub change_only: usize,
    /// Selected entries both reach.
    pub floor_and_change: usize,
    /// Entries per signal, by signal name.
    pub signal_counts: BTreeMap<&'static str, usize>,
    /// Per kind of source, all its sources together: largest exclusive count
    /// first.
    pub kinds: Vec<KindTotal>,
    /// Sources: the greedy picks in order, then other sources by exclusive
    /// count.
    pub sources: Vec<AuditSource>,
    /// Implausible entries still selected after the last greedy pick.
    pub left: usize,
    /// Steering files.
    pub steering: SteeringStats,
    /// One verdict per entry, in path order.
    pub verdicts: Vec<EntryVerdict>,
    /// Changed files and packages.
    pub changes: usize,
    /// Nodes and edges of the audit graph.
    pub graph: (usize, usize),
    /// Timings.
    pub timing: AuditTiming,
}

// -------------------------------------------------------------------------------------
// The audit graph
// -------------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum Virt {
    Root,
    Floor,
    /// Change `i` (index into `Ag::changes`).
    Changed(u32),
    /// A non-direct cause of change `i`; the label is the group's id.
    Cause(u32),
    /// The open uncertainty of a site.
    Site(u32),
    /// Steering change `i` reaching through non-explicit means.
    Steer(u32),
    /// Framework edges into a node.
    Fw(u32, u16),
    /// Source reads by a reader.
    Read(u32),
    /// Type-only imports of a node.
    TypeOnly(u32),
    /// Imports of a node from a package that doesn't depend on the node's
    /// package.
    Undeclared(u32, u32),
    /// Cross-language references by a reader.
    CrossLang(u32),
    /// Dependents of an entry.
    Via(u32),
}

struct Csr {
    off: Vec<u32>,
    to: Vec<u32>,
}

impl Csr {
    fn build(total: usize, edges: &[(u32, u32)], reverse: bool) -> Csr {
        let mut off = vec![0u32; total + 1];
        for &(a, b) in edges {
            let k = if reverse { b } else { a };
            off[k as usize + 1] += 1;
        }
        for i in 0..total {
            off[i + 1] += off[i];
        }
        let mut cursor = off.clone();
        let mut to = vec![0u32; edges.len()];
        for &(a, b) in edges {
            let (k, v) = if reverse { (b, a) } else { (a, b) };
            let c = &mut cursor[k as usize];
            to[*c as usize] = v;
            *c += 1;
        }
        Csr { off, to }
    }

    #[inline]
    fn of(&self, n: u32) -> &[u32] {
        &self.to[self.off[n as usize] as usize..self.off[n as usize + 1] as usize]
    }
}

/// A changed file or package.
struct ChangeRec {
    /// Display: the path, or `package <name>`.
    label: String,
    kind: Option<ChangeKind>,
    eco: Option<Ecosystem>,
    steering: Option<&'static str>,
    /// The package of a changed file, or the importers of a changed package;
    /// empty with `global`.
    packages: Vec<u32>,
    /// Related to every entry by package: a root file or a repo-wide package
    /// change.
    global: bool,
    /// The audit node.
    node: u32,
}

struct Group {
    id: String,
    kind: SourceKind,
    members: Vec<u32>,
}

const F_CODE: u8 = 1;
const F_TASK: u8 = 2;
const F_HUB: u8 = 4;
const F_ENTRY: u8 = 8;
const F_FIRST_PARTY: u8 = 16;

struct Ag {
    /// Real nodes are `0..real`; virtual nodes follow.
    real: u32,
    virt: Vec<Virt>,
    root: u32,
    floor: u32,
    succ: Csr,
    pred: Csr,
    /// Group of each node, `NONE` for nodes that are not bridges.
    group_of: Vec<u32>,
    groups: Vec<Group>,
    changes: Vec<ChangeRec>,
    /// Per real node: `F_*` flags.
    flags: Vec<u8>,
    /// Labels of framework roles.
    fw_labels: Vec<String>,
    /// Cause labels (rendered in chains), indexed by `Virt::Cause`.
    cause_labels: Vec<String>,
    /// Edges that carry changes but not uncertainty ([`RefFlags::DATA`]):
    /// present because a change reaches their source; the floor does not
    /// pass them.
    data_edges: HashSet<(u32, u32)>,
}

impl Ag {
    fn total(&self) -> usize {
        self.group_of.len()
    }

    fn virt_of(&self, n: u32) -> Option<Virt> {
        (n >= self.real).then(|| self.virt[(n - self.real) as usize])
    }

    /// Whether `n` may be on a plain path.
    fn plain_ok(&self, n: u32) -> bool {
        match self.virt_of(n) {
            None => self.flags[n as usize] & (F_TASK | F_HUB) == 0,
            Some(v) => matches!(v, Virt::Root | Virt::Changed(_)),
        }
    }

    /// Breadth-first search from `starts` over nodes not in `blocked`. Returns
    /// the visited marks; with `parent`, records the node each visited node
    /// was reached from.
    fn bfs(
        &self,
        starts: &[u32],
        blocked: &[bool],
        mut parent: Option<&mut Vec<u32>>,
    ) -> Vec<bool> {
        let mut seen = vec![false; self.total()];
        let mut q: Vec<u32> = Vec::new();
        for &s in starts {
            if !blocked[s as usize] && !seen[s as usize] {
                seen[s as usize] = true;
                if let Some(p) = parent.as_deref_mut() {
                    p[s as usize] = s;
                }
                q.push(s);
            }
        }
        let mut head = 0;
        while head < q.len() {
            let u = q[head];
            head += 1;
            for &v in self.succ.of(u) {
                if !seen[v as usize] && !blocked[v as usize] {
                    seen[v as usize] = true;
                    if let Some(p) = parent.as_deref_mut() {
                        p[v as usize] = u;
                    }
                    q.push(v);
                }
            }
        }
        seen
    }

    /// What the uncertainty floor reaches: a search from the floor node that
    /// does not pass data edges ([`Ag::data_edges`]).
    fn floor_reach(&self) -> Vec<bool> {
        let mut seen = vec![false; self.total()];
        let mut q = vec![self.floor];
        seen[self.floor as usize] = true;
        while let Some(u) = q.pop() {
            for &v in self.succ.of(u) {
                if !seen[v as usize] && !self.data_edges.contains(&(u, v)) {
                    seen[v as usize] = true;
                    q.push(v);
                }
            }
        }
        seen
    }

    fn blocked_where(&self, f: impl Fn(u32) -> bool) -> Vec<bool> {
        (0..self.total() as u32).map(f).collect()
    }

    fn label<G: GraphView>(&self, g: &G, n: u32) -> Option<String> {
        Some(match self.virt_of(n) {
            None => g.path(NodeId(n)).to_owned(),
            Some(Virt::Root) => return None,
            Some(Virt::Floor) => "floor".to_owned(),
            Some(Virt::Changed(i)) => {
                let c = &self.changes[i as usize];
                match c.kind {
                    Some(k) => format!("{} {}", k.as_str(), c.label),
                    None => c.label.clone(),
                }
            }
            Some(Virt::Cause(i)) => format!("[{}]", self.cause_labels[i as usize]),
            Some(Virt::Site(_)) => "[open uncertainty]".to_owned(),
            Some(Virt::Steer(_)) => "[steering]".to_owned(),
            Some(Virt::Fw(_, l)) => format!("[{}]", self.fw_labels[l as usize]),
            Some(Virt::Read(_)) => "[reads source]".to_owned(),
            Some(Virt::TypeOnly(_)) => "[type-only]".to_owned(),
            Some(Virt::Undeclared(..)) => "[undeclared dependency]".to_owned(),
            Some(Virt::CrossLang(_)) => "[cross-language]".to_owned(),
            Some(Virt::Via(_)) => "[via test]".to_owned(),
        })
    }

    /// The chain `parent` records from a search start to `n`, start first, as
    /// labels.
    fn chain<G: GraphView>(&self, g: &G, parent: &[u32], n: u32) -> Vec<String> {
        let mut nodes = vec![n];
        let mut x = n;
        while parent[x as usize] != x && parent[x as usize] != NONE {
            x = parent[x as usize];
            nodes.push(x);
        }
        nodes.reverse();
        nodes.iter().filter_map(|&n| self.label(g, n)).collect()
    }
}

/// The dominator tree of the part of the audit graph reachable from the root
/// without the removed nodes (semi-NCA).
struct Dom {
    /// Preorder index → node.
    vertex: Vec<u32>,
    /// Node → preorder index, `NONE` if unreachable.
    pre: Vec<u32>,
    /// Immediate dominator, in preorder indexes (`idom[0] == 0`).
    idom: Vec<u32>,
}

// Invariant: the DFS path is non-empty inside the loop (pushed above).
#[allow(clippy::expect_used)]
fn dominators(ag: &Ag, removed: &[bool]) -> Dom {
    let total = ag.total();
    let mut pre = vec![NONE; total];
    let mut vertex: Vec<u32> = Vec::new();
    let mut parent: Vec<u32> = Vec::new();
    // (node, next edge position)
    let mut stack: Vec<(u32, u32)> = Vec::new();
    pre[ag.root as usize] = 0;
    vertex.push(ag.root);
    parent.push(0);
    stack.push((ag.root, ag.succ.off[ag.root as usize]));
    while let Some(top) = stack.last_mut() {
        let u = top.0;
        if top.1 < ag.succ.off[u as usize + 1] {
            let v = ag.succ.to[top.1 as usize];
            top.1 += 1;
            if !removed[v as usize] && pre[v as usize] == NONE {
                pre[v as usize] = vertex.len() as u32;
                vertex.push(v);
                parent.push(pre[u as usize]);
                stack.push((v, ag.succ.off[v as usize]));
            }
        } else {
            stack.pop();
        }
    }
    let n = vertex.len();
    let mut label: Vec<u32> = (0..n as u32).collect();
    let mut semi: Vec<u32> = label.clone();
    let mut ancestor = parent.clone();
    let mut path: Vec<u32> = Vec::new();
    for w in (1..n).rev() {
        let mut best = w as u32;
        for &p in ag.pred.of(vertex[w]) {
            let v = pre[p as usize];
            if v == NONE {
                continue;
            }
            let l = if v as usize > w {
                // Compress the path from `v` up to the first ancestor that is not linked
                // yet (index <= w), keeping the minimum label.
                path.clear();
                path.push(v);
                loop {
                    let a = ancestor[*path.last().expect("path") as usize];
                    if a as usize > w {
                        path.push(a);
                    } else {
                        break;
                    }
                }
                for i in (0..path.len() - 1).rev() {
                    let (x, u) = (path[i] as usize, path[i + 1] as usize);
                    if label[u] < label[x] {
                        label[x] = label[u];
                    }
                    ancestor[x] = ancestor[u];
                }
                label[v as usize]
            } else {
                v
            };
            if l < best {
                best = l;
            }
        }
        label[w] = best;
        semi[w] = best;
    }
    let mut idom = vec![0u32; n];
    for w in 1..n {
        let mut x = parent[w];
        while x > semi[w] {
            x = idom[x as usize];
        }
        idom[w] = x;
    }
    Dom { vertex, pre, idom }
}

struct Builder {
    real: u32,
    virt: Vec<Virt>,
    virt_ids: HashMap<Virt, u32>,
    edges: Vec<(u32, u32)>,
    groups: Vec<Group>,
    group_ids: HashMap<String, u32>,
    /// Group of each virtual node (parallel to `virt`).
    virt_group: Vec<u32>,
    cause_labels: Vec<String>,
}

impl Builder {
    /// The virtual bridge node `v`, created on first use with the source
    /// `group()` names: `(id, created)`.
    fn virt_with(&mut self, v: Virt, group: impl FnOnce() -> (SourceKind, String)) -> (u32, bool) {
        match self.virt_ids.get(&v) {
            Some(&i) => (i, false),
            None => self.virt(v, Some(group())),
        }
    }

    fn group(&mut self, kind: SourceKind, id: String) -> u32 {
        if let Some(&i) = self.group_ids.get(&id) {
            return i;
        }
        let i = self.groups.len() as u32;
        self.group_ids.insert(id.clone(), i);
        self.groups.push(Group {
            id,
            kind,
            members: Vec::new(),
        });
        i
    }

    /// The virtual node `v`, created on first use: `(id, created)`.
    fn virt(&mut self, v: Virt, group: Option<(SourceKind, String)>) -> (u32, bool) {
        if let Some(&i) = self.virt_ids.get(&v) {
            return (i, false);
        }
        let i = self.real + self.virt.len() as u32;
        self.virt.push(v);
        self.virt_ids.insert(v, i);
        let gi = match group {
            Some((kind, id)) => {
                let gi = self.group(kind, id);
                self.groups[gi as usize].members.push(i);
                gi
            }
            None => NONE,
        };
        self.virt_group.push(gi);
        (i, true)
    }

    /// A virtual node that is always new (cause nodes).
    fn fresh(&mut self, v: Virt, group: Option<(SourceKind, String)>) -> u32 {
        let i = self.real + self.virt.len() as u32;
        self.virt.push(v);
        let gi = match group {
            Some((kind, id)) => {
                let gi = self.group(kind, id);
                self.groups[gi as usize].members.push(i);
                gi
            }
            None => NONE,
        };
        self.virt_group.push(gi);
        i
    }
}

/// The transitive closure of the declared package dependencies: `closure[p]`
/// has bit `q` set when `p` depends on `q` (or `p == q`). Only first-party
/// packages have rows.
struct PackageRel {
    np: usize,
    words: usize,
    bits: Vec<u64>,
    root: Vec<bool>,
    first_party: Vec<bool>,
    /// Per package: the ecosystems its manifests cover (0: any).
    ecosystems: Vec<u8>,
}

impl PackageRel {
    fn new<G: GraphView>(g: &G, deps: &[Vec<PackageId>], ecosystems: Option<&[u8]>) -> PackageRel {
        let np = g.package_count() as usize;
        let words = np.div_ceil(64).max(1);
        let mut root = vec![false; np];
        let mut first_party = vec![false; np];
        for p in 0..np {
            let info = g.package_info(PackageId(p as u32));
            first_party[p] = info.version == PackageVersion::Workspace;
            root[p] = first_party[p] && info.root.is_empty();
        }
        let mut bits = vec![0u64; np * words];
        let mut stack = Vec::new();
        for p in 0..np {
            if !first_party[p] {
                continue;
            }
            let row = p * words;
            bits[row + p / 64] |= 1 << (p % 64);
            stack.clear();
            stack.push(p);
            while let Some(x) = stack.pop() {
                for d in deps.get(x).map(Vec::as_slice).unwrap_or(&[]) {
                    let d = d.0 as usize;
                    if d < np && bits[row + d / 64] & (1 << (d % 64)) == 0 {
                        bits[row + d / 64] |= 1 << (d % 64);
                        stack.push(d);
                    }
                }
            }
        }
        let mut ecos = vec![0u8; np];
        if let Some(e) = ecosystems {
            for (slot, &m) in ecos.iter_mut().zip(e) {
                *slot = m;
            }
        }
        PackageRel {
            np,
            words,
            bits,
            root,
            first_party,
            ecosystems: ecos,
        }
    }

    /// Whether the manifests of package `p` declare dependencies for a file of
    /// ecosystem `eco` (`None`: a file of no ecosystem, which any manifest
    /// covers).
    fn covers(&self, p: u32, eco: Option<Ecosystem>) -> bool {
        let m = self.ecosystems[p as usize];
        m == 0 || eco.is_none_or(|e| m & e.bit() != 0)
    }

    /// Whether package `p` depends on `q` (declared, transitively), or is `q`.
    fn depends(&self, p: u32, q: u32) -> bool {
        let (p, q) = (p as usize, q as usize);
        p < self.np && q < self.np && self.bits[p * self.words + q / 64] & (1 << (q % 64)) != 0
    }

    /// Whether a plain edge from a file of `p` to a file of `q` crosses an
    /// undeclared boundary: both first-party, neither the root package, and
    /// `p` doesn't depend on `q`.
    fn undeclared(&self, p: u32, q: u32) -> bool {
        p != q
            && self.first_party[p as usize]
            && self.first_party[q as usize]
            && !self.root[p as usize]
            && !self.root[q as usize]
            && !self.depends(p, q)
    }

    /// Whether an entry of package `p` is related to a change in package `q`.
    fn related(&self, p: u32, q: u32) -> bool {
        !self.first_party[p as usize]
            || !self.first_party[q as usize]
            || self.root[p as usize]
            || self.root[q as usize]
            || self.depends(p, q)
    }
}

impl<G: GraphView> Engine<'_, G> {
    /// Audits the selection of `entries` for `changes`: per-entry plausibility
    /// signals and the sources of implausible selections, ranked by trigger
    /// (see the [module documentation](crate::audit)).
    ///
    /// `selected` is the selection to explain, normally the affected entries of
    /// [`Engine::affected`] for the same arguments. Entries in it that the
    /// audit graph doesn't reach are counted as
    /// [`AuditReport::unexplained`]; reachable entries not in it (skipped
    /// by barrel pruning) are ignored. Follows the engine's traversal filter.
    // Invariant: signal_counts has a key for every Signal (initialized from Signal::ALL).
    #[allow(clippy::expect_used)]
    pub fn audit(
        &self,
        changes: &ChangeSet,
        entries: &NodeSet,
        selected: &NodeSet,
        opts: &AffectedOptions,
        audit: &AuditOptions<'_>,
    ) -> AuditReport {
        let g = self.graph();
        let t0 = Instant::now();
        let rel = audit
            .package_deps
            .map(|d| PackageRel::new(g, d, audit.package_ecosystems));
        let filter = audit.filter.unwrap_or(self.options().traversal);
        let ag = self.audit_graph(changes, entries, opts, rel.as_ref(), filter);
        let graph_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t1 = Instant::now();

        let entry_list: Vec<NodeId> = entries.iter().collect();
        let total = ag.total();
        let none_blocked = vec![false; total];

        // Signals. Every search starts at the root unless noted.
        let mut parent = vec![NONE; total];
        let all = ag.bfs(&[ag.root], &none_blocked, Some(&mut parent));
        let change_nodes: Vec<u32> = ag.changes.iter().map(|c| c.node).collect();
        // Chains prefer a change over the floor: what a change reaches is explained by
        // it.
        let mut parent_change = vec![NONE; total];
        let by_change = ag.bfs(&change_nodes, &none_blocked, Some(&mut parent_change));
        let by_floor = ag.floor_reach();
        let plain = {
            let b = ag.blocked_where(|n| !ag.plain_ok(n));
            ag.bfs(&[ag.root], &b, None)
        };
        let has = |f: &dyn Fn(Virt) -> bool| ag.virt.iter().any(|v| f(*v));
        let avoid = |f: &dyn Fn(Virt) -> bool| -> Vec<bool> {
            let b = ag.blocked_where(|n| ag.virt_of(n).is_some_and(f));
            ag.bfs(&[ag.root], &b, None)
        };
        let no_type = has(&|v| matches!(v, Virt::TypeOnly(_)))
            .then(|| avoid(&|v| matches!(v, Virt::TypeOnly(_))));
        let no_via =
            has(&|v| matches!(v, Virt::Via(_))).then(|| avoid(&|v| matches!(v, Virt::Via(_))));
        let no_steer =
            has(&|v| matches!(v, Virt::Steer(_))).then(|| avoid(&|v| matches!(v, Virt::Steer(_))));
        let steer_nodes: Vec<u32> = (0..ag.virt.len() as u32)
            .filter(|&i| matches!(ag.virt[i as usize], Virt::Steer(_)))
            .map(|i| ag.real + i)
            .collect();
        let by_steer = (!steer_nodes.is_empty()).then(|| ag.bfs(&steer_nodes, &none_blocked, None));

        // Ecosystems: per ecosystem among the entries, what changes of that ecosystem
        // (or neutral ones) reach, plus what other changes reach over known
        // bridges.
        let eco_of = |n: NodeId| g.lang(n).and_then(Ecosystem::of_lang);
        let mut ecos: Vec<Ecosystem> = entry_list.iter().filter_map(|&e| eco_of(e)).collect();
        ecos.sort_unstable();
        ecos.dedup();
        let legit_blocked = ag.blocked_where(|n| match ag.virt_of(n) {
            None => ag.flags[n as usize] & (F_TASK | F_HUB) != 0,
            Some(v) => !matches!(v, Virt::Changed(_) | Virt::Read(_) | Virt::CrossLang(_)),
        });
        let mut eco_reach: Vec<(Ecosystem, Vec<bool>)> = Vec::new();
        for &eco in &ecos {
            let same: Vec<u32> = ag
                .changes
                .iter()
                .filter(|c| c.eco.is_none_or(|e| e == eco))
                .map(|c| c.node)
                .collect();
            let mut reach = ag.bfs(&same, &none_blocked, None);
            // Known bridges for the other changes: explicit reads and cross-language
            // references anywhere on the path, and a task that has the changed file as a
            // direct input and generates a file (one hop through the task).
            let mut starts: Vec<u32> = Vec::new();
            for c in ag
                .changes
                .iter()
                .filter(|c| c.eco.is_some_and(|e| e != eco))
            {
                starts.push(c.node);
                for &n in ag.succ.of(c.node) {
                    if n >= ag.real {
                        continue;
                    }
                    for &t in ag.succ.of(n) {
                        if t < ag.real && ag.flags[t as usize] & F_TASK != 0 {
                            starts.extend(ag.succ.of(t).iter().copied().filter(|&x| {
                                x < ag.real && ag.flags[x as usize] & (F_TASK | F_HUB) == 0
                            }));
                        }
                    }
                }
            }
            if !starts.is_empty() {
                let legit = ag.bfs(&starts, &legit_blocked, None);
                for (r, l) in reach.iter_mut().zip(&legit) {
                    *r |= *l;
                }
            }
            eco_reach.push((eco, reach));
        }

        // Packages: per changed package, what its changes reach; an entry is related
        // when one of them is in its dependency closure.
        let mut pkg_related = vec![rel.is_none(); total];
        if let Some(rel) = &rel {
            // (global, packages) → the change nodes.
            type Key = (bool, Vec<u32>);
            let mut groups: BTreeMap<Key, Vec<u32>> = BTreeMap::new();
            for c in &ag.changes {
                groups
                    .entry((c.global, c.packages.clone()))
                    .or_default()
                    .push(c.node);
            }
            let mut groups: Vec<(Key, Vec<u32>)> = groups.into_iter().collect();
            // Global changes first, then the packages with the most changed files; beyond
            // the cap the rest counts as related to what it reaches (conservative).
            groups.sort_by(|a, b| b.0.0.cmp(&a.0.0).then(b.1.len().cmp(&a.1.len())));
            const CAP: usize = 64;
            if groups.len() > CAP {
                let rest: Vec<u32> = groups.drain(CAP..).flat_map(|(_, n)| n).collect();
                groups.push(((true, Vec::new()), rest));
            }
            for ((global, pkgs), nodes) in &groups {
                let reach = ag.bfs(nodes, &none_blocked, None);
                for &e in &entry_list {
                    if reach[e.index()]
                        && !pkg_related[e.index()]
                        && (*global || pkgs.iter().any(|&q| rel.related(g.package(e).0, q)))
                    {
                        pkg_related[e.index()] = true;
                    }
                }
            }
            // Entries whose language their package's manifests don't cover: unknown.
            for &e in &entry_list {
                if !rel.covers(g.package(e).0, eco_of(e)) {
                    pkg_related[e.index()] = true;
                }
            }
        }

        // Verdicts.
        let n_changes = ag.changes.len();
        let all_steering = n_changes + audit.ignored_steering.len() > 0
            && ag.changes.iter().all(|c| c.steering.is_some());
        let mut verdicts: Vec<EntryVerdict> = Vec::with_capacity(entry_list.len());
        let mut signal_counts: BTreeMap<&'static str, usize> =
            Signal::ALL.iter().map(|s| (s.as_str(), 0)).collect();
        let (mut n_selected, mut implausible, mut unexplained) = (0, 0, 0);
        let (mut floor_only, mut change_only, mut both) = (0, 0, 0);
        let mut steering = SteeringStats::default();
        for c in ag.changes.iter() {
            if let Some(class) = c.steering {
                *steering.classes.entry(class).or_default() += 1;
                steering.changes += 1;
            }
        }
        for p in audit.ignored_steering {
            *steering
                .classes
                .entry(steering_class(p).unwrap_or("other"))
                .or_default() += 1;
            steering.changes += 1;
            steering.ignored += 1;
        }
        // Targets: selected entries with no plain path, by index into `verdicts`.
        let mut targets: Vec<u32> = Vec::new();
        for &e in &entry_list {
            let i = e.index();
            let eco = eco_of(e);
            let mut a_priori = 0;
            if n_changes > 0
                && let Some(eco) = eco
                && !ag.changes.iter().any(|c| c.eco.is_none_or(|x| x == eco))
            {
                a_priori |= Signal::CrossEcosystem.bit();
            }
            if n_changes > 0
                && let Some(rel) = &rel
                && rel.covers(g.package(e).0, eco)
                && !ag
                    .changes
                    .iter()
                    .any(|c| c.global || c.packages.iter().any(|&q| rel.related(g.package(e).0, q)))
            {
                a_priori |= Signal::NoPackageDep.bit();
            }
            if all_steering {
                a_priori |= Signal::Steering.bit();
            }
            let is_selected = selected.contains(e);
            let mut signals = 0;
            if is_selected {
                n_selected += 1;
                if !all[i] {
                    unexplained += 1;
                } else {
                    match (by_floor[i], by_change[i]) {
                        (true, false) => floor_only += 1,
                        (false, true) => change_only += 1,
                        _ => both += 1,
                    }
                    if by_steer.as_ref().is_some_and(|r| r[i]) {
                        steering.inclusive += 1;
                    }
                    if !plain[i] {
                        signals |= Signal::NonImport.bit();
                        if !by_change[i] {
                            signals |= Signal::Floor.bit();
                            signals |= a_priori
                                & (Signal::CrossEcosystem.bit() | Signal::NoPackageDep.bit());
                        } else {
                            if let Some(eco) = eco
                                && let Some((_, r)) = eco_reach.iter().find(|(x, _)| *x == eco)
                                && !r[i]
                            {
                                signals |= Signal::CrossEcosystem.bit();
                            }
                            if !pkg_related[i] {
                                signals |= Signal::NoPackageDep.bit();
                            }
                        }
                        if no_steer.as_ref().is_some_and(|r| !r[i]) {
                            signals |= Signal::Steering.bit();
                            steering.exclusive += 1;
                        }
                        if no_type.as_ref().is_some_and(|r| !r[i]) {
                            signals |= Signal::TypeOnly.bit();
                        }
                        if no_via.as_ref().is_some_and(|r| !r[i]) {
                            signals |= Signal::ViaTest.bit();
                        }
                        implausible += 1;
                        targets.push(verdicts.len() as u32);
                        for s in Signal::of_mask(signals) {
                            *signal_counts.get_mut(s.as_str()).expect("signal") += 1;
                        }
                    }
                }
            }
            let (mut bridge, mut chain) = (None, Vec::new());
            if is_selected && all[i] {
                let parent = if by_change[i] {
                    &parent_change
                } else {
                    &parent
                };
                let mut x = e.0;
                let mut first = NONE;
                while parent[x as usize] != x {
                    x = parent[x as usize];
                    if ag.group_of[x as usize] != NONE {
                        first = ag.group_of[x as usize];
                    }
                }
                if first != NONE {
                    let gr = &ag.groups[first as usize];
                    bridge = Some((gr.kind, gr.id.clone()));
                }
                if audit.chains {
                    chain = ag.chain(g, parent, e.0);
                }
            }
            verdicts.push(EntryVerdict {
                entry: e,
                selected: is_selected,
                signals,
                a_priori,
                ecosystem: eco,
                bridge,
                source: None,
                chain,
            });
        }
        let signals_ms = t1.elapsed().as_secs_f64() * 1e3;
        let t2 = Instant::now();

        // The greedy order.
        let target_nodes: Vec<u32> = targets
            .iter()
            .map(|&v| verdicts[v as usize].entry.0)
            .collect();
        let mut removed = vec![false; total];
        let mut live = vec![true; target_nodes.len()];
        let mut left = target_nodes.len();
        // (group, freed target indexes, left)
        let mut picks: Vec<(u32, Vec<u32>, usize)> = Vec::new();
        let mut count = vec![0u32; ag.groups.len()];
        let mut depth = vec![0u64; ag.groups.len()];
        let mut stamp = vec![NONE; ag.groups.len()];
        let mut picked = vec![false; ag.groups.len()];
        // Per group: targets with a member among their dominators in the full graph.
        let mut dominated0: Vec<u32> = Vec::new();
        let mut dom = dominators(&ag, &removed);
        loop {
            count.fill(0);
            depth.fill(0);
            stamp.fill(NONE);
            for (ti, &t) in target_nodes.iter().enumerate() {
                if !live[ti] {
                    continue;
                }
                let mut x = dom.idom[dom.pre[t as usize] as usize];
                let mut steps = 1u64;
                while x != 0 {
                    let gi = ag.group_of[dom.vertex[x as usize] as usize];
                    if gi != NONE && stamp[gi as usize] != ti as u32 {
                        stamp[gi as usize] = ti as u32;
                        count[gi as usize] += 1;
                        depth[gi as usize] += steps;
                    }
                    x = dom.idom[x as usize];
                    steps += 1;
                }
            }
            if picks.is_empty() {
                dominated0 = count.clone();
            }
            if picks.len() >= audit.top || left == 0 {
                break;
            }
            // Most dominated entries; on ties the source closest to the entries, then by
            // id.
            let mut best: Option<u32> = None;
            for gi in 0..ag.groups.len() as u32 {
                let i = gi as usize;
                if picked[i] || count[i] == 0 {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some(b) => {
                        let b = b as usize;
                        // depth / count compared by cross-multiplication.
                        (count[i], depth[b] * count[i] as u64, &ag.groups[b].id)
                            > (count[b], depth[i] * count[b] as u64, &ag.groups[i].id)
                    }
                };
                if better {
                    best = Some(gi);
                }
            }
            if best.is_none() {
                best = most_reached(&ag, &dom, &removed, &picked, &target_nodes, &live);
            }
            let Some(gi) = best else { break };
            picked[gi as usize] = true;
            for &m in &ag.groups[gi as usize].members {
                removed[m as usize] = true;
            }
            dom = dominators(&ag, &removed);
            let mut freed = Vec::new();
            for (ti, &t) in target_nodes.iter().enumerate() {
                if live[ti] && dom.pre[t as usize] == NONE {
                    live[ti] = false;
                    freed.push(ti as u32);
                }
            }
            left -= freed.len();
            picks.push((gi, freed, left));
        }
        let greedy_ms = t2.elapsed().as_secs_f64() * 1e3;
        let t3 = Instant::now();

        // Without the floor (`--on-uncertain ignore`): the sources that dominate the
        // entries a change reaches.
        let mut no_floor = vec![false; total];
        no_floor[ag.floor as usize] = true;
        let dom_nf = dominators(&ag, &no_floor);
        count.fill(0);
        stamp.fill(NONE);
        for (ti, &t) in target_nodes.iter().enumerate() {
            let p = dom_nf.pre[t as usize];
            if p == NONE {
                continue;
            }
            let mut x = dom_nf.idom[p as usize];
            while x != 0 {
                let gi = ag.group_of[dom_nf.vertex[x as usize] as usize];
                if gi != NONE && stamp[gi as usize] != ti as u32 {
                    stamp[gi as usize] = ti as u32;
                    count[gi as usize] += 1;
                }
                x = dom_nf.idom[x as usize];
            }
        }
        for (d, c) in dominated0.iter_mut().zip(&count) {
            *d = (*d).max(*c);
        }
        drop(dom_nf);

        // Reported sources: the picks, then the sources that dominate the most entries
        // alone (with or without the floor), and every steering source.
        let mut reported: Vec<u32> = picks.iter().map(|p| p.0).collect();
        let mut others: Vec<u32> = (0..ag.groups.len() as u32)
            .filter(|&gi| !picked[gi as usize] && dominated0[gi as usize] > 0)
            .collect();
        others.sort_by(|&a, &b| {
            dominated0[b as usize]
                .cmp(&dominated0[a as usize])
                .then_with(|| ag.groups[a as usize].id.cmp(&ag.groups[b as usize].id))
        });
        others.truncate(audit.top);
        reported.extend(others);
        for gi in 0..ag.groups.len() as u32 {
            if ag.groups[gi as usize].kind == SourceKind::Steering && !reported.contains(&gi) {
                reported.push(gi);
            }
        }
        let mut sources: Vec<AuditSource> = Vec::with_capacity(reported.len());
        let mut blocked = vec![false; total];
        let mut from = vec![NONE; total];
        for &gi in &reported {
            let gr = &ag.groups[gi as usize];
            // Inclusive: what the members reach.
            from.fill(NONE);
            let through = ag.bfs(&gr.members, &none_blocked, Some(&mut from));
            // Exclusive: what the root no longer reaches without them.
            for &m in &gr.members {
                blocked[m as usize] = true;
            }
            let without = ag.bfs(&[ag.root], &blocked, None);
            // The same without the floor: what the changes no longer reach without them.
            let without_nf = ag.bfs(&change_nodes, &blocked, None);
            for &m in &gr.members {
                blocked[m as usize] = false;
            }
            let pick = picks.iter().enumerate().find(|(_, p)| p.0 == gi);
            let mut signals: BTreeMap<&'static str, usize> = BTreeMap::new();
            let (mut exclusive, mut inclusive, mut exclusive_no_floor) = (0, 0, 0);
            let mut attributed: Vec<u32> = Vec::new();
            let mut by_change_only: Vec<u32> = Vec::new();
            for (ti, &t) in target_nodes.iter().enumerate() {
                if through[t as usize] {
                    inclusive += 1;
                }
                if !without[t as usize] {
                    exclusive += 1;
                    attributed.push(ti as u32);
                }
                if by_change[t as usize] && !without_nf[t as usize] {
                    exclusive_no_floor += 1;
                    by_change_only.push(ti as u32);
                }
            }
            if let Some((rank, (_, freed, _))) = pick {
                for &ti in freed {
                    verdicts[targets[ti as usize] as usize].source = Some(rank);
                }
                attributed.extend(freed);
                attributed.sort_unstable();
                attributed.dedup();
            }
            if attributed.is_empty() {
                attributed = by_change_only;
            }
            if attributed.is_empty() {
                attributed = (0..target_nodes.len() as u32)
                    .filter(|&ti| through[target_nodes[ti as usize] as usize])
                    .collect();
            }
            for &ti in &attributed {
                for s in Signal::of_mask(verdicts[targets[ti as usize] as usize].signals) {
                    *signals.entry(s.as_str()).or_default() += 1;
                }
            }
            // An example: a change (else the floor) → member → an attributed entry.
            let mut example = Vec::new();
            if let Some(&ti) = attributed.first() {
                let t = target_nodes[ti as usize];
                let mut tail = vec![t];
                let mut x = t;
                while from[x as usize] != x && from[x as usize] != NONE {
                    x = from[x as usize];
                    tail.push(x);
                }
                tail.reverse();
                // `x` is the member the chain starts at.
                let mut head = if by_change[x as usize] {
                    ag.chain(g, &parent_change, x)
                } else {
                    ag.chain(g, &parent, x)
                };
                head.pop();
                example = head;
                example.extend(tail.iter().filter_map(|&n| ag.label(g, n)));
            }
            // Changes that enter through the source: backwards from the members.
            let mut seen = vec![false; total];
            let mut q: Vec<u32> = gr.members.clone();
            for &m in &q {
                seen[m as usize] = true;
            }
            let mut head = 0;
            let mut entering: Vec<u32> = Vec::new();
            while head < q.len() {
                let u = q[head];
                head += 1;
                if let Some(Virt::Changed(i)) = ag.virt_of(u) {
                    entering.push(i);
                    continue;
                }
                for &p in ag.pred.of(u) {
                    if !seen[p as usize] {
                        seen[p as usize] = true;
                        q.push(p);
                    }
                }
            }
            entering.sort_unstable();
            sources.push(AuditSource {
                id: gr.id.clone(),
                kind: gr.kind,
                members: gr.members.len(),
                exclusive,
                exclusive_no_floor,
                inclusive,
                rank: pick.map(|(r, _)| r + 1),
                freed: pick.map(|(_, p)| p.1.len()),
                left: pick.map(|(_, p)| p.2),
                signals,
                example,
                changes: entering
                    .iter()
                    .take(5)
                    .map(|&i| ag.changes[i as usize].label.clone())
                    .collect(),
                changes_total: entering.len(),
            });
        }
        // Sources beyond the picks: by exclusive count (exact now), then inclusive.
        let n_picks = picks.len();
        sources[n_picks..].sort_by(|a, b| {
            (b.exclusive.max(b.exclusive_no_floor), b.inclusive)
                .cmp(&(a.exclusive.max(a.exclusive_no_floor), a.inclusive))
                .then_with(|| a.id.cmp(&b.id))
        });
        // Per kind: every source of the kind removed at once.
        let mut kinds: Vec<KindTotal> = Vec::new();
        for &kind in SourceKind::ALL {
            let members: Vec<u32> = ag
                .groups
                .iter()
                .filter(|gr| gr.kind == kind)
                .flat_map(|gr| gr.members.iter().copied())
                .collect();
            if members.is_empty() {
                continue;
            }
            let through = ag.bfs(&members, &none_blocked, None);
            for &m in &members {
                blocked[m as usize] = true;
            }
            let without = ag.bfs(&[ag.root], &blocked, None);
            let without_nf = ag.bfs(&change_nodes, &blocked, None);
            for &m in &members {
                blocked[m as usize] = false;
            }
            let mut k = KindTotal {
                kind,
                sources: ag.groups.iter().filter(|gr| gr.kind == kind).count(),
                exclusive: 0,
                exclusive_no_floor: 0,
                inclusive: 0,
            };
            for &t in &target_nodes {
                k.inclusive += usize::from(through[t as usize]);
                k.exclusive += usize::from(!without[t as usize]);
                k.exclusive_no_floor +=
                    usize::from(by_change[t as usize] && !without_nf[t as usize]);
            }
            if k.inclusive > 0 {
                kinds.push(k);
            }
        }
        kinds.sort_by(|a, b| {
            (b.exclusive.max(b.exclusive_no_floor), b.inclusive)
                .cmp(&(a.exclusive.max(a.exclusive_no_floor), a.inclusive))
                .then(a.kind.cmp(&b.kind))
        });
        let sources_ms = t3.elapsed().as_secs_f64() * 1e3;

        AuditReport {
            entries: entry_list.len(),
            selected: n_selected,
            implausible,
            unexplained,
            floor_only,
            change_only,
            floor_and_change: both,
            signal_counts,
            kinds,
            sources,
            left,
            steering,
            verdicts,
            changes: n_changes + audit.ignored_steering.len(),
            graph: (total, ag.succ.to.len()),
            timing: AuditTiming {
                graph_ms,
                signals_ms,
                greedy_ms,
                sources_ms,
            },
        }
    }

    /// Builds the audit graph: the part of the selection graph the root
    /// reaches.
    fn audit_graph(
        &self,
        changes: &ChangeSet,
        entries: &NodeSet,
        opts: &AffectedOptions,
        rel: Option<&PackageRel>,
        filter: EdgeFilter,
    ) -> Ag {
        let g = self.graph();
        let real = g.node_count();
        // Per-node flags and languages.
        let mut flags = vec![0u8; real as usize];
        let mut lang_ids: HashMap<&str, u8> = HashMap::new();
        let mut langs = vec![u8::MAX; real as usize];
        for i in 0..real {
            let n = NodeId(i);
            let mut f = 0;
            if let Some(l) = g.lang(n) {
                if l == TASK_LANG {
                    f |= F_TASK;
                } else if l == HUB_LANG {
                    f |= F_HUB;
                } else {
                    f |= F_CODE;
                    let next = lang_ids.len() as u8;
                    langs[i as usize] = *lang_ids.entry(l).or_insert(next);
                }
            }
            if !g.is_external(n) {
                f |= F_FIRST_PARTY;
            }
            if entries.contains(n) {
                f |= F_ENTRY;
            }
            flags[i as usize] = f;
        }

        let mut b = Builder {
            real,
            virt: Vec::new(),
            virt_ids: HashMap::new(),
            edges: Vec::new(),
            groups: Vec::new(),
            group_ids: HashMap::new(),
            virt_group: Vec::new(),
            cause_labels: Vec::new(),
        };
        let mut fw_labels: Vec<String> = Vec::new();
        let (root, _) = b.virt(Virt::Root, None);
        let (floor, _) = b.virt(Virt::Floor, Some((SourceKind::Floor, "floor".to_owned())));

        // Framework edges of the entries: (test, target) → label.
        let mut fw_edges: HashMap<(u32, u32), u16> = HashMap::new();
        let mut fw_ids: HashMap<String, u16> = HashMap::new();
        for e in entries.iter() {
            for edge in g.out_edges(e) {
                if let (Some(to), Some(spec)) = (edge.to, edge.specifier)
                    && let Some(label) = framework_role(spec)
                {
                    let id = match fw_ids.get(label) {
                        Some(&i) => i,
                        None => {
                            let i = fw_labels.len() as u16;
                            fw_labels.push(label.to_owned());
                            fw_ids.insert(label.to_owned(), i);
                            i
                        }
                    };
                    fw_edges.insert((e.0, to.0), id);
                }
            }
        }

        // Seeds.
        let mut queue: Vec<u32> = Vec::new();
        let mut reached = vec![false; real as usize];
        let mut reach = |n: u32, queue: &mut Vec<u32>| {
            if !reached[n as usize] {
                reached[n as usize] = true;
                queue.push(n);
            }
        };
        let mut recs: Vec<ChangeRec> = Vec::new();
        // Steering-changed nodes → their steering node.
        let mut steer_of: HashMap<u32, u32> = HashMap::new();
        let pkg_roots: HashMap<&str, u32> = (0..g.package_count())
            .filter_map(|p| {
                let info = g.package_info(PackageId(p));
                (info.version == PackageVersion::Workspace).then_some((info.root, p))
            })
            .collect();
        let package_of_path = |path: &str| -> Option<u32> {
            if let Some(n) = g.node_by_path(path) {
                return Some(g.package(n).0);
            }
            let mut dir = path;
            loop {
                dir = dir.rsplit_once('/').map_or("", |x| x.0);
                if let Some(&p) = pkg_roots.get(dir) {
                    return Some(p);
                }
                if dir.is_empty() {
                    return None;
                }
            }
        };
        // The changed files, then the directories the change made or removed
        // (`ChangeSet::dirs`): a change each.
        let files = changes.files.iter().map(|c| (c, false));
        for (c, dir) in files.chain(changes.dirs.iter().map(|c| (c, true))) {
            let i = recs.len() as u32;
            let (node, _) = b.virt(Virt::Changed(i), None);
            b.edges.push((root, node));
            let steering = steering_class(&c.path);
            let pkg = package_of_path(&c.path);
            let eco = Ecosystem::of_path(&c.path);
            let global = match (pkg, rel) {
                (Some(p), Some(rel)) => {
                    rel.root[p as usize] || !rel.first_party[p as usize] || !rel.covers(p, eco)
                }
                _ => true,
            };
            recs.push(ChangeRec {
                label: c.path.clone(),
                kind: Some(c.kind),
                eco,
                steering,
                packages: pkg.into_iter().collect(),
                global,
                node,
            });
            let steer = steering.map(|class| {
                let (s, _) = b.virt(
                    Virt::Steer(i),
                    Some((SourceKind::Steering, format!("steering:{class}"))),
                );
                b.edges.push((node, s));
                s
            });
            let one = vec![Change {
                path: c.path.clone(),
                kind: c.kind,
            }];
            let single = if dir {
                ChangeSet {
                    dirs: one,
                    ..ChangeSet::default()
                }
            } else {
                ChangeSet {
                    files: one,
                    spared: changes.spared.clone(),
                    ..ChangeSet::default()
                }
            };
            let (seeds, _) = self.seeds(&single, opts.after_change, filter);
            let own = g.node_by_path(&c.path);
            // Cause nodes of this change, by source id.
            let mut causes: HashMap<String, u32> = HashMap::new();
            for (n, cause) in seeds {
                let (kind, id, label) = match &cause {
                    Cause::Modified { .. } | Cause::Deleted { .. } if own == Some(n) => {
                        b.edges.push((node, n.0));
                        if let Some(s) = steer {
                            steer_of.insert(n.0, s);
                        }
                        reach(n.0, &mut queue);
                        continue;
                    }
                    Cause::Package { .. } => {
                        b.edges.push((node, n.0));
                        reach(n.0, &mut queue);
                        continue;
                    }
                    Cause::Modified { path } | Cause::Deleted { path } => (
                        SourceKind::Input,
                        format!("input:{path}"),
                        "read or probed".to_owned(),
                    ),
                    Cause::Probe { added, .. } => (
                        SourceKind::Probe,
                        format!("probe:{added}"),
                        "probed".to_owned(),
                    ),
                    Cause::Membership { dir, pattern, .. } => {
                        let s = crate::output::join_scope(dir, pattern);
                        (
                            SourceKind::Listing,
                            format!("listing:{s}"),
                            format!("listing {s}"),
                        )
                    }
                    Cause::Scope { dir, pattern, .. } => {
                        let s = crate::output::join_scope(dir, pattern);
                        (
                            SourceKind::Scope,
                            format!("scope:{s}"),
                            format!("scope {s}"),
                        )
                    }
                    // Named changes are added after the engine's pass; seeds never carry one.
                    Cause::Named { path, by, .. } => (
                        SourceKind::Scope,
                        format!("named:{path}"),
                        format!("named by {by}"),
                    ),
                };
                let cn = match causes.get(&id) {
                    Some(&cn) => cn,
                    None => {
                        let ci = b.cause_labels.len() as u32;
                        b.cause_labels.push(label);
                        let cn = b.fresh(Virt::Cause(ci), Some((kind, id.clone())));
                        // A steering file's indirect causes hang below its steering node.
                        b.edges.push((steer.unwrap_or(node), cn));
                        causes.insert(id, cn);
                        cn
                    }
                };
                b.edges.push((cn, n.0));
                reach(n.0, &mut queue);
            }
        }
        // Changed packages: one change each.
        let pkg_changes = changes.packages.iter().map(|&p| (p, None)).chain(
            changes
                .scoped
                .iter()
                .map(|s| (s.package, Some(&s.importers))),
        );
        for (p, importers) in pkg_changes {
            let i = recs.len() as u32;
            let (node, _) = b.virt(Virt::Changed(i), None);
            b.edges.push((root, node));
            let info = g.package_info(p);
            let eco = match info.name.split_once(':') {
                Some(("pypi", _)) => Ecosystem::Python,
                Some(("go", _)) => Ecosystem::Go,
                Some(("crates", _)) => Ecosystem::Rust,
                _ => Ecosystem::Js,
            };
            recs.push(ChangeRec {
                label: format!("package {}@{}", info.name, info.version.as_str()),
                kind: None,
                eco: Some(eco),
                steering: None,
                packages: importers
                    .map(|v| v.iter().map(|p| p.0).collect())
                    .unwrap_or_default(),
                global: importers.is_none(),
                node,
            });
            let mut single = ChangeSet::default();
            match importers {
                Some(v) => single.push_scoped(p, v.clone()),
                None => single.push_package(p),
            };
            let (seeds, _) = self.seeds(&single, opts.after_change, filter);
            for (n, _) in seeds {
                b.edges.push((node, n.0));
                reach(n.0, &mut queue);
            }
        }
        // What the changes reach (everything queued so far is a change seed).
        // Uncertainty does not propagate along data edges
        // (`EdgeFilter::for_uncertainty`), so below a data edge is followed
        // only from a node a change reaches, and the floor's own reach skips it
        // (`Ag::floor_reach`). In the attribution (dominators), the floor still
        // shares such an edge with the change that passes it.
        let rev = self.reverse();
        let all = filter.is_all();
        let mut changed = vec![false; real as usize];
        {
            let mut stack = queue.clone();
            for &n in &stack {
                changed[n as usize] = true;
            }
            while let Some(n) = stack.pop() {
                let (srcs, attrs) = rev.neighbors(NodeId(n));
                for (&d, &a) in srcs.iter().zip(attrs) {
                    if (all || filter.accepts(a)) && !changed[d as usize] {
                        changed[d as usize] = true;
                        stack.push(d);
                    }
                }
            }
        }
        // The floor.
        b.edges.push((root, floor));
        if opts.policy != crate::change::Policy::Ignore {
            for &n in self.uncertain_nodes() {
                if !g.is_external(n) && g.uncertainties(n).any(|u| opts.triggers(&u, g.lang(n))) {
                    let (s, _) = b.virt(
                        Virt::Site(n.0),
                        Some((SourceKind::Uncertain, format!("uncertain:{}", g.path(n)))),
                    );
                    b.edges.push((floor, s));
                    b.edges.push((s, n.0));
                    reach(n.0, &mut queue);
                }
            }
        }

        // The reached part of the reverse graph, with bridges on non-plain edges.
        let mut data_edges: HashSet<(u32, u32)> = HashSet::new();
        let mut head = 0;
        while head < queue.len() {
            let n = queue[head];
            head += 1;
            let nf = flags[n as usize];
            let (srcs, attrs) = rev.neighbors(NodeId(n));
            for (&d, &a) in srcs.iter().zip(attrs) {
                if !(all || filter.accepts(a)) {
                    continue;
                }
                if a.flags().contains(RefFlags::DATA) {
                    if !changed[n as usize] {
                        continue;
                    }
                    data_edges.insert((n, d));
                }
                let df = flags[d as usize];
                reach(d, &mut queue);
                let pseudo = (nf | df) & (F_TASK | F_HUB) != 0;
                if pseudo {
                    // A steering file reaching a task or hub goes through its steering node.
                    if df & (F_TASK | F_HUB) != 0
                        && let Some(&s) = steer_of.get(&n)
                    {
                        b.edges.push((n, s));
                        b.edges.push((s, d));
                    } else {
                        b.edges.push((n, d));
                    }
                    continue;
                }
                let kind = a.kind();
                let read = matches!(
                    kind,
                    RefKind::FsRead | RefKind::FsReadDir | RefKind::Asset | RefKind::Embed
                );
                // The bridge of a non-plain edge: its node, kind, and whether it groups the
                // edges by their target `n` or by their reader `d`.
                let bridge = if nf & F_ENTRY != 0 {
                    Some((Virt::Via(n), SourceKind::ViaTest, true))
                } else if a.flags().contains(RefFlags::TYPE_ONLY) {
                    Some((Virt::TypeOnly(n), SourceKind::TypeOnly, true))
                } else if let Some(&l) = fw_edges.get(&(d, n)) {
                    Some((Virt::Fw(n, l), SourceKind::Framework, true))
                } else if read && nf & F_CODE != 0 {
                    Some((Virt::Read(d), SourceKind::Read, false))
                } else if nf & df & F_CODE != 0
                    && nf & df & F_FIRST_PARTY != 0
                    && langs[n as usize] != langs[d as usize]
                {
                    Some((Virt::CrossLang(d), SourceKind::CrossLang, false))
                } else if let Some(rel) = rel
                    && !read
                    && nf & df & F_CODE != 0
                    && rel.undeclared(g.package(NodeId(d)).0, g.package(NodeId(n)).0)
                    && rel.covers(g.package(NodeId(d)).0, eco_of_node(g, NodeId(d)))
                    && rel.covers(g.package(NodeId(n)).0, eco_of_node(g, NodeId(n)))
                {
                    let pd = g.package(NodeId(d)).0;
                    Some((Virt::Undeclared(n, pd), SourceKind::Undeclared, true))
                } else {
                    None
                };
                let Some((virt, kind, by_target)) = bridge else {
                    b.edges.push((n, d));
                    continue;
                };
                let (v, created) = b.virt_with(virt, || {
                    let id = match virt {
                        Virt::Via(n) => format!("via-test:{}", g.path(NodeId(n))),
                        Virt::TypeOnly(n) => format!("type-only:{}", g.path(NodeId(n))),
                        Virt::Fw(n, l) => {
                            format!("{} {}", fw_labels[l as usize], g.path(NodeId(n)))
                        }
                        Virt::Read(d) => format!("read-by:{}", g.path(NodeId(d))),
                        Virt::CrossLang(d) => format!("cross-lang:{}", g.path(NodeId(d))),
                        Virt::Undeclared(n, pd) => format!(
                            "undeclared:{} -> {}",
                            g.package_info(PackageId(pd)).root,
                            g.package_info(g.package(NodeId(n))).root
                        ),
                        _ => unreachable!("not an edge bridge"),
                    };
                    (kind, id)
                });
                // By target: n → v once, v → d per edge. By reader: v → d once.
                if by_target {
                    if created {
                        b.edges.push((n, v));
                    }
                    b.edges.push((v, d));
                } else {
                    b.edges.push((n, v));
                    if created {
                        b.edges.push((v, d));
                    }
                }
            }
        }

        // Task and hub nodes are bridges themselves.
        let total = (real as usize) + b.virt.len();
        let mut group_of = vec![NONE; total];
        for (i, &gi) in b.virt_group.iter().enumerate() {
            group_of[real as usize + i] = gi;
        }
        for &n in &queue {
            let f = flags[n as usize];
            if f & (F_TASK | F_HUB) != 0 {
                let kind = if f & F_TASK != 0 {
                    SourceKind::Task
                } else {
                    SourceKind::Members
                };
                let gi = b.group(kind, g.path(NodeId(n)).to_owned());
                b.groups[gi as usize].members.push(n);
                group_of[n as usize] = gi;
            }
        }
        b.edges.sort_unstable();
        b.edges.dedup();
        let succ = Csr::build(total, &b.edges, false);
        let pred = Csr::build(total, &b.edges, true);
        Ag {
            real,
            virt: b.virt,
            root,
            floor,
            succ,
            pred,
            group_of,
            groups: b.groups,
            changes: recs,
            flags,
            fw_labels,
            cause_labels: b.cause_labels,
            data_edges,
        }
    }
}

fn eco_of_node<G: GraphView>(g: &G, n: NodeId) -> Option<Ecosystem> {
    g.lang(n).and_then(Ecosystem::of_lang)
}

/// The unpicked source that the most live targets reach, estimated on a sample
/// of up to 64 of them: per node, the sampled targets it reaches, propagated to
/// a fixpoint.
fn most_reached(
    ag: &Ag,
    dom: &Dom,
    removed: &[bool],
    picked: &[bool],
    targets: &[u32],
    live: &[bool],
) -> Option<u32> {
    let alive: Vec<u32> = targets
        .iter()
        .zip(live)
        .filter(|(_, l)| **l)
        .map(|(t, _)| *t)
        .collect();
    if alive.is_empty() {
        return None;
    }
    let step = alive.len().div_ceil(64);
    let mut mask = vec![0u64; ag.total()];
    for (bit, t) in alive.iter().step_by(step).enumerate() {
        mask[*t as usize] |= 1 << bit;
    }
    // Reverse preorder visits most successors before their predecessors.
    for _ in 0..16 {
        let mut changed = false;
        for &u in dom.vertex.iter().rev() {
            let mut m = mask[u as usize];
            for &v in ag.succ.of(u) {
                if !removed[v as usize] {
                    m |= mask[v as usize];
                }
            }
            if m != mask[u as usize] {
                mask[u as usize] = m;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Most sampled targets reached; on ties the smaller id.
    let mut best: Option<(u32, u32)> = None;
    for (gi, gr) in ag.groups.iter().enumerate() {
        if picked[gi] {
            continue;
        }
        let m = gr
            .members
            .iter()
            .filter(|&&n| dom.pre[n as usize] != NONE)
            .fold(0u64, |m, &n| m | mask[n as usize]);
        let c = m.count_ones();
        let better = match best {
            None => c > 0,
            Some((bc, bg)) => c > bc || (c == bc && gr.id < ag.groups[bg as usize].id),
        };
        if better {
            best = Some((c, gi as u32));
        }
    }
    best.map(|(_, gi)| gi)
}

// -------------------------------------------------------------------------------------
// Output
// -------------------------------------------------------------------------------------

/// Which entries the JSON lists.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum VerdictFilter {
    /// Every entry.
    #[default]
    All,
    /// Selected entries.
    Selected,
    /// Entries with a signal.
    Flagged,
    /// None.
    None,
}

impl VerdictFilter {
    /// Parses `all`, `selected`, `flagged` or `none`.
    pub fn parse(s: &str) -> Option<VerdictFilter> {
        Some(match s {
            "all" => VerdictFilter::All,
            "selected" => VerdictFilter::Selected,
            "flagged" => VerdictFilter::Flagged,
            "none" => VerdictFilter::None,
            _ => return None,
        })
    }
}

impl AuditReport {
    /// Adds the report of another pass: other entries of the same query,
    /// audited with another traversal filter ([`AuditOptions::filter`]).
    /// Counts add up; sources with the same id are added; the greedy picks
    /// of both passes are ordered by the entries they freed, and `left`
    /// counts the implausible entries not freed by a source and the ones
    /// above it, each in its own pass's order.
    pub fn merge(mut self, other: AuditReport) -> AuditReport {
        // Verdicts name their source by index: remember the ids.
        let ids = |r: &AuditReport| -> Vec<Option<String>> {
            r.verdicts
                .iter()
                .map(|v| v.source.map(|s| r.sources[s].id.clone()))
                .collect()
        };
        let (mine, theirs) = (ids(&self), ids(&other));
        self.entries += other.entries;
        self.selected += other.selected;
        self.implausible += other.implausible;
        self.unexplained += other.unexplained;
        self.floor_only += other.floor_only;
        self.change_only += other.change_only;
        self.floor_and_change += other.floor_and_change;
        self.left += other.left;
        self.changes = self.changes.max(other.changes);
        self.graph = (
            self.graph.0.max(other.graph.0),
            self.graph.1.max(other.graph.1),
        );
        self.timing.graph_ms += other.timing.graph_ms;
        self.timing.signals_ms += other.timing.signals_ms;
        self.timing.greedy_ms += other.timing.greedy_ms;
        self.timing.sources_ms += other.timing.sources_ms;
        for (k, n) in other.signal_counts {
            *self.signal_counts.entry(k).or_default() += n;
        }
        // The change set is the same in both passes.
        for (k, n) in other.steering.classes {
            let e = self.steering.classes.entry(k).or_default();
            *e = (*e).max(n);
        }
        self.steering.changes = self.steering.changes.max(other.steering.changes);
        self.steering.ignored = self.steering.ignored.max(other.steering.ignored);
        self.steering.inclusive += other.steering.inclusive;
        self.steering.exclusive += other.steering.exclusive;
        for k in other.kinds {
            match self.kinds.iter_mut().find(|x| x.kind == k.kind) {
                Some(x) => {
                    x.sources = x.sources.max(k.sources);
                    x.exclusive += k.exclusive;
                    x.exclusive_no_floor += k.exclusive_no_floor;
                    x.inclusive += k.inclusive;
                }
                None => self.kinds.push(k),
            }
        }
        self.kinds.sort_by(|a, b| {
            (b.exclusive.max(b.exclusive_no_floor), b.inclusive)
                .cmp(&(a.exclusive.max(a.exclusive_no_floor), a.inclusive))
                .then(a.kind.cmp(&b.kind))
        });
        for s in other.sources {
            match self.sources.iter_mut().find(|x| x.id == s.id) {
                Some(x) => {
                    if s.exclusive.max(s.exclusive_no_floor) > x.exclusive.max(x.exclusive_no_floor)
                        || x.example.is_empty()
                    {
                        x.example = s.example;
                    }
                    x.members = x.members.max(s.members);
                    x.exclusive += s.exclusive;
                    x.exclusive_no_floor += s.exclusive_no_floor;
                    x.inclusive += s.inclusive;
                    x.freed = match (x.freed, s.freed) {
                        (None, None) => None,
                        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
                    };
                    for (k, n) in s.signals {
                        *x.signals.entry(k).or_default() += n;
                    }
                    for c in s.changes {
                        if x.changes.len() < 5 && !x.changes.contains(&c) {
                            x.changes.push(c);
                        }
                    }
                    x.changes_total = x.changes_total.max(s.changes_total);
                }
                None => self.sources.push(s),
            }
        }
        // Picks first, by the entries they freed; then the rest by exclusive count.
        self.sources.sort_by(|a, b| {
            let key = |s: &AuditSource| {
                (
                    s.freed.is_some(),
                    s.freed.unwrap_or(0),
                    s.exclusive.max(s.exclusive_no_floor),
                    s.inclusive,
                )
            };
            key(b).cmp(&key(a)).then_with(|| a.id.cmp(&b.id))
        });
        let mut left = self.implausible;
        let mut rank = 0;
        for s in &mut self.sources {
            if let Some(f) = s.freed {
                rank += 1;
                left = left.saturating_sub(f);
                s.rank = Some(rank);
                s.left = Some(left);
            }
        }
        let index: HashMap<&str, usize> = self
            .sources
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.as_str(), i))
            .collect();
        for (v, id) in self.verdicts.iter_mut().zip(&mine) {
            v.source = id.as_deref().and_then(|id| index.get(id).copied());
        }
        let mut theirs_v = other.verdicts;
        for (v, id) in theirs_v.iter_mut().zip(&theirs) {
            v.source = id.as_deref().and_then(|id| index.get(id).copied());
        }
        self.verdicts.extend(theirs_v);
        self.verdicts.sort_by_key(|v| v.entry);
        self
    }

    /// The report as JSON (`version` 1): the counts, `signals`, `steering`,
    /// `sources` ([`AuditSource`]) and `entries`, one object per entry that
    /// passes `filter`: `entry`, `selected`, `plausible` (selected with no
    /// signal), `signals`, `a_priori` (`cross_ecosystem`, `no_package_dep`,
    /// `steering_only`: the relation of the entry to the whole diff, also
    /// for unselected entries), `ecosystem`, `bridge` (`kind` and
    /// `source` of the first bridge on the witness chain), `source` (the id of
    /// the greedy pick that unselects it) and `chain`.
    // Invariant: `o` is a `json!` object literal.
    #[allow(clippy::expect_used)]
    pub fn to_value<G: GraphView>(&self, g: &G, filter: VerdictFilter) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = self
            .verdicts
            .iter()
            .filter(|v| match filter {
                VerdictFilter::All => true,
                VerdictFilter::Selected => v.selected,
                VerdictFilter::Flagged => v.signals != 0,
                VerdictFilter::None => false,
            })
            .map(|v| {
                let mut o = serde_json::json!({
                    "entry": g.path(v.entry),
                    "selected": v.selected,
                    "plausible": v.selected && v.signals == 0,
                    "signals": Signal::of_mask(v.signals).map(Signal::as_str).collect::<Vec<_>>(),
                    "a_priori": {
                        "cross_ecosystem": v.a_priori & Signal::CrossEcosystem.bit() != 0,
                        "no_package_dep": v.a_priori & Signal::NoPackageDep.bit() != 0,
                        "steering_only": v.a_priori & Signal::Steering.bit() != 0,
                    },
                    "ecosystem": v.ecosystem,
                });
                let m = o.as_object_mut().expect("object");
                if let Some((kind, id)) = &v.bridge {
                    m.insert(
                        "bridge".into(),
                        serde_json::json!({ "kind": kind, "source": id }),
                    );
                }
                if let Some(s) = v.source {
                    m.insert("source".into(), self.sources[s].id.clone().into());
                }
                if !v.chain.is_empty() {
                    m.insert("chain".into(), v.chain.clone().into());
                }
                o
            })
            .collect();
        serde_json::json!({
            "version": 1,
            "entries_total": self.entries,
            "selected": self.selected,
            "implausible": self.implausible,
            "plausible": self.selected - self.implausible - self.unexplained,
            "unexplained": self.unexplained,
            "floor_only": self.floor_only,
            "change_only": self.change_only,
            "floor_and_change": self.floor_and_change,
            "changes": self.changes,
            "signals": self.signal_counts,
            "steering": self.steering,
            "kinds": self.kinds,
            "sources": self.sources,
            "left": self.left,
            "graph": { "nodes": self.graph.0, "edges": self.graph.1 },
            "timing_ms": self.timing,
            "entries": entries,
        })
    }

    /// Writes the readable report: the counts and the ranked sources.
    pub fn write_text(&self, out: &mut dyn Write) -> io::Result<()> {
        let pct = |n: usize, d: usize| {
            if d == 0 {
                0.0
            } else {
                100.0 * n as f64 / d as f64
            }
        };
        writeln!(
            out,
            "audit: {} of {} entries selected ({:.1}%); {} implausible ({:.1}% of the selection): \
             no path of ordinary imports from a changed file",
            self.selected,
            self.entries,
            pct(self.selected, self.entries),
            self.implausible,
            pct(self.implausible, self.selected),
        )?;
        writeln!(
            out,
            "selected by: the uncertainty floor only {}, a change only {}, both {}{}",
            self.floor_only,
            self.change_only,
            self.floor_and_change,
            if self.unexplained > 0 {
                format!(", unexplained {}", self.unexplained)
            } else {
                String::new()
            }
        )?;
        let sig: Vec<String> = self
            .signal_counts
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(s, n)| format!("{s} {n}"))
            .collect();
        writeln!(
            out,
            "signals: {}",
            if sig.is_empty() {
                "none".to_owned()
            } else {
                sig.join(", ")
            }
        )?;
        if self.steering.changes > 0 {
            let classes: Vec<String> = self
                .steering
                .classes
                .iter()
                .map(|(c, n)| format!("{c} {n}"))
                .collect();
            if self.steering.ignored == self.steering.changes {
                writeln!(
                    out,
                    "steering files: {} changed ({}); the selection counts them only through \
                     explicit references",
                    self.steering.changes,
                    classes.join(", "),
                )?;
            } else {
                writeln!(
                    out,
                    "steering files: {} changed ({}); they reach {} selected entr(ies), {} only \
                     through them",
                    self.steering.changes,
                    classes.join(", "),
                    self.steering.inclusive,
                    self.steering.exclusive
                )?;
            }
        }
        if self.sources.is_empty() {
            return Ok(());
        }
        writeln!(out)?;
        writeln!(
            out,
            "by kind of source, all sources of the kind removed together"
        )?;
        writeln!(
            out,
            "{:>4} {:>9} {:>9} {:>9} {:>8}  kind",
            "", "exclusive", "no-floor", "inclusive", "sources"
        )?;
        for k in &self.kinds {
            writeln!(
                out,
                "{:>4} {:>9} {:>9} {:>9} {:>8}  {}",
                "", k.exclusive, k.exclusive_no_floor, k.inclusive, k.sources, k.kind
            )?;
        }
        writeln!(out)?;
        writeln!(
            out,
            "sources of implausible selections, by trigger (exclusive: entries unselected if only \
             this source is removed; no-floor: the same without the uncertainty floor, as under \
             --on-uncertain ignore; inclusive: entries with a path through it; freed: unselected \
             with this and the sources above removed; left: implausible entries after it)"
        )?;
        writeln!(
            out,
            "{:>4} {:>9} {:>9} {:>9} {:>8} {:>8}  source",
            "#", "exclusive", "no-floor", "inclusive", "freed", "left"
        )?;
        for s in &self.sources {
            let opt = |v: Option<usize>| v.map_or("-".to_owned(), |v| v.to_string());
            writeln!(
                out,
                "{:>4} {:>9} {:>9} {:>9} {:>8} {:>8}  {}",
                opt(s.rank),
                s.exclusive,
                s.exclusive_no_floor,
                s.inclusive,
                opt(s.freed),
                opt(s.left),
                s.id
            )?;
            let pad = "";
            let sig: Vec<String> = s.signals.iter().map(|(k, n)| format!("{k} {n}")).collect();
            if !sig.is_empty() {
                writeln!(out, "{pad:>54}signals: {}", sig.join(", "))?;
            }
            if !s.example.is_empty() {
                writeln!(out, "{pad:>54}example: {}", s.example.join(" → "))?;
            }
            if s.changes_total > 0 {
                let more = s.changes_total - s.changes.len();
                writeln!(
                    out,
                    "{pad:>54}changes: {}{}",
                    s.changes.join(", "),
                    if more > 0 {
                        format!(" and {more} more")
                    } else {
                        String::new()
                    }
                )?;
            }
        }
        if self.left > 0 {
            writeln!(
                out,
                "{} implausible entr(ies) still selected after the picks (--top N)",
                self.left
            )?;
        }
        Ok(())
    }
}
