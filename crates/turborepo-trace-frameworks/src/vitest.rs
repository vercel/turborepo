//! The vitest adapter.
//!
//! - Configs: `vitest.config.*` (preferred) or `vite.config.*` in the nearest
//!   ancestor of the test; workspaces: `vitest.workspace.*` files and
//!   `test.projects` / `test.workspace` arrays (strings are globs of
//!   directories or config files, objects are inline projects, `extends` merges
//!   a config file or, with `true`, the root config).
//! - Evaluation: `defineConfig`, `defineProject`, `defineWorkspace`,
//!   `mergeConfig` of local configs, function configs (called with an unknown
//!   `{ mode, command }`), and `configDefaults` from `vitest/config` (known
//!   defaults). A config written for vite-plus (`defineConfig` and
//!   `configDefaults` from `vite-plus`, or a plain object that `satisfies
//!   UserConfig`) is read the same way.
//! - Fields: `root`, `test.root`, `test.dir`, `test.include`, `test.exclude`,
//!   `test.setupFiles`, `test.globalSetup`, `test.environment`, `test.alias`,
//!   `resolve.alias` (object and array forms, string and regex `find`),
//!   `test.name`, `test.resolveSnapshotPath`, `plugins` (known plugins that
//!   cannot redirect imports are accepted, loader plugins are scoped to the
//!   modules they compile, anything else is reported as not modelled:
//!   [`crate::viteplugins`]), `test.reporters` (data). A regex `find` is
//!   applied as `request.replace(find, replacement)`; an alias `customResolver`
//!   is modelled when it is `require.resolve` of its argument and reported
//!   otherwise. Path values of these fields are reported as
//!   [`crate::inputs::ConfigValues`].
//! - Versions: the default `exclude` is the long list up to vitest 3 and
//!   `node_modules` and `.git` since vitest 4 ([`default_exclude`]), by the
//!   version the project runs ([`project_major`]; not known: the short list,
//!   the larger set of tests). A run started below a config loads it up to
//!   vitest 4 with the run's directory as root ([`rebased`]) and has the
//!   defaults since vitest 5 ([`default_discovery`]).
//! - Tests: `vi.mock` / `vi.doMock` (with `__mocks__` lookup when no factory),
//!   `vi.importActual`, `vi.importMock`, `@vitest-environment`, default
//!   snapshot paths.

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_lang_js::{
    Dialect,
    extract::config::{ConfigObj, ConfigVal, FunctionShape, merge_deep},
};

use crate::{
    Framework,
    common::{
        Item, ModuleResolver, Reader, common_test_inputs, config_values, drop_runner_files,
        is_path_request, module_target,
    },
    fsx,
    fsx::PathFsx,
    inputs::{
        Alias, AliasMatcher, FrameworkInputs, FrameworkUncertainty, GlobalInputs, InputRef,
        InputRole, MockPolicy, Project, ResolutionOverrides, SnapshotPolicy, TestDiscovery,
        TestInputs, TestPattern,
    },
    jsconfig::{self, LoadOptions, Loaded},
    pattern::{JsRegex, is_glob},
    scan,
};

/// Config file names, in lookup order (vitest configs before vite configs).
pub const CONFIG_NAMES: &[&str] = &[
    "vitest.config.ts",
    "vitest.config.mts",
    "vitest.config.cts",
    "vitest.config.js",
    "vitest.config.mjs",
    "vitest.config.cjs",
    "vite.config.ts",
    "vite.config.mts",
    "vite.config.cts",
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.cjs",
];

/// Workspace file names.
pub const WORKSPACE_NAMES: &[&str] = &[
    "vitest.workspace.ts",
    "vitest.workspace.mts",
    "vitest.workspace.cts",
    "vitest.workspace.js",
    "vitest.workspace.mjs",
    "vitest.workspace.cjs",
    "vitest.workspace.json",
];

/// The package a project runs its tests with ([`InputRole::Runner`]).
pub const RUNNER_PACKAGE: &str = "vitest";

/// vitest default `include`.
pub const DEFAULT_INCLUDE: &[&str] = &["**/*.{test,spec}.?(c|m)[jt]s?(x)"];

/// vitest's default `exclude` up to version 3 (`configDefaults.exclude`).
/// Since version 4 it is [`DEFAULT_EXCLUDE_4`]; which one applies is decided
/// by the version the project runs ([`project_major`]).
pub const DEFAULT_EXCLUDE: &[&str] = &[
    "**/node_modules/**",
    "**/dist/**",
    "**/cypress/**",
    "**/.{idea,git,cache,output,temp}/**",
    "**/{karma,rollup,webpack,vite,vitest,jest,ava,babel,nyc,cypress,tsup,build,eslint,prettier}.\
     config.*",
];

/// The fields whose path values this adapter turns into inputs itself
/// ([`crate::inputs::ConfigValues`]).
const MODELLED_FIELDS: &[&str] = &[
    "root",
    "test.root",
    "test.dir",
    "test.alias",
    "resolve.alias",
    "test.setupFiles",
    "test.globalSetup",
    "test.include",
    "test.exclude",
];

