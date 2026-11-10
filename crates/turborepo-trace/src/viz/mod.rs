//! The impact of a change, shown instead of listed (README "See the impact of a
//! change"): `affected --html[=FILE]` writes one self-contained HTML report,
//! `affected --format summary` prints the same answers as text.
//!
//! Both are views of one [`Model`], built here from what `affected` computed.
//! Nothing in this module decides a verdict. The model answers six questions:
//!
//! | Question | Model |
//! |---|---|
//! | How much is selected, of what, and why | [`Model::totals`], [`Model::kinds`], [`Model::causes`] |
//! | From where | [`Model::sources`], [`Model::changes`] |
//! | Through what | [`Model::hubs`] |
//! | To where | [`Model::entries`], [`Model::groups`] |
//! | What keeps the floor | [`Model::floor`] |
//! | What was not selected, and why | the entries with verdict 0 and their notes |
//!
//! Counts per cause, source and hub follow the attribution of [`attr`]. The
//! model holds paths, package names and the details of uncertainty records,
//! never file contents.

pub(crate) mod attr;
mod html;
mod summary;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use attr::{CHANGE_SIDE, CLASSES, Kind, Recorded, Rev, Selection};
use serde::Serialize;
use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Cause, Change, Engine, EntryResult, FloorOrder, GraphView,
    NodeId, NodeSet, Verdict,
    attr::{PassRecord, cause_fields},
    barrel::PruneReport,
    floor::FloorAssumptions,
    names::NamedGroup,
};
use turborepo_trace_store::Graph;

/// The flags of the report.
#[derive(Debug, Default)]
pub struct VizArgs {
    /// Also write the impact report: one self-contained HTML file (no network
    /// requests) that shows which changed files select which entries,
    /// through which hubs, and what was not selected. Without FILE it goes
    /// to `impact.html` in the graph directory. The path is printed on
    /// stderr. Write the value as `--html=FILE`.
    pub html: Option<Option<PathBuf>>,
    /// Open the HTML report in the browser (implies --html). Ignored without a
    /// terminal and in CI.
    pub open: bool,
}

impl VizArgs {
    /// Whether an HTML report was asked for.
    pub(crate) fn html_on(&self) -> bool {
        self.html.is_some() || self.open
    }
}

