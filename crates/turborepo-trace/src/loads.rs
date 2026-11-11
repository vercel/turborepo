//! Bounded loads (`turborepo_trace_graph::loads`): which packages a load with
//! an unknown target can reach, from the repository's manifests, and the layer
//! that `affected` applies.
//!
//! A module load whose target cannot be evaluated is resolved by the module
//! system from the package of the loading file, so it can only bring in a file
//! of that package or of a package it declares. `reach` reads what every
//! first-party package declares (`dependencies`, `devDependencies`,
//! `optionalDependencies`, `peerDependencies`: every kind that an install
//! links) and builds, per package, what one load can reach:
//!
//! - the package itself and the packages nested in it (their files lie below
//!   its directory). Not the packages around it: a package in whose directory
//!   other packages live (a Go module at `scripts/` above twenty JavaScript
//!   packages) is no dependency of theirs, and counting it as one chains them
//!   all together;
//! - the first-party packages it declares, and those that a package around it
//!   declares, the root included (resolution walks up the directories, so the
//!   root's dependencies resolve from every package), each with what is nested
//!   in it.
//!
//! The root package is the exception: its directory is the repository, so a
//! load from one of its own files (the root minus the other packages) reaches
//! every package. Other packages do not reach the root package's files unless
//! they are nested in nothing else: the root is never declared.
//!
//! `TURBO_TRACE_DEBUG_LOADS_PACKAGE=0` turns the bound off for A/B measurements
//! (the loads are open again, as with `--strict`).
//! `TURBO_TRACE_DEBUG_LOADS_STEP=0` holds a load only to the declared closure
//! of the entry's package instead of to one step ([`step`]).

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    time::Instant,
};

use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Change, ChangeSet, Engine, GraphView, LoadReach, LoadStats,
    NodeSet, PackageId, PackageVersion,
};

/// Whether a load is held to what one resolution step reaches from the loading
/// file's package or from the entry's package: the rule in its direct form ("a
/// file loads only files of its own package and of the packages that package
/// declares", `turborepo_trace_graph::loads`). On by default.
/// `TURBO_TRACE_DEBUG_LOADS_STEP=0` turns it off for A/B measurements: a load
/// is then held only to the declared closure of the entry's package.
pub(crate) fn step() -> bool {
    turborepo_trace_core::debug::var_os("LOADS_STEP").is_none_or(|v| v != "0")
}

/// How the repository's packages are installed, as far as resolution between
/// first-party packages is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// One directory of links per package: a package resolves what it declares
    /// (pnpm's default, Yarn Plug'n'Play). `public`: name patterns that are
    /// also linked into the root `node_modules` (`public-hoist-pattern`),
    /// so every package resolves them.
    Strict { public: Vec<String> },
    /// Every workspace package is linked where every other one finds it (npm,
    /// Yarn classic and bun workspaces, `node-linker=hoisted`,
    /// `shamefully-hoist`), or the repository is one package: the declared
    /// packages bound nothing. The text says why.
    Hoisted(&'static str),
}

/// The value of `key` in npmrc-style text (`key=value` lines), and every value
/// of the list form `key[]=value`.
fn npmrc_values(text: &str, key: &str) -> Vec<String> {
    let list = format!("{key}[]");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.starts_with(';'))
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| k.trim() == key || k.trim() == list)
        .map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_owned())
        .collect()
}

/// The scalar value of a top-level `key:` of YAML text and the items of its
/// block list (`key:` followed by `  - item` lines). Enough for the few
/// settings read here.
fn yaml_values(text: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) else {
            continue;
        };
        let clean = |s: &str| {
            s.split(" #")
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['"', '\''])
                .to_owned()
        };
        let v = clean(rest);
        if !v.is_empty() {
            out.push(v);
            continue;
        }
        while let Some(next) = lines.peek() {
            let t = next.trim_start();
            if t.is_empty() || t.starts_with('#') {
                lines.next();
            } else if next.starts_with([' ', '\t', '-']) && t.starts_with('-') {
                out.push(clean(t.trim_start_matches('-')));
                lines.next();
            } else {
                break;
            }
        }
    }
    out
}

