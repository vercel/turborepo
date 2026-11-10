//! Attribution of a selection to its causes and funnels, from what the engine
//! recorded ([`turborepo_trace_graph::attr::PassRecord`]) and the edges of the
//! evaluated view. Nothing here decides a verdict: the selection is the
//! engine's.
//!
//! The definitions:
//!
//! - **First cause.** Every selected entry gets the first class of [`CLASSES`]
//!   that reaches it, from layered reverse searches over classes of edges
//!   ([`first_causes`]).
//! - **Funnels.** The selection graph ([`Selection`]) is `ROOT → mechanism →
//!   changed path → seed group → copies of the graph nodes a pass reached →
//!   entry`. A node *dominates* an entry when every path from the root to the
//!   entry passes through it: cut it and the entry drops out. `today` counts
//!   dominated entries in the whole graph, `own` inside the node's mechanism
//!   (change, floor or names) alone.
//!
//! One addition to the script: seed groups hang below a node per changed path
//! ([`Kind::Origin`]), so that a file that seeds nodes in several ways (as
//! itself, as a config another file read, through a listing) has one count.
//! That does not change the counts of any other node.

use std::collections::{BTreeMap, HashMap, HashSet};

use turborepo_trace_graph::{
    GraphView, NodeId, RefFlags, RefKind, attr::PassRecord, names::NamedGroup,
};

/// Edge classes (one per edge).
pub(crate) const PLAIN: u8 = 1;
pub(crate) const FW: u8 = 2;
pub(crate) const READ: u8 = 4;
pub(crate) const MANIFEST: u8 = 8;
pub(crate) const TASK: u8 = 16;
pub(crate) const ALL_CLASSES: u8 = PLAIN | FW | READ | MANIFEST | TASK;
/// Edge modifiers.
pub(crate) const M_TYPE: u8 = 1;
pub(crate) const M_DATA: u8 = 2;

/// First causes, most plausible first. An entry gets the first one that
/// applies.
pub(crate) const CLASSES: [&str; 18] = [
    "self",
    "import",
    "data-import",
    "fw-edge",
    "read",
    "config-file",
    "config-input",
    "lockfile",
    "listing",
    "task",
    "type-only",
    "workflow",
    "services",
    "steering",
    "named",
    "loads",
    "floor",
    "unexplained",
];
/// The classes on the change side: they nest (a later one includes the paths of
/// the earlier ones).
pub(crate) const CHANGE_SIDE: usize = 11;

// Invariant: callers name classes of the fixed CLASSES table (unit-tested).
#[allow(clippy::expect_used)]
pub(crate) fn class_index(name: &str) -> usize {
    CLASSES
        .iter()
        .position(|c| *c == name)
        .expect("a known class")
}

/// The first class of a mask, as an index into [`CLASSES`] (`unexplained` for
/// none).
pub(crate) fn first_of(mask: u32) -> usize {
    if mask == 0 {
        CLASSES.len() - 1
    } else {
        mask.trailing_zeros() as usize
    }
}

const LOCKFILES: [&str; 9] = [
    "pnpm-lock.yaml",
    "package-lock.json",
    "yarn.lock",
    "bun.lockb",
    "bun.lock",
    "go.sum",
    "Cargo.lock",
    "poetry.lock",
    "uv.lock",
];
const CODE_EXT: [&str; 12] = [
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "go", "py", "rs", "sh",
];
const ASSET_EXT: [&str; 20] = [
    "css", "scss", "sass", "less", "svg", "png", "jpg", "jpeg", "gif", "webp", "ico", "woff",
    "woff2", "ttf", "otf", "mp4", "webm", "avif", "glsl", "frag",
];
const TOOLS: [&str; 24] = [
    "jest",
    "vitest",
    "vite",
    "next",
    "babel",
    "playwright",
    "webpack",
    "rollup",
    "tsup",
    "eslint",
    "prettier",
    "postcss",
    "tailwind",
    "swc",
    "turbo",
    "nx",
    "lerna",
    "cypress",
    "storybook",
    "commitlint",
    "lint-staged",
    "stylelint",
    "biome",
    "knip",
];
const CONFIG_EXT: [&str; 9] = [
    "js", "cjs", "mjs", "ts", "mts", "cts", "json", "yaml", "yml",
];

/// Whether a file name is a tool's config (the script's `TOOL_CONFIG` pattern).
fn is_tool_config(base: &str) -> bool {
    let exact = [
        ".swcrc",
        "turbo.json",
        "turbo.jsonc",
        ".npmrc",
        ".nvmrc",
        ".node-version",
        "pnpm-workspace.yaml",
        "go.mod",
        "go.work",
        "Cargo.toml",
        "pyproject.toml",
        "Makefile",
        "compose.yaml",
        "compose.yml",
        "vercel.json",
        ".eslintignore",
        ".prettierignore",
        ".gitignore",
        ".dockerignore",
    ];
    if exact.contains(&base) {
        return true;
    }
    if [
        ".babelrc",
        ".eslintrc",
        ".prettierrc",
        "Dockerfile",
        "docker-compose",
    ]
    .iter()
    .any(|p| base.starts_with(p))
    {
        return true;
    }
    // `\.?<tool>[\w.-]*\.(config|setup|preset)?\.?<ext>`
    let b = base.strip_prefix('.').unwrap_or(base);
    for tool in TOOLS.iter().chain(["syncpack"].iter()) {
        let Some(rest) = b.strip_prefix(tool) else {
            continue;
        };
        if !rest
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            continue;
        }
        for ext in CONFIG_EXT {
            let Some(head) = rest.strip_suffix(ext) else {
                continue;
            };
            if head.ends_with('.')
                || ["config", "setup", "preset"]
                    .iter()
                    .any(|w| head.strip_suffix(w).is_some_and(|h| h.ends_with('.')))
            {
                return true;
            }
        }
    }
    false
}