/// What `affected` hands over (see `crate::affected`).
pub(crate) struct Input<'a, 'g, V: GraphView> {
    pub(crate) g: &'a Graph,
    pub(crate) engine: &'a Engine<'g, V>,
    pub(crate) opts: &'a AffectedOptions,
    pub(crate) passes: &'a [PassRecord],
    pub(crate) report: &'a AffectedReport,
    /// Entries of the engine's own report with a cause (not the floor).
    pub(crate) engine_change: &'a [u32],
    /// The entries each later layer selects by itself: steering, workflow,
    /// services.
    pub(crate) layers: [(&'static str, Vec<u32>); 4],
    pub(crate) named: &'a [NamedGroup],
    pub(crate) entries: &'a NodeSet,
    /// Entries evaluated without type-only imports.
    pub(crate) runtime: &'a NodeSet,
    pub(crate) missing: &'a [String],
    pub(crate) missing_verdict: Option<Verdict>,
    /// Every change, and what became of it.
    pub(crate) changes: &'a [Change],
    pub(crate) kept: &'a [Change],
    pub(crate) dropped_lock: Vec<&'a str>,
    pub(crate) dropped_workflow: Vec<&'a str>,
    pub(crate) dropped_config: Vec<(&'a str, String)>,
    pub(crate) steering: &'a [Change],
    pub(crate) steering_only: bool,
    pub(crate) pruned: &'a PruneReport,
    pub(crate) notes: Vec<String>,
    pub(crate) since: Option<&'a str>,
    pub(crate) root: &'a Path,
    pub(crate) command: String,
}

/// The data of the report. Indexes point into [`Model::nodes`],
/// [`Model::texts`], [`Model::kinds`], [`Model::causes`], [`Model::groups`],
/// [`Model::sources`] and [`Model::hubs`].
#[derive(Serialize)]
pub(crate) struct Model {
    version: u32,
    meta: Meta,
    totals: Totals,
    kinds: Vec<KindRow>,
    causes: Vec<CauseRow>,
    groups: Vec<GroupRow>,
    nodes: Vec<String>,
    texts: Vec<String>,
    sources: Vec<SourceRow>,
    hubs: Vec<HubRow>,
    entries: Vec<EntryRow>,
    changes: Vec<ChangeRow>,
    floor: serde_json::Value,
    pruning: Pruning,
    attr: AttrCheck,
}

#[derive(Serialize)]
struct Meta {
    tool: String,
    /// The name of the repository directory.
    repo: String,
    since: Option<String>,
    policy: &'static str,
    command: String,
    steering_only: bool,
    /// Whether open uncertainty selects entries under this policy.
    floor_applies: bool,
    notes: Vec<String>,
}

#[derive(Serialize, Default)]
struct Totals {
    entries: usize,
    selected: usize,
    unknown: usize,
    unaffected: usize,
    /// Entries that exist but are not in the graph.
    missing: usize,
    changes: usize,
    /// Selected entries a change reaches, the floor reaches, a name reaches.
    by_change: usize,
    by_floor: usize,
    by_named: usize,
    only_change: usize,
    only_floor: usize,
    only_named: usize,
}

#[derive(Serialize)]
struct KindRow {
    id: String,
    label: String,
    total: usize,
    selected: usize,
}

#[derive(Serialize)]
struct CauseRow {
    id: &'static str,
    label: &'static str,
    /// The colour family: `direct`, `indirect`, `build`, `named`, `floor`,
    /// `other`.
    family: &'static str,
    /// One sentence for the legend.
    about: &'static str,
    /// Entries whose first cause this is.
    first: usize,
    /// Entries it reaches.
    inclusive: usize,
    /// Entries only it reaches.
    exclusive: usize,
}

#[derive(Serialize)]
struct GroupRow {
    /// The package root (or top-level directory), relative to the repository
    /// root.
    path: String,
    /// The package name, when the group is a package.
    name: String,
    total: usize,
    selected: usize,
}

#[derive(Serialize)]
struct SourceRow {
    id: String,
    /// `change`, `package`, `site` (open uncertainty), `name`, `layer` or
    /// `other`.
    kind: &'static str,
    label: String,
    /// Kind of file (`source`, `test`, `manifest`, …), for changes.
    class: String,
    /// Entries only this source selects inside its mechanism.
    own: u32,
    /// Entries that are not selected at all without it.
    today: u32,
    /// Entries it reaches; `null` past the limit of sources counted.
    inclusive: Option<u32>,
}

#[derive(Serialize)]
struct HubRow {
    node: u32,
    /// `change` or `floor`: the mechanism whose paths pass through it.
    mech: &'static str,
    /// `file`, `task`, `members` or `part`.
    kind: &'static str,
    own: u32,
    today: u32,
    inclusive: Option<u32>,
}

/// One entry: `[path, kind, group, verdict, cause, causes, source, exclusive,
/// hubs, chain, why, others, others_total, notes, hub, witness]`.
///
/// - verdict: 0 not selected, 1 affected, 2 unknown;
/// - cause: the first cause (`-1` when not selected); causes: a mask over the
///   causes;
/// - source: the changed file, site or name it is drawn from (`-1`: none);
///   exclusive: 1 when every path to the entry starts there;
/// - hubs: the hubs every path passes through, from the source to the entry;
/// - chain: the shortest chain of the first cause's kind, from the change (or
///   the open site) to the entry;
/// - why: the words `--explain` prints before its chain, in [`Model::texts`];
/// - others: more sources that reach it (at most 12), and how many there are;
/// - notes: why changes in its closure did not select it, in [`Model::texts`];
/// - hub: the hub on the chain that holds the most entries (`-1`: none). It is
///   the first of `hubs` when every path crosses one;
/// - witness: the chain `--explain` prints (one shortest chain of any kind),
///   when it is not `chain`.
#[derive(Serialize)]
struct EntryRow(
    u32,
    u8,
    u32,
    u8,
    i8,
    u32,
    i32,
    u8,
    Vec<u32>,
    Vec<u32>,
    i32,
    Vec<u32>,
    u32,
    Vec<u32>,
    i32,
    Vec<u32>,
);

#[derive(Serialize)]
struct ChangeRow {
    path: String,
    kind: &'static str,
    class: String,
    /// `seed`, `lockfile`, `config`, `workflow`, `steering`, `erased`,
    /// `barrel`, `unmatched`.
    status: &'static str,
    note: String,
    source: Option<u32>,
}

#[derive(Serialize, Default)]
struct Pruning {
    /// Modified files whose JavaScript did not change.
    erased: usize,
    /// Modified files skipped behind a barrel that does not pass their names
    /// on.
    barrels: usize,
    /// Entries a change reaches only through type-only imports.
    type_only: usize,
    /// Entries that reach a skipped file and were not selected by it.
    spared: usize,
}

/// The numbers an offline analysis of the attribution dump
/// (`crate::attrdump`) gives for the same selection.
#[derive(Serialize, Default)]
struct AttrCheck {
    selected: usize,
    sinks: u32,
    sinks_change: u32,
    sinks_floor: u32,
    sinks_named: u32,
    unexplained: usize,
    first: BTreeMap<&'static str, usize>,
    inclusive: BTreeMap<&'static str, usize>,
    exclusive: BTreeMap<&'static str, usize>,
    /// Seed groups, names, open sites and layers: id, kind, today, own.
    sources: Vec<(String, &'static str, u32, u32)>,
}

/// Label, colour family and legend sentence per cause, in the order of
/// [`CLASSES`], plus `not-in-graph`.
const CAUSES: [(&str, &str, &str); 19] = [
    (
        "Own file changed",
        "direct",
        "The entry file itself changed.",
    ),
    (
        "Imports a changed file",
        "direct",
        "A chain of ordinary imports leads from the entry to a changed source file.",
    ),
    (
        "Imports changed data",
        "direct",
        "A chain of ordinary imports leads to a changed data file, style or document.",
    ),
    (
        "Shared test setup",
        "indirect",
        "The path crosses a setup file, config or transform shared by the tests of a project.",
    ),
    (
        "Reads a changed file",
        "indirect",
        "The path crosses a file read, an asset or a manifest.",
    ),
    (
        "Tool config or manifest",
        "indirect",
        "A changed package.json, tsconfig, lockfile or tool config is in the entry's closure.",
    ),
    (
        "Resolution config",
        "indirect",
        "A config that the resolution of an import read changed.",
    ),
    (
        "Dependency version",
        "indirect",
        "A third-party package resolves to another version or closure.",
    ),
    (
        "Directory listing",
        "build",
        "An added or deleted path matches a glob, a directory listing or a probed path.",
    ),
    (
        "Build task",
        "build",
        "The only paths cross a build task or the files of a package a task depends on.",
    ),
    (
        "Type-only import",
        "build",
        "The only paths cross type-only imports, for an entry that type-checks.",
    ),
    (
        "CI workflow",
        "build",
        "A CI job that runs the entry changed.",
    ),
    (
        "Service",
        "build",
        "A service that is started before the entry runs changed.",
    ),
    (
        "Steering file",
        "build",
        "The entry references a repository steering file that changed.",
    ),
    (
        "Named by a string",
        "named",
        "Code loaded with a read whose path is computed names a changed path.",
    ),
    (
        "Load with an unknown target",
        "named",
        "The entry reaches a load whose target is not known, and the change affects a package \
         that load can reach: the package of the loading file or of the entry, or one they \
         declare.",
    ),
    (
        "Uncertainty floor",
        "floor",
        "The entry reaches code that nothing bounds. It is selected on every change.",
    ),
    (
        "Unexplained",
        "other",
        "No recorded cause reaches the entry.",
    ),
    (
        "Not in the graph",
        "other",
        "The file exists but is not a traced source file, so it cannot be cleared.",
    ),
];
const NOT_IN_GRAPH: usize = 18;

#[derive(Default)]
struct Interner {
    map: HashMap<String, u32>,
    list: Vec<String>,
}

impl Interner {
    fn get(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.map.get(s) {
            return i;
        }
        let i = self.list.len() as u32;
        self.map.insert(s.to_owned(), i);
        self.list.push(s.to_owned());
        i
    }
}

/// The group of a node: its package root, the directory of a task, or the
/// top-level directory of a file of the root package.
fn group_of<G: GraphView>(g: &G, n: NodeId, is_task: bool) -> (String, String) {
    let path = g.path(n);
    if is_task {
        // `task:apps/web#build`, `members:apps/web/**`,
        // `gha:.github/workflows/ci.yml#job`.
        let rest = path.split_once(':').map_or(path, |(_, r)| r);
        let dir = rest.split('#').next().unwrap_or(rest);
        let dir = dir.trim_end_matches("/**").trim_end_matches('/');
        return (
            if dir.is_empty() { "." } else { dir }.to_owned(),
            String::new(),
        );
    }
    let info = g.package_info(g.package(n));
    if !info.root.is_empty() {
        return (info.root.to_owned(), info.name.to_owned());
    }
    match path.split_once('/') {
        Some((top, _)) => (top.to_owned(), String::new()),
        None => (".".to_owned(), info.name.to_owned()),
    }
}

fn kind_named(kinds: &mut Vec<KindRow>, id: &str, label: &str) -> usize {
    if let Some(i) = kinds.iter().position(|k| k.id == id) {
        return i;
    }
    kinds.push(KindRow {
        id: id.to_owned(),
        label: label.to_owned(),
        total: 0,
        selected: 0,
    });
    kinds.len() - 1
}

fn file_group(path: &str) -> String {
    match path.split_once('/') {
        Some((top, _)) => top.to_owned(),
        None => ".".to_owned(),
    }
}

/// Chains over a manual mock that is not registered for their entry.
///
/// The edge from a module to its manual mock that another file registers
/// (`<runner>:manual_mock <mock> registered by <file>`) counts for a jest or
/// vitest test only when the test loads a registering file
/// (`turborepo_trace_graph::barrel::mocks::lazy`). The chains of the
/// attribution are searched over the edges of the view, without that condition,
/// so the shortest one may use such an edge for a test that the change reaches
/// in another way. The engine's witness never does.
#[derive(Default)]
struct UnregisteredMocks {
    /// Per registering file: the nodes that load it.
    loads: HashMap<u32, Vec<bool>>,
}

impl UnregisteredMocks {
    /// Whether `chain` (from the change to `entry`) uses such an edge and
    /// `entry` is a test that loads none of the files that register the
    /// mock.
    fn on<G: GraphView>(&mut self, g: &G, rev: &Rev, chain: &[u32], entry: NodeId) -> bool {
        use turborepo_trace_graph::barrel::mocks::lazy;
        let mut is_test = None;
        for w in chain.windows(2) {
            let (mock, module) = (NodeId(w[0]), NodeId(w[1]));
            // The files that register the mock; none when the module also loads the mock
            // file in another way.
            let mut by: Vec<u32> = Vec::new();
            let mut conditional = true;
            for e in g.out_edges(module) {
                if e.to != Some(mock) {
                    continue;
                }
                match e.specifier.and_then(lazy::parse) {
                    Some((_, file)) => by.extend(g.node_by_path(file).map(|n| n.0)),
                    None => conditional = false,
                }
            }
            if !conditional || by.is_empty() {
                continue;
            }
            if !*is_test.get_or_insert_with(|| lazy::is_test(g, entry)) {
                return false;
            }
            let loaded = by.iter().any(|&f| {
                self.loads
                    .entry(f)
                    .or_insert_with(|| rev.up(std::iter::once(f), attr::ALL_CLASSES, attr::M_TYPE))
                    [entry.index()]
            });
            if !loaded {
                return true;
            }
        }
        false
    }
}

/// The source id of an entry from its witness, for an entry that no single
/// source holds.
fn witness_source<G: GraphView>(g: &G, kept: &HashSet<&str>, e: &EntryResult) -> String {
    let first = e.witness.first().copied().unwrap_or(e.entry);
    match (&e.cause, &e.uncertain) {
        // `the string 'fixtures'` is the name group `shape:fixtures`.
        (Some(Cause::Named { by, .. }), _) => match by
            .strip_prefix("the string '")
            .and_then(|s| s.strip_suffix('\''))
        {
            Some(shape) => format!("name:shape:{shape}"),
            None => format!("named-by:{by}"),
        },
        (Some(c), _) => {
            let (kind, detail) = cause_fields(g, first, c);
            attr::origin_of(&attr::seed_group(g.path(first), kept, kind, &detail).0)
        }
        (None, Some(u)) => format!("open:{}", g.path(u.node)),
        (None, None) => "other:".to_owned(),
    }
}

fn source_row(key: &str, kind: Kind) -> SourceRow {
    let (prefix, rest) = key.split_once(':').unwrap_or(("", key));
    let (k, label, class) = match (kind, prefix) {
        (Kind::Origin, "pkg") => ("package", rest.to_owned(), "package".to_owned()),
        (Kind::Origin, _) => ("change", rest.to_owned(), attr::file_class(rest)),
        (Kind::Open, _) => ("site", rest.to_owned(), "site".to_owned()),
        (Kind::Name, "named-by") => ("name", rest.to_owned(), "name".to_owned()),
        (Kind::Name, _) => {
            let (kind, label) = rest.split_once(':').unwrap_or(("shape", rest));
            let dir = |d: &str| {
                if d.is_empty() || d == "." {
                    "the repository root".to_owned()
                } else {
                    d.to_owned()
                }
            };
            let label = match kind {
                "shape" => format!("the string '{label}'"),
                "location" => format!("the location of a file in {}", dir(label)),
                "cwd" => format!("the working directory ({})", dir(label)),
                "testdir" => format!("the directory of the test, {}", dir(label)),
                "everything" => "a name that matches every path".to_owned(),
                _ => label.to_owned(),
            };
            ("name", label, "name".to_owned())
        }
        (Kind::Layer, _) => (
            "layer",
            match rest {
                "workflow" => "CI workflow changes",
                "services" => "Services started before tests",
                "steering" => "Steering files referenced",
                "loads" => "Loads with an unknown target that the change can reach",
                other => other,
            }
            .to_owned(),
            "layer".to_owned(),
        ),
        _ => (
            "other",
            if rest.is_empty() { "Other" } else { rest }.to_owned(),
            "other".to_owned(),
        ),
    };
    SourceRow {
        id: key.to_owned(),
        kind: k,
        label,
        class,
        own: 0,
        today: 0,
        inclusive: None,
    }
}

/// The copies of one graph node in one mechanism (one per pass), and what they
/// hold.
#[derive(Default)]
struct HubCopies {
    own: u32,
    today: u32,
    copies: Vec<u32>,
}

/// Sources and hubs with an exact count of the entries they reach. Past these
/// limits the count is left out (`null`).
const INCLUSIVE_SOURCES: usize = 1024;
const INCLUSIVE_HUBS: usize = 256;
/// Hubs kept: the ones that hold the most entries.
const MAX_HUBS: usize = 4000;
/// Skipped files whose spared entries get a note.
const MAX_SKIP_NOTES: usize = 400;

/// Phase timings of [`build`], for `--stats`.
#[derive(Default)]
pub(crate) struct Timings {
    attribution: f64,
    floor: f64,
    notes: f64,
}

/// Builds the model.
// Invariant: `ranked` keeps only the indexes whose `source_sel` is Some (filtered above);
// `missing_kind` is Some whenever `inp.missing` is non-empty, which this loop needs.
#[allow(clippy::expect_used)]
pub(crate) fn build<V: GraphView>(inp: &Input<'_, '_, V>) -> (Model, Timings) {
    let mut timings = Timings::default();
    let t = Instant::now();
    let g = inp.g;
    let view = inp.engine.graph();
    let n = g.node_count() as usize;
    let rev = Rev::build(view);
    let by_entry: HashMap<u32, &EntryResult> =
        inp.report.entries.iter().map(|e| (e.entry.0, e)).collect();
    let mut selected = vec![false; n];
    for e in &inp.report.entries {
        selected[e.entry.index()] = true;
    }
    let mut engine_change = vec![false; n];
    for &e in inp.engine_change {
        engine_change[e as usize] = true;
    }
    let kept: HashSet<&str> = inp.kept.iter().map(|c| c.path.as_str()).collect();
    let rec = Recorded {
        passes: inp.passes,
        kept: kept.clone(),
        selected,
        engine_change,
        named: inp.named,
        layers: inp.layers.clone(),
    };
    let reach = attr::first_causes(view, &rev, &rec);
    let chains = attr::class_chains(view, &rev, &rec, &reach);
    let mut unregistered = UnregisteredMocks::default();
    let sel = Selection::build(view, &rev, &rec);
    let all = sel.dominated(&[
        attr::MECH_CHANGE,
        attr::MECH_FLOOR,
        attr::MECH_NAMED,
        attr::MECH_LAYER,
    ]);
    // Per mechanism: change (with the later layers), floor, names.
    let runs = [
        sel.dominated(&[attr::MECH_CHANGE, attr::MECH_LAYER]),
        sel.dominated(&[attr::MECH_FLOOR]),
        sel.dominated(&[attr::MECH_NAMED]),
    ];
    let run_of_class = |c: usize| -> Option<usize> {
        match CLASSES[c] {
            "floor" => Some(1),
            "named" => Some(2),
            "unexplained" => None,
            _ => Some(0),
        }
    };

    // Sources: changed paths and packages, open sites, names, layers.
    let mut sources: Vec<SourceRow> = Vec::new();
    let mut source_of_sel: HashMap<u32, u32> = HashMap::new();
    let mut source_by_key: HashMap<String, u32> = HashMap::new();
    let mut source_sel: Vec<Option<u32>> = Vec::new();
    for v in 0..sel.kind.len() {
        let kind = sel.kind[v];
        if !matches!(kind, Kind::Origin | Kind::Open | Kind::Name | Kind::Layer) {
            continue;
        }
        let key = sel.key[v].as_deref().unwrap_or_default();
        let mut row = source_row(key, kind);
        let run = match kind {
            Kind::Open => 1,
            Kind::Name => 2,
            _ => 0,
        };
        row.own = runs[run].count[v];
        row.today = all.count[v];
        source_of_sel.insert(v as u32, sources.len() as u32);
        source_by_key.insert(key.to_owned(), sources.len() as u32);
        source_sel.push(Some(v as u32));
        sources.push(row);
    }

    // Hubs: the copies of one graph node in one mechanism, over the passes.
    let mut hub_counts: HashMap<(u8, u32), HubCopies> = HashMap::new();
    for v in 0..sel.kind.len() {
        if sel.kind[v] != Kind::Copy {
            continue;
        }
        let m = sel.mech_of[v];
        let (own, today) = (runs[m as usize].count[v], all.count[v]);
        if own == 0 && today == 0 {
            continue;
        }
        let e = hub_counts.entry((m, sel.node[v])).or_default();
        e.own += own;
        e.today += today;
        e.copies.push(v as u32);
    }
    let mut hub_list: Vec<((u8, u32), HubCopies)> = hub_counts
        .into_iter()
        .filter(|(_, h)| h.own >= 2 || h.today >= 2)
        .collect();
    hub_list.sort_by(|a, b| {
        (b.1.own, b.1.today)
            .cmp(&(a.1.own, a.1.today))
            .then_with(|| g.path(NodeId(a.0.1)).cmp(g.path(NodeId(b.0.1))))
            .then(a.0.0.cmp(&b.0.0))
    });
    hub_list.truncate(MAX_HUBS);
    let mut nodes = Interner::default();
    let mut texts = Interner::default();
    let mut hubs: Vec<HubRow> = Vec::new();
    let mut hub_index: HashMap<(u8, u32), u32> = HashMap::new();
    let mut hub_sel: Vec<Vec<u32>> = Vec::new();
    for ((m, node), h) in hub_list {
        let HubCopies { own, today, copies } = h;
        hub_index.insert((m, node), hubs.len() as u32);
        let lang = g.lang(NodeId(node)).unwrap_or("");
        hubs.push(HubRow {
            node: nodes.get(g.path(NodeId(node))),
            mech: if m == 0 { "change" } else { "floor" },
            kind: match lang {
                "task" => "task",
                "task-members" => "members",
                "task-part" => "part",
                _ => "file",
            },
            own,
            today,
            inclusive: None,
        });
        hub_sel.push(copies);
    }

    // Entries.
    let mut fw_of: HashMap<u32, usize> = HashMap::new();
    let mut kinds: Vec<KindRow> = Vec::new();
    for (fw, _) in turborepo_trace_store::fw::GRAPH_FRAMEWORKS {
        let tests = g.tests(Some(fw));
        if !tests.iter().any(|t| inp.entries.contains(*t)) {
            continue;
        }
        for t in tests {
            fw_of.entry(t.0).or_insert(kinds.len());
        }
        kinds.push(KindRow {
            id: (*fw).to_owned(),
            label: format!("{fw} tests"),
            total: 0,
            selected: 0,
        });
    }
    let mut groups: Vec<GroupRow> = Vec::new();
    let mut group_index: HashMap<String, u32> = HashMap::new();
    let mut group = |path: String, name: String, groups: &mut Vec<GroupRow>| -> u32 {
        if let Some(&i) = group_index.get(&path) {
            return i;
        }
        let i = groups.len() as u32;
        group_index.insert(path.clone(), i);
        groups.push(GroupRow {
            path,
            name,
            total: 0,
            selected: 0,
        });
        i
    };
    let mut order: Vec<NodeId> = inp.entries.iter().collect();
    order.sort_by(|a, b| g.path(*a).cmp(g.path(*b)));
    let mut entries: Vec<EntryRow> = Vec::with_capacity(order.len() + inp.missing.len());
    let mut row_of: HashMap<u32, usize> = HashMap::new();
    let mut totals = Totals {
        changes: inp.changes.len(),
        missing: inp.missing.len(),
        ..Totals::default()
    };
    let mut cause_first = [0usize; 19];
    let mut cause_incl = [0usize; 19];
    let mut cause_only = [0usize; 19];
    let mut check = AttrCheck::default();
    let bit = |c: &str| 1u32 << attr::class_index(c);
    let change_bits = (1u32 << CHANGE_SIDE) - 1;
    let layer_bits = bit("workflow") | bit("services") | bit("steering") | bit("loads");
    for &node in &order {
        let is_task = rev.is_task[node.index()];
        let k = match fw_of.get(&node.0) {
            Some(&k) => k,
            None if is_task => kind_named(&mut kinds, "task", "build tasks"),
            None => kind_named(&mut kinds, "file", "files"),
        };
        let (gp, gn) = group_of(g, node, is_task);
        let gi = group(gp, gn, &mut groups);
        let p = nodes.get(g.path(node));
        kinds[k].total += 1;
        groups[gi as usize].total += 1;
        totals.entries += 1;
        let Some(e) = by_entry.get(&node.0) else {
            totals.unaffected += 1;
            row_of.insert(node.0, entries.len());
            entries.push(EntryRow(
                p,
                k as u8,
                gi,
                0,
                -1,
                0,
                -1,
                0,
                Vec::new(),
                Vec::new(),
                -1,
                Vec::new(),
                0,
                Vec::new(),
                -1,
                Vec::new(),
            ));
            continue;
        };
        kinds[k].selected += 1;
        groups[gi as usize].selected += 1;
        let verdict = match e.verdict {
            Verdict::Unknown => {
                totals.unknown += 1;
                2
            }
            _ => {
                totals.selected += 1;
                1
            }
        };
        let mask = reach[node.index()];
        let first = attr::first_of(mask);
        cause_first[first] += 1;
        for (c, slot) in cause_incl.iter_mut().enumerate().take(CLASSES.len()) {
            if mask & (1 << c) != 0 {
                *slot += 1;
            }
        }
        // Classes nest on the change side: reduce to the first of them before asking
        // whether one class stands alone.
        let side = mask & change_bits;
        let rest = mask & !change_bits;
        let eff = if side != 0 {
            (1u32 << side.trailing_zeros()) | rest
        } else {
            rest
        };
        if eff.count_ones() == 1 {
            cause_only[eff.trailing_zeros() as usize] += 1;
        }
        let (chg, flo, nam) = (
            mask & (change_bits | layer_bits) != 0,
            mask & bit("floor") != 0,
            mask & bit("named") != 0,
        );
        totals.by_change += usize::from(chg);
        totals.by_floor += usize::from(flo);
        totals.by_named += usize::from(nam);
        totals.only_change += usize::from(chg && !flo && !nam);
        totals.only_floor += usize::from(flo && !chg && !nam);
        totals.only_named += usize::from(nam && !chg && !flo);

        // The dominators of the entry inside the mechanism of its first cause.
        let mut chain: Vec<u32> = Vec::new();
        if let (Some(run), Some(&sink)) = (run_of_class(first), sel.sinks.get(&node.0)) {
            let idom = &runs[run].idom;
            let mut v = idom[sink as usize];
            while v != u32::MAX && v != 0 {
                chain.push(v);
                v = idom[v as usize];
            }
            chain.reverse();
        }
        let held = chain.iter().find_map(|v| source_of_sel.get(v).copied());
        // The chain shown: the shortest of the first cause's kind, else the witness. A
        // chain over a manual mock that no file the test loads registers is not a way
        // the change reaches it: the witness is.
        let shown = chains
            .get(&node.0)
            .filter(|(list, _)| !unregistered.on(view, &rev, list, node));
        let source = match held {
            Some(s) => s,
            None => {
                let key = match shown {
                    Some((_, key)) if key.starts_with("open") => key.clone(),
                    Some((_, key)) if !key.is_empty() => attr::origin_of(key),
                    _ => witness_source(g, &kept, e),
                };
                let key = if key.starts_with("open:") && !source_by_key.contains_key(&key) {
                    key.replacen("open:", "open-per-entry:", 1)
                } else {
                    key
                };
                match source_by_key.get(&key) {
                    Some(&s) => s,
                    None => {
                        let kind = match key.split_once(':').map_or("", |(p, _)| p) {
                            "named-by" => Kind::Name,
                            "open" | "open-per-entry" => Kind::Open,
                            "path" | "pkg" => Kind::Origin,
                            _ => Kind::Root,
                        };
                        let s = sources.len() as u32;
                        sources.push(source_row(&key, kind));
                        source_sel.push(None);
                        source_by_key.insert(key, s);
                        s
                    }
                }
            }
        };
        let source_label = sources[source as usize].label.as_str();
        let through: Vec<u32> = chain
            .iter()
            .filter(|&&v| sel.kind[v as usize] == Kind::Copy)
            .filter(|&&v| {
                let x = sel.node[v as usize];
                x != node.0 && g.path(NodeId(x)) != source_label
            })
            .filter_map(|&v| {
                hub_index
                    .get(&(sel.mech_of[v as usize], sel.node[v as usize]))
                    .copied()
            })
            .collect();
        let engine_witness: Vec<u32> = e.witness.iter().map(|w| nodes.get(g.path(*w))).collect();
        let (witness, hub) = match shown {
            Some((list, _)) => {
                let mech = u8::from(CLASSES[first] == "floor");
                let hub = list
                    .iter()
                    .filter(|&&x| x != node.0 && g.path(NodeId(x)) != source_label)
                    .filter_map(|&x| hub_index.get(&(mech, x)).copied())
                    .max_by_key(|&h| (hubs[h as usize].own, std::cmp::Reverse(h)));
                (
                    list.iter().map(|x| nodes.get(g.path(NodeId(*x)))).collect(),
                    hub,
                )
            }
            None => (engine_witness.clone(), None),
        };
        let hub = through.first().copied().or(hub);
        let engine_witness = if engine_witness == witness {
            Vec::new()
        } else {
            engine_witness
        };
        let mut why = turborepo_trace_graph::output::affected_why(view, e);
        if let Some(u) = &e.uncertain {
            for (kind, line, detail) in u.reasons.iter().take(3) {
                why.push_str(&format!("\n{}:{line} {kind}", g.path(u.node)));
                if !detail.is_empty() {
                    why.push_str(&format!(": {detail}"));
                }
            }
        }
        row_of.insert(node.0, entries.len());
        entries.push(EntryRow(
            p,
            k as u8,
            gi,
            verdict,
            first as i8,
            mask,
            source as i32,
            u8::from(held.is_some()),
            through,
            witness,
            texts.get(&why) as i32,
            Vec::new(),
            0,
            Vec::new(),
            hub.map_or(-1, |h| h as i32),
            engine_witness,
        ));
    }
    // Entries that are not in the graph.
    let missing_kind = (!inp.missing.is_empty()).then(|| kind_named(&mut kinds, "file", "files"));
    for m in inp.missing {
        let k = missing_kind.expect("a kind for missing entries");
        let gi = group(file_group(m), String::new(), &mut groups);
        kinds[k].total += 1;
        groups[gi as usize].total += 1;
        totals.entries += 1;
        let p = nodes.get(m);
        let verdict = match inp.missing_verdict {
            Some(Verdict::Affected) => {
                totals.selected += 1;
                1
            }
            Some(Verdict::Unknown) => {
                totals.unknown += 1;
                2
            }
            _ => {
                totals.unaffected += 1;
                0
            }
        };
        let (mut cause, mut mask, mut source, mut why) = (-1i8, 0u32, -1i32, -1i32);
        if verdict != 0 {
            kinds[k].selected += 1;
            groups[gi as usize].selected += 1;
            cause_first[NOT_IN_GRAPH] += 1;
            cause_incl[NOT_IN_GRAPH] += 1;
            cause_only[NOT_IN_GRAPH] += 1;
            cause = NOT_IN_GRAPH as i8;
            mask = 1 << NOT_IN_GRAPH;
            let key = "other:not-in-graph";
            source = match source_by_key.get(key) {
                Some(&s) => s as i32,
                None => {
                    let s = sources.len() as u32;
                    let mut row = source_row(key, Kind::Root);
                    row.label = "Entries outside the graph".to_owned();
                    sources.push(row);
                    source_sel.push(None);
                    source_by_key.insert(key.to_owned(), s);
                    s as i32
                }
            };
            why = texts.get("not in the graph (not a traced source file)") as i32;
        }
        entries.push(EntryRow(
            p,
            k as u8,
            gi,
            verdict,
            cause,
            mask,
            source,
            0,
            Vec::new(),
            Vec::new(),
            why,
            Vec::new(),
            0,
            Vec::new(),
            -1,
            Vec::new(),
        ));
    }

    // Entries reached per source and per hub.
    let mut flow = vec![0u32; sources.len()];
    for e in &entries {
        if e.6 >= 0 {
            flow[e.6 as usize] += 1;
        }
    }
    let mut ranked: Vec<usize> = (0..sources.len())
        .filter(|&i| source_sel[i].is_some())
        .collect();
    ranked.sort_by_key(|&i| std::cmp::Reverse((flow[i] + sources[i].own, sources[i].today)));
    ranked.truncate(INCLUSIVE_SOURCES);
    let n_sources = ranked.len();
    let mut starts: Vec<Vec<u32>> = ranked
        .iter()
        .map(|&i| vec![source_sel[i].expect("a selection node")])
        .collect();
    let n_hubs = hubs.len().min(INCLUSIVE_HUBS);
    starts.extend(hub_sel.iter().take(n_hubs).cloned());
    let mut others: HashMap<u32, (Vec<u32>, u32)> = HashMap::new();
    let below = sel.below(&starts, |entry, group| {
        if group < n_sources {
            let o = others.entry(entry).or_default();
            o.1 += 1;
            if o.0.len() < 12 {
                o.0.push(ranked[group] as u32);
            }
        }
    });
    for (j, &i) in ranked.iter().enumerate() {
        sources[i].inclusive = Some(below[j]);
    }
    for (h, hub) in hubs.iter_mut().enumerate().take(n_hubs) {
        hub.inclusive = Some(below[n_sources + h]);
    }
    for (entry, (mut list, total)) in others {
        if let Some(&r) = row_of.get(&entry) {
            list.sort_unstable();
            entries[r].11 = list;
            entries[r].12 = total;
        }
    }
    timings.attribution = t.elapsed().as_secs_f64() * 1e3;

    // What the selection was checked against (an offline analysis of the
    // attribution dump gives the same).
    check.selected = inp.report.entries.len();
    check.sinks = all.sinks;
    check.sinks_change = runs[0].sinks;
    check.sinks_floor = runs[1].sinks;
    check.sinks_named = runs[2].sinks;
    check.unexplained = cause_first[CLASSES.len() - 1];
    for (c, name) in CLASSES.iter().enumerate() {
        for (map, counts) in [
            (&mut check.first, &cause_first),
            (&mut check.inclusive, &cause_incl),
            (&mut check.exclusive, &cause_only),
        ] {
            if counts[c] > 0 {
                map.insert(*name, counts[c]);
            }
        }
    }
    for v in 0..sel.kind.len() {
        let (kind, run) = match sel.kind[v] {
            Kind::Seed => ("seed", 0),
            Kind::Name => ("name", 2),
            Kind::Open => ("open", 1),
            Kind::Layer => ("layer", 0),
            _ => continue,
        };
        check.sources.push((
            sel.key[v].clone().unwrap_or_default(),
            kind,
            all.count[v],
            runs[run].count[v],
        ));
    }
    check.sources.sort();

    // Why changes in an entry's closure did not select it.
    let t = Instant::now();
    let mut pruning = Pruning {
        erased: inp.pruned.erased.len(),
        barrels: inp.pruned.skipped.len(),
        type_only: inp.pruned.type_only.len(),
        spared: inp.pruned.unaffected.len(),
    };
    let mut skipped: Vec<(&turborepo_trace_graph::barrel::Skipped, bool)> = inp
        .pruned
        .erased
        .iter()
        .map(|s| (s, true))
        .chain(inp.pruned.skipped.iter().map(|s| (s, false)))
        .collect();
    skipped.sort_by_key(|(s, _)| std::cmp::Reverse(s.entries));
    for (s, erased) in skipped.into_iter().take(MAX_SKIP_NOTES) {
        let mut from = NodeSet::new(g.node_count());
        from.insert(s.node);
        let up = inp.engine.rdeps(&from, None);
        let path = g.path(s.node);
        let note = if erased {
            format!(
                "{path} changed, but only its types, comments or formatting: the JavaScript is \
                 the same as at the base revision"
            )
        } else {
            format!(
                "{path} changed, but the entry does not use its exports: it is only re-exported \
                 by a barrel on the way, and it has no side effects"
            )
        };
        let note = texts.get(&note);
        for e in up.iter() {
            if let Some(&r) = row_of.get(&e.0)
                && entries[r].3 == 0
                && entries[r].13.len() < 4
            {
                entries[r].13.push(note);
            }
        }
    }
    let type_note = texts.get(
        "A changed file is reached only through type-only imports, which the test runner does not \
         load",
    );
    for e in &inp.pruned.type_only {
        if let Some(&r) = row_of.get(&e.0)
            && entries[r].3 == 0
        {
            entries[r].13.push(type_note);
        }
    }
    if pruning.erased + pruning.barrels + pruning.type_only == 0 {
        pruning.spared = 0;
    }
    timings.notes = t.elapsed().as_secs_f64() * 1e3;

    // The floor: the open sites, as the uncertainty report lists them.
    let t = Instant::now();
    let floor_applies =
        inp.report.policy != turborepo_trace_graph::Policy::Ignore && !inp.steering_only;
    let floor = inp.engine.floor_for_runners(
        inp.entries,
        inp.runtime,
        inp.opts,
        FloorOrder::Freed,
        25,
        &FloorAssumptions::default(),
    );
    let mut floor_value = floor.to_value(g);
    if let Some(sites) = floor_value.get_mut("sites").and_then(|s| s.as_array_mut()) {
        sites.truncate(500);
        for s in sites {
            if let Some(r) = s.get_mut("records").and_then(|r| r.as_array_mut()) {
                r.truncate(6);
            }
        }
    }
    floor_value["applies"] = serde_json::json!(floor_applies);
    timings.floor = t.elapsed().as_secs_f64() * 1e3;

    // The changes and what became of each.
    let steering: HashSet<&str> = inp.steering.iter().map(|c| c.path.as_str()).collect();
    let lock: HashSet<&str> = inp.dropped_lock.iter().copied().collect();
    let workflow: HashSet<&str> = inp.dropped_workflow.iter().copied().collect();
    let config: HashMap<&str, &str> = inp
        .dropped_config
        .iter()
        .map(|(p, note)| (*p, note.as_str()))
        .collect();
    let unmatched: HashSet<&str> = inp
        .report
        .unmatched
        .iter()
        .map(|c| c.path.as_str())
        .collect();
    let erased: BTreeSet<&str> = inp.pruned.erased.iter().map(|s| g.path(s.node)).collect();
    let barrel: BTreeSet<&str> = inp.pruned.skipped.iter().map(|s| g.path(s.node)).collect();
    let changes: Vec<ChangeRow> = inp
        .changes
        .iter()
        .map(|c| {
            let p = c.path.as_str();
            let source = source_by_key.get(&format!("path:{p}")).copied();
            let (status, note) = if steering.contains(p) {
                (
                    "steering",
                    "Repository steering file: it counts only through explicit references"
                        .to_owned(),
                )
            } else if workflow.contains(p) {
                (
                    "workflow",
                    "Read field by field: only the jobs it changes count".to_owned(),
                )
            } else if lock.contains(p) {
                (
                    "lockfile",
                    "Compared with the base revision: only the packages that resolve differently \
                     count"
                        .to_owned(),
                )
            } else if let Some(note) = config.get(p) {
                ("config", (*note).to_owned())
            } else if erased.contains(p) {
                (
                    "erased",
                    "Only types, comments or formatting changed: skipped for tests whose runner \
                     only transpiles"
                        .to_owned(),
                )
            } else if barrel.contains(p) {
                (
                    "barrel",
                    "Skipped for the entries that reach it only through a barrel whose names they \
                     do not use"
                        .to_owned(),
                )
            } else if unmatched.contains(p) {
                ("unmatched", "Nothing in the graph depends on it".to_owned())
            } else {
                ("seed", String::new())
            };
            ChangeRow {
                path: p.to_owned(),
                kind: c.kind.as_str(),
                class: attr::file_class(p),
                status,
                note,
                source,
            }
        })
        .collect();

    let causes: Vec<CauseRow> = CAUSES
        .iter()
        .enumerate()
        .map(|(i, (label, family, about))| CauseRow {
            id: if i < CLASSES.len() {
                CLASSES[i]
            } else {
                "not-in-graph"
            },
            label,
            family,
            about,
            first: cause_first[i],
            inclusive: cause_incl[i],
            exclusive: cause_only[i],
        })
        .collect();
    let model = Model {
        version: 1,
        meta: Meta {
            tool: format!("turbotrace {}", env!("CARGO_PKG_VERSION")),
            repo: inp
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            since: inp.since.map(str::to_owned),
            policy: match inp.report.policy {
                turborepo_trace_graph::Policy::Affected => "affected",
                turborepo_trace_graph::Policy::Unknown => "unknown",
                turborepo_trace_graph::Policy::Ignore => "ignore",
            },
            command: inp.command.clone(),
            steering_only: inp.steering_only,
            floor_applies,
            notes: inp.notes.clone(),
        },
        totals,
        kinds,
        causes,
        groups,
        nodes: nodes.list,
        texts: texts.list,
        sources,
        hubs,
        entries,
        changes,
        floor: floor_value,
        pruning,
        attr: check,
    };
    (model, timings)
}

/// Where the HTML report goes: the given file, or `impact.html` in the graph
/// directory.
pub(crate) fn html_path(args: &VizArgs, graph_dir: &Path) -> PathBuf {
    match &args.html {
        Some(Some(p)) if !p.as_os_str().is_empty() => p.clone(),
        _ => graph_dir.join("impact.html"),
    }
}

/// Writes the HTML report (readable by the owner only: it holds the
/// repository's paths), prints its path on stderr and, with `--open`, opens it.
pub(crate) fn write_html(
    args: &VizArgs,
    graph_dir: &Path,
    model: &Model,
    timings: &Timings,
    stats: bool,
) -> Result<()> {
    let t = Instant::now();
    let path = html_path(args, graph_dir);
    let page = html::render(model)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    write_private(&path, page.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
    let shown = std::fs::canonicalize(&path).unwrap_or(path.clone());
    eprintln!("impact report: {}", shown.display());
    if stats {
        eprintln!(
            "viz: attribution {:.1} ms, skip notes {:.1} ms, floor {:.1} ms, html {:.1} ms ({} \
             bytes; {} entries, {} sources, {} hubs, {} paths)",
            timings.attribution,
            timings.notes,
            timings.floor,
            t.elapsed().as_secs_f64() * 1e3,
            page.len(),
            model.entries.len(),
            model.sources.len(),
            model.hubs.len(),
            model.nodes.len(),
        );
    }
    if args.open {
        open_in_browser(&shown);
    }
    Ok(())
}

/// Prints the terminal report.
pub(crate) fn write_summary(model: &Model, out: &mut dyn std::io::Write) -> Result<()> {
    let tty = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let width = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse::<usize>().ok())
        .filter(|w| *w >= 40)
        .unwrap_or(100);
    summary::write(model, out, width.min(160), tty)?;
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        // An existing file keeps its mode on open: narrow it.
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(bytes)?;
    f.flush()
}

/// Opens `path` with the platform's opener, only for a person at a terminal.
fn open_in_browser(path: &Path) {
    if std::env::var_os("CI").is_some() || !std::io::stderr().is_terminal() {
        eprintln!("note: --open ignored: no terminal (or CI is set)");
        return;
    }
    let (cmd, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/c", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    let r = std::process::Command::new(cmd)
        .args(args)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = r {
        eprintln!("note: could not open the report with `{cmd}`: {e}");
    }
}
