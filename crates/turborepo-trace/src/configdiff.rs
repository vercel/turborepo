//! Field-level changes of JSON config files for `affected --since`.
//!
//! A config file read during resolution is an input of every node whose
//! resolution read it, and a `package.json` is also the target of a manifest
//! edge from every file of its package and from every importer that resolved
//! through it. As a whole-file change, a `version` bump therefore selects every
//! test of the package and of its dependents. With the old
//! contents (`--since`), the changed fields decide instead.
//!
//! # `package.json`
//!
//! [`manifest_delta`] sorts the changed top-level fields into four classes:
//!
//! | Class | Fields | Who is affected |
//! |---|---|---|
//! | inert | `version`, `description`, `keywords`, `author`, `contributors`, `maintainers`, `license`, `licenses`, `homepage`, `bugs`, `repository`, `funding`, `private`, `files` | nobody through resolution. `version` also reaches the scripts that name `$npm_package_version` and every first-party source file whose text names `npm_package_version` (a package manager exports the field to what it runs) |
//! | scripts | `scripts.<name>` | the task `task:<pkg>#<name>` (and what depends on it); a removed script: every task that read the manifest |
//! | dependencies | `dependencies`, `devDependencies`, `peerDependencies`, `optionalDependencies` | per changed name, its consumers in the package: the nodes that link a package of that name, that probed its `node_modules` location or that stand on the manifest for it, and the ones that use it without an import ([`implicit_consumers`]: the JSX runtime, a name in a source string or a test-runner config, a script, `@types`). A config that names it: the field decides ([`crate::confignames`]: a list of packages a tool treats specially when a module loads them adds no consumer; a field through which the tool loads the package is consumed by what reads the config); a config that imports it counts as changed for everything that reads it ([`config_imports`]). No consumer at all: nothing is seeded when the dependency was removed or when a lockfile diff says what the install changes ([`Report::unconsumed`]); otherwise, and when the config of a tool whose readers the graph does not know names it (Babel, SWC, PostCSS, ESLint, a file that is not a node), the consumers are not known and the file stays a whole-file change |
//! | resolution | `exports`, `imports`, `main`, `module`, `browser` (a path), `types`, `typings`, `typesVersions` | entry by entry ([`crate::exportsdiff`]): the importers whose recorded requests (`.`, a subpath, a `#` import; resolved or refused) the manifest answers differently. A difference only in what type resolution finds reaches entries that type-check, not tests whose runner transpiles file by file. Shapes outside the model (`exports` at one revision only, a `browser` object, mixed keys) keep the whole-file treatment |
//! | other | everything else (`type`, `name`, `jest`, `workspaces`, unknown fields, ...) | the file stays a whole-file change, as without a base |
//!
//! In every class, a node that reads the file as data keeps whole-file
//! semantics: an `import`/`require` of the JSON, a read by a resolved path, a
//! task's declared input. An import that a module mapper or an alias rewrites
//! to a path (jest `moduleNameMapper`, vitest `resolve.alias`), a mock or a
//! transform is not such a read: the framework edge lists the manifests its
//! resolution consulted next to the module it loads, and those are resolution
//! uses ([`classify_use`]).
//!
//! Either side not parsing as a JSON object, an added or deleted file, and a
//! manifest that is not a workspace package root (with script or dependency
//! changes) are whole-file changes.
//!
//! One reader is told apart in every class: the load of a `tsconfig.json`,
//! which gets `name`, `exports`, `imports`, `main`, `tsconfig` and
//! `typesVersions` of a manifest and nothing else ([`loader_readers`]). With
//! those four the same at both revisions, a whole-file change is none for the
//! nodes that have the manifest through a load alone; with one of them changed,
//! the nodes below every config whose load read the manifest are seeded, also
//! where the changed field is applied entry by entry for the package's
//! importers.
//!
//! Why each class is sound:
//!
//! - *inert*: the package manager and registry read these fields; resolution,
//!   test runners and build tools do not. What does read them does so through
//!   the file (kept) or through `npm_package_version` (scanned). A `version`
//!   that changes how a workspace range links is visible in the lockfile, whose
//!   importer diff seeds the linking nodes
//!   ([`crate::lockdiff::Effects::importer_deps`]).
//! - *scripts*: a script is the definition of its own task only. The graph is
//!   refreshed after the change, so a task that runs another script has an edge
//!   to that script's task.
//! - *dependencies*: resolution never reads the maps; the installed layout and
//!   the lockfile decide, and the lockfile diff marks the packages whose
//!   content or closure changed, per importer. What the manifest adds is the
//!   set of names whose link can change for this package; their consumers are
//!   the nodes that link them and the ones that use them without an import. A
//!   dependency that nothing links, probes or names cannot be observed by the
//!   package's own files: removed, it is gone with its last reference; added or
//!   changed, it can only act through other packages (a peer it satisfies),
//!   which the lockfile diff reports. The residual risk is a consumer that
//!   refers to the package under a name the search does not derive (a binary
//!   named differently from its package in a script that is not recognized).
//!
//! Membership hubs (a task whose input is the whole package directory) do not
//! see a field-level change of the manifest: the task's own script, the readers
//! of the file among the directory's files, and the linking files are seeded,
//! and they reach the hub.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;
use turborepo_trace_graph::{
    Cause, Change, ChangeKind, Engine, GraphView, NodeId, PackageId, PackageVersion,
};
use turborepo_trace_store::Graph;

use crate::{exportsdiff::Json, lockdiff::Git};

/// The graph the CLI's engine runs on: the store graph with the derived layer
/// (references resolved per test). Node ids are the store graph's; edges are
/// read from the view.
type View<'g> = turborepo_trace_graph::Patched<'g, Graph>;

/// Top-level `package.json` fields that no resolution, test runner or build
/// tool reads: registry and documentation metadata, and what goes into a
/// published tarball.
const INERT: &[&str] = &[
    "version",
    "description",
    "keywords",
    "author",
    "contributors",
    "maintainers",
    "license",
    "licenses",
    "homepage",
    "bugs",
    "repository",
    "funding",
    "private",
    "files",
];

/// Dependency maps: their effect on resolution is the set of linked packages.
const DEP_MAPS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// What changed between two versions of a `package.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestDelta {
    /// Every changed field, for display: top-level names, `scripts.<name>`,
    /// `<dependency map>.<name>`. Sorted.
    pub fields: Vec<String>,
    /// Changed fields with whole-file semantics (class "other").
    pub other: Vec<String>,
    /// Scripts added or edited.
    pub scripts_changed: BTreeSet<String>,
    /// Scripts removed.
    pub scripts_removed: BTreeSet<String>,
    /// Dependency names added, removed, moved between maps or with an edited
    /// range.
    pub deps: BTreeSet<String>,
    /// Of `deps`, the names that no dependency map declares any more.
    pub deps_removed: BTreeSet<String>,
    /// Whether `version` changed.
    pub version: bool,
    /// The resolution fields ([`crate::exportsdiff::FIELDS`]) that differ as
    /// written, key order included. They are applied entry by entry.
    pub resolution: Vec<String>,
    /// Both manifests with keys in document order, when `resolution` is not
    /// empty.
    pub docs: Option<Box<(Json, Json)>>,
}

impl ManifestDelta {
    /// Whether the change needs no whole-file treatment (no field of class
    /// "other").
    pub fn is_field_level(&self) -> bool {
        self.other.is_empty()
    }
}

/// The entries of an object-valued field as `name → value`; `None` if the field
/// is present and not an object.
fn entries<'a>(v: &'a Value, key: &str) -> Option<Vec<(&'a str, &'a Value)>> {
    match v.get(key) {
        None => Some(Vec::new()),
        Some(Value::Object(m)) => Some(m.iter().map(|(k, v)| (k.as_str(), v)).collect()),
        Some(_) => None,
    }
}

/// Names whose value differs between two entry lists: `(changed or added,
/// removed)`.
fn diff_entries<'a>(
    old: &[(&'a str, &'a Value)],
    new: &[(&'a str, &'a Value)],
) -> (BTreeSet<&'a str>, BTreeSet<&'a str>) {
    let get = |l: &[(&'a str, &'a Value)], k: &str| l.iter().find(|(n, _)| *n == k).map(|x| x.1);
    let mut changed = BTreeSet::new();
    let mut removed = BTreeSet::new();
    for (k, v) in new {
        if get(old, k) != Some(*v) {
            changed.insert(*k);
        }
    }
    for (k, _) in old {
        if get(new, k).is_none() {
            removed.insert(*k);
        }
    }
    (changed, removed)
}