/// `\.(test|spec|e2e|test-d)\.[cm]?[jt]sx?$` somewhere at the end of a file
/// name.
fn is_test_name(base: &str) -> bool {
    let Some((stem, ext)) = base.rsplit_once('.') else {
        return false;
    };
    let e = ext
        .strip_prefix(['c', 'm'])
        .filter(|r| r.starts_with(['j', 't']))
        .unwrap_or(ext);
    let ext_ok = matches!(e, "js" | "ts" | "jsx" | "tsx");
    ext_ok
        && [".test", ".spec", ".e2e", ".test-d"]
            .iter()
            .any(|s| stem.ends_with(s))
}

/// A coarse kind of a changed path, for grouping seeds (the script's
/// `file_class`).
pub(crate) fn file_class(path: &str) -> String {
    let first = path.split('/').next().unwrap_or(path);
    if first.contains(':') {
        return format!("node:{}", path.split(':').next().unwrap_or(""));
    }
    let base = path.rsplit('/').next().unwrap_or(path);
    let ext = base
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let ext = ext.as_str();
    let c = if LOCKFILES.contains(&base) {
        "lockfile"
    } else if base == "package.json" {
        "manifest"
    } else if (base.starts_with("tsconfig") && base.ends_with(".json")) || base == "jsconfig.json" {
        "tsconfig"
    } else if base.starts_with(".env") {
        "env-file"
    } else if is_tool_config(base) {
        "tool-config"
    } else if base.ends_with(".snap") || path.contains("/__snapshots__/") {
        "snapshot"
    } else if base.contains(".stories.") {
        "story"
    } else if base.ends_with(".d.ts") || base.ends_with(".d.mts") || base.ends_with(".d.cts") {
        "dts"
    } else if path.contains("/__mocks__/") {
        "mock"
    } else if is_test_name(base) || base.ends_with("_test.go") || path.contains("/__tests__/") {
        "test"
    } else if path.contains("/__fixtures__/")
        || path.contains("/fixtures/")
        || path.contains("/fixture/")
        || path.contains("/testdata/")
    {
        "fixture"
    } else if ASSET_EXT.contains(&ext) || ext == "vert" {
        "asset"
    } else if matches!(ext, "md" | "mdx" | "txt" | "rst") {
        "doc"
    } else if matches!(
        ext,
        "json"
            | "yaml"
            | "yml"
            | "toml"
            | "json5"
            | "jsonc"
            | "csv"
            | "sql"
            | "graphql"
            | "gql"
            | "proto"
            | "html"
            | "xml"
    ) {
        "data"
    } else if CODE_EXT.contains(&ext) {
        "source"
    } else {
        "other"
    };
    c.to_owned()
}

fn is_code_like(c: &str) -> bool {
    matches!(c, "source" | "test" | "mock" | "story" | "dts" | "fixture")
}

fn is_config_like(c: &str) -> bool {
    matches!(
        c,
        "manifest" | "tsconfig" | "env-file" | "tool-config" | "lockfile"
    )
}

/// The label of a test-framework edge whose role is shared by the tests of a
/// project (`jest:setup_file`), from its specifier.
fn shared_fw_role(spec: &str) -> bool {
    let Some((fw, rest)) = spec.split_once(':') else {
        return false;
    };
    if !matches!(
        fw,
        "jest" | "vitest" | "pytest" | "gotest" | "cargo-test" | "playwright"
    ) {
        return false;
    }
    let end = rest
        .find(|c: char| !(c.is_ascii_alphabetic() || c == '_' || c == '.'))
        .unwrap_or(rest.len());
    matches!(
        &rest[..end],
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
            | "runner"
            | "conftest"
            | "plugin"
    )
}

/// The reverse adjacency of the evaluated view (dependency → dependents) with a
/// class and modifiers per edge.
pub(crate) struct Rev {
    ptr: Vec<u32>,
    src: Vec<u32>,
    cls: Vec<u8>,
    mods: Vec<u8>,
    /// Task, membership and part nodes.
    pub(crate) is_task: Vec<bool>,
}