/// Properties whose values name modules that only report results
/// ([`crate::inputs::ConfigValues::runner`]): `test.reporters`.
const REPORTER_FIELDS: &[&str] = &["reporters"];

/// Vite's default `resolve.extensions`.
pub const DEFAULT_EXTENSIONS: &[&str] = &["mjs", "js", "mts", "ts", "jsx", "tsx", "json"];

/// The vitest adapter.
#[derive(Copy, Clone, Debug, Default)]
pub struct Vitest;

/// vitest's default `exclude` since version 4, a part of [`DEFAULT_EXCLUDE`].
pub const DEFAULT_EXCLUDE_4: &[&str] = &["**/node_modules/**", "**/.git/**"];

/// The default `exclude` of vitest `major` (`None`: not known). Up to version
/// 3 the long list; since version 4 `node_modules` and `.git` only, so a test
/// below `dist/` or one called `vite.config.test.ts` runs. A version that is
/// not known gets the short list: the larger set of tests.
pub fn default_exclude(major: Option<u32>) -> &'static [&'static str] {
    match major {
        Some(m) if m <= 3 => DEFAULT_EXCLUDE,
        _ => DEFAULT_EXCLUDE_4,
    }
}

/// The major version a semver range resolves to, when it can resolve to one
/// only: `^3.2.4`, `~3.2`, `3.2.4`, `3.x`, `3`. `None` for anything else
/// (`>=3`, `*`, `latest`, `catalog:`, `workspace:*`, an alias, a URL).
fn range_major(range: &str) -> Option<u32> {
    let r = range.trim();
    let r = r
        .strip_prefix('^')
        .or_else(|| r.strip_prefix('~'))
        .or_else(|| r.strip_prefix('='))
        .unwrap_or(r);
    let r = r.strip_prefix('v').unwrap_or(r);
    if r.is_empty()
        || !r
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        return None;
    }
    let head = r.split('.').next()?;
    (!head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()))
        .then(|| head.parse().ok())
        .flatten()
}