/// The files at the repository root that decide the install layout
/// ([`layout`]).
pub(crate) const LAYOUT_FILES: [&str; 3] = [".npmrc", "pnpm-workspace.yaml", ".yarnrc.yml"];

/// The install layout of the repository at `root`, from its configuration (no
/// `node_modules` is needed).
pub(crate) fn layout(root: &Path) -> Layout {
    let read = |name: &str| std::fs::read_to_string(root.join(name)).ok();
    let [npmrc, pnpm, yarnrc] = LAYOUT_FILES.map(read);
    layout_of(npmrc.as_deref(), pnpm.as_deref(), yarnrc.as_deref())
}

/// [`layout`] from the texts of [`LAYOUT_FILES`] (`None`: the file does not
/// exist).
pub(crate) fn layout_of(npmrc: Option<&str>, pnpm: Option<&str>, yarnrc: Option<&str>) -> Layout {
    let npmrc = npmrc.unwrap_or_default();
    if let Some(pnpm) = pnpm {
        let setting = |npm: &str, yaml: &str| {
            let mut v = npmrc_values(npmrc, npm);
            v.extend(yaml_values(pnpm, yaml));
            v
        };
        if setting("node-linker", "nodeLinker")
            .iter()
            .any(|v| v == "hoisted")
        {
            return Layout::Hoisted("node-linker is hoisted");
        }
        if setting("shamefully-hoist", "shamefullyHoist")
            .iter()
            .any(|v| v == "true")
        {
            return Layout::Hoisted("shamefully-hoist is on");
        }
        // The patterns pnpm hoisted publicly by default before version 10 stay in: a
        // workspace package with such a name was linked into the root.
        let mut public = vec!["*eslint*".to_owned(), "*prettier*".to_owned()];
        public.extend(
            setting("public-hoist-pattern", "publicHoistPattern")
                .into_iter()
                .filter(|p| !p.starts_with('!')),
        );
        if public.iter().any(|p| p == "*") {
            return Layout::Hoisted("public-hoist-pattern is *");
        }
        return Layout::Strict { public };
    }
    if let Some(yarnrc) = yarnrc {
        let linker = yaml_values(yarnrc, "nodeLinker");
        return match linker.first().map(String::as_str) {
            None | Some("pnp" | "pnpm") => Layout::Strict { public: Vec::new() },
            Some(_) => Layout::Hoisted("Yarn's nodeLinker is node-modules"),
        };
    }
    Layout::Hoisted("no pnpm workspace and no Yarn Plug'n'Play: workspace packages are hoisted")
}