impl Rev {
    pub(crate) fn build<G: GraphView>(g: &G) -> Rev {
        let n = g.node_count() as usize;
        let is_task: Vec<bool> = (0..n)
            .map(|i| {
                g.lang(NodeId(i as u32))
                    .is_some_and(|l| l.starts_with("task"))
            })
            .collect();
        let mut edges: Vec<(u32, u32, u8, u8)> = Vec::new();
        let mut ptr = vec![0u32; n + 1];
        for s in 0..n {
            for e in g.out_edges(NodeId(s as u32)) {
                let Some(to) = e.to else { continue };
                let t = to.index();
                let cls = if is_task[s] || is_task[t] {
                    TASK
                } else if e.specifier.is_some_and(shared_fw_role) {
                    FW
                } else {
                    match e.kind() {
                        RefKind::FsRead | RefKind::FsReadDir | RefKind::Asset | RefKind::Embed => {
                            READ
                        }
                        RefKind::Manifest => MANIFEST,
                        _ => PLAIN,
                    }
                };
                let f = e.flags();
                let mods = if f.contains(RefFlags::TYPE_ONLY) {
                    M_TYPE
                } else {
                    0
                } | if f.contains(RefFlags::DATA) {
                    M_DATA
                } else {
                    0
                };
                edges.push((t as u32, s as u32, cls, mods));
                ptr[t + 1] += 1;
            }
        }
        for i in 0..n {
            ptr[i + 1] += ptr[i];
        }
        let m = edges.len();
        let (mut src, mut cls, mut mods) = (vec![0u32; m], vec![0u8; m], vec![0u8; m]);
        let mut pos = ptr.clone();
        for (t, s, c, md) in edges {
            let p = pos[t as usize] as usize;
            src[p] = s;
            cls[p] = c;
            mods[p] = md;
            pos[t as usize] += 1;
        }
        Rev {
            ptr,
            src,
            cls,
            mods,
            is_task,
        }
    }

    pub(crate) fn node_count(&self) -> usize {
        self.is_task.len()
    }

    fn range(&self, x: u32) -> std::ops::Range<usize> {
        self.ptr[x as usize] as usize..self.ptr[x as usize + 1] as usize
    }

    /// [`Rev::up`] breadth first, with the node each reached node was reached
    /// from (itself for a start, `u32::MAX` for an unreached node): the
    /// shortest chains from the starts.
    pub(crate) fn bfs(
        &self,
        starts: impl IntoIterator<Item = u32>,
        allow: u8,
        deny: u8,
    ) -> Vec<u32> {
        let mut pred = vec![NONE; self.node_count()];
        let mut q: std::collections::VecDeque<u32> = std::collections::VecDeque::default();
        for s in starts {
            if pred[s as usize] == NONE {
                pred[s as usize] = s;
                q.push_back(s);
            }
        }
        while let Some(x) = q.pop_front() {
            for i in self.range(x) {
                if self.cls[i] & allow != 0 && self.mods[i] & deny == 0 {
                    let s = self.src[i];
                    if pred[s as usize] == NONE {
                        pred[s as usize] = x;
                        q.push_back(s);
                    }
                }
            }
        }
        pred
    }

    /// The dependents of `starts` (reverse closure) over edges whose class is
    /// in `allow` and that carry no modifier of `deny`.
    pub(crate) fn up(
        &self,
        starts: impl IntoIterator<Item = u32>,
        allow: u8,
        deny: u8,
    ) -> Vec<bool> {
        let mut seen = vec![false; self.node_count()];
        let mut q: Vec<u32> = Vec::new();
        for s in starts {
            if !seen[s as usize] {
                seen[s as usize] = true;
                q.push(s);
            }
        }
        while let Some(x) = q.pop() {
            for i in self.range(x) {
                if self.cls[i] & allow != 0 && self.mods[i] & deny == 0 {
                    let s = self.src[i];
                    if !seen[s as usize] {
                        seen[s as usize] = true;
                        q.push(s);
                    }
                }
            }
        }
        seen
    }
}