/// The major versions a semver range can resolve to: one for a simple range
/// ([`range_major`]), one for each alternative of `a || b` (`^3.2.0 || ^4.0.0`:
/// 3 and 4). Empty when an alternative names no single major version: nothing
/// says which it is.
fn range_majors(range: &str) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for alt in range.split("||") {
        match range_major(alt) {
            Some(m) => out.push(m),
            None => return Vec::new(),
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The range a pnpm catalog gives `name` in the `pnpm-workspace.yaml` text
/// `yaml`, as major versions, when every catalog that lists it gives the same.
fn catalog_majors(yaml: &str, name: &str) -> Vec<u32> {
    let mut found: Option<Vec<u32>> = None;
    let mut in_catalog = false;
    for line in yaml.lines() {
        let indented = line.starts_with([' ', '\t']);
        let text = line.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        if !indented {
            in_catalog = text.starts_with("catalog:") || text.starts_with("catalogs:");
            continue;
        }
        if !in_catalog {
            continue;
        }
        let Some((key, value)) = text.split_once(':') else {
            continue;
        };
        if key.trim().trim_matches(['\'', '"']) != name {
            continue;
        }
        let value = value.split(" #").next().unwrap_or(value);
        let majors = range_majors(value.trim().trim_matches(['\'', '"']));
        if majors.is_empty() || found.as_ref().is_some_and(|f| *f != majors) {
            return Vec::new();
        }
        found = Some(majors);
    }
    found.unwrap_or_default()
}

/// How the files that say a version are read: recorded as inputs
/// ([`fsx::read`], for a task), or as what decides a result without being an
/// input of it ([`fsx::read_key`], for a config's load).
type Read<'a> = &'a dyn Fn(&Path) -> Option<Vec<u8>>;

fn recorded(p: &Path) -> Option<Vec<u8>> {
    fsx::read(p).ok()
}

fn key_only(p: &Path) -> Option<Vec<u8>> {
    fsx::read_key(p).ok()
}

fn read_json(read: Read<'_>, p: &Path) -> Option<serde_json::Value> {
    read(p).and_then(|b| serde_json::from_slice(&b).ok())
}

/// The range the manifest `m` declares for the package `name`.
fn declared(m: &serde_json::Value, name: &str) -> Option<String> {
    [
        "devDependencies",
        "dependencies",
        "peerDependencies",
        "optionalDependencies",
    ]
    .iter()
    .find_map(|k| m.get(k)?.get(name)?.as_str().map(str::to_owned))
}

/// The major version of the vitest that the command `vitest`, started in
/// `dir`, executes, when the tree says it: the installed package
/// (`node_modules/vitest`, or the vitest that an installed `vite-plus` depends
/// on) in the nearest directory at or above `dir` that has one; else the range
/// the nearest manifest at or above `dir` declares for `vitest` (through the
/// pnpm catalog when it says `catalog:`), when that range resolves to one
/// major version only. `None` when nothing says it (a manifest that declares
/// only `vite-plus`, a range over several major versions): the caller then
/// takes every possibility.
///
/// What is read is read through [`fsx`], so it is recorded like the config.
pub fn vitest_major(dir: &Path) -> Option<u32> {
    match plain_majors(&recorded, dir).as_slice() {
        [m] => Some(*m),
        _ => None,
    }
}

/// [`vitest_major`] as the major versions it can be (a range `^3 || ^4`: 3 and
/// 4); empty when nothing says it.
fn plain_majors(read: Read<'_>, dir: &Path) -> Vec<u32> {
    let version = |v: &serde_json::Value| -> Vec<u32> {
        v.get("version")
            .and_then(|v| v.as_str())
            .map(range_majors)
            .unwrap_or_default()
    };
    // A catalog entry is looked up in the workspace file of a directory at or above
    // the manifest.
    let catalog = |from: &Path| -> Vec<u32> {
        from.ancestors()
            .find_map(|d| {
                read(&d.join("pnpm-workspace.yaml"))
                    .and_then(|y| String::from_utf8(y).ok())
                    .map(|y| catalog_majors(&y, "vitest"))
            })
            .unwrap_or_default()
    };
    for d in dir.ancestors() {
        if let Some(v) = read_json(read, &d.join("node_modules/vitest/package.json")) {
            return version(&v);
        }
        if let Some(v) = read_json(read, &d.join("node_modules/vite-plus/package.json")) {
            return declared(&v, "vitest")
                .as_deref()
                .map(range_majors)
                .unwrap_or_default();
        }
        let Some(m) = read_json(read, &d.join("package.json")) else {
            continue;
        };
        if let Some(range) = declared(&m, "vitest") {
            return if range.trim().starts_with("catalog:") {
                catalog(d)
            } else {
                range_majors(&range)
            };
        }
        if declared(&m, "vite-plus").is_some() {
            return Vec::new();
        }
    }
    Vec::new()
}

/// The major version of the vitest that `vp test`, started in `dir`, executes:
/// the one the installed `vite-plus` of the nearest directory at or above
/// `dir` depends on, whatever the manifests declare for `vitest` itself.
/// `None` when vite-plus is not installed: its manifest alone says the range.
pub fn vite_plus_major(dir: &Path) -> Option<u32> {
    match vp_majors(&recorded, dir).as_slice() {
        [m] => Some(*m),
        _ => None,
    }
}

fn vp_majors(read: Read<'_>, dir: &Path) -> Vec<u32> {
    dir.ancestors()
        .find_map(|d| read_json(read, &d.join("node_modules/vite-plus/package.json")))
        .and_then(|installed| declared(&installed, "vitest"))
        .as_deref()
        .map(range_majors)
        .unwrap_or_default()
}

/// What a run without a config of its own, started in `dir`, may do: load the
/// config of a directory above (vitest up to 4), and run with the defaults
/// (vitest 5 and later). By the versions the run can have (`vite_plus`: it is
/// `vp test`, which executes the vitest vite-plus depends on); both when
/// nothing says the version.
pub fn run_readings(dir: &Path, vite_plus: bool) -> (bool, bool) {
    let majors = if vite_plus {
        vp_majors(&recorded, dir)
    } else {
        plain_majors(&recorded, dir)
    };
    (
        majors.is_empty() || majors.iter().any(|m| *m <= 4),
        majors.is_empty() || majors.iter().any(|m| *m >= 5),
    )
}

/// The major version of the vitest that runs the project of a config in `dir`,
/// as far as the default `exclude` goes ([`default_exclude`]): `Some` when
/// every version that may run it has the same list. The versions are those of
/// [`vitest_major`], and where a manifest at or above `dir` declares
/// `vite-plus` (or it is installed there) also those of [`vite_plus_major`],
/// since `vp test` runs the project with its own vitest.
///
/// The files that say it decide what the config's load finds and are no
/// inputs of the project's tests ([`fsx::read_key`]): a manifest that changes
/// elsewhere changes no test, and a runner whose version changes reaches the
/// tests as the runner.
pub fn project_major(dir: &Path) -> Option<u32> {
    let read: Read<'_> = &key_only;
    let mut majors = plain_majors(read, dir);
    if majors.is_empty() {
        return None;
    }
    let vite_plus = dir.ancestors().any(|d| {
        read_json(read, &d.join("node_modules/vite-plus/package.json")).is_some()
            || read_json(read, &d.join("package.json"))
                .is_some_and(|m| declared(&m, "vite-plus").is_some())
    });
    if vite_plus {
        let vp = vp_majors(read, dir);
        if vp.is_empty() {
            return None;
        }
        majors.extend(vp);
    }
    let long = |m: &u32| *m <= 3;
    if majors.iter().all(long) {
        majors.iter().max().copied()
    } else if !majors.iter().any(long) {
        majors.iter().min().copied()
    } else {
        None
    }
}

/// Known values from `vitest/config` / `vitest/node`, and from vite-plus,
/// which hands on vitest's (`vite-plus`, `vite-plus/test/config`). The default
/// exclude list is the one of the version the project runs ([`project_major`],
/// asked through `major` only when the config uses the list); vite-plus 1
/// depends on vitest 5, whose list is the short one.
fn externals(module: &str, path: &[String], major: &dyn Fn() -> Option<u32>) -> Option<ConfigVal> {
    let vite_plus = matches!(
        module,
        "vite-plus" | "vite-plus/test" | "vite-plus/test/config" | "vite-plus/test/node"
    );
    if !vite_plus && !matches!(module, "vitest/config" | "vitest/node" | "vitest") {
        return None;
    }
    let strs = |xs: &[&str]| ConfigVal::Arr(xs.iter().map(|s| ConfigVal::str(*s)).collect());
    match path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["configDefaults", "exclude"] | ["defaultExclude"] if vite_plus => {
            Some(strs(DEFAULT_EXCLUDE_4))
        }
        ["configDefaults", "exclude"] | ["defaultExclude"] => Some(strs(default_exclude(major()))),
        ["configDefaults", "include"] | ["defaultInclude"] => Some(strs(DEFAULT_INCLUDE)),
        _ => None,
    }
}

/// Loads the config file `path` with the values vitest's own modules export.
fn load_js(path: &Path) -> Loaded {
    let dir = path.parent().unwrap_or(Path::new("/"));
    let major = std::cell::OnceCell::new();
    let ext = |module: &str, member: &[String]| {
        externals(module, member, &|| {
            *major.get_or_init(|| project_major(dir))
        })
    };
    jsconfig::load(
        path,
        LoadOptions {
            externals: &ext,
            ..LoadOptions::default()
        },
    )
}

/// The discovery of a project as a run sees it that is started in `dir`, a
/// directory below the project's config without a config of its own, and
/// loads that config (vitest up to 4 takes the config of the nearest directory
/// above): the root and the base of the patterns are `dir`.
pub fn rebased(discovery: &TestDiscovery, dir: &Path) -> TestDiscovery {
    TestDiscovery {
        roots: vec![dir.to_path_buf()],
        include: discovery.include.clone(),
        exclude: discovery.exclude.clone(),
        glob_base: Some(dir.to_path_buf()),
        ..TestDiscovery::default()
    }
}

/// Whether the patterns of `discovery` match a file the same way from every
/// base above it: each one is a glob that starts with `**/`.
pub fn base_free(discovery: &TestDiscovery) -> bool {
    discovery
        .include
        .iter()
        .chain(&discovery.exclude)
        .all(|p| matches!(p, TestPattern::Glob(g) if g.starts_with("**/")))
}

/// The discovery of a run without a config, started in `dir`: vitest's
/// defaults (the exclude list of version 4 and later: the larger set; a run
/// that loads no config of a directory above is vitest 5 or later, or has
/// none above).
pub fn default_discovery(dir: &Path) -> TestDiscovery {
    let globs = |xs: &[&str]| xs.iter().map(|g| TestPattern::Glob((*g).into())).collect();
    TestDiscovery {
        roots: vec![dir.to_path_buf()],
        include: globs(DEFAULT_INCLUDE),
        exclude: globs(DEFAULT_EXCLUDE_4),
        glob_base: Some(dir.to_path_buf()),
        ..TestDiscovery::default()
    }
}

impl Framework for Vitest {
    fn id(&self) -> &'static str {
        "vitest"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        CONFIG_NAMES
    }

    fn is_config_file(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        CONFIG_NAMES.contains(&name) || WORKSPACE_NAMES.contains(&name)
    }

    fn discover(&self, root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fsx::walk_dirs(root, &mut |d| {
            if let Some(w) = fsx::first_existing(d, WORKSPACE_NAMES) {
                out.push(w);
            } else if let Some(c) = fsx::first_existing(d, CONFIG_NAMES) {
                out.push(c);
            }
        });
        out
    }

    fn config_for(&self, test: &Path) -> Option<PathBuf> {
        let mut d = test.parent();
        while let Some(dir) = d {
            if let Some(c) = fsx::first_existing(dir, CONFIG_NAMES) {
                return Some(c);
            }
            d = dir.parent();
        }
        None
    }

    fn load(&self, config: &Path) -> FrameworkInputs {
        load_config(config)
    }

    fn test_inputs(
        &self,
        project: &Project,
        test: &Path,
        src: &[u8],
        resolver: &dyn ModuleResolver,
    ) -> TestInputs {
        self.test_inputs_scanned(project, test, src, resolver, None)
    }

    fn test_inputs_scanned(
        &self,
        project: &Project,
        test: &Path,
        src: &[u8],
        resolver: &dyn ModuleResolver,
        mocks: Option<&crate::mocks::MockScan>,
    ) -> TestInputs {
        let dialect = Dialect::from_path(test);
        let scanned = scan::scan(src, dialect, &["vi"], "vitest-environment");
        let mut out = common_test_inputs(project, test, &scanned, resolver, &env_request);
        let own;
        let mocks = match mocks {
            Some(m) => m,
            None => {
                own = turborepo_trace_lang_js::mocks::scan(src, dialect);
                &own
            }
        };
        crate::mocks::mock_inputs(
            Some(project),
            self.id(),
            test,
            mocks,
            true,
            resolver,
            &mut out,
        );
        out
    }
}