/// The field-level difference of two `package.json` texts; `None` if either is
/// not a JSON object.
pub fn manifest_delta(old: &str, new: &str) -> Option<ManifestDelta> {
    let (old_doc, new_doc) = (Json::parse_object(old)?, Json::parse_object(new)?);
    let old: Value = serde_json::from_str(old).ok()?;
    let new: Value = serde_json::from_str(new).ok()?;
    let (o, n) = (old.as_object()?, new.as_object()?);
    let mut d = ManifestDelta::default();
    // Resolution fields are compared as written: the order of conditions decides.
    for f in crate::exportsdiff::fields_differ(&old_doc, &new_doc) {
        d.fields.push(f.to_owned());
        d.resolution.push(f.to_owned());
    }
    if !d.resolution.is_empty() {
        d.docs = Some(Box::new((old_doc, new_doc)));
    }
    let keys: BTreeSet<&str> = o.keys().chain(n.keys()).map(String::as_str).collect();
    for k in keys {
        if crate::exportsdiff::FIELDS.contains(&k) || o.get(k) == n.get(k) {
            continue;
        }
        if INERT.contains(&k) {
            d.version |= k == "version";
            d.fields.push(k.to_owned());
        } else if k == "scripts" {
            match (entries(&old, k), entries(&new, k)) {
                (Some(a), Some(b)) => {
                    let (changed, removed) = diff_entries(&a, &b);
                    for s in changed.iter().chain(&removed) {
                        d.fields.push(format!("scripts.{s}"));
                    }
                    d.scripts_changed
                        .extend(changed.into_iter().map(str::to_owned));
                    d.scripts_removed
                        .extend(removed.into_iter().map(str::to_owned));
                }
                _ => {
                    d.fields.push(k.to_owned());
                    d.other.push(k.to_owned());
                }
            }
        } else if DEP_MAPS.contains(&k) {
            match (entries(&old, k), entries(&new, k)) {
                (Some(a), Some(b)) => {
                    let (changed, removed) = diff_entries(&a, &b);
                    for s in changed.iter().chain(&removed) {
                        d.fields.push(format!("{k}.{s}"));
                        d.deps.insert((*s).to_owned());
                    }
                }
                _ => {
                    d.fields.push(k.to_owned());
                    d.other.push(k.to_owned());
                }
            }
        } else {
            d.fields.push(k.to_owned());
            d.other.push(k.to_owned());
        }
    }
    for name in &d.deps {
        let declared = DEP_MAPS
            .iter()
            .any(|m| n.get(*m).and_then(|v| v.get(name)).is_some());
        if !declared {
            d.deps_removed.insert(name.clone());
        }
    }
    d.fields.sort();
    Some(d)
}

/// The package a module specifier names: `@scope/name/sub` → `@scope/name`,
/// `name/sub` → `name`; `None` for relative, absolute and `#` specifiers.
pub fn package_of_spec(spec: &str) -> Option<&str> {
    if spec.is_empty() || spec.starts_with(['.', '/', '#']) || spec.contains(':') {
        return None;
    }
    let mut end = spec.len();
    let mut slashes = 0;
    for (i, b) in spec.bytes().enumerate() {
        if b == b'/' {
            slashes += 1;
            if slashes == if spec.starts_with('@') { 2 } else { 1 } {
                end = i;
                break;
            }
        }
    }
    if spec.starts_with('@') && slashes == 0 {
        return None;
    }
    Some(&spec[..end])
}

/// A recorded specifier as `(label, target)`: framework edges are labelled
/// `<framework>:<role> <target>` (`jest:moduleNameMapper`,
/// `vitest:resolve.alias`).
fn spec_target(spec: &str) -> (Option<&str>, &str) {
    match spec.split_once(' ') {
        Some((l, t))
            if l.split_once(':').is_some_and(|(f, r)| {
                !f.is_empty()
                    && !r.is_empty()
                    && l.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.')
                    })
            }) =>
        {
            (Some(l), t.trim())
        }
        _ => (None, spec),
    }
}

/// How an edge into a config file uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Use {
    /// Reads the file as data (an import of the JSON, a read by a resolved
    /// path, a task's declared input): whole-file semantics.
    Data,
    /// Resolution read it (a manifest edge).
    Resolution,
    /// A test framework read its configuration from it (`jest:config <path>`).
    Framework,
    /// The file stands for a package that did not resolve from it: the package
    /// name.
    StandIn(String),
    /// A membership hub lists it as a member of a directory.
    Hub,
}

/// The request a framework edge resolved: for a module mapper or an alias
/// (`jest:moduleNameMapper <specifier> -> <request>`) what the import was
/// rewritten to, else the recorded target.
fn fw_request(target: &str) -> &str {
    target.rsplit_once(" -> ").map_or(target, |(_, r)| r.trim())
}

/// Classifies an edge into the config file named `file_name` from its kind, its
/// specifier as recorded, and the language of its source (`task`,
/// `task-members`, or a real one).
///
/// `loads_module`: the same reference of the same source also has a successor
/// that is not a file of that name. A framework edge (a mapped import, a mock,
/// a transform) lists everything the resolution of its request returned under
/// one kind: the module it loads and the manifests consulted on the way (the
/// module type of the requesting package, the `main` of a directory). With a
/// module next to it, the config file is one of those: a resolution use, like
/// the manifest edge of a plain import. Alone, it is what the request
/// loaded, or it stands for a package that did not resolve.
pub fn classify_use(
    kind: turborepo_trace_graph::RefKind,
    spec: Option<&str>,
    src_lang: Option<&str>,
    file_name: &str,
    loads_module: bool,
) -> Use {
    use turborepo_trace_graph::RefKind as K;
    if src_lang == Some(turborepo_trace_store::tasks::MEMBERS_LANG) {
        return Use::Hub;
    }
    if kind == K::Manifest {
        return Use::Resolution;
    }
    let Some(spec) = spec.map(str::trim).filter(|s| !s.is_empty()) else {
        return Use::Data;
    };
    if src_lang == Some(turborepo_trace_store::tasks::TASK_LANG) {
        // `input <path>` / `entry <path>`: the tool reads the file. Otherwise a
        // toolchain package resolved from the package root (the manifest of an
        // unresolved one).
        if spec.starts_with("input ") || spec.starts_with("entry ") {
            return Use::Data;
        }
        return match package_of_spec(spec) {
            Some(p) => Use::StandIn(p.to_owned()),
            None => Use::Data,
        };
    }
    let (label, target) = spec_target(spec);
    if label.is_some_and(|l| l.ends_with(":config")) {
        return Use::Framework;
    }
    // What was resolved: for a mapper or alias edge, the rewritten request.
    let request = if label.is_some() {
        fw_request(target)
    } else {
        target
    };
    let names_file = request.rsplit('/').next() == Some(file_name);
    let module_load = matches!(
        kind,
        K::Import | K::Require | K::DynamicImport | K::ReExport
    );
    if module_load && !names_file {
        if label.is_some() && loads_module {
            return Use::Resolution;
        }
        if let Some(p) = package_of_spec(request) {
            return Use::StandIn(p.to_owned());
        }
    }
    Use::Data
}

/// How one changed config file was applied, for `--stats`, `--explain` and
/// JSON.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Report {
    /// The file.
    pub path: String,
    /// Its kind (`package.json`).
    pub kind: &'static str,
    /// The changed fields (empty when the sides did not parse, or only
    /// formatting changed).
    pub fields: Vec<String>,
    /// Whether only the consumers of the changed fields were seeded. Otherwise
    /// the file is a whole-file change.
    pub field_level: bool,
    /// Why.
    pub reason: String,
    /// Field-level: nodes seeded because they read the file as data.
    pub readers: usize,
    /// Field-level: tasks seeded for a changed script (or a script naming a
    /// changed field).
    pub tasks: usize,
    /// Field-level: nodes seeded for a changed dependency.
    pub links: usize,
    /// Field-level: importers seeded because the manifest answers one of their
    /// requests (`.`, a subpath, a `#` import) differently at run time.
    pub importers: usize,
    /// Field-level: importers for which only type resolution changed (seeded
    /// for entries that type-check).
    pub type_importers: usize,
    /// Changed dependencies for which no consumer was found in the package (no
    /// link, no probe, no mention in its files or scripts): nothing is
    /// selected for them. A removed dependency without a reference left has
    /// none; an added or changed one can only matter through what the
    /// lockfile diff reports.
    pub unconsumed: Vec<String>,
    /// Field-level: how a changed dependency that a tool's config names was
    /// counted ([`crate::confignames`]), and what is assumed about a config
    /// that imports one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub named: Vec<String>,
}

/// Everything derived from field-level config changes.
#[derive(Debug, Default)]
pub struct Effects {
    /// File changes that are applied through `seeds` instead.
    pub drop: BTreeSet<String>,
    /// The nodes to seed, each caused by its config file.
    pub seeds: Vec<(NodeId, Cause)>,
    /// Nodes for which only type resolution changed
    /// ([`turborepo_trace_graph::ChangeSet::type_seeds`]).
    pub type_seeds: Vec<(NodeId, Cause)>,
    /// One report per changed config file of a known kind.
    pub reports: Vec<Report>,
    /// Seeds for direct dependencies that a lockfile diff re-resolved for their
    /// importer.
    pub lock_links: usize,
    /// First-party source files that name `npm_package_version`, seeded for a
    /// `version` change.
    pub env_readers: usize,
    /// What is assumed about a config that imports a re-resolved dependency
    /// ([`config_imports`]), for `--stats` and `--explain`.
    pub notes: Vec<String>,
    /// Per changed manifest that stays a whole-file change: its readers for
    /// which the change is none
    /// ([`turborepo_trace_graph::ChangeSet::spared`], see [`loader_readers`]).
    pub spared: Vec<(String, Vec<NodeId>)>,
}