/// What the attribution reads besides the edges.
pub(crate) struct Recorded<'a> {
    /// The evaluation passes.
    pub(crate) passes: &'a [PassRecord],
    /// The changed paths the engine was given as file changes.
    pub(crate) kept: HashSet<&'a str>,
    /// Per node: selected in the end.
    pub(crate) selected: Vec<bool>,
    /// Per node: an entry the engine selected through a change (after barrel
    /// and erase pruning), not through the floor.
    pub(crate) engine_change: Vec<bool>,
    /// The groups of entries selected through a name.
    pub(crate) named: &'a [NamedGroup],
    /// The entries each later layer selects by itself: steering, workflow,
    /// services, and the loads with an unknown target that the change can
    /// reach (`crate::loads`).
    pub(crate) layers: [(&'static str, Vec<u32>); 4],
}

fn deny_of(p: &PassRecord) -> u8 {
    if p.runtime_only { M_TYPE } else { 0 }
}

/// The funnel id and the seed class of one seed. The engine keeps the first
/// cause of a node only: a node whose own file changed counts as that change,
/// whatever else seeded it first.
pub(crate) fn seed_group(
    path: &str,
    kept: &HashSet<&str>,
    kind: &str,
    detail: &str,
) -> (String, String) {
    if kept.contains(path) {
        return (format!("chg:{path}"), file_class(path));
    }
    let first = detail.split('\u{1f}').next().unwrap_or(detail);
    match kind {
        "modified" | "deleted" => (format!("chg:{detail}"), file_class(detail)),
        "input" => {
            let base = detail.rsplit('/').next().unwrap_or(detail);
            if LOCKFILES.contains(&base) {
                (format!("lock-link:{detail}"), "package".to_owned())
            } else {
                (format!("input:{detail}"), "input".to_owned())
            }
        }
        "package" => (format!("pkg:{detail}"), "package".to_owned()),
        "probe" | "membership" | "scope" => (format!("{kind}:{first}"), "listing".to_owned()),
        _ => (format!("{kind}:{first}"), kind.to_owned()),
    }
}

/// The changed path (or package) behind a seed group id: `chg:a.ts`,
/// `input:a.ts`, `probe:a.ts` and `membership:a.ts` are all the change of
/// `a.ts`.
pub(crate) fn origin_of(key: &str) -> String {
    match key.split_once(':') {
        Some(("pkg", _)) | None => key.to_owned(),
        Some((_, rest)) => format!("path:{rest}"),
    }
}

/// One layered search of a pass: the entries its seeds reach over a class of
/// edges get `class`.
struct Search {
    class: usize,
    /// The start nodes with the id of what they stand for (a seed group, an
    /// open site).
    seeds: Vec<(u32, String)>,
    allow: u8,
    deny: u8,
    /// Whether only entries the engine selected through a change count.
    change: bool,
    /// Entries the search does not count for.
    exempt: HashSet<u32>,
}

/// The searches of one pass, in the order of [`CLASSES`], and the entries whose
/// own file changed.
fn searches<G: GraphView>(g: &G, rec: &Recorded<'_>, p: &PassRecord) -> (Vec<Search>, Vec<u32>) {
    let deny = deny_of(p);
    let is_entry: HashSet<u32> = p.entries.iter().copied().collect();
    let mut own = Vec::new();
    // Per seed class: the seeds.
    let mut groups: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
    for (node, kind, detail) in &p.seeds {
        let path = g.path(NodeId(*node));
        let (key, cls) = seed_group(path, &rec.kept, kind, detail);
        groups.entry(cls).or_default().push((*node, key));
        if is_entry.contains(node) && rec.kept.contains(path) {
            own.push(*node);
        }
    }
    for &node in &p.deferred {
        let path = g.path(NodeId(node));
        groups
            .entry(file_class(path))
            .or_default()
            .push((node, format!("chg:{path}")));
        if is_entry.contains(&node) {
            own.push(node);
        }
    }
    let pick = |f: &dyn Fn(&str) -> bool| -> Vec<(u32, String)> {
        groups
            .iter()
            .filter(|(c, _)| f(c))
            .flat_map(|(_, s)| s.iter().cloned())
            .collect()
    };
    let code = pick(&is_code_like);
    let data = pick(&|c| matches!(c, "data" | "asset" | "doc" | "snapshot" | "other"));
    let tasks = pick(&|c| c.starts_with("node:"));
    let code_data: Vec<(u32, String)> = code.iter().chain(&data).cloned().collect();
    let every: Vec<(u32, String)> = pick(&|_| true);
    let every_tasks: Vec<(u32, String)> = every.iter().chain(&tasks).cloned().collect();
    let below_task = PLAIN | FW | READ | MANIFEST;
    let d = deny | M_TYPE;
    let mut out = Vec::new();
    let mut add = |class: &str, seeds: Vec<(u32, String)>, allow: u8, deny: u8| {
        if !seeds.is_empty() {
            out.push(Search {
                class: class_index(class),
                seeds,
                allow,
                deny,
                change: true,
                exempt: HashSet::new(),
            });
        }
    };
    add("import", code, PLAIN, d);
    add("data-import", data, PLAIN, d);
    add("fw-edge", code_data.clone(), PLAIN | FW, d);
    add("read", code_data, below_task, d);
    add("config-file", pick(&is_config_like), below_task, d);
    add("config-input", pick(&|c| c == "input"), below_task, d);
    add("lockfile", pick(&|c| c == "package"), below_task, d);
    add("listing", pick(&|c| c == "listing"), below_task, d);
    add("task", every_tasks, ALL_CLASSES, d);
    if deny == 0 {
        add("type-only", every, ALL_CLASSES, 0);
    }
    if p.floor {
        let site = |prefix: &str, nodes: &[u32]| -> Vec<(u32, String)> {
            nodes
                .iter()
                .map(|&n| (n, format!("{prefix}:{}", g.path(NodeId(n)))))
                .collect()
        };
        if !p.open.is_empty() {
            out.push(Search {
                class: class_index("floor"),
                seeds: site("open", &p.open),
                allow: ALL_CLASSES,
                deny: deny | M_DATA,
                change: false,
                exempt: HashSet::new(),
            });
        }
        if !p.per_entry.is_empty() {
            out.push(Search {
                class: class_index("floor"),
                seeds: site("open-per-entry", &p.per_entry),
                allow: ALL_CLASSES,
                deny: deny | M_DATA,
                change: false,
                exempt: p.exempt.iter().copied().collect(),
            });
        }
    }
    (out, own)
}

/// Per node (meaningful for entries): the classes of cause that reach it, as a
/// mask over [`CLASSES`].
pub(crate) fn first_causes<G: GraphView>(g: &G, rev: &Rev, rec: &Recorded<'_>) -> Vec<u32> {
    let n = rev.node_count();
    let mut reach = vec![0u32; n];
    let bit = |c: &str| 1u32 << class_index(c);
    for p in rec.passes {
        let (list, own) = searches(g, rec, p);
        for e in own {
            reach[e as usize] |= bit("self");
        }
        for s in &list {
            let r = rev.up(s.seeds.iter().map(|x| x.0), s.allow, s.deny);
            for &e in &p.entries {
                if r[e as usize]
                    && (!s.change || rec.engine_change[e as usize])
                    && !s.exempt.contains(&e)
                {
                    reach[e as usize] |= 1 << s.class;
                }
            }
        }
    }
    for grp in rec.named {
        for &e in &grp.entries {
            reach[e as usize] |= bit("named");
        }
    }
    for (name, rows) in &rec.layers {
        for &e in rows {
            reach[e as usize] |= bit(name);
        }
    }
    reach
}

/// Per selected entry whose first cause is a class of paths (a change or the
/// floor): the shortest chain of that class, from the changed node (or the open
/// site) to the entry, and the id of what the chain starts at (a seed group,
/// `open:<site>`).
pub(crate) fn class_chains<G: GraphView>(
    g: &G,
    rev: &Rev,
    rec: &Recorded<'_>,
    reach: &[u32],
) -> HashMap<u32, (Vec<u32>, String)> {
    let mut out: HashMap<u32, (Vec<u32>, String)> = HashMap::new();
    for p in rec.passes {
        let (list, own) = searches(g, rec, p);
        for e in own {
            if rec.selected[e as usize] {
                let key = format!("chg:{}", g.path(NodeId(e)));
                out.entry(e).or_insert((vec![e], key));
            }
        }
        for s in &list {
            let wanted: Vec<u32> = p
                .entries
                .iter()
                .copied()
                .filter(|&e| {
                    rec.selected[e as usize]
                        && first_of(reach[e as usize]) == s.class
                        && !s.exempt.contains(&e)
                        && !out.contains_key(&e)
                })
                .collect();
            if wanted.is_empty() {
                continue;
            }
            let pred = rev.bfs(s.seeds.iter().map(|x| x.0), s.allow, s.deny);
            let keys: HashMap<u32, &str> = s
                .seeds
                .iter()
                .rev()
                .map(|(n, k)| (*n, k.as_str()))
                .collect();
            for e in wanted {
                if pred[e as usize] == NONE {
                    continue;
                }
                let mut chain = vec![e];
                let mut v = e;
                while pred[v as usize] != v {
                    v = pred[v as usize];
                    chain.push(v);
                }
                chain.reverse();
                let key = keys.get(&v).copied().unwrap_or_default().to_owned();
                out.insert(e, (chain, key));
            }
        }
    }
    out
}

/// The kind of a node of the selection graph.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Root,
    Mech,
    /// A changed path or package: the parent of its seed groups.
    Origin,
    Seed,
    /// A file with open uncertainty.
    Open,
    /// A name group.
    Name,
    /// A later layer: steering, workflow, services.
    Layer,
    /// A copy of a graph node in the reach of one pass.
    Copy,
    /// An entry.
    Sink,
}