/// `test.environment` → module request (`None` for the built-in `node`).
pub fn env_request(e: &str) -> Option<String> {
    Some(match e {
        "node" => return None,
        "jsdom" => "jsdom".into(),
        "happy-dom" => "happy-dom".into(),
        "edge-runtime" => "@edge-runtime/vm".into(),
        e if is_path_request(e) || e.starts_with("vitest-environment-") || e.contains('/') => {
            e.into()
        }
        e => format!("vitest-environment-{e}"),
    })
}

/// Loads a vitest/vite config or workspace file.
pub fn load_config(path: &Path) -> FrameworkInputs {
    // A config that its own list of projects names (`projects: ['.']`, a glob that
    // matches its directory) is, as that project, its options without the list.
    match jsconfig::listing_projects(path, || load_with(path, true)) {
        Some(inputs) => inputs,
        None => load_with(path, false),
    }
}

/// [`load_config`]; `lists`: whether the config's list of projects is followed.
fn load_with(path: &Path, lists: bool) -> FrameworkInputs {
    let loaded = load_js(path);
    let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut out = FrameworkInputs {
        framework: "vitest",
        config_file: Some(path.to_path_buf()),
        ..FrameworkInputs::default()
    };
    let cfg = loaded
        .value
        .clone()
        .unwrap_or(ConfigVal::Obj(ConfigObj::default()));
    if lists && WORKSPACE_NAMES.contains(&name) {
        let mut r = Reader::new(cfg.clone(), path.to_path_buf(), &loaded.src);
        out.projects = workspace_projects(&mut r, &cfg, &dir, &loaded, None);
        out.uncertainties.extend(r.unc);
        out.uncertainties
            .extend(loaded.uncertainties.iter().cloned());
        return out;
    }
    let mut r = Reader::new(cfg.clone(), path.to_path_buf(), &loaded.src);
    for field in ["test.projects", "test.workspace"] {
        if lists
            && let Some(list) = r.get(field)
            && !list.is_null()
        {
            let list = match list {
                // `test.workspace: './vitest.workspace.ts'`.
                ConfigVal::Str(v) if v.is_known() => {
                    let wp = fsx::join(&dir, v.as_str().unwrap_or(""));
                    let mut fi = load_config(&wp);
                    for p in &mut fi.projects {
                        add_parent_files(p, &loaded);
                    }
                    out.projects = fi.projects;
                    out.uncertainties.extend(fi.uncertainties);
                    return out;
                }
                l => l,
            };
            out.projects = workspace_projects(&mut r, &list, &dir, &loaded, Some(&cfg));
            out.uncertainties.extend(r.unc);
            out.uncertainties
                .extend(loaded.uncertainties.iter().cloned());
            return out;
        }
    }
    out.projects
        .push(build_project(&mut r, Some(path), &dir, &loaded));
    out
}