/// The nodes that have the manifest at `path` as an input because the load of a
/// `tsconfig.json` read it (`turborepo_trace_store::Graph::loader_readers`):
/// all of them, and the ones for which that is the only reading of it.
///
/// A config loader gets four fields of a manifest
/// (`turborepo_trace_core::manifest:: LOADER_FIELDS`: what the resolution of an
/// `extends` entry can read), and the nearest manifest of a config is the
/// root's for a base config at the repository's root. So:
///
/// - **The view is the same at both revisions**: the load read the same bytes.
///   A node that has the manifest this way alone is not a reader of what
///   changed, whatever the field (`packageManager`, `pnpm`, `engines`,
///   `workspaces`, a tool's own key).
/// - **The view differs**: every node below a config whose load read it may
///   resolve its imports through other `paths` now. They are seeded whatever
///   the fields say for the importers of the package.
///
/// A node that also has the manifest as an input of a derived layer (a read by
/// a path relative to a runner's working directory) is no "only" reader.
fn loader_readers(engine: &Engine<'_, View<'_>>, path: &str) -> (Vec<NodeId>, Vec<NodeId>) {
    let view = engine.graph();
    let (mut all, mut only) = (Vec::new(), Vec::new());
    for (n, alone) in view.base().loader_readers(path) {
        all.push(n);
        // (The base's inputs name a path once; a second time is the layer's.)
        if alone && view.inputs(n).filter(|p| *p == path).count() <= 1 {
            only.push(n);
        }
    }
    (all, only)
}

/// Whether a manifest change is told apart for the nodes that read the manifest
/// through the load of a tsconfig ([`loader_readers`];
/// `TURBO_TRACE_DEBUG_LOADER_VIEW=0` turns it off, for A/B measurements: every
/// reader of a whole-file change is selected then, and a field-level change of
/// `exports`, `imports` or `main` goes by the importers alone).
fn loader_view_rule() -> bool {
    turborepo_trace_core::debug::var_os("LOADER_VIEW").is_none_or(|v| v != "0")
}

/// Whether the field-level treatment is on (`TURBO_TRACE_DEBUG_CONFIG_FIELDS=0`
/// turns it off, for A/B measurements and as an escape hatch).
pub fn enabled() -> bool {
    turborepo_trace_core::debug::var_os("CONFIG_FIELDS").is_none_or(|v| v != "0")
}

/// Whether dependency-map changes are applied per dependency
/// (`TURBO_TRACE_DEBUG_CONFIG_FIELDS=nodeps` keeps them whole-file changes).
fn dependency_fields() -> bool {
    turborepo_trace_core::debug::var_os("CONFIG_FIELDS").is_none_or(|v| v != "nodeps")
}

/// Whether a changed package that a config module imports also counts for the
/// nodes that read that config's values without loading it as a module. On by
/// default; `TURBO_TRACE_DEBUG_CONFIG_IMPORTS=0` assumes that it does not (for
/// A/B measurements; `--strict` never assumes it).
///
/// The graph has two kinds of readers of a config module. The files and tasks
/// that load it have an edge to it. The jest tests of a `next/jest` project
/// have it as a content input: `next/jest` loads `next.config.*` in the jest
/// process and hands its `transpilePackages`, `modularizeImports`, `compiler`
/// and `experimental.swcPlugins` to the transform of every
/// test file (`turborepo_trace_frameworks::inputs::GlobalInputs::reads`). A
/// package the config imports runs when the config loads: a wrapper
/// (`withPlugin(nextConfig)`) returns the object those fields are read from and
/// can set `process.env` for the tests. So when such a package changes, the
/// config counts as changed for both kinds of readers, as it does for an edit
/// of the config file.
///
/// What stays as it was: an edit of a *file of the repository* that the config
/// imports reaches the config through its edge and is not an input change for
/// those tests (`crates/turborepo-trace-frameworks/src/jest.rs`: the config's
/// own imports reach through the application).
pub fn config_imports(strict: bool) -> bool {
    strict || turborepo_trace_core::debug::var_os("CONFIG_IMPORTS").is_none_or(|v| v != "0")
}

/// The nodes that read the values of the config module `n` without loading it
/// (see [`config_imports`]): none unless `n` is a tool config.
fn value_readers(engine: &Engine<'_, View<'_>>, n: NodeId) -> Vec<NodeId> {
    let path = engine.graph().base().path(n);
    let mut out = Vec::new();
    if is_tool_config(file_name(path)) {
        engine.match_inputs(path, &mut |m| out.push(m));
        out.sort_unstable();
        out.dedup();
    }
    out
}

/// Applies [`config_imports`] to the linkers `found` of the changed dependency
/// `name`: returns the value readers to seed (when they count) and adds the
/// sentence that says what was counted or assumed.
fn config_linkers(
    engine: &Engine<'_, View<'_>>,
    found: &[NodeId],
    name: &str,
    count: bool,
    notes: &mut Vec<String>,
) -> Vec<NodeId> {
    let g = engine.graph().base();
    let mut seeds = Vec::new();
    for &n in found {
        let readers = value_readers(engine, n);
        if readers.is_empty() {
            continue;
        }
        let path = g.path(n);
        let note = if count {
            format!(
                "{path} imports {name}, which runs when the config loads: counted for the {} \
                 node(s) that read the config's values without loading it (the tests of a \
                 next/jest project) as for an edit of the config",
                readers.len()
            )
        } else {
            format!(
                "assumed: {name}, which {path} imports, changes what loads the config, not the \
                 values that {} node(s) read from it without loading it",
                readers.len()
            )
        };
        if !notes.contains(&note) {
            notes.push(note);
        }
        if count {
            seeds.extend(readers);
        }
    }
    seeds
}

/// The value readers of the configs that link a package of `packages` from one
/// of its importers ([`config_imports`]): a lockfile diff that changes the
/// package or its closure seeds the linking files as modified
/// (`ChangeSet::scoped`), and a config among them counts for the nodes that
/// read its values too. Returns the seeds; the sentences go to
/// [`Effects::notes`].
pub fn package_links(
    engine: &Engine<'_, View<'_>>,
    packages: &[(PackageId, &[PackageId], &str)],
    strict: bool,
    fx: &mut Effects,
) {
    if packages.is_empty() || !enabled() {
        return;
    }
    let g = engine.graph().base();
    let rev = engine.reverse();
    let count = config_imports(strict);
    // (config, package, the file that says the package changed)
    let mut linkers: Vec<(NodeId, PackageId, &str)> = Vec::new();
    for i in 0..g.node_count() {
        let n = NodeId(i);
        let p = g.package(n);
        let Some((_, importers, source)) = packages.iter().find(|(q, _, _)| *q == p) else {
            continue;
        };
        for &s in rev.neighbors(n).0 {
            let s = NodeId(s);
            if importers.contains(&g.package(s))
                && is_tool_config(file_name(g.path(s)))
                && !linkers.iter().any(|(c, q, _)| *c == s && *q == p)
            {
                linkers.push((s, p, source));
            }
        }
    }
    for (n, p, source) in linkers {
        let mut notes = std::mem::take(&mut fx.notes);
        let readers = config_linkers(engine, &[n], g.package_info(p).name, count, &mut notes);
        fx.notes = notes;
        for r in readers {
            fx.lock_links += 1;
            fx.seeds.push((
                r,
                Cause::Modified {
                    path: source.to_owned(),
                },
            ));
        }
    }
}

/// First-party packages by root.
fn package_roots(g: &Graph) -> HashMap<&str, PackageId> {
    let mut roots = HashMap::new();
    for i in 0..g.package_count() {
        let info = g.package_info(PackageId(i));
        if matches!(info.version, PackageVersion::Workspace) {
            roots.insert(info.root, PackageId(i));
        }
    }
    roots
}

/// For some first-party packages: which of their nodes link which other
/// packages.
#[derive(Default)]
struct Links {
    /// Package → `(node, linked package)`.
    by_package: HashMap<u32, Vec<(NodeId, PackageId)>>,
}

impl Links {
    fn of(engine: &Engine<'_, View<'_>>, packages: &BTreeSet<PackageId>) -> Links {
        let mut out = Links::default();
        if packages.is_empty() {
            return out;
        }
        let g = engine.graph();
        let fwd = engine.forward();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            let p = g.package(n);
            if !packages.contains(&p) {
                continue;
            }
            let list = out.by_package.entry(p.0).or_default();
            for &t in fwd.neighbors(n).0 {
                let tp = g.package(NodeId(t));
                if tp != p && list.last() != Some(&(n, tp)) {
                    list.push((n, tp));
                }
            }
        }
        out
    }

    /// The nodes of `package` (rooted at `dir`) that depend on how the
    /// dependency `name` resolves: those with an edge into a package of
    /// that name (any version, first- or third-party), and those that
    /// probed its location in the package's `node_modules` (they failed to
    /// resolve it, or resolved it elsewhere).
    fn consumers(&self, g: &Graph, package: PackageId, dir: &str, name: &str) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self
            .by_package
            .get(&package.0)
            .into_iter()
            .flatten()
            .filter(|(_, tp)| g.package_info(*tp).name == name)
            .map(|(n, _)| *n)
            .collect();
        let probe = if dir.is_empty() {
            format!("node_modules/{name}")
        } else {
            format!("{dir}/node_modules/{name}")
        };
        g.probe_lookup(&probe, &mut |n, _| out.push(n));
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Seeds the consumers of the direct dependencies that a lockfile diff
/// re-resolved ([`crate::lockdiff::Effects::importer_deps`]).
fn lock_links(
    engine: &Engine<'_, View<'_>>,
    importer_deps: &[crate::lockdiff::ImporterDep],
    strict: bool,
    fx: &mut Effects,
) {
    if importer_deps.is_empty() {
        return;
    }
    let g = engine.graph().base();
    let roots = package_roots(g);
    let packages: BTreeSet<PackageId> = importer_deps
        .iter()
        .filter_map(|d| roots.get(d.importer.as_str()).copied())
        .collect();
    let links = Links::of(engine, &packages);
    for d in importer_deps {
        let Some(&p) = roots.get(d.importer.as_str()) else {
            continue;
        };
        let found = links.consumers(g, p, &d.importer, &d.name);
        let mut notes = std::mem::take(&mut fx.notes);
        let readers = config_linkers(engine, &found, &d.name, config_imports(strict), &mut notes);
        fx.notes = notes;
        for n in found.into_iter().chain(readers) {
            fx.lock_links += 1;
            fx.seeds.push((
                n,
                Cause::Modified {
                    path: d.source.clone(),
                },
            ));
        }
    }
}