/// Whether `name` matches `pattern` (`*` stands for any text).
fn name_matches(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };
    for p in parts {
        match rest.find(p) {
            Some(i) => rest = &rest[i + p.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Whether loads with an unknown target are bounded by the declared packages:
/// the switch (`TURBO_TRACE_DEBUG_LOADS_PACKAGE=0` turns it off) and a layout
/// in which a package resolves only what it declares.
pub(crate) fn enabled(root: &Path) -> bool {
    turborepo_trace_core::debug::var_os("LOADS_PACKAGE").is_none_or(|v| v != "0")
        && matches!(layout(root), Layout::Strict { .. })
}

const KINDS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

/// The names a manifest declares: the keys of its dependency maps, and for a
/// `workspace:` alias (`"a": "workspace:@scope/b@*"`) the package it points to.
fn declared_names(manifest: &Path) -> Vec<String> {
    // The file itself, with its dependency lists: not the view of it that module
    // resolution gets (`turborepo_trace_core::manifest`), which leaves them
    // out.
    match std::fs::read(manifest) {
        Ok(bytes) => declared_in(&bytes),
        Err(_) => Vec::new(),
    }
}

/// [`declared_names`] of a manifest's text.
fn declared_in(bytes: &[u8]) -> Vec<String> {
    let Ok(serde_json::Value::Object(m)) = serde_json::from_slice::<serde_json::Value>(bytes)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for k in KINDS {
        let Some(serde_json::Value::Object(deps)) = m.get(k) else {
            continue;
        };
        for (name, range) in deps {
            out.push(name.clone());
            if let Some(alias) = range.as_str().and_then(|r| r.strip_prefix("workspace:"))
                && let Some(at) = alias.rfind('@').filter(|&i| i > 0)
            {
                out.push(alias[..at].to_owned());
            }
        }
    }
    out
}

/// What a load from a file of each first-party package of `g` can reach in one
/// step (see the module documentation), with the number of manifests read.
pub(crate) fn reach<G: GraphView>(g: &G, root: &Path) -> (LoadReach, usize) {
    let np = g.package_count();
    // First-party packages: id, root directory.
    let mut first: Vec<(u32, String)> = Vec::new();
    let mut by_name: HashMap<&str, u32> = HashMap::new();
    let mut by_root: HashMap<String, u32> = HashMap::new();
    for p in 0..np {
        let info = g.package_info(PackageId(p));
        if !matches!(info.version, PackageVersion::Workspace) {
            continue;
        }
        let r = info.root.trim_matches('/');
        let r = if r == "." { "" } else { r };
        first.push((p, r.to_owned()));
        by_name.entry(info.name).or_insert(p);
        by_root.entry(r.to_owned()).or_insert(p);
    }
    // What each one declares, read in parallel (thousands of small files in a
    // monorepo).
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let chunk = first.len().div_ceil(threads).max(1);
    let declared: Vec<Vec<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = first
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|(_, r)| declared_names(&root.join(r).join("package.json")))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    });
    let manifests = declared.iter().filter(|d| !d.is_empty()).count();
    let mut decl: HashMap<u32, Vec<u32>> = HashMap::new();
    for ((p, _), names) in first.iter().zip(&declared) {
        let ids: Vec<u32> = names
            .iter()
            .filter_map(|n| by_name.get(n.as_str()).copied())
            .filter(|q| q != p)
            .collect();
        if !ids.is_empty() {
            decl.insert(*p, ids);
        }
    }
    // The packages around each one, nearest first; the root package (`""`) last.
    let around = |r: &str| -> Vec<u32> {
        let mut out = Vec::new();
        let mut d = r;
        while !d.is_empty() {
            d = d.rsplit_once('/').map_or("", |x| x.0);
            if let Some(&q) = by_root.get(d) {
                out.push(q);
            }
        }
        out
    };
    let root_id = by_root.get("").copied();
    // Packages linked into the root `node_modules` whatever declares them: every
    // package resolves them. In a hoisted layout that is every package.
    let public = match layout(root) {
        Layout::Strict { public } => public,
        Layout::Hoisted(_) => vec!["*".to_owned()],
    };
    let everywhere: Vec<u32> = first
        .iter()
        .map(|(p, _)| *p)
        .filter(|&p| {
            let name = g.package_info(PackageId(p)).name;
            Some(p) != root_id && public.iter().any(|pat| name_matches(pat, name))
        })
        .collect();
    // Nested packages: for every package, the ones at or below its directory. The
    // root package holds only its own files (the root minus the other
    // packages).
    let mut below: HashMap<u32, Vec<u32>> = HashMap::new();
    for (p, r) in &first {
        below.entry(*p).or_default().push(*p);
        for q in around(r) {
            if Some(q) != root_id {
                below.entry(q).or_default().push(*p);
            }
        }
    }
    let mut out = LoadReach::new(np);
    for (p, r) in &first {
        if Some(*p) == root_id {
            // The directory of the root package is the repository: a load from one of its
            // files reaches every package below it (a relative specifier, a runner that
            // looks packages up below its root directory).
            for (q, _) in &first {
                out.add(PackageId(*p), PackageId(*q));
            }
            continue;
        }
        let mut set: BTreeSet<u32> = BTreeSet::new();
        set.extend(below.get(p).into_iter().flatten().copied());
        // The packages around it lend what they declare (resolution walks up), not
        // themselves: a Go module in `scripts/` is around every package below
        // `scripts/`, and counting it as part of each would make them all one.
        let up = around(r);
        for q in std::iter::once(*p).chain(up.iter().copied()) {
            for d in decl.get(&q).into_iter().flatten() {
                set.extend(below.get(d).into_iter().flatten().copied());
            }
        }
        for d in &everywhere {
            set.extend(below.get(d).into_iter().flatten().copied());
        }
        for q in set {
            out.add(PackageId(*p), PackageId(q));
        }
    }
    out.finish();
    (out, manifests)
}