fn add_parent_files(p: &mut Project, parent: &Loaded) {
    for f in parent.files.iter().rev() {
        if !p.global.config_files.contains(f) {
            p.global.config_files.insert(0, f.clone());
        }
    }
    for f in parent.code_files.iter().rev() {
        if !p.global.js_entries.contains(f) {
            p.global.js_entries.insert(0, f.clone());
        }
    }
}

/// Projects of a workspace list (`defineWorkspace([...])`, `test.projects`).
fn workspace_projects(
    r: &mut Reader<'_>,
    list: &ConfigVal,
    dir: &Path,
    loaded: &Loaded,
    root_cfg: Option<&ConfigVal>,
) -> Vec<Project> {
    let mut out = Vec::new();
    let field = "projects";
    let items: Vec<ConfigVal> = match list {
        ConfigVal::Arr(xs) => xs.clone(),
        ConfigVal::Alts(_) => list
            .alternatives()
            .into_iter()
            .flat_map(|a| match a {
                ConfigVal::Arr(xs) => xs.clone(),
                other => vec![other.clone()],
            })
            .collect(),
        other => {
            r.report_unknown(field, other);
            return out;
        }
    };
    let mut negated: Vec<String> = Vec::new();
    for it in &items {
        if let Some(s) = it.as_str()
            && let Some(neg) = s.strip_prefix('!')
        {
            negated.push(fsx::join(dir, neg).to_string_lossy().into_owned());
        }
    }
    for it in items {
        match &it {
            ConfigVal::Str(v) if v.is_known() => {
                let s = v.as_str().unwrap_or("");
                if s.starts_with('!') {
                    continue;
                }
                let abs = fsx::join(dir, s);
                let mut paths = if is_glob(s) {
                    // tinyglobby (fast-glob before it) follows links. (A link to a
                    // directory that the pattern itself names is a project for vitest 2
                    // and none for 3.2: taken as one.)
                    let links = fsx::Links::Follow;
                    let mut ps = fsx::expand_glob(&abs.to_string_lossy(), true, links);
                    ps.extend(fsx::expand_glob(&abs.to_string_lossy(), false, links));
                    ps.sort();
                    ps
                } else {
                    vec![abs]
                };
                paths.retain(|p| {
                    !negated
                        .iter()
                        .any(|n| crate::pattern::glob_matches(n, &p.to_string_lossy()))
                });
                for p in paths {
                    let cfg_path = if p.fx_is_dir() {
                        fsx::first_existing(&p, CONFIG_NAMES)
                    } else if p.fx_is_file() && is_config_like(&p) {
                        Some(p.clone())
                    } else {
                        None
                    };
                    let mut projects = match cfg_path {
                        Some(c) => load_config(&c).projects,
                        None if p.fx_is_dir() => {
                            let empty = Loaded::default();
                            let mut pr =
                                Reader::new(ConfigVal::Obj(ConfigObj::default()), p.clone(), &[]);
                            vec![build_project(&mut pr, None, &p, &empty)]
                        }
                        None => Vec::new(),
                    };
                    for pr in &mut projects {
                        add_parent_files(pr, loaded);
                    }
                    out.extend(projects);
                }
            }
            ConfigVal::Obj(_) => {
                // Inline project, possibly extending a config.
                let mut cfg = it.clone();
                let mut extra = Loaded::default();
                match it.field("extends") {
                    Some(ConfigVal::Str(v)) if v.is_known() => {
                        let base = fsx::join(dir, v.as_str().unwrap_or(""));
                        let bl = load_js(&base);
                        if let Some(bv) = &bl.value {
                            cfg = merge_deep(bv.clone(), cfg);
                        }
                        extra = bl;
                    }
                    Some(ConfigVal::Bool(true)) => {
                        if let Some(rc) = root_cfg {
                            let mut base = rc.clone();
                            if let ConfigVal::Obj(o) = &mut base
                                && let Some(ConfigVal::Obj(t)) = o
                                    .entries
                                    .iter_mut()
                                    .find(|(k, _)| k == "test")
                                    .map(|(_, v)| v)
                            {
                                t.remove("projects");
                                t.remove("workspace");
                            }
                            cfg = merge_deep(base, cfg);
                        }
                    }
                    Some(other) if !other.is_null() => r.report_unknown("extends", &other),
                    _ => {}
                }
                let mut combined = loaded.clone();
                for f in extra.files {
                    if !combined.files.contains(&f) {
                        combined.files.push(f);
                    }
                }
                for f in extra.code_files {
                    if !combined.code_files.contains(&f) {
                        combined.code_files.push(f);
                    }
                }
                combined.uncertainties.extend(extra.uncertainties);
                let mut pr = Reader::new(cfg, r.file.clone(), r.src);
                out.push(build_project(
                    &mut pr,
                    Some(&r.file.clone()),
                    dir,
                    &combined,
                ));
            }
            other => r.report_unknown(field, other),
        }
    }
    out
}