/// The mechanisms below the root.
pub(crate) const MECH_CHANGE: usize = 0;
pub(crate) const MECH_FLOOR: usize = 1;
pub(crate) const MECH_NAMED: usize = 2;
pub(crate) const MECH_LAYER: usize = 3;

/// The selection graph: `ROOT → mechanisms → … → entry sinks`.
pub(crate) struct Selection {
    pub(crate) kind: Vec<Kind>,
    /// For keyed nodes (mechanisms, origins, seeds, open sites, names, layers):
    /// the id.
    pub(crate) key: Vec<Option<String>>,
    /// For copies and sinks: the graph node.
    pub(crate) node: Vec<u32>,
    /// For copies: the mechanism ([`MECH_CHANGE`] or [`MECH_FLOOR`]).
    pub(crate) mech_of: Vec<u8>,
    pub(crate) succ: Vec<Vec<u32>>,
    pub(crate) ids: HashMap<String, u32>,
    /// Graph node → sink.
    pub(crate) sinks: HashMap<u32, u32>,
    pub(crate) mech: [u32; 4],
    /// Seed group id → seed class.
    pub(crate) seed_class: HashMap<String, String>,
}

const NONE: u32 = u32::MAX;

impl Selection {
    fn add(&mut self, kind: Kind, key: Option<String>, node: u32, mech: u8) -> u32 {
        let i = self.kind.len() as u32;
        self.kind.push(kind);
        self.key.push(key);
        self.node.push(node);
        self.mech_of.push(mech);
        self.succ.push(Vec::new());
        i
    }

    fn keyed(&mut self, key: &str, kind: Kind) -> u32 {
        if let Some(&i) = self.ids.get(key) {
            return i;
        }
        let i = self.add(kind, Some(key.to_owned()), NONE, 0);
        self.ids.insert(key.to_owned(), i);
        i
    }

    fn sink(&mut self, e: u32) -> u32 {
        if let Some(&i) = self.sinks.get(&e) {
            return i;
        }
        let i = self.add(Kind::Sink, None, e, 0);
        self.sinks.insert(e, i);
        i
    }

    /// Adds the copies of the nodes reached from `starts` (graph node →
    /// parents) and their edges; the entries of `ents` for which `ok` holds
    /// get an edge to their sink.
    fn space(
        &mut self,
        rev: &Rev,
        mech: u8,
        starts: &BTreeMap<u32, Vec<u32>>,
        deny: u8,
        ents: &[u32],
        ok: &dyn Fn(u32) -> bool,
    ) {
        let reach = rev.up(starts.keys().copied(), ALL_CLASSES, deny);
        let mut nid = vec![NONE; rev.node_count()];
        for (x, r) in reach.iter().enumerate() {
            if *r {
                nid[x] = self.add(Kind::Copy, None, x as u32, mech);
            }
        }
        for x in 0..rev.node_count() {
            if !reach[x] {
                continue;
            }
            let cx = nid[x] as usize;
            for i in rev.range(x as u32) {
                if rev.mods[i] & deny == 0 {
                    let s = nid[rev.src[i] as usize];
                    debug_assert_ne!(s, NONE);
                    self.succ[cx].push(s);
                }
            }
        }
        for (x, parents) in starts {
            for &p in parents {
                self.succ[p as usize].push(nid[*x as usize]);
            }
        }
        for &e in ents {
            if reach[e as usize] && ok(e) {
                let s = self.sink(e);
                self.succ[nid[e as usize] as usize].push(s);
            }
        }
    }