/// The changed paths that alter what a package can resolve without lying in a
/// package it depends on (`turborepo_trace_graph::loads`, "A change of the
/// bound itself"), as `(index into paths, package)`:
///
/// - the manifest of a package in whose directory other packages live, the root
///   above all: its declarations resolve from every package below it. When the
///   first-party packages it declares differ between the base and the tree,
///   every package below it resolves something else;
/// - a file that decides the install layout ([`LAYOUT_FILES`]): when the layout
///   differs, every package does;
/// - the manifest of a package that the change removes: the package's files are
///   in no package of the tree (they count for the nearest one that is left)
///   and no manifest of the tree can name it, so the packages that could
///   resolve it are affected by its removal: the ones that still declare its
///   name, the ones below them, and every package when the layout links the
///   name for all or the name is not known.
///
/// `base`: the text of a path at the revision the change is measured from
/// (`None`: not known, or the path did not exist). Without a known base a
/// change of such a file counts as a change of what it decides.
pub(crate) fn lent<G: GraphView>(
    g: &G,
    root: &Path,
    paths: &[Change],
    base: &dyn Fn(&str) -> Option<String>,
) -> Vec<(u32, PackageId)> {
    use turborepo_trace_graph::ChangeKind;
    let mut out = Vec::new();
    let manifest = |c: &Change| c.path == "package.json" || c.path.ends_with("/package.json");
    let layout_file = |c: &Change| LAYOUT_FILES.contains(&c.path.as_str());
    if !paths.iter().any(|c| manifest(c) || layout_file(c)) {
        return out;
    }
    // First-party packages by root directory and by name.
    let mut first: Vec<(u32, String)> = Vec::new();
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for p in 0..g.package_count() {
        let info = g.package_info(PackageId(p));
        if !matches!(info.version, PackageVersion::Workspace) {
            continue;
        }
        let r = info.root.trim_matches('/');
        first.push((p, if r == "." { "" } else { r }.to_owned()));
        names.insert(info.name);
    }
    // The text of a path before the change: `Ok(None)` if it did not exist, `Err`
    // if that is not known.
    let before = |c: &Change| -> Result<Option<String>, ()> {
        match (c.kind, base(&c.path)) {
            (ChangeKind::Added, _) => Ok(None),
            (_, Some(text)) => Ok(Some(text)),
            (_, None) => Err(()),
        }
    };
    let mut everything: Option<u32> = None;
    // Packages that the change removes: a manifest below the root that is gone,
    // with the name it had. The tree has neither the package nor a way to name
    // it, so what could resolve it is read from before the change. Without the
    // name every package counts.
    let mut removed: Vec<(u32, String)> = Vec::new();
    for (i, c) in paths.iter().enumerate() {
        if !manifest(c)
            || c.path == "package.json"
            || c.kind == ChangeKind::Added
            || root.join(&c.path).exists()
        {
            continue;
        }
        let name = base(&c.path)
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("name")?.as_str().map(str::to_owned));
        match name {
            Some(n) => removed.push((i as u32, n)),
            None => everything = everything.or(Some(i as u32)),
        }
    }
    // The install layout, before and after.
    if let Some(i) = paths.iter().position(layout_file) {
        let head = layout(root);
        let texts: Vec<Result<Option<String>, ()>> = LAYOUT_FILES
            .iter()
            .map(|name| match paths.iter().find(|c| c.path == *name) {
                Some(c) => before(c),
                None => Ok(std::fs::read_to_string(root.join(name)).ok()),
            })
            .collect();
        let same = match &texts[..] {
            [Ok(npmrc), Ok(pnpm), Ok(yarnrc)] => {
                layout_of(npmrc.as_deref(), pnpm.as_deref(), yarnrc.as_deref()) == head
            }
            _ => false,
        };
        if !same {
            everything = Some(i as u32);
        }
    }
    for (i, c) in paths.iter().enumerate() {
        if everything.is_some() {
            break;
        }
        if !manifest(c) {
            continue;
        }
        let dir = c.path.strip_suffix("package.json").unwrap_or("");
        let dir = dir.trim_end_matches('/');
        // The packages below this manifest's directory (every package for the root's).
        let below: Vec<u32> = first
            .iter()
            .filter(|(_, r)| {
                r != dir
                    && (dir.is_empty()
                        || r.strip_prefix(dir)
                            .is_some_and(|rest| rest.starts_with('/')))
            })
            .map(|(p, _)| *p)
            .collect();
        if below.is_empty() {
            continue;
        }
        // A declaration of a removed package was one of a first-party package.
        let declared = |bytes: &[u8]| -> BTreeSet<String> {
            declared_in(bytes)
                .into_iter()
                .filter(|n| names.contains(n.as_str()) || removed.iter().any(|r| r.1 == *n))
                .collect()
        };
        let head = std::fs::read(root.join(&c.path))
            .map(|b| declared(&b))
            .unwrap_or_default();
        let same = match before(c) {
            Ok(Some(text)) => declared(text.as_bytes()) == head,
            Ok(None) => head.is_empty(),
            Err(()) => false,
        };
        if same {
            continue;
        }
        if dir.is_empty() {
            everything = Some(i as u32);
        } else {
            out.extend(below.into_iter().map(|p| (i as u32, PackageId(p))));
        }
    }
    // A removed package could be resolved by every package when the layout links
    // its name for all, and by the packages whose manifest still declares it (a
    // declaration that went with the package is a changed manifest: its package
    // is affected, and what it lent is compared above) with the packages below
    // them.
    if everything.is_none()
        && !removed.is_empty()
        && let Layout::Strict { public } = layout(root)
    {
        everything = removed
            .iter()
            .find(|r| public.iter().any(|pat| name_matches(pat, &r.1)))
            .map(|r| r.0);
        for (p, r) in &first {
            if everything.is_some() {
                break;
            }
            let declares = declared_names(&root.join(r).join("package.json"));
            let Some(i) = removed
                .iter()
                .find(|x| declares.contains(&x.1))
                .map(|x| x.0)
            else {
                continue;
            };
            if r.is_empty() {
                everything = Some(i);
                break;
            }
            out.push((i, PackageId(*p)));
            out.extend(
                first
                    .iter()
                    .filter(|(_, below)| {
                        below
                            .strip_prefix(r.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                    })
                    .map(|(q, _)| (i, PackageId(*q))),
            );
        }
    }
    if let Some(i) = everything {
        out = first.iter().map(|(p, _)| (i, PackageId(*p))).collect();
    }
    out
}