fn is_config_like(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    CONFIG_NAMES.contains(&n) || n.contains(".config.") || n.contains("vitest.")
}

/// Reads an alias value (object or array form) into `out`. `modelled` receives
/// the byte ranges of the `customResolver` functions that are modelled.
fn read_aliases(
    r: &mut Reader<'_>,
    field: &str,
    out: &mut Vec<Alias>,
    source: &'static str,
    loaded: &Loaded,
    modelled: &mut Vec<(u32, u32)>,
) {
    let Some(v) = r.get(field) else {
        return;
    };
    for alt in v.alternatives() {
        match alt {
            ConfigVal::Obj(o) => {
                if let Some(why) = &o.open {
                    r.report(field, UncertaintyKind::ComputedSpecifier, why.to_string());
                }
                for (k, rep) in &o.entries {
                    let reps = strings(r, field, rep);
                    out.push(Alias {
                        matcher: AliasMatcher::Prefix(k.clone()),
                        replacements: reps,
                        source,
                    });
                }
            }
            ConfigVal::Arr(xs) => {
                for x in xs {
                    let find = x.field("find");
                    let rep = x.field("replacement").unwrap_or(ConfigVal::Null);
                    let matcher = match find {
                        Some(ConfigVal::Str(v)) if v.is_known() => {
                            AliasMatcher::Prefix(v.as_str().unwrap_or("").to_string())
                        }
                        Some(ConfigVal::Regex { source, flags }) => {
                            let re = JsRegex::new(source.clone(), flags.clone());
                            if !re.is_supported() {
                                r.report(
                                    field,
                                    UncertaintyKind::UnsupportedSyntax,
                                    format!("regex not supported by the matcher: {source}"),
                                );
                            }
                            // Vite applies `importee.replace(find, replacement)`.
                            AliasMatcher::Replace(re)
                        }
                        other => {
                            r.report_unknown(
                                field,
                                &other.unwrap_or(ConfigVal::Unknown("missing find".into())),
                            );
                            continue;
                        }
                    };
                    // `customResolver`: the function vite calls with the rewritten request
                    // instead of resolving it itself. `(id) => require.resolve(id)` is Node's
                    // resolution of that request, which is what the alias edge does; any
                    // other resolver may send the request anywhere.
                    match x.field("customResolver") {
                        None | Some(ConfigVal::Null) => {}
                        Some(ConfigVal::Function(f))
                            if loaded.function(&f).is_some_and(|i| {
                                i.shape == Some(FunctionShape::RequireResolve)
                            }) =>
                        {
                            modelled.extend(loaded.function(&f).map(|i| i.span));
                        }
                        Some(_) => r.report(
                            field,
                            UncertaintyKind::ConditionalResolution,
                            "alias with a customResolver: resolution may differ from the tracer's"
                                .into(),
                        ),
                    }
                    let reps = strings(r, field, &rep);
                    out.push(Alias {
                        matcher,
                        replacements: reps,
                        source,
                    });
                }
            }
            ConfigVal::Null => {}
            other => r.report_unknown(field, other),
        }
    }
}