    pub(crate) fn build<G: GraphView>(g: &G, rev: &Rev, rec: &Recorded<'_>) -> Selection {
        let mut s = Selection {
            kind: Vec::new(),
            key: Vec::new(),
            node: Vec::new(),
            mech_of: Vec::new(),
            succ: Vec::new(),
            ids: HashMap::new(),
            sinks: HashMap::new(),
            mech: [0; 4],
            seed_class: HashMap::new(),
        };
        s.add(Kind::Root, None, NONE, 0);
        let names = [
            "mechanism:change",
            "mechanism:floor",
            "mechanism:named",
            "mechanism:layers",
        ];
        for (i, name) in names.iter().enumerate() {
            s.mech[i] = s.keyed(name, Kind::Mech);
        }
        s.succ[0] = s.mech.to_vec();
        let (chg_root, floor_root, named_root, layer_root) =
            (s.mech[0], s.mech[1], s.mech[2], s.mech[3]);
        let selected = |e: u32| rec.selected[e as usize];
        for p in rec.passes {
            let deny = deny_of(p);
            let mut starts: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            let mut seed = |s: &mut Selection, node: u32, key: String, cls: String| {
                let v = s.keyed(&key, Kind::Seed);
                let o = s.keyed(&origin_of(&key), Kind::Origin);
                s.seed_class.insert(key, cls);
                if !s.succ[chg_root as usize].contains(&o) {
                    s.succ[chg_root as usize].push(o);
                }
                if !s.succ[o as usize].contains(&v) {
                    s.succ[o as usize].push(v);
                }
                starts.entry(node).or_default().push(v);
            };
            for (node, kind, detail) in &p.seeds {
                let (key, cls) = seed_group(g.path(NodeId(*node)), &rec.kept, kind, detail);
                seed(&mut s, *node, key, cls);
            }
            for &node in &p.deferred {
                let path = g.path(NodeId(node));
                seed(&mut s, node, format!("chg:{path}"), file_class(path));
            }
            s.space(rev, MECH_CHANGE as u8, &starts, deny, &p.entries, &|e| {
                selected(e) && rec.engine_change[e as usize]
            });
            if p.floor {
                let mut starts: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
                for &node in &p.open {
                    let v = s.keyed(&format!("open:{}", g.path(NodeId(node))), Kind::Open);
                    s.succ[floor_root as usize].push(v);
                    starts.entry(node).or_default().push(v);
                }
                s.space(
                    rev,
                    MECH_FLOOR as u8,
                    &starts,
                    deny | M_DATA,
                    &p.entries,
                    &selected,
                );
                if !p.per_entry.is_empty() {
                    let ex: HashSet<u32> = p.exempt.iter().copied().collect();
                    let mut starts: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
                    for &node in &p.per_entry {
                        let key = format!("open-per-entry:{}", g.path(NodeId(node)));
                        let v = s.keyed(&key, Kind::Open);
                        s.succ[floor_root as usize].push(v);
                        starts.entry(node).or_default().push(v);
                    }
                    s.space(
                        rev,
                        MECH_FLOOR as u8,
                        &starts,
                        deny | M_DATA,
                        &p.entries,
                        &|e| selected(e) && !ex.contains(&e),
                    );
                }
            }
        }
        for grp in rec.named {
            let v = s.keyed(&format!("name:{}:{}", grp.kind, grp.label), Kind::Name);
            s.succ[named_root as usize].push(v);
            for &e in &grp.entries {
                if selected(e) {
                    let k = s.sink(e);
                    s.succ[v as usize].push(k);
                }
            }
        }
        for (name, rows) in &rec.layers {
            if rows.is_empty() {
                continue;
            }
            let v = s.keyed(&format!("layer:{name}"), Kind::Layer);
            s.succ[layer_root as usize].push(v);
            for &e in rows {
                if selected(e) {
                    let k = s.sink(e);
                    s.succ[v as usize].push(k);
                }
            }
        }
        for list in &mut s.succ {
            if list.len() > 1 {
                list.sort_unstable();
                list.dedup();
            }
        }
        s
    }

    /// Per node: the number of sinks it dominates, with the root's children
    /// restricted to the mechanisms `mechs`; the number of sinks reached;
    /// and the immediate dominators (`u32::MAX` for unreached nodes).
    pub(crate) fn dominated(&self, mechs: &[usize]) -> Dominated {
        let roots: Vec<u32> = mechs.iter().map(|&m| self.mech[m]).collect();
        let (idom, rpo) = dominators(&self.succ, &roots);
        let mut count = vec![0u32; self.succ.len()];
        let mut sinks = 0;
        for &v in rpo.iter().rev() {
            if self.kind[v as usize] == Kind::Sink {
                count[v as usize] += 1;
                sinks += 1;
            }
            if v != 0 && idom[v as usize] != NONE {
                count[idom[v as usize] as usize] += count[v as usize];
            }
        }
        Dominated { count, sinks, idom }
    }