/// The variable a package manager sets to the `version` of the package whose
/// script runs.
const VERSION_ENV: &[u8] = b"npm_package_version";

/// Larger files are not scanned for [`VERSION_ENV`] (bundles, data).
const ENV_SCAN_MAX_BYTES: u64 = 2 << 20;

/// First-party source files whose text names `npm_package_version`.
fn env_version_readers(g: &Graph) -> Vec<NodeId> {
    let root = std::path::Path::new(g.root());
    let files: Vec<NodeId> = (0..g.node_count())
        .map(NodeId)
        .filter(|&n| {
            !g.is_external(n)
                && g.lang(n).is_some()
                && !turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n))
        })
        .collect();
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(16);
    let chunk = files.len().div_ceil(threads).max(1);
    let names = |n: NodeId| -> bool {
        let p = root.join(g.path(n));
        if std::fs::metadata(&p).is_ok_and(|m| m.len() > ENV_SCAN_MAX_BYTES) {
            return false;
        }
        std::fs::read(&p).is_ok_and(|b| {
            b.windows(VERSION_ENV.len())
                .any(|w| w[0] == VERSION_ENV[0] && w == VERSION_ENV)
        })
    };
    let mut out: Vec<NodeId> = std::thread::scope(|s| {
        let hs: Vec<_> = files
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().copied().filter(|&n| names(n)).collect::<Vec<_>>()))
            .collect();
        hs.into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    });
    out.sort_unstable();
    out
}

/// A file's node (`None` for a well-known config file that is not in the
/// graph), its root-relative path and its bytes.
type Text = (Option<NodeId>, String, Vec<u8>);

/// The text of a package's files, for dependencies that are used without an
/// import. Config files are read at once (a handful); the other files only when
/// a dependency has no linker ([`PackageTexts::sources`]): reading every file
/// of a large package costs a few hundred milliseconds.
struct PackageTexts {
    /// Tool configs of the package, and the well-known ones of its ancestor
    /// directories.
    configs: Vec<Text>,
    /// The other files of the package: paths, and their texts once read.
    other: Vec<(NodeId, String)>,
    sources: Option<Vec<Text>>,
    /// Task nodes of the package with their command lines.
    tasks: Vec<(NodeId, String)>,
    /// JSX and TSX files of the package.
    jsx: Vec<NodeId>,
    /// Source files of the package.
    code: Vec<NodeId>,
}

/// Files larger than this are not read for mentions (bundles, data).
const MENTION_MAX_BYTES: u64 = 1 << 20;

/// Config files that tools read by name and that are not source files of any
/// language: looked for in the package directory and its ancestors.
const DOT_CONFIGS: &[&str] = &[
    ".babelrc",
    ".babelrc.json",
    "babel.config.json",
    ".eslintrc",
    ".eslintrc.json",
    ".eslintrc.yml",
    ".eslintrc.yaml",
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yml",
    ".prettierrc.yaml",
    ".swcrc",
    ".postcssrc",
    ".postcssrc.json",
    ".stylelintrc",
    ".stylelintrc.json",
    ".mocharc.json",
    ".mocharc.yml",
    ".lintstagedrc",
    ".lintstagedrc.json",
    "tsconfig.json",
    "jsconfig.json",
];

impl PackageTexts {
    fn read(g: &Graph, package: PackageId, dir: &str) -> PackageTexts {
        let root = std::path::Path::new(g.root());
        let mut out = PackageTexts {
            configs: Vec::new(),
            other: Vec::new(),
            sources: None,
            tasks: Vec::new(),
            jsx: Vec::new(),
            code: Vec::new(),
        };
        let read = |rel: &str| read_text(root, rel);
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            if g.package(n) != package || g.is_external(n) {
                continue;
            }
            if let Some(t) = g.task(n) {
                let lines: Vec<&str> = t.lines.iter().map(|l| l.as_str()).collect();
                out.tasks.push((n, lines.join("\n")));
                continue;
            }
            if turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n)) {
                continue;
            }
            let path = g.path(n);
            if file_name(path) == "package.json" {
                continue;
            }
            if g.lang(n).is_some() {
                out.code.push(n);
                if path.ends_with(".tsx") || path.ends_with(".jsx") {
                    out.jsx.push(n);
                }
            }
            if !is_tool_config(file_name(path)) {
                out.other.push((n, path.to_owned()));
            } else if let Some(bytes) = read(path) {
                seen.insert(path.to_owned());
                out.configs.push((Some(n), path.to_owned(), bytes));
            }
        }
        // Config files of the package directory and its ancestors (a root `.babelrc`
        // applies to the package).
        let mut at = Some(dir);
        while let Some(d) = at {
            for name in DOT_CONFIGS {
                let rel = if d.is_empty() {
                    (*name).to_owned()
                } else {
                    format!("{d}/{name}")
                };
                if seen.insert(rel.clone())
                    && let Some(bytes) = read(&rel)
                {
                    out.configs.push((g.node_by_path(&rel), rel, bytes));
                }
            }
            at = match d {
                "" => None,
                d => Some(dir_of(d)),
            };
        }
        out
    }

    /// The texts of the package's files that are not tool configs, read on
    /// first use.
    fn sources(&mut self, g: &Graph) -> &[Text] {
        let root = std::path::Path::new(g.root());
        let other = &self.other;
        self.sources.get_or_insert_with(|| {
            other
                .iter()
                .filter_map(|(n, path)| Some((Some(*n), path.clone(), read_text(root, path)?)))
                .collect()
        })
    }
}

/// The bytes of a root-relative file, unless it is missing or too large to be
/// source text.
fn read_text(root: &std::path::Path, rel: &str) -> Option<Vec<u8>> {
    let p = root.join(rel);
    let meta = std::fs::metadata(&p).ok()?;
    if !meta.is_file() || meta.len() > MENTION_MAX_BYTES {
        return None;
    }
    std::fs::read(&p).ok()
}

/// The first byte offset at or after `from` at which `text` names `needle` as a
/// whole package name: not as part of a longer name (`react` in `react-dom`,
/// `x` in `@scope/x`), a subpath after it is fine.
pub(crate) fn mention_at(text: &[u8], needle: &str, from: usize) -> Option<usize> {
    let n = needle.as_bytes();
    if n.is_empty() || text.len() < n.len() {
        return None;
    }
    let word = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-');
    let mut i = from;
    while i < text.len() {
        let off = text[i..].windows(n.len()).position(|w| w == n)?;
        let at = i + off;
        let before = at.checked_sub(1).map(|j| text[j]);
        let after = text.get(at + n.len()).copied();
        let installed = at >= 13 && &text[at - 13..at] == b"node_modules/";
        let start_ok =
            before.is_none_or(|b| !(word(b) || matches!(b, b'@' | b'/' | b'.'))) || installed;
        let end_ok = after.is_none_or(|b| !word(b));
        if start_ok && end_ok {
            return Some(at);
        }
        i = at + 1;
    }
    None
}

/// Whether `text` names `needle` as a whole package name ([`mention_at`]).
fn mentions(text: &[u8], needle: &str) -> bool {
    mention_at(text, needle, 0).is_some()
}

/// Other spellings under which tools accept the package `name`, each with the
/// word a file's name must contain for the spelling to count there: eslint and
/// babel load plugins and presets by short names (`eslint-plugin-react` as
/// `react`, `@babel/preset-env` as `@babel/env`), jest an environment by its
/// suffix.
fn short_names(name: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let (scope, base) = match name.split_once('/') {
        Some((s, b)) if s.starts_with('@') => (Some(s), b),
        _ => (None, name),
    };
    for (prefix, tool) in [
        ("eslint-plugin", "eslint"),
        ("eslint-config", "eslint"),
        ("babel-plugin", "babel"),
        ("babel-preset", "babel"),
        ("plugin", "babel"),
        ("preset", "babel"),
        ("jest-environment", "jest"),
    ] {
        let scoped_only = matches!(prefix, "plugin" | "preset");
        if scoped_only && scope != Some("@babel") {
            continue;
        }
        let rest = if base == prefix {
            Some("")
        } else {
            base.strip_prefix(prefix).and_then(|r| r.strip_prefix('-'))
        };
        let Some(rest) = rest else { continue };
        match (scope, rest) {
            (Some(s), "") => out.push((s.to_owned(), tool)),
            (Some(s), r) => out.push((format!("{s}/{r}"), tool)),
            (None, "") => {}
            (None, r) => out.push((r.to_owned(), tool)),
        }
    }
    out
}