fn strings(r: &mut Reader<'_>, field: &str, v: &ConfigVal) -> Vec<String> {
    r.items_of(field, v)
        .into_iter()
        .filter_map(|i| match i {
            Item::Str(s) => Some(s),
            Item::Resolve { spec, from_dir } => Some(if is_path_request(&spec) {
                fsx::join(Path::new(&from_dir), &spec)
                    .to_string_lossy()
                    .into_owned()
            } else {
                spec
            }),
            Item::Regex { .. } => None,
        })
        .collect()
}

/// vitest `resolvePath(file, root)`: module resolution from the root, falling
/// back to a path relative to the root (`setupFiles: 'src/setup.ts'`).
fn vitest_target(it: &Item, root: &Path) -> Option<crate::inputs::Target> {
    if let Item::Str(s) = it
        && !is_path_request(s)
        && let Some(p) = jsconfig::resolve_local(&fsx::join(root, s))
    {
        return Some(crate::inputs::Target::Path(p));
    }
    module_target(it, root, root)
}

fn build_project(
    r: &mut Reader<'_>,
    config_file: Option<&Path>,
    config_dir: &Path,
    loaded: &Loaded,
) -> Project {
    let root = match r.string("test.root").or_else(|| r.string("root")) {
        Some(s) => fsx::join(config_dir, &s.replace("<cwd>/", "")),
        None => config_dir.to_path_buf(),
    };
    let test_dir = match r.string("test.dir") {
        Some(s) => fsx::join(&root, &s),
        None => root.clone(),
    };
    let mut global = GlobalInputs {
        config_files: loaded.files.clone(),
        js_entries: loaded.code_files.clone(),
        ..GlobalInputs::default()
    };
    for (field, role) in [
        ("test.setupFiles", InputRole::SetupFile),
        ("test.globalSetup", InputRole::GlobalSetup),
    ] {
        if let Some(items) = r.items(field) {
            for it in items {
                if let Some(t) = vitest_target(&it, &root) {
                    let e = InputRef { role, target: t };
                    if !global.entries.contains(&e) {
                        global.entries.push(e);
                    }
                }
            }
        }
    }
    if let Some(env) = r.string("test.environment")
        && let Some(req) = env_request(&env)
        && let Some(t) = module_target(&Item::Str(req), &root, &root)
    {
        global.entries.push(InputRef {
            role: InputRole::TestEnvironment,
            target: t,
        });
    }
    // The runner itself. vitest loads a built-in environment (the request above,
    // when the project also installs it) as its own dependency: the instance
    // the project's `vitest` resolves to decides which `jsdom` the tests get.
    global.entries.push(InputRef {
        role: InputRole::Runner,
        target: crate::inputs::Target::Module {
            request: RUNNER_PACKAGE.into(),
            from_dir: root.clone(),
        },
    });
    // Resolution.
    let mut resolution = ResolutionOverrides {
        roots: vec![test_dir.clone()],
        module_directories: vec!["node_modules".into()],
        extensions: DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        ..ResolutionOverrides::default()
    };
    let mut modelled_spans = Vec::new();
    for field in ["test.alias", "resolve.alias"] {
        read_aliases(
            r,
            field,
            &mut resolution.aliases,
            field,
            loaded,
            &mut modelled_spans,
        );
    }
    // Paths the config only hands to the fields read here ([`ConfigValues`]).
    if let Some(cf) = config_file {
        global.values = config_values(
            &r.cfg,
            MODELLED_FIELDS,
            loaded,
            cf,
            modelled_spans,
            REPORTER_FIELDS,
        );
    }
    if let Some(items) = r.items("resolve.extensions") {
        resolution.extensions = items
            .into_iter()
            .filter_map(|i| match i {
                Item::Str(s) => Some(s.trim_start_matches('.').to_string()),
                _ => None,
            })
            .collect();
    }
    // Plugins that may redirect imports or serve modules ([`crate::viteplugins`]).
    if let Some(p) = r.get("plugins") {
        let unmodelled = crate::viteplugins::unmodelled(&p);
        if !unmodelled.is_empty() {
            r.report(
                "plugins",
                UncertaintyKind::ConditionalResolution,
                format!(
                    "vite plugins are not modelled; they may change resolution or transforms: {}",
                    unmodelled.join("; ")
                ),
            );
        }
        // Loader plugins make only the importers of the modules they compile uncertain.
        resolution.loaders = crate::viteplugins::loaders(&p);
    }
    // Discovery.
    let globs = |r: &mut Reader<'_>, field: &str, default: &[&str]| -> Vec<TestPattern> {
        let gs: Vec<String> = match r.items(field) {
            Some(items) => items
                .into_iter()
                .filter_map(|i| match i {
                    Item::Str(s) => Some(s),
                    _ => None,
                })
                .collect(),
            None => default.iter().map(|s| s.to_string()).collect(),
        };
        gs.into_iter()
            .map(|g| {
                if crate::pattern::glob_to_regex(&g).1 {
                    r.report(
                        field,
                        UncertaintyKind::PatternTooBroad,
                        format!("negated extglob approximated by '*': {g}"),
                    );
                }
                TestPattern::Glob(g)
            })
            .collect()
    };
    let discovery = TestDiscovery {
        roots: vec![test_dir.clone()],
        include: globs(r, "test.include", DEFAULT_INCLUDE),
        // The default list is the one of the version the project runs.
        exclude: match r.get("test.exclude") {
            Some(v) if !v.is_null() => globs(r, "test.exclude", &[]),
            _ => globs(
                r,
                "test.exclude",
                default_exclude(project_major(config_dir)),
            ),
        },
        glob_base: Some(test_dir.clone()),
        extensions: Vec::new(),
        absolute: false,
        choice: None,
        spec: None,
    };
    // Whether a module a test replaces with a mock stays unloaded
    // ([`crate::mocks`]): not when test files share a module registry, when the
    // tests run in a browser, or when first-party code may be loaded by Node
    // itself instead of vitest's module runner (externalized dependencies,
    // preserved symlinks).
    let set = |r: &Reader, field: &str, safe: bool| {
        r.get(field)
            .is_some_and(|v| !matches!(&v, ConfigVal::Null) && v != ConfigVal::Bool(safe))
    };
    let unreliable = [
        ("test.isolate", true),
        ("test.poolOptions.threads.isolate", true),
        ("test.poolOptions.forks.isolate", true),
        ("test.poolOptions.vmThreads.isolate", true),
        ("test.poolOptions.vmForks.isolate", true),
        ("test.browser.enabled", false),
        ("resolve.preserveSymlinks", false),
    ]
    .iter()
    .find(|(field, safe)| set(r, field, *safe))
    .map(|(field, safe)| format!("{field} is not {safe}"))
    .or_else(|| {
        [
            "test.server.deps.external",
            "test.deps.external",
            "ssr.external",
        ]
        .iter()
        .find(|f| {
            r.get(f).is_some_and(|v| {
                !matches!(&v, ConfigVal::Null) && !matches!(&v, ConfigVal::Arr(xs) if xs.is_empty())
            })
        })
        .map(|f| format!("{f}: externalized modules are loaded by Node, which no mock replaces"))
    });
    let mocks = MockPolicy {
        root_mock_dirs: vec![root.clone()],
        haste_mocks: Vec::new(),
        implicit_package_mocks: false,
        automock: false,
        extensions: resolution.extensions.clone(),
        unreliable,
    };
    let snapshots = match r.get("test.resolveSnapshotPath") {
        Some(v) if !v.is_null() => SnapshotPolicy::Custom { resolver: None },
        _ => SnapshotPolicy::Default,
    };
    if let Some(cf) = config_file
        && let Some(dir) = cf.parent()
        && CONFIG_NAMES.iter().any(|n| cf.ends_with(n))
    {
        for n in CONFIG_NAMES {
            let p = dir.join(n);
            if !p.fx_exists() {
                global.probes.push(p);
            }
        }
    }
    let name = match r.get("test.name") {
        Some(ConfigVal::Str(v)) if v.is_known() => v.as_str().unwrap_or("").to_string(),
        Some(o @ ConfigVal::Obj(_)) => o
            .field("label")
            .and_then(|n| n.as_str().map(str::to_string))
            .unwrap_or_else(|| root.to_string_lossy().into_owned()),
        _ => root.to_string_lossy().into_owned(),
    };
    let mut unc: Vec<FrameworkUncertainty> = loaded.uncertainties.clone();
    unc.append(&mut r.unc);
    drop_runner_files(&mut global, &mut unc);
    Project {
        name,
        config_file: config_file.map(Path::to_path_buf),
        root_dir: root,
        global,
        resolution,
        discovery,
        mocks,
        snapshots,
        uncertainties: unc,
    }
}