    /// Per group of start nodes: the number of sinks reachable from any of
    /// them. `each` is called once per (sink's graph node, group) pair.
    pub(crate) fn below(&self, groups: &[Vec<u32>], mut each: impl FnMut(u32, usize)) -> Vec<u32> {
        let mut out = vec![0u32; groups.len()];
        let mut mask = vec![0u64; self.succ.len()];
        let mut stack: Vec<u32> = Vec::new();
        for (b, batch) in groups.chunks(64).enumerate() {
            mask.iter_mut().for_each(|m| *m = 0);
            for (i, grp) in batch.iter().enumerate() {
                for &v in grp {
                    if mask[v as usize] == 0 {
                        stack.push(v);
                    }
                    mask[v as usize] |= 1 << i;
                }
            }
            while let Some(v) = stack.pop() {
                let m = mask[v as usize];
                for &w in &self.succ[v as usize] {
                    let old = mask[w as usize];
                    if old | m != old {
                        mask[w as usize] = old | m;
                        stack.push(w);
                    }
                }
            }
            for (&e, &k) in &self.sinks {
                let mut m = mask[k as usize];
                while m != 0 {
                    let i = m.trailing_zeros() as usize;
                    m &= m - 1;
                    out[b * 64 + i] += 1;
                    each(e, b * 64 + i);
                }
            }
        }
        out
    }
}

/// The result of [`Selection::dominated`].
pub(crate) struct Dominated {
    pub(crate) count: Vec<u32>,
    pub(crate) sinks: u32,
    pub(crate) idom: Vec<u32>,
}

/// Immediate dominators (Cooper, Harvey, Kennedy) of the nodes reachable from
/// node 0, whose successors are `roots` instead of `succ[0]`. Returns the
/// dominators (`u32::MAX` for unreached nodes) and the reverse postorder.
// Invariant: the DFS stack is non-empty inside `while let Some(..) = stack.last()`.
#[allow(clippy::expect_used)]
fn dominators(succ: &[Vec<u32>], roots: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let n = succ.len();
    let out = |v: u32| -> &[u32] { if v == 0 { roots } else { &succ[v as usize] } };
    let mut order: Vec<u32> = Vec::new();
    let mut seen = vec![false; n];
    let mut stack: Vec<(u32, u32)> = vec![(0, 0)];
    seen[0] = true;
    while let Some(&(v, i)) = stack.last() {
        let s = out(v);
        if (i as usize) < s.len() {
            stack.last_mut().expect("non-empty").1 += 1;
            let w = s[i as usize];
            if !seen[w as usize] {
                seen[w as usize] = true;
                stack.push((w, 0));
            }
        } else {
            order.push(v);
            stack.pop();
        }
    }
    let mut po = vec![NONE; n];
    for (i, &v) in order.iter().enumerate() {
        po[v as usize] = i as u32;
    }
    // Predecessors, as a CSR over the reached nodes.
    let mut ptr = vec![0u32; n + 1];
    for &v in &order {
        for &w in out(v) {
            ptr[w as usize + 1] += 1;
        }
    }
    for i in 0..n {
        ptr[i + 1] += ptr[i];
    }
    let mut pred = vec![0u32; ptr[n] as usize];
    let mut pos = ptr.clone();
    for &v in &order {
        for &w in out(v) {
            pred[pos[w as usize] as usize] = v;
            pos[w as usize] += 1;
        }
    }
    let mut idom = vec![NONE; n];
    idom[0] = 0;
    let rpo: Vec<u32> = order.iter().rev().copied().collect();
    let mut changed = true;
    while changed {
        changed = false;
        for &v in &rpo[1..] {
            let mut new = NONE;
            for &p in &pred[ptr[v as usize] as usize..ptr[v as usize + 1] as usize] {
                if idom[p as usize] == NONE {
                    continue;
                }
                if new == NONE {
                    new = p;
                    continue;
                }
                let (mut a, mut b) = (p, new);
                while a != b {
                    while po[a as usize] < po[b as usize] {
                        a = idom[a as usize];
                    }
                    while po[b as usize] < po[a as usize] {
                        b = idom[b as usize];
                    }
                }
                new = a;
            }
            if idom[v as usize] != new {
                idom[v as usize] = new;
                changed = true;
            }
        }
    }
    (idom, rpo)
}

#[cfg(test)]
mod tests {
    use turborepo_trace_graph::MemGraph;

    use super::*;

    #[test]
    fn file_classes() {
        for (path, class) in [
            ("pnpm-lock.yaml", "lockfile"),
            ("apps/web/package.json", "manifest"),
            ("tsconfig.base.json", "tsconfig"),
            (".env.local", "env-file"),
            ("jest.config.ts", "tool-config"),
            ("apps/web/next.config.mjs", "tool-config"),
            (".eslintrc.cjs", "tool-config"),
            ("vitest.setup.ts", "tool-config"),
            ("turbo.json", "tool-config"),
            ("src/__snapshots__/a.snap", "snapshot"),
            ("src/button.stories.tsx", "story"),
            ("types/global.d.ts", "dts"),
            ("src/__mocks__/fs.js", "mock"),
            ("src/a.test.tsx", "test"),
            ("pkg/a_test.go", "test"),
            ("src/a.spec.mjs", "test"),
            ("test/fixtures/a.json", "fixture"),
            ("src/a.css", "asset"),
            ("README.md", "doc"),
            ("data/a.yaml", "data"),
            ("src/a.ts", "source"),
            ("LICENSE", "other"),
            ("task:apps/web#build", "node:task"),
            ("jestish/x.ts", "source"),
        ] {
            assert_eq!(file_class(path), class, "{path}");
        }
    }