/// Consumers of a dependency that no import shows.
#[derive(Default)]
struct Implicit {
    /// Nodes to seed.
    nodes: Vec<NodeId>,
    /// Nodes for which only type checking can observe the dependency.
    types: Vec<NodeId>,
    /// A file whose readers the graph does not know names the dependency: a
    /// file that is not a node, or the config of a tool (babel, swc,
    /// postcss, eslint, ...) that applies to files which have no edge to
    /// it.
    outside_graph: bool,
    /// Configs of tools whose readers the graph knows that name the dependency
    /// where the tool itself loads it, or in a role that is not decided
    /// here: their readers are the consumers, as for an edit of the
    /// config ([`crate::confignames::Named::Readers`]). The node is in
    /// `nodes`; the caller adds the nodes that have the file as an input.
    as_changed: Vec<NodeId>,
    /// How the configs that name the dependency were counted, for display.
    named: Vec<String>,
}

/// Who consumes a package that a config names: the answer of
/// [`crate::confignames`] for the config of a tool whose readers the graph
/// knows, `None` for every other file (and
/// with `TURBO_TRACE_DEBUG_CONFIG_NAMES=0`).
fn named_by_field(file: &str, src: &[u8], needles: &[&str], links: bool) -> Option<Named> {
    let tool = crate::confignames::Tool::of(file).filter(|_| crate::confignames::enabled())?;
    Some(crate::confignames::named(tool, file, src, needles, links))
}

/// Who consumes a package that a config of a known tool names (see
/// [`crate::confignames`], which decides it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Named {
    /// Only the nodes that load the package. The fields that name it, for
    /// display (empty: only comments, or imports of a config that is an
    /// importer, name it).
    Importers(Vec<String>),
    /// Everything an edit of the config would select. Where it is named, for
    /// display.
    Readers(Vec<String>),
}

/// Whether `file` (a base name) is a tool's configuration: a dotfile, an `rc`
/// file, `<tool>.config.<ext>`, a tsconfig.
fn is_tool_config(file: &str) -> bool {
    file.starts_with('.')
        || file.contains(".config.")
        || file.starts_with("tsconfig.")
        || file.starts_with("jsconfig.")
        || file
            .rsplit_once('.')
            .map_or(file, |(stem, _)| stem)
            .ends_with("rc")
}

/// Whether `file` is the config of a test runner: tests have an edge to it.
fn is_runner_config(file: &str) -> bool {
    [
        "jest.config.",
        "vitest.config.",
        "vite.config.",
        "vitest.workspace.",
    ]
    .iter()
    .any(|p| file.starts_with(p))
}

/// The consumers of the dependency `name` in a package that reference it
/// without an import edge:
///
/// - **named in text**: for a dependency that no node links (`linked` is
///   false), any source file of the package that names the package as a word (a
///   module string, a path under `node_modules/<name>`, a `@jsxImportSource`
///   pragma): the file is seeded. A script whose command line names it (a
///   binary run by name, also without its scope): the task is seeded. The
///   config of a tool whose readers the graph knows (a test runner's config,
///   `next.config.*`): the field that names the package decides
///   ([`crate::confignames`]): either nothing beyond the importers, or
///   everything that reads the config ([`Implicit::as_changed`]). Named by
///   another tool's config (a babel preset, a postcss plugin, also under the
///   short name the tool accepts, [`short_names`]) or by a file that is not a
///   node: the tool applies to files that have no edge to its config, so the
///   consumers are not known ([`Implicit::outside_graph`]);
/// - **the JSX runtime**: `react` (the default import source of the automatic
///   runtime), or a package a config of the package names as `jsxImportSource`:
///   every JSX/TSX file of the package;
/// - **`@types/<n>`**: type checking of the files that link `<n>` (every source
///   file when nothing links it: global types such as `@types/node`). Tests
///   whose runner transpiles file by file do not observe it.
fn implicit_consumers(
    g: &Graph,
    links: &Links,
    texts: &mut PackageTexts,
    package: PackageId,
    dir: &str,
    name: &str,
    linkers: &[NodeId],
) -> Implicit {
    let linked = !linkers.is_empty();
    let mut out = Implicit::default();
    if let Some(runtime) = name.strip_prefix("@types/") {
        // `@types/babel__core` types `@babel/core`.
        let runtime = match runtime.split_once("__") {
            Some((scope, base)) => format!("@{scope}/{base}"),
            None => runtime.to_owned(),
        };
        out.types = links.consumers(g, package, dir, &runtime);
        if out.types.is_empty() {
            out.types = texts.code.clone();
        }
        return out;
    }
    let short = short_names(name);
    let mut jsx_source = name == "react";
    // Configs always; the package's other files only for a dependency without a
    // linker (a linked one is seeded through its linkers, as for any lockfile
    // change).
    if !linked {
        texts.sources(g);
    }
    let sources = texts.sources.as_deref().filter(|_| !linked).unwrap_or(&[]);
    for (node, path, bytes) in texts.configs.iter().chain(sources) {
        let file = file_name(path);
        let named = mentions(bytes, name)
            || short
                .iter()
                .any(|(s, tool)| file.contains(tool) && mentions(bytes, s));
        if !named {
            continue;
        }
        if mentions(bytes, "jsxImportSource") {
            // The JSX files are the consumers (below).
            jsx_source = true;
            continue;
        }
        // The config of a tool whose readers the graph knows: the field that names the
        // package decides (crate::confignames).
        let links = (*node).is_some_and(|n| linkers.contains(&n));
        let needles: Vec<&str> = std::iter::once(name)
            .chain(
                short
                    .iter()
                    .filter(|(_, tool)| file.contains(tool))
                    .map(|(s, _)| s.as_str()),
            )
            .collect();
        match (node, named_by_field(file, bytes, &needles, links)) {
            (Some(_), Some(Named::Importers(fields))) => {
                if !fields.is_empty() {
                    out.named.push(format!(
                        "{path} names {name} in {}: counted for the files that load it",
                        fields.join(", ")
                    ));
                }
            }
            (Some(n), Some(Named::Readers(fields))) => {
                out.nodes.push(*n);
                out.as_changed.push(*n);
                out.named.push(format!(
                    "{path} names {name} ({}): counted for everything that reads the config",
                    fields.join(", ")
                ));
            }
            // A config that imports the package is one of its linkers: seeded as such.
            (Some(n), _) if linkers.contains(n) => {}
            (Some(n), _) if !is_tool_config(file) || is_runner_config(file) => out.nodes.push(*n),
            _ => out.outside_graph = true,
        }
    }
    let unscoped = name.rsplit('/').next().unwrap_or(name);
    for (task, lines) in &texts.tasks {
        if mentions(lines.as_bytes(), name) || mentions(lines.as_bytes(), unscoped) {
            out.nodes.push(*task);
        }
    }
    if jsx_source {
        out.nodes.extend(texts.jsx.iter().copied());
    }
    out.nodes.sort_unstable();
    out.nodes.dedup();
    out
}

/// The importers a change of resolution fields affects.
struct ResolutionSeeds {
    /// What they load at run time changes.
    runtime: Vec<NodeId>,
    /// Only what type resolution finds changes.
    types: Vec<NodeId>,
}