/// What the layer did, for `--stats`.
pub(crate) struct Applied {
    pub stats: LoadStats,
    pub manifests: usize,
    pub elapsed: std::time::Duration,
    /// [`step`] was on.
    pub step: bool,
}

impl Applied {
    pub(crate) fn stats_line(&self) -> String {
        let held = if self.step {
            "held to the package of its file and of each entry and what they declare"
        } else {
            "held to what each entry's package depends on"
        };
        format!(
            "loads: {} file(s) with a load of unknown target, {held} ({} manifest(s) read); the \
             change affects {} package(s), {} load(s) can reach one; of {} entr(ies) that only \
             such loads could select, {} selected ({} through a load, {} through their own \
             package), {:.1} ms (--strict opens them)",
            self.stats.sites,
            self.manifests,
            self.stats.affected_units,
            self.stats.fired,
            self.stats.candidates,
            self.stats.by_site + self.stats.by_entry,
            self.stats.by_site,
            self.stats.by_entry,
            self.elapsed.as_secs_f64() * 1e3
        )
    }
}

/// Adds to `report` the entries that the change selects through loads with an
/// unknown target (`Engine::bounded_loads`). `paths`: the changed paths that
/// count (all of them except repo steering files). `base`: the text of a path
/// before the change, for the paths that change the bound itself ([`lent`]).
/// Returns `None` when the graph has no such load.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply<V: GraphView>(
    engine: &Engine<'_, V>,
    root: &Path,
    changes: &ChangeSet,
    paths: &[Change],
    base: &dyn Fn(&str) -> Option<String>,
    entries: &NodeSet,
    runtime_entries: &NodeSet,
    opts: &AffectedOptions,
    report: &mut AffectedReport,
) -> Option<Applied> {
    if !opts.loads_declared || engine.declared_sites(opts).is_empty() {
        return None;
    }
    let t = Instant::now();
    let g = engine.graph();
    let (reach, manifests) = reach(g, root);
    let lent = lent(g, root, paths, base);
    let mut selected = NodeSet::new(g.node_count());
    for e in &report.entries {
        selected.insert(e.entry);
    }
    let (found, stats) = engine.bounded_loads(
        &reach,
        changes,
        paths,
        &lent,
        entries,
        Some(runtime_entries),
        &selected,
        opts,
    );
    let added = !found.is_empty();
    if added {
        report.entries.extend(found);
        report
            .entries
            .sort_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
    }
    Some(Applied {
        stats,
        manifests,
        elapsed: t.elapsed(),
        step: opts.loads_step,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_hoist_patterns() {
        assert!(name_matches("*eslint*", "@acme/eslint-config"));
        assert!(name_matches("prettier-plugin-*", "prettier-plugin-x"));
        assert!(name_matches("@types/react", "@types/react"));
        assert!(name_matches("*", "anything"));
        assert!(!name_matches("@types/react", "@types/react-dom"));
        assert!(!name_matches("prettier-plugin-*", "x-prettier-plugin-y"));
        assert!(!name_matches("*eslint*", "@acme/lint"));
    }

    #[test]
    fn settings_are_read_from_npmrc_and_yaml() {
        let npmrc = "# c\nnode-linker = \
                     hoisted\npublic-hoist-pattern[]=*types*\npublic-hoist-pattern[]=\"a\"\n;x=1\n";
        assert_eq!(npmrc_values(npmrc, "node-linker"), ["hoisted"]);
        assert_eq!(
            npmrc_values(npmrc, "public-hoist-pattern"),
            ["*types*", "a"]
        );
        assert!(npmrc_values(npmrc, "x").is_empty());
        let yaml = "packages:\n  - 'a/*'\nnodeLinker: isolated # default\npublicHoistPattern:\n  \
                    - pretty-bytes\n  # a comment\n  - '@types/react'\nsavePrefix: ''\n";
        assert_eq!(yaml_values(yaml, "nodeLinker"), ["isolated"]);
        assert_eq!(
            yaml_values(yaml, "publicHoistPattern"),
            ["pretty-bytes", "@types/react"]
        );
        assert_eq!(yaml_values(yaml, "packages"), ["a/*"]);
        assert!(yaml_values(yaml, "shamefullyHoist").is_empty());
    }

    #[test]
    fn the_layout_follows_the_package_manager() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        let hoisted = |r: &Path| matches!(layout(r), Layout::Hoisted(_));
        // No workspace file: npm, Yarn classic, bun, or a single package.
        assert!(hoisted(r));
        std::fs::write(r.join("pnpm-workspace.yaml"), "packages:\n  - 'p/*'\n").unwrap();
        let Layout::Strict { public } = layout(r) else {
            panic!("a pnpm workspace is strict by default");
        };
        assert_eq!(public, ["*eslint*", "*prettier*"]);
        std::fs::write(r.join(".npmrc"), "shamefully-hoist=true\n").unwrap();
        assert!(hoisted(r));
        std::fs::write(r.join(".npmrc"), "public-hoist-pattern[]=@x/*\n").unwrap();
        let Layout::Strict { public } = layout(r) else {
            panic!("a public hoist pattern keeps the layout strict");
        };
        assert_eq!(public, ["*eslint*", "*prettier*", "@x/*"]);
        std::fs::write(
            r.join("pnpm-workspace.yaml"),
            "packages:\n  - 'p/*'\nnodeLinker: hoisted\n",
        )
        .unwrap();
        assert!(hoisted(r));
        // Yarn: Plug'n'Play (the default with a .yarnrc.yml) and the pnpm linker are
        // strict.
        std::fs::remove_file(r.join("pnpm-workspace.yaml")).unwrap();
        std::fs::write(r.join(".yarnrc.yml"), "yarnPath: .yarn/releases/yarn.cjs\n").unwrap();
        assert!(!hoisted(r));
        std::fs::write(r.join(".yarnrc.yml"), "nodeLinker: node-modules\n").unwrap();
        assert!(hoisted(r));
    }
}