    #[test]
    fn origins() {
        assert_eq!(origin_of("chg:src/a.ts"), "path:src/a.ts");
        assert_eq!(origin_of("input:tsconfig.json"), "path:tsconfig.json");
        assert_eq!(origin_of("membership:src/new.ts"), "path:src/new.ts");
        assert_eq!(origin_of("pkg:react@18.0.0"), "pkg:react@18.0.0");
    }

    /// ```text
    /// a.ts ← hub.ts ← t1.test.ts, t2.test.ts
    /// b.ts ← t2.test.ts, t3.test.ts
    /// dyn.ts (open) ← t3.test.ts, t4.test.ts
    /// ```
    /// `a.ts` and `b.ts` changed.
    #[test]
    fn causes_and_funnels() {
        let mut b = MemGraph::builder();
        let a = b.file("a.ts");
        let bb = b.file("b.ts");
        let hub = b.file("hub.ts");
        let dynf = b.file("dyn.ts");
        let t: Vec<NodeId> = (1..=4).map(|i| b.file(&format!("t{i}.test.ts"))).collect();
        b.edge(hub, a, RefKind::Import);
        b.edge(t[0], hub, RefKind::Import);
        b.edge(t[1], hub, RefKind::Import);
        b.edge(t[1], bb, RefKind::Import);
        b.edge(t[2], bb, RefKind::Import);
        b.edge(t[2], dynf, RefKind::Import);
        b.edge(t[3], dynf, RefKind::Import);
        let g = b.build().unwrap();
        let rev = Rev::build(&g);
        let entries: Vec<u32> = t.iter().map(|n| n.0).collect();
        let pass = PassRecord {
            entries: entries.clone(),
            seeds: vec![
                (a.0, "modified", "a.ts".to_owned()),
                (bb.0, "modified", "b.ts".to_owned()),
            ],
            open: vec![dynf.0],
            floor: true,
            ..PassRecord::default()
        };
        let n = g.node_count() as usize;
        let mut selected = vec![false; n];
        let mut engine_change = vec![false; n];
        for (i, e) in entries.iter().enumerate() {
            selected[*e as usize] = true;
            engine_change[*e as usize] = i < 3;
        }
        let passes = [pass];
        let rec = Recorded {
            passes: &passes,
            kept: ["a.ts", "b.ts"].into_iter().collect(),
            selected,
            engine_change,
            named: &[],
            layers: [
                ("steering", Vec::new()),
                ("workflow", Vec::new()),
                ("services", Vec::new()),
                ("loads", Vec::new()),
            ],
        };
        let reach = first_causes(&g, &rev, &rec);
        let first: Vec<&str> = entries
            .iter()
            .map(|e| CLASSES[first_of(reach[*e as usize])])
            .collect();
        assert_eq!(first, ["import", "import", "import", "floor"]);
        assert_ne!(reach[t[2].index()] & (1 << class_index("floor")), 0);
        let chains = class_chains(&g, &rev, &rec, &reach);
        assert_eq!(
            chains[&t[0].0],
            (vec![a.0, hub.0, t[0].0], "chg:a.ts".to_owned())
        );
        // The shortest import chain of t2 starts at b.ts, not at the hub.
        assert_eq!(chains[&t[1].0], (vec![bb.0, t[1].0], "chg:b.ts".to_owned()));
        assert_eq!(
            chains[&t[3].0],
            (vec![dynf.0, t[3].0], "open:dyn.ts".to_owned())
        );

        let s = Selection::build(&g, &rev, &rec);
        let all = s.dominated(&[MECH_CHANGE, MECH_FLOOR, MECH_NAMED, MECH_LAYER]);
        assert_eq!(all.sinks, 4);
        let own = s.dominated(&[MECH_CHANGE, MECH_LAYER]);
        assert_eq!(own.sinks, 3);
        let id = |k: &str| s.ids[k] as usize;
        // Today: t1 needs a.ts; t2 has two changes; t3 a change and the floor; t4 the
        // floor.
        assert_eq!(all.count[id("path:a.ts")], 1);
        assert_eq!(all.count[id("path:b.ts")], 0);
        assert_eq!(all.count[id("open:dyn.ts")], 1);
        // Inside the change mechanism b.ts alone holds t3.
        assert_eq!(own.count[id("path:a.ts")], 1);
        assert_eq!(own.count[id("path:b.ts")], 1);
        assert_eq!(own.count[id("chg:b.ts")], 1);
        // The hub holds t1 only: t2 is also reached from b.ts.
        let hub_copy = (0..s.kind.len())
            .find(|&v| s.kind[v] == Kind::Copy && s.node[v] == hub.0 && s.mech_of[v] == 0)
            .unwrap();
        assert_eq!(own.count[hub_copy], 1);
        let below = s.below(
            &[
                vec![id("path:a.ts") as u32],
                vec![id("path:b.ts") as u32],
                vec![id("open:dyn.ts") as u32],
                vec![hub_copy as u32],
            ],
            |_, _| {},
        );
        assert_eq!(below, [2, 2, 2, 2]);
    }
}