/// `spec` (relative, from the file `from`) as a root-relative path without `.`
/// and `..`.
fn join_relative(from: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir_of(from).split('/').filter(|p| !p.is_empty()).collect();
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// The importers whose requests the manifest at `path` answers differently in
/// `new` than in `old` ([`crate::exportsdiff`]).
///
/// Candidates are the nodes that resolved through the manifest (a manifest edge
/// or a config input); a request the manifest refused is a config input too
/// (the resolver records the manifest of an installed package that did not
/// answer). Each candidate's requests are read from its recorded specifiers,
/// resolved or not: `<name>` and `<name>/<subpath>` for the package's name,
/// `#...` for the package's own files, and a relative specifier that names the
/// manifest's directory (the root request). For a module mapper or alias edge
/// the request is what the import was rewritten to. A candidate outside the
/// package without any such specifier reached the manifest some other way: it
/// counts as a root request.
fn resolution_seeds(
    engine: &Engine<'_, View<'_>>,
    path: &str,
    old: &Json,
    new: &Json,
) -> Result<ResolutionSeeds, crate::exportsdiff::Shape> {
    use turborepo_trace_graph::RefFlags;
    let view = engine.graph();
    let g = view.base();
    let dir = dir_of(path);
    let name = new.get("name").and_then(Json::as_str);
    let mut candidates: Vec<NodeId> = Vec::new();
    if let Some(m) = g.node_by_path(path) {
        candidates.extend(engine.reverse().neighbors(m).0.iter().map(|&s| NodeId(s)));
    }
    engine.match_inputs(path, &mut |n| candidates.push(n));
    candidates.sort_unstable();
    candidates.dedup();
    let mut memo: HashMap<String, crate::exportsdiff::Outcome> = HashMap::new();
    // Whether a field that can answer a request for a file of the package changed.
    let remaps_files = ["exports", "browser", "typesVersions"]
        .iter()
        .any(|f| old.get(f) != new.get(f));
    let mut out = ResolutionSeeds {
        runtime: Vec::new(),
        types: Vec::new(),
    };
    for n in candidates {
        if turborepo_trace_store::tasks::is_pseudo_lang(g.lang(n)) {
            continue;
        }
        let from = g.path(n);
        let inside = dir.is_empty()
            || from
                .strip_prefix(dir)
                .is_some_and(|rest| rest.starts_with('/'));
        // (request, whether only type-only references make it)
        let mut requests: Vec<(String, bool)> = Vec::new();
        for e in view.out_edges(n) {
            let Some(spec) = e.specifier else { continue };
            let (label, target) = spec_target(spec);
            // A mapper or alias edge asks for what the import was rewritten to.
            let mapped = label.is_some().then(|| fw_request(target));
            let target = mapped.unwrap_or(target);
            let request = if target.starts_with('#') {
                if !inside {
                    continue;
                }
                target.to_owned()
            } else if let Some(p) = package_of_spec(target).filter(|p| Some(*p) == name) {
                match target[p.len()..].trim_end_matches('/') {
                    "" => ".".to_owned(),
                    sub => format!(".{sub}"),
                }
            } else if target.starts_with('.')
                && join_relative(from, target).is_some_and(|p| p == dir)
            {
                ".".to_owned()
            } else if let Some(r) = mapped.filter(|r| !r.starts_with(['.', '/'])) {
                // A mapped request is usually a path, recorded relative to the repository
                // root. The package directory is the root request. A file below it is
                // answered by the manifest only where a `browser` object remaps files; it
                // is checked like a subpath (also against `exports`, which errs on the
                // side of a change).
                let r = r.trim_end_matches('/');
                let below = if dir.is_empty() {
                    Some(r)
                } else {
                    r.strip_prefix(dir).and_then(|x| x.strip_prefix('/'))
                };
                if r == dir {
                    ".".to_owned()
                } else if let Some(x) = below.filter(|x| remaps_files && !x.is_empty()) {
                    format!("./{x}")
                } else {
                    continue;
                }
            } else {
                continue;
            };
            let type_only = e.flags().contains(RefFlags::TYPE_ONLY);
            match requests.iter_mut().find(|(r, _)| *r == request) {
                Some((_, t)) => *t &= type_only,
                None => requests.push((request, type_only)),
            }
        }
        if requests.is_empty() && !inside {
            requests.push((".".to_owned(), false));
        }
        let (mut runtime, mut types) = (false, false);
        for (request, type_only) in requests {
            let o = match memo.get(&request) {
                Some(o) => *o,
                None => {
                    let o = crate::exportsdiff::request_changed(old, new, &request)?;
                    memo.insert(request, o);
                    o
                }
            };
            if o.runtime && !type_only {
                runtime = true;
            } else if o.any() {
                types = true;
            }
        }
        if runtime {
            out.runtime.push(n);
        } else if types {
            out.types.push(n);
        }
    }
    Ok(out)
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The nodes that read the config node `m` as data, and the `(node, package)`
/// pairs for which it stands in for a package that did not resolve.
fn uses_of(
    engine: &Engine<'_, View<'_>>,
    m: NodeId,
    name: &str,
) -> (Vec<NodeId>, Vec<(NodeId, String)>) {
    let g = engine.graph();
    let (srcs, _) = engine.reverse().neighbors(m);
    let mut readers = Vec::new();
    let mut stand_ins = Vec::new();
    let mut prev = None;
    // Per source: the labelled specifiers of its edges into `m`, and whether the
    // same reference also has a successor that is not a file named `name` (the
    // module it loads).
    let mut labelled: Vec<(&str, bool)> = Vec::new();
    for &s in srcs {
        if prev == Some(s) {
            continue;
        }
        prev = Some(s);
        let src = NodeId(s);
        let lang = g.lang(src);
        labelled.clear();
        for e in g.out_edges(src).filter(|e| e.to == Some(m)) {
            if let Some(spec) = e.specifier
                && spec_target(spec).0.is_some()
                && !labelled.iter().any(|(l, _)| *l == spec)
            {
                labelled.push((spec, false));
            }
        }
        if !labelled.is_empty() {
            for e in g.out_edges(src) {
                let (Some(spec), Some(to)) = (e.specifier, e.to) else {
                    continue;
                };
                if to != m
                    && let Some(l) = labelled.iter_mut().find(|(l, loads)| !*loads && *l == spec)
                    && file_name(g.path(to)) != name
                {
                    l.1 = true;
                }
            }
        }
        let mut data = false;
        for e in g.out_edges(src).filter(|e| e.to == Some(m)) {
            let loads_module = e
                .specifier
                .is_some_and(|spec| labelled.iter().any(|(l, loads)| *loads && *l == spec));
            match classify_use(e.kind(), e.specifier, lang, name, loads_module) {
                Use::Data => data = true,
                Use::StandIn(p) => stand_ins.push((src, p)),
                Use::Resolution | Use::Framework | Use::Hub => {}
            }
        }
        if data {
            readers.push(src);
        }
    }
    (readers, stand_ins)
}

/// Applies the changed `package.json` files of `changes` field by field (see
/// the module docs). `skip` names changes that are handled elsewhere (steering
/// files).
pub fn effects(
    engine: &Engine<'_, View<'_>>,
    changes: &[Change],
    base: Option<&Git>,
    importer_deps: &[crate::lockdiff::ImporterDep],
    lockfile_diffed: bool,
    strict: bool,
    skip: &dyn Fn(&str) -> bool,
) -> Effects {
    let mut fx = Effects::default();
    let Some(base) = base.filter(|_| enabled()) else {
        return fx;
    };
    let g = engine.graph().base();
    lock_links(engine, importer_deps, strict, &mut fx);
    let root = std::path::Path::new(g.root());
    let todo: Vec<&Change> = changes
        .iter()
        .filter(|c| {
            c.kind == ChangeKind::Modified && file_name(&c.path) == "package.json" && !skip(&c.path)
        })
        .collect();
    if todo.is_empty() {
        return fx;
    }
    // Old and new sides, read in parallel (one `git show` each). With each
    // difference: whether what a config loader gets of the manifest is the same
    // at both.
    let deltas: Vec<Option<(ManifestDelta, bool)>> = std::thread::scope(|s| {
        let hs: Vec<_> = todo
            .iter()
            .map(|c| {
                s.spawn(move || {
                    let old = base.show(&c.path)?;
                    let new = std::fs::read_to_string(root.join(&c.path)).ok()?;
                    let view = turborepo_trace_core::manifest::loader_view;
                    let same_for_loads = matches!(
                        (view(old.as_bytes()), view(new.as_bytes())),
                        (Some(a), Some(b)) if a == b
                    );
                    Some((manifest_delta(&old, &new)?, same_for_loads))
                })
            })
            .collect();
        hs.into_iter()
            .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    });
    let roots = package_roots(g);
    // The links of the packages whose dependency maps changed.
    let dep_pkgs: BTreeSet<PackageId> = todo
        .iter()
        .zip(&deltas)
        .filter(|(_, d)| {
            d.as_ref()
                .is_some_and(|(d, _)| d.is_field_level() && !d.deps.is_empty())
        })
        .filter_map(|(c, _)| roots.get(dir_of(&c.path)).copied())
        .collect();
    let links = Links::of(engine, &dep_pkgs);
    let mut tasks: Option<Vec<(NodeId, &turborepo_trace_store::shard::ArchivedTaskRec)>> = None;
    // The first manifest whose `version` change is applied field by field.
    let mut version_bumps: Option<String> = None;
    for (c, delta) in todo.into_iter().zip(deltas) {
        let mut report = Report {
            path: c.path.clone(),
            kind: "package.json",
            fields: Vec::new(),
            field_level: false,
            reason: String::new(),
            readers: 0,
            tasks: 0,
            links: 0,
            importers: 0,
            type_importers: 0,
            unconsumed: Vec::new(),
            named: Vec::new(),
        };
        let Some((d, same_for_loads)) = delta else {
            report.reason = "one side is not a JSON object".to_owned();
            fx.reports.push(report);
            continue;
        };
        report.fields = d.fields.clone();
        // What the loads of tsconfig files got of the manifest ([`loader_readers`]).
        let (loads, only_loads) = if loader_view_rule() {
            loader_readers(engine, &c.path)
        } else {
            (Vec::new(), Vec::new())
        };
        // The same at both revisions: wherever the file stays a whole-file change
        // below, it is none for the nodes that have it through a load alone.
        let spared = if same_for_loads && !only_loads.is_empty() {
            fx.spared.push((c.path.clone(), only_loads.clone()));
            format!(
                "; not for {} file(s) that read it only through the load of a tsconfig, which \
                 gets its name, exports, imports and main: unchanged",
                only_loads.len()
            )
        } else {
            String::new()
        };
        if !d.is_field_level() {
            report.reason = format!(
                "{} changed: read by resolution or by tools turbo trace does not model{spared}",
                d.other.join(", ")
            );
            fx.reports.push(report);
            continue;
        }
        let dir = dir_of(&c.path);
        let is_root = roots.contains_key(dir);
        if !is_root
            && !(d.scripts_changed.is_empty() && d.scripts_removed.is_empty() && d.deps.is_empty())
        {
            report.reason =
                "scripts or dependencies of a manifest that is not a package root".to_owned();
            fx.reports.push(report);
            continue;
        }
        let cause = || Cause::Modified {
            path: c.path.clone(),
        };
        let mut seeds: Vec<NodeId> = Vec::new();
        // The loads of tsconfig files that read another view of the manifest than at
        // the base: the files below those configs, whatever follows for its
        // importers.
        if !same_for_loads {
            seeds.extend(loads.iter().copied());
        }
        // Readers of the file as data.
        let mut stand_ins = Vec::new();
        if let Some(m) = g.node_by_path(&c.path) {
            let (readers, s) = uses_of(engine, m, "package.json");
            report.readers = readers.len();
            seeds.extend(readers);
            stand_ins = s;
        }
        // Resolution fields, entry by entry: the importers whose requests the manifest
        // answers differently.
        let mut type_seeds: Vec<NodeId> = Vec::new();
        if let Some(docs) = &d.docs {
            match resolution_seeds(engine, &c.path, &docs.0, &docs.1) {
                Ok(r) => {
                    report.importers = r.runtime.len();
                    report.type_importers = r.types.len();
                    seeds.extend(r.runtime);
                    type_seeds = r.types;
                }
                Err(shape) => {
                    report.reason = format!("{} changed: {}", d.resolution.join(", "), shape.0);
                    fx.reports.push(report);
                    continue;
                }
            }
        }
        // Dependencies: per changed name, its consumers in the package: the nodes that
        // link it, and the ones that use it without an import (see
        // `implicit_consumers`).
        if !d.deps.is_empty() {
            if !dependency_fields() {
                report.reason = "dependencies changed".to_owned();
                fx.reports.push(report);
                continue;
            }
            let pkg = roots.get(dir).copied();
            let mut texts: Option<PackageTexts> = None;
            let mut unknown: Vec<&str> = Vec::new();
            let mut linked: Vec<NodeId> = Vec::new();
            for name in &d.deps {
                let mut found = pkg.map_or_else(Vec::new, |p| links.consumers(g, p, dir, name));
                found.extend(stand_ins.iter().filter(|(_, p)| p == name).map(|(n, _)| *n));
                let Some(p) = pkg else {
                    if found.is_empty() {
                        unknown.push(name);
                    }
                    linked.extend(found);
                    continue;
                };
                let texts = texts.get_or_insert_with(|| PackageTexts::read(g, p, dir));
                let implicit = implicit_consumers(g, &links, texts, p, dir, name, &found);
                let none = found.is_empty()
                    && implicit.nodes.is_empty()
                    && implicit.types.is_empty()
                    && !implicit.outside_graph;
                // A config that names the package where its tool loads it: the nodes that
                // have the config as an input read it like the ones with an edge to it.
                for &c in &implicit.as_changed {
                    engine.match_inputs(g.path(c), &mut |m| linked.push(m));
                }
                report.named.extend(implicit.named);
                // A config that imports the package: see `config_imports`.
                linked.extend(config_linkers(
                    engine,
                    &found,
                    name,
                    config_imports(strict),
                    &mut report.named,
                ));
                linked.extend(found);
                linked.extend(implicit.nodes);
                type_seeds.extend(implicit.types);
                if implicit.outside_graph {
                    // Named by a file that is not a node: its readers are not known.
                    unknown.push(name);
                } else if none {
                    if d.deps_removed.contains(name) || lockfile_diffed {
                        report.unconsumed.push(name.clone());
                    } else {
                        // No lockfile diff to say what the install changes.
                        unknown.push(name);
                    }
                }
            }
            if !unknown.is_empty() {
                report.unconsumed.clear();
                report.named.clear();
                report.reason = format!(
                    "changed dependenc(ies) whose consumers are not known: {}",
                    unknown.join(", ")
                );
                fx.reports.push(report);
                continue;
            }
            linked.sort_unstable();
            linked.dedup();
            report.links = linked.len();
            seeds.extend(linked);
        }
        // Scripts: the task of each added or edited script; for a removed one, every
        // task that read the manifest (its dependents no longer find it).
        let pkg = if dir.is_empty() { "." } else { dir };
        for s in &d.scripts_changed {
            if let Some(t) = g.node_by_path(&format!("task:{pkg}#{s}")) {
                report.tasks += 1;
                seeds.push(t);
            }
        }
        if !d.scripts_removed.is_empty() {
            engine.match_inputs(&c.path, &mut |n| {
                if g.is_task(n) {
                    report.tasks += 1;
                    seeds.push(n);
                }
            });
        }
        // `version` is `$npm_package_version` in the package's scripts (and in the
        // environment of what they run, see below).
        if d.version {
            version_bumps.get_or_insert_with(|| c.path.clone());
            let prefix = format!("task:{pkg}#");
            for (n, t) in tasks.get_or_insert_with(|| g.tasks()).iter() {
                if t.id.as_str().starts_with(&prefix)
                    && t.lines
                        .iter()
                        .any(|l| l.as_str().contains("npm_package_version"))
                {
                    report.tasks += 1;
                    seeds.push(*n);
                }
            }
        }
        report.field_level = true;
        report.reason = if d.fields.is_empty() {
            "formatting only".to_owned()
        } else if !d.resolution.is_empty() {
            "resolution fields applied per request of each importer".to_owned()
        } else if d.deps.is_empty() {
            "no changed field is read by resolution".to_owned()
        } else if report.unconsumed.is_empty() {
            "every changed dependency has known consumers in the package".to_owned()
        } else {
            format!(
                "dependencies applied per consumer; none found for {} (not linked, probed or \
                 named in the package: {})",
                report.unconsumed.join(", "),
                if lockfile_diffed {
                    "only the lockfile diff can matter"
                } else {
                    "removed with its last reference"
                }
            )
        };
        fx.drop.insert(c.path.clone());
        fx.seeds.extend(seeds.into_iter().map(|n| (n, cause())));
        fx.type_seeds
            .extend(type_seeds.into_iter().map(|n| (n, cause())));
        fx.reports.push(report);
    }
    // A package manager exports the running package's version as
    // `npm_package_version`: first-party code that names the variable reads the
    // field without reading the file.
    if let Some(path) = version_bumps {
        for n in env_version_readers(g) {
            fx.env_readers += 1;
            fx.seeds.push((n, Cause::Modified { path: path.clone() }));
        }
    }
    fx
}

#[cfg(test)]
mod tests {
    use turborepo_trace_graph::RefKind as K;

    use super::*;

    fn delta(old: &str, new: &str) -> ManifestDelta {
        manifest_delta(old, new).expect("both sides parse")
    }

    #[test]
    fn inert_fields() {
        let d = delta(
            r#"{"name":"a","version":"1.0.0","description":"x","files":["dist"]}"#,
            r#"{"name":"a","version":"1.0.1","description":"y","files":["dist","src"],"license":"MIT"}"#,
        );
        assert!(d.is_field_level());
        assert!(d.version);
        assert_eq!(d.fields, ["description", "files", "license", "version"]);
        assert!(d.deps.is_empty() && d.scripts_changed.is_empty());
        // Formatting only: no field changed.
        let d = delta("{\"name\":\"a\"}", "{\n  \"name\": \"a\"\n}\n");
        assert!(d.is_field_level() && d.fields.is_empty());
    }

    #[test]
    fn scripts_and_dependencies() {
        let d = delta(
            r#"{"scripts":{"build":"tsc","lint":"eslint .","old":"x"},
                "dependencies":{"a":"1","b":"1","c":"1"},"devDependencies":{"d":"1"}}"#,
            r#"{"scripts":{"build":"tsc -b","lint":"eslint .","new":"y"},
                "dependencies":{"a":"2","b":"1"},"devDependencies":{"c":"1","d":"1"},
                "peerDependencies":{"e":"*"}}"#,
        );
        assert!(d.is_field_level());
        assert_eq!(
            d.scripts_changed.iter().collect::<Vec<_>>(),
            ["build", "new"]
        );
        assert_eq!(d.scripts_removed.iter().collect::<Vec<_>>(), ["old"]);
        assert_eq!(d.deps.iter().collect::<Vec<_>>(), ["a", "c", "e"]);
        assert_eq!(
            d.fields,
            [
                "dependencies.a",
                "dependencies.c",
                "devDependencies.c",
                "peerDependencies.e",
                "scripts.build",
                "scripts.new",
                "scripts.old"
            ]
        );
    }

    #[test]
    fn other_fields_are_whole_file() {
        for (field, old, new) in [
            ("type", r#"{}"#, r#"{"type":"module"}"#),
            ("name", r#"{"name":"a"}"#, r#"{"name":"b"}"#),
            ("jest", r#"{"jest":{}}"#, r#"{"jest":{"preset":"x"}}"#),
            (
                "workspaces",
                r#"{"workspaces":["a"]}"#,
                r#"{"workspaces":["b"]}"#,
            ),
            (
                "pnpm",
                r#"{"pnpm":{}}"#,
                r#"{"pnpm":{"overrides":{"a":"1"}}}"#,
            ),
            ("sideEffects", r#"{"sideEffects":false}"#, r#"{}"#),
            (
                "engines",
                r#"{"engines":{"node":"20"}}"#,
                r#"{"engines":{"node":"22"}}"#,
            ),
            ("somethingNew", r#"{}"#, r#"{"somethingNew":1}"#),
            // Maps that are not objects are not understood.
            ("scripts", r#"{"scripts":{}}"#, r#"{"scripts":[]}"#),
            (
                "dependencies",
                r#"{"dependencies":"x"}"#,
                r#"{"dependencies":{}}"#,
            ),
        ] {
            let d = delta(old, new);
            assert!(!d.is_field_level(), "{field}");
            assert_eq!(d.other, [field]);
        }
        // Mixed with inert fields: still whole-file.
        let d = delta(
            r#"{"version":"1","type":"commonjs"}"#,
            r#"{"version":"2","type":"module"}"#,
        );
        assert_eq!(d.other, ["type"]);
        assert_eq!(d.fields, ["type", "version"]);
    }

    #[test]
    fn resolution_fields_are_kept_for_the_entry_diff() {
        // Not whole-file by themselves: applied per request (crate::exportsdiff).
        let d = delta(
            r##"{"main":"a.js","exports":{".":{"import":"./a.mjs","default":"./a.js"}},"imports":{"#a":"./a.js"},"version":"1"}"##,
            r##"{"main":"b.js","exports":{".":{"default":"./a.js","import":"./a.mjs"}},"imports":{"#a":"./a.js"},"version":"2"}"##,
        );
        assert!(d.is_field_level());
        // `exports` differs only in the order of its conditions.
        assert_eq!(d.resolution, ["exports", "main"]);
        assert_eq!(d.fields, ["exports", "main", "version"]);
        assert!(d.docs.is_some());
        let d = delta(
            r#"{"main":"a.js","version":"1"}"#,
            r#"{"main":"a.js","version":"2"}"#,
        );
        assert!(d.resolution.is_empty() && d.docs.is_none());
    }

    #[test]
    fn unparsable_sides() {
        assert!(manifest_delta("{", "{}").is_none());
        assert!(manifest_delta("{}", "{\"a\":").is_none());
        assert!(manifest_delta("[]", "{}").is_none());
        assert!(manifest_delta("{}", "null").is_none());
        assert!(manifest_delta("", "{}").is_none());
    }

    #[test]
    fn spec_packages() {
        assert_eq!(package_of_spec("react"), Some("react"));
        assert_eq!(package_of_spec("react/jsx-runtime"), Some("react"));
        assert_eq!(package_of_spec("@a/b"), Some("@a/b"));
        assert_eq!(package_of_spec("@a/b/c/d"), Some("@a/b"));
        assert_eq!(package_of_spec("@a"), None);
        assert_eq!(package_of_spec("./x"), None);
        assert_eq!(package_of_spec("#x"), None);
        assert_eq!(package_of_spec("node:fs"), None);
    }

    #[test]
    fn names_in_text() {
        assert!(mentions(b"require('left-pad')", "left-pad"));
        assert!(mentions(b"from \"@acme/ui/button\"", "@acme/ui"));
        assert!(mentions(b"presets: ['fancy']", "fancy"));
        assert!(mentions(b"cli-tool deploy --prod", "cli-tool"));
        assert!(mentions(b"npx cli-tool@2 deploy", "cli-tool"));
        assert!(mentions(b"./node_modules/pkg/bin.js", "pkg"));
        // Part of a longer name is not a mention.
        assert!(!mentions(b"require('react-dom')", "react"));
        assert!(!mentions(b"require('preact')", "react"));
        assert!(!mentions(b"require('@scope/react')", "react"));
        assert!(!mentions(b"x.react.y", "react"));
        assert!(!mentions(b"", "react"));
        assert!(mentions(b"a react-dom and react", "react"));
    }

    #[test]
    fn tool_short_names() {
        let names = |n: &str| -> Vec<String> { short_names(n).into_iter().map(|x| x.0).collect() };
        assert_eq!(names("eslint-plugin-react"), ["react"]);
        assert_eq!(names("@acme/eslint-plugin"), ["@acme"]);
        assert_eq!(names("@acme/eslint-config-base"), ["@acme/base"]);
        assert_eq!(names("babel-preset-fancy"), ["fancy"]);
        assert_eq!(names("@babel/preset-env"), ["@babel/env"]);
        assert_eq!(
            names("@babel/plugin-transform-runtime"),
            ["@babel/transform-runtime"]
        );
        assert_eq!(names("jest-environment-jsdom"), ["jsdom"]);
        assert!(names("react").is_empty());
        assert!(names("@acme/plugin-x").is_empty());
        assert!(names("eslint-plugin").is_empty());
        for f in [
            ".babelrc",
            ".eslintrc.json",
            "babel.config.js",
            "postcss.config.mjs",
            "tsconfig.json",
            "tsconfig.build.json",
            ".swcrc",
            "nodemonrc.json",
        ] {
            assert!(is_tool_config(f), "{f}");
        }
        for f in ["index.ts", "config.ts", "source.tsx"] {
            assert!(!is_tool_config(f), "{f}");
        }
        assert!(is_runner_config("jest.config.ts") && is_runner_config("vitest.config.mts"));
        assert!(!is_runner_config("babel.config.js"));
    }

    #[test]
    fn edge_uses() {
        let pj = "package.json";
        let classify =
            |kind, spec: Option<&str>, lang: &str| classify_use(kind, spec, Some(lang), pj, false);
        assert_eq!(classify(K::Manifest, Some("./util"), "js"), Use::Resolution);
        assert_eq!(classify(K::FsRead, Some(""), "task-members"), Use::Hub);
        // Reads of the file as data.
        assert_eq!(
            classify(K::Require, Some("../package.json"), "js"),
            Use::Data
        );
        assert_eq!(
            classify(K::Import, Some("@acme/a/package.json"), "js"),
            Use::Data
        );
        assert_eq!(
            classify(K::FsRead, Some("../package.json"), "js"),
            Use::Data
        );
        assert_eq!(classify(K::FsRead, Some(".."), "js"), Use::Data);
        assert_eq!(classify(K::Asset, None, "js"), Use::Data);
        assert_eq!(
            classify(K::FsRead, Some("input packages/a/package.json"), "task"),
            Use::Data
        );
        // The manifest standing for a package that did not resolve.
        assert_eq!(
            classify(K::Require, Some("eslint"), "task"),
            Use::StandIn("eslint".into())
        );
        assert_eq!(
            classify(
                K::Require,
                Some("jest:test_environment jest-environment-node"),
                "js"
            ),
            Use::StandIn("jest-environment-node".into())
        );
        assert_eq!(
            classify(K::Import, Some("@acme/b/missing"), "js"),
            Use::StandIn("@acme/b".into())
        );
        // A framework's configuration inside the manifest.
        assert_eq!(
            classify(K::FsRead, Some("jest:config packages/a/package.json"), "js"),
            Use::Framework
        );
    }

    #[test]
    fn framework_edges_into_a_manifest() {
        let pj = "package.json";
        let fw = |spec: &str, loads_module| {
            classify_use(K::Require, Some(spec), Some("js"), pj, loads_module)
        };
        // An import a module mapper rewrites to a path: the edge lists the module and
        // the manifest that says which module type the requesting package has.
        let mapped = "jest:moduleNameMapper #/swr/use-team -> apps/site/swr/use-team";
        assert_eq!(fw(mapped, true), Use::Resolution);
        assert_eq!(
            fw("vitest:resolve.alias @/lib/x -> apps/site/src/lib/x", true),
            Use::Resolution
        );
        // Mocks and transforms resolve requests the same way.
        assert_eq!(fw("jest:automocked #/lib/flags", true), Use::Resolution);
        assert_eq!(fw("jest:require_actual ./store", true), Use::Resolution);
        // A request rewritten to a package that did not resolve: the manifest stands
        // for it (the edge has no other successor).
        assert_eq!(
            fw(
                "jest:moduleNameMapper ./a.module.css -> identity-obj-proxy",
                false
            ),
            Use::StandIn("identity-obj-proxy".into())
        );
        assert_eq!(
            fw("jest:moduleNameMapper flags -> @acme/flags/next", false),
            Use::StandIn("@acme/flags".into())
        );
        // The manifest itself is what the request loads.
        assert_eq!(
            fw("jest:moduleNameMapper #pkg -> apps/site/package.json", true),
            Use::Data
        );
        assert_eq!(fw("jest:require_actual ../package.json", false), Use::Data);
        // Nothing else was loaded: the request resolved to the manifest (an `imports`
        // entry that names it), or nothing says otherwise.
        assert_eq!(fw("jest:automocked #manifest", false), Use::Data);
        assert_eq!(fw(mapped, false), Use::StandIn("apps".into()));
        // An import of the plain graph has its own manifest edges: its module edge into
        // a manifest is a read.
        assert_eq!(
            classify_use(K::Require, Some("#manifest"), Some("js"), pj, true),
            Use::Data
        );
    }
}
