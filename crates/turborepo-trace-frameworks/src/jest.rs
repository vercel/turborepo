//! The jest adapter.
//!
//! - Config discovery: `jest.config.{js,ts,mjs,cjs,mts,cts,json}` (jest's
//!   order) or the `"jest"` key of `package.json`, nearest ancestor of the
//!   test; `projects` (globs of directories or config files, inline objects);
//!   `rootDir` (relative to the config) and `<rootDir>` substitution (a leading
//!   `<rootDir>` in paths, every occurrence in `*PathIgnorePatterns`); local
//!   `preset`s and presets found in `node_modules` are merged.
//! - Fields: `moduleNameMapper`, `setupFiles`, `setupFilesAfterEnv`,
//!   `globalSetup`, `globalTeardown`, `testEnvironment`, `transform`,
//!   `moduleDirectories`, `modulePaths`, `roots`, `modulePathIgnorePatterns`,
//!   `snapshotResolver`, `testMatch`, `testRegex`, `testPathIgnorePatterns`,
//!   `moduleFileExtensions`, `projects`, `preset`, `resolver`, `testRunner`,
//!   `automock`, `displayName`, `reporters` (and `testResultsProcessor`,
//!   `watchPlugins`: data, [`crate::inputs::ConfigValues::runner`]).
//! - A `resolver` module of the repository is read: the default resolver with
//!   other export conditions for named requests is modelled
//!   ([`crate::inputs::ResolutionOverrides::condition_requests`]); any other
//!   resolver is reported. A `testMatch` that is read exactly and matches no
//!   file means no tests; one that could not be read exactly falls back to
//!   jest's default patterns, with a report.
//! - Path values of the fields above are reported as values the config hands to
//!   the runner ([`crate::inputs::ConfigValues`]), so that the graph does not
//!   treat them as files the config module reads.
//! - Tests: `jest.mock`, `jest.doMock`, `jest.setMock`,
//!   `jest.unstable_mockModule`, `jest.requireActual`, `jest.requireMock`,
//!   `jest.createMockFromModule`, `jest.enableAutomock`, the
//!   `@jest-environment` docblock, default snapshot paths. Manual mocks: jest's
//!   haste mock map (any `__mocks__/<name>` file in the roots that is not
//!   ignored by `modulePathIgnorePatterns` mocks the package `<name>`, with or
//!   without a `jest.mock` call) and `__mocks__/<basename>` next to mocked
//!   local modules.

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_lang_js::{
    Dialect,
    extract::config::{ConfigObj, ConfigVal, FunctionShape},
};

use crate::{
    Framework,
    common::{
        Item, ModuleResolver, Reader, common_test_inputs, config_values, drop_runner_files,
        module_target, replace_root_dir,
    },
    fsx,
    fsx::PathFsx,
    inputs::{
        Alias, AliasMatcher, DiscoveryChoice, FrameworkInputs, FrameworkUncertainty, GlobalInputs,
        InputRef, InputRole, MembershipSpec, MockPolicy, Project, ResolutionOverrides,
        SnapshotPolicy, Target, TestDiscovery, TestInputs, TestPattern,
    },
    jsconfig::{self, LoadOptions, Loaded},
    location::{DiscoverySpec, PatternSpec, Written},
    pattern::{JsRegex, is_glob},
    scan,
};

/// Config file names, in jest's lookup order.
pub const CONFIG_NAMES: &[&str] = &[
    "jest.config.js",
    "jest.config.ts",
    "jest.config.mjs",
    "jest.config.cjs",
    "jest.config.mts",
    "jest.config.cts",
    "jest.config.json",
];

/// The package a project runs its tests with ([`InputRole::Runner`]).
pub const RUNNER_PACKAGE: &str = "jest";

/// What `next/jest` reads in the project directory besides the jest config: the
/// Next.js config (first existing name wins in Next.js; all are watched) and
/// the env files that `loadEnvConfig` loads when `NODE_ENV` is `test`
/// (`.env.local` is skipped in test mode).
const NEXT_JEST_READS: &[&str] = &[
    "next.config.js",
    "next.config.mjs",
    "next.config.cjs",
    "next.config.ts",
    "next.config.mts",
    ".env.test.local",
    ".env.test",
    ".env",
];

/// Whether the config file gets its config from `next/jest` (`import nextJest
/// from 'next/jest'`, `require('next/jest')`).
fn uses_next_jest(loaded: &Loaded) -> bool {
    let named = |s: &str| matches!(s, "next/jest" | "next/jest.js");
    loaded.imports.iter().any(|i| named(&i.spec))
        || [
            &b"require('next/jest')"[..],
            b"require(\"next/jest\")",
            b"require('next/jest.js')",
            b"require(\"next/jest.js\")",
        ]
        .iter()
        .any(|needle| loaded.src.windows(needle.len()).any(|w| w == *needle))
}

/// jest 30 default `testMatch`.
pub const DEFAULT_TEST_MATCH: &[&str] = &[
    "**/__tests__/**/*.?([mc])[jt]s?(x)",
    "**/?(*.)+(spec|test).?([mc])[jt]s?(x)",
];

/// The fields whose path values this adapter turns into inputs itself
/// ([`crate::inputs::ConfigValues`]). `modulePaths` and `moduleDirectories` are
/// not here: they are read, but the graph does not resolve imports through them
/// yet, so a directory named there stays a dependency of the config as a whole.
const MODELLED_FIELDS: &[&str] = &[
    "rootDir",
    "roots",
    "moduleNameMapper",
    "setupFiles",
    "setupFilesAfterEnv",
    "globalSetup",
    "globalTeardown",
    "testEnvironment",
    "transform",
    "snapshotResolver",
    "resolver",
    "testRunner",
    "preset",
    "testMatch",
    "testRegex",
    "testPathIgnorePatterns",
    "modulePathIgnorePatterns",
];

/// The fields that decide which files are tests: when all of them were read
/// exactly, a discovery that matches no file is a fact.
const DISCOVERY_FIELDS: &[&str] = &[
    "rootDir",
    "roots",
    "testMatch",
    "testRegex",
    "testPathIgnorePatterns",
    "modulePathIgnorePatterns",
    "moduleFileExtensions",
    "preset",
    "projects",
];

/// Properties whose values name modules that only report results or serve the
/// watch mode ([`crate::inputs::ConfigValues::runner`]).
const REPORTER_FIELDS: &[&str] = &["reporters", "testResultsProcessor", "watchPlugins"];

/// jest 30 default `moduleFileExtensions`.
pub const DEFAULT_EXTENSIONS: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "json", "node",
];

/// The jest adapter.
#[derive(Copy, Clone, Debug, Default)]
pub struct Jest;

impl Framework for Jest {
    fn id(&self) -> &'static str {
        "jest"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        CONFIG_NAMES
    }

    fn is_config_file(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        CONFIG_NAMES.contains(&name) || (name == "package.json" && package_json_has_jest(path))
    }

    fn discover(&self, root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fsx::walk_dirs(root, &mut |d| {
            if let Some(c) = config_in_dir(d) {
                out.push(c);
            }
        });
        out
    }

    fn config_for(&self, test: &Path) -> Option<PathBuf> {
        let mut d = test.parent();
        while let Some(dir) = d {
            if let Some(c) = config_in_dir(dir) {
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
        let scanned = scan::scan(src, dialect, &["jest"], "jest-environment");
        let mut out =
            common_test_inputs(project, test, &scanned, resolver, &|e| Some(env_request(e)));
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

/// Whether the manifest at `path` has a `jest` key: a question for its
/// resolution view (the key is a part of it), so a dependency entry or a script
/// that changes in the manifest is no change for whoever asked.
fn package_json_has_jest(path: &Path) -> bool {
    fsx::read_manifest(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v.get("jest").is_some())
}

/// The jest config in `dir`: a config file, else a `package.json` with a
/// `"jest"` key.
pub fn config_in_dir(dir: &Path) -> Option<PathBuf> {
    fsx::first_existing(dir, CONFIG_NAMES).or_else(|| {
        let p = dir.join("package.json");
        (p.fx_is_file() && package_json_has_jest(&p)).then_some(p)
    })
}

/// `testEnvironment` value → module request.
pub fn env_request(e: &str) -> String {
    match e {
        "node" => "jest-environment-node".into(),
        "jsdom" => "jest-environment-jsdom".into(),
        e if e.starts_with('.') || e.starts_with('/') || e.starts_with("<rootDir>") => e.into(),
        e if e.starts_with("jest-environment-") || e.starts_with('@') || e.contains('/') => {
            e.into()
        }
        e => format!("jest-environment-{e}"),
    }
}

fn load_opts(path: &Path) -> LoadOptions<'static> {
    let is_pkg = path.file_name().is_some_and(|n| n == "package.json");
    LoadOptions {
        json_key: is_pkg.then_some("jest"),
        ..LoadOptions::default()
    }
}

/// Loads a jest config (file or `package.json`).
pub fn load_config(path: &Path) -> FrameworkInputs {
    let loaded = jsconfig::load(path, load_opts(path));
    let config_dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut out = FrameworkInputs {
        framework: "jest",
        config_file: Some(path.to_path_buf()),
        ..FrameworkInputs::default()
    };
    let cfg = match &loaded.value {
        Some(v) => v.clone(),
        None => {
            if loaded.uncertainties.is_empty() {
                // A package.json without a "jest" key, or an empty module: defaults.
                ConfigVal::Obj(ConfigObj::default())
            } else {
                out.uncertainties = loaded.uncertainties.clone();
                ConfigVal::Obj(ConfigObj::default())
            }
        }
    };
    if matches!(cfg, ConfigVal::Unknown(_)) {
        out.uncertainties.push(FrameworkUncertainty {
            kind: UncertaintyKind::ComputedSpecifier,
            file: path.to_path_buf(),
            line: 0,
            field: None,
            detail: format!("config not statically known: {}", reasons(&cfg)),
        });
    }
    let mut reader = Reader::new(cfg.clone(), path.to_path_buf(), &loaded.src);
    let root_dir = root_dir_of(&mut reader, &config_dir);
    // Multi-project. (A config that its own list names is, as that project, its
    // options without the list.)
    let listed = jsconfig::listing_projects(path, || {
        list_projects(path, &mut reader, &root_dir, &config_dir, &loaded)
    });
    if let Some(Some(projects)) = listed {
        out.projects = projects;
        out.uncertainties.append(&mut reader.unc);
        out.uncertainties
            .extend(loaded.uncertainties.iter().cloned());
        return out;
    }
    let proj = build_project(&mut reader, Some(path), &config_dir, &loaded, true);
    out.projects.push(proj);
    out
}

/// The projects that the config `path` lists (`projects`), `None` when it lists
/// none.
fn list_projects(
    path: &Path,
    reader: &mut Reader<'_>,
    root_dir: &Path,
    config_dir: &Path,
    loaded: &Loaded,
) -> Option<Vec<Project>> {
    let mut out: Vec<Project> = Vec::new();
    if let Some(projects) = reader.get("projects") {
        let mut found = false;
        for p in projects.alternatives() {
            let ConfigVal::Arr(items) = p else {
                if !p.is_null() {
                    reader.report_unknown("projects", p);
                }
                continue;
            };
            for it in items {
                found = true;
                match it {
                    ConfigVal::Str(v) if v.is_known() => {
                        let s = replace_root_dir(v.as_str().unwrap_or(""), root_dir);
                        let abs = fsx::join(root_dir, &s);
                        let paths = if is_glob(&s) {
                            // jest expands the pattern with node-glob, then drops what
                            // `lstat` says is no directory and has no config file's
                            // extension: a link to a directory is no project directory.
                            let links = fsx::Links::NodeGlob;
                            let mut ps = fsx::expand_glob(&abs.to_string_lossy(), true, links);
                            ps.retain(|p| !fsx::is_link(p));
                            ps.extend(fsx::expand_glob(&abs.to_string_lossy(), false, links));
                            ps.sort();
                            ps
                        } else {
                            vec![abs]
                        };
                        for p in paths {
                            out.extend(project_from_path(&p, loaded));
                        }
                    }
                    ConfigVal::Obj(_) => {
                        let mut pr = Reader::new(it.clone(), path.to_path_buf(), &loaded.src);
                        let proj = build_project(&mut pr, Some(path), config_dir, loaded, false);
                        out.push(proj);
                    }
                    other => reader.report_unknown("projects", other),
                }
            }
        }
        if found {
            return Some(out);
        }
    }
    None
}

fn reasons(v: &ConfigVal) -> String {
    let mut rs = Vec::new();
    v.unknown_reasons(&mut rs);
    rs.join("; ")
}

/// A `projects` entry that is a path: a config file, or a directory (its
/// config, else defaults rooted there).
fn project_from_path(p: &Path, parent: &Loaded) -> Vec<Project> {
    let cfg_path = if p.fx_is_dir() {
        config_in_dir(p)
    } else if p.fx_is_file() {
        Some(p.to_path_buf())
    } else {
        None
    };
    let mut projects = match cfg_path {
        Some(c) => {
            let fi = load_config(&c);
            let mut ps = fi.projects;
            for pr in &mut ps {
                pr.uncertainties.extend(fi.uncertainties.iter().cloned());
            }
            ps
        }
        None => {
            let empty = Loaded::default();
            let mut r = Reader::new(ConfigVal::Obj(ConfigObj::default()), p.to_path_buf(), &[]);
            vec![build_project(&mut r, None, p, &empty, false)]
        }
    };
    // The root config decides which projects exist: it is an input of every
    // project.
    for pr in &mut projects {
        for f in parent.files.iter().rev() {
            if !pr.global.config_files.contains(f) {
                pr.global.config_files.insert(0, f.clone());
            }
        }
        for f in parent.code_files.iter().rev() {
            if !pr.global.js_entries.contains(f) {
                pr.global.js_entries.insert(0, f.clone());
            }
        }
    }
    projects
}

fn root_dir_of(r: &mut Reader<'_>, config_dir: &Path) -> PathBuf {
    match r.string("rootDir") {
        Some(s) => {
            let s = replace_root_dir(&s.replace("<cwd>/", ""), config_dir);
            fsx::join(config_dir, &s)
        }
        None => config_dir.to_path_buf(),
    }
}

/// Merges a preset into the config (jest semantics: arrays of setup files
/// concatenate preset-first, `moduleNameMapper` and `transform` merge with
/// config keys first, other keys from the config win).
fn merge_preset(cfg: &ConfigVal, preset: &ConfigVal) -> ConfigVal {
    let (ConfigVal::Obj(c), ConfigVal::Obj(p)) = (cfg, preset) else {
        return cfg.clone();
    };
    let mut out = p.clone();
    for (k, v) in &c.entries {
        let merged = match (k.as_str(), p.get(k), v) {
            ("setupFiles" | "setupFilesAfterEnv", Some(ConfigVal::Arr(a)), ConfigVal::Arr(b)) => {
                let mut x = a.clone();
                x.extend(b.iter().cloned());
                ConfigVal::Arr(x)
            }
            ("moduleNameMapper" | "transform", Some(ConfigVal::Obj(a)), ConfigVal::Obj(b)) => {
                let mut x = b.clone();
                for (ka, va) in &a.entries {
                    if x.get(ka).is_none() {
                        x.entries.push((ka.clone(), va.clone()));
                    }
                }
                ConfigVal::Obj(x)
            }
            _ => v.clone(),
        };
        out.set(k.clone(), merged);
    }
    out.entries.retain(|(k, _)| k != "preset");
    if c.open.is_some() {
        out.open = c.open.clone();
    }
    ConfigVal::Obj(out)
}

/// The preset file for `preset` from `root`: a local path, or
/// `node_modules/<preset>/jest-preset.*` found walking up.
fn find_preset(preset: &str, root: &Path) -> Option<PathBuf> {
    if preset.starts_with('.') || preset.starts_with('/') {
        let p = fsx::join(root, preset);
        return jsconfig::resolve_local(&p).or_else(|| {
            fsx::first_existing(
                &p,
                &[
                    "jest-preset.json",
                    "jest-preset.js",
                    "jest-preset.cjs",
                    "jest-preset.mjs",
                ],
            )
        });
    }
    let mut d = Some(root);
    while let Some(dir) = d {
        let pkg = dir.join("node_modules").join(preset);
        if let Some(f) = fsx::first_existing(
            &pkg,
            &[
                "jest-preset.json",
                "jest-preset.js",
                "jest-preset.cjs",
                "jest-preset.mjs",
            ],
        ) {
            return Some(f);
        }
        d = dir.parent();
    }
    None
}

/// jest's default discovery under `root` (default `testMatch`, only
/// `node_modules` ignored).
fn fallback_discovery(d: &TestDiscovery, root: &Path) -> TestDiscovery {
    let mut roots = d.roots.clone();
    if !roots.iter().any(|r| root.starts_with(r)) {
        roots.push(root.to_path_buf());
    }
    TestDiscovery {
        roots,
        include: DEFAULT_TEST_MATCH
            .iter()
            .map(|g| TestPattern::Glob((*g).to_string()))
            .collect(),
        exclude: vec![TestPattern::Regex(JsRegex::new("/node_modules/", ""))],
        glob_base: None,
        extensions: DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        absolute: true,
        choice: None,
        spec: None,
    }
}

/// Whether `d` matches a file below its roots.
fn finds_a_test(d: &TestDiscovery) -> bool {
    let m = d.matcher();
    let mut found = false;
    for r in &d.roots {
        if found {
            break;
        }
        fsx::walk_files(r, &mut |p| found = found || m.is_test(p));
    }
    found
}

/// The directories, extensions and patterns of the discovery that the config
/// object of `r` writes for a project rooted at `root`: what [`build_project`]
/// makes its discovery of, read again without its reports, for a config that
/// was loaded as if the repository were somewhere else ([`jsconfig::as_if`]).
/// The patterns that leave files out are in [`build_project`]'s order, with the
/// indexes of the ones that leave a file out of the
/// file map. [`DiscoverySpec::proved`] compares the result with the discovery
/// as loaded.
fn discovery_fields(r: &mut Reader<'_>, root: &Path) -> DiscoveryFields {
    let roots = match r.items("roots") {
        Some(items) => items
            .into_iter()
            .filter_map(|i| match i {
                Item::Str(s) => Some(fsx::join(root, &replace_root_dir(&s, root))),
                _ => None,
            })
            .collect(),
        None => vec![root.to_path_buf()],
    };
    let extensions = match r.items("moduleFileExtensions") {
        Some(items) => items
            .into_iter()
            .filter_map(|i| match i {
                Item::Str(s) => Some(s.trim_start_matches('.').to_string()),
                _ => None,
            })
            .collect(),
        None => DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
    };
    let root_s = root.to_string_lossy().into_owned();
    let regexes = |r: &mut Reader<'_>, field: &str| -> Option<Vec<TestPattern>> {
        Some(
            r.items(field)?
                .into_iter()
                .filter_map(|i| match i {
                    Item::Str(s) => Some(JsRegex::new(s.replace("<rootDir>", &root_s), "")),
                    Item::Regex { source, flags } => Some(JsRegex::new(source, flags)),
                    Item::Resolve { .. } => None,
                })
                .map(TestPattern::Regex)
                .collect(),
        )
    };
    let (mut include, mut exclude) = (Vec::new(), Vec::new());
    let test_regex = regexes(r, "testRegex").unwrap_or_default();
    if !test_regex.is_empty() {
        include = test_regex;
    } else {
        let globs: Vec<String> = match r.items("testMatch") {
            Some(items) => items
                .into_iter()
                .filter_map(|i| match i {
                    Item::Str(s) => Some(s),
                    _ => None,
                })
                .collect(),
            None => DEFAULT_TEST_MATCH.iter().map(|s| s.to_string()).collect(),
        };
        for g in globs {
            match g.strip_prefix('!') {
                Some(rest) => {
                    exclude.push(TestPattern::Glob(crate::location::jest_glob(rest, root)))
                }
                None => include.push(TestPattern::Glob(crate::location::jest_glob(&g, root))),
            }
        }
    }
    exclude.extend(
        regexes(r, "testPathIgnorePatterns")
            .unwrap_or_else(|| vec![TestPattern::Regex(JsRegex::new("/node_modules/", ""))]),
    );
    let unmapped_from = exclude.len();
    exclude.extend(regexes(r, "modulePathIgnorePatterns").unwrap_or_default());
    let unmapped = (unmapped_from..exclude.len()).collect();
    DiscoveryFields {
        roots,
        extensions,
        include,
        exclude,
        unmapped,
    }
}

/// [`discovery_fields`].
struct DiscoveryFields {
    roots: Vec<PathBuf>,
    extensions: Vec<String>,
    include: Vec<TestPattern>,
    exclude: Vec<TestPattern>,
    unmapped: Vec<usize>,
}

fn build_project(
    r: &mut Reader<'_>,
    config_file: Option<&Path>,
    config_dir: &Path,
    loaded: &Loaded,
    whole: bool,
) -> Project {
    let root = root_dir_of(r, config_dir);
    // The preset file that was merged, if one was.
    let mut preset_file: Option<PathBuf> = None;
    let mut global = GlobalInputs {
        config_files: loaded.files.clone(),
        js_entries: loaded.code_files.clone(),
        ..GlobalInputs::default()
    };
    let mut extra_unc = Vec::new();
    // Preset.
    if let Some(preset) = r.string("preset") {
        let preset = replace_root_dir(&preset, &root);
        match find_preset(&preset, &root) {
            Some(f) => {
                let pl = jsconfig::load(&f, LoadOptions::default());
                if let Some(pv) = &pl.value {
                    r.cfg = merge_preset(&r.cfg, pv);
                }
                preset_file = Some(f.clone());
                for x in pl.files {
                    if !global.config_files.contains(&x) {
                        global.config_files.push(x);
                    }
                }
                extra_unc.extend(pl.uncertainties);
            }
            None => {
                global.entries.push(InputRef {
                    role: InputRole::Preset,
                    target: Target::Module {
                        request: format!("{preset}/jest-preset"),
                        from_dir: root.clone(),
                    },
                });
                r.report(
                    "preset",
                    UncertaintyKind::UnresolvedSpecifier,
                    format!(
                        "preset '{preset}' not found; its options (transform, moduleNameMapper, \
                         testMatch, ...) are not merged"
                    ),
                );
            }
        }
    }
    // Module-ish entries.
    let add = |r: &mut Reader<'_>, field: &str, role: InputRole, global: &mut GlobalInputs| {
        if let Some(items) = r.items(field) {
            for it in items {
                if let Some(t) = module_target(&it, &root, &root) {
                    let e = InputRef { role, target: t };
                    if !global.entries.contains(&e) {
                        global.entries.push(e);
                    }
                }
            }
        }
    };
    add(r, "setupFiles", InputRole::SetupFile, &mut global);
    add(
        r,
        "setupFilesAfterEnv",
        InputRole::SetupFileAfterEnv,
        &mut global,
    );
    add(r, "globalSetup", InputRole::GlobalSetup, &mut global);
    add(r, "globalTeardown", InputRole::GlobalTeardown, &mut global);
    add(
        r,
        "snapshotResolver",
        InputRole::SnapshotResolver,
        &mut global,
    );
    add(r, "resolver", InputRole::Resolver, &mut global);
    add(r, "testRunner", InputRole::TestRunner, &mut global);
    // A custom resolver decides resolution. One that is the default resolver with
    // other export conditions for named requests
    // (`options.defaultResolver(request, { ...options, conditions: request ===
    // 'x' ? [...] : options.conditions })`) changes nothing but which file of
    // those requests' packages is picked: recorded per request
    // ([`ResolutionOverrides::condition_requests`]). Any other resolver makes the
    // project's resolution uncertain.
    let mut condition_requests: Vec<String> = Vec::new();
    // Whether a resolver other than the default one with more conditions is
    // configured.
    let mut resolver_decides = r.get("resolver").is_some_and(|v| !v.is_null());
    if r.get("resolver")
        .is_some_and(|v| !v.is_null() && !matches!(v, ConfigVal::Unknown(_)))
    {
        let shape = global
            .entries
            .iter()
            .find(|e| e.role == InputRole::Resolver)
            .and_then(|e| match &e.target {
                Target::Path(p) => Some(p.clone()),
                Target::Module { .. } => None,
            })
            .and_then(|p| jsconfig::load(&p, LoadOptions::default()).exported_function)
            .and_then(|f| f.shape);
        match shape {
            Some(FunctionShape::DefaultResolverConditions(Some(requests))) => {
                condition_requests = requests;
                resolver_decides = false;
            }
            Some(FunctionShape::DefaultResolverConditions(None)) => r.report(
                "resolver",
                UncertaintyKind::ConditionalResolution,
                "custom resolver changes the export conditions of every request: packages with \
                 conditional exports may resolve differently"
                    .into(),
            ),
            _ => r.report(
                "resolver",
                UncertaintyKind::ConditionalResolution,
                "custom resolver: resolution may differ from the tracer's".into(),
            ),
        }
    }
    // Environment.
    let env = r.string("testEnvironment").unwrap_or_else(|| "node".into());
    let env_req = replace_root_dir(&env_request(&env), &root);
    if let Some(t) = module_target(&Item::Str(env_req), &root, &root) {
        global.entries.push(InputRef {
            role: InputRole::TestEnvironment,
            target: t,
        });
    }
    // The runner itself: the `jest` the project resolves, with its resolved
    // dependencies (the default `jest-circus` runner, `jest-runtime`, the
    // built-in node environment).
    global.entries.push(InputRef {
        role: InputRole::Runner,
        target: Target::Module {
            request: RUNNER_PACKAGE.into(),
            from_dir: root.clone(),
        },
    });
    // `next/jest` reads the Next.js project the config belongs to before it returns
    // the jest config: `next.config.*` (its `transpilePackages`,
    // `modularizeImports`, `compiler` and `experimental.swcPlugins` become
    // options of the SWC transform every test file goes through) and the env
    // files of test mode (they become `process.env` of the tests). The
    // files themselves are inputs of every test; what `next.config` imports is not
    // followed (an app's config reaches half the app through its rewrites and
    // flags, and only those four fields matter here).
    if uses_next_jest(loaded) {
        for name in NEXT_JEST_READS {
            let p = config_dir.join(name);
            if p.fx_is_file() {
                global.reads.push(p);
            } else {
                global.probes.push(p);
            }
        }
    }
    // Transforms: `{ regex: 'module' | ['module', options] }`.
    if let Some(tv) = r.get("transform") {
        for alt in tv.alternatives() {
            match alt {
                ConfigVal::Obj(o) => {
                    if let Some(why) = &o.open {
                        r.report(
                            "transform",
                            UncertaintyKind::ComputedSpecifier,
                            why.to_string(),
                        );
                    }
                    for (_, v) in &o.entries {
                        // Each alternative of an entry (a config that replaces a transformer
                        // under a condition) names one module: all of them are inputs.
                        let modules: Vec<ConfigVal> = v
                            .alternatives()
                            .into_iter()
                            .map(|a| match a {
                                ConfigVal::Arr(xs) => {
                                    xs.first().cloned().unwrap_or(ConfigVal::Null)
                                }
                                a => a.clone(),
                            })
                            .collect();
                        for it in modules.iter().flat_map(|m| r.items_of("transform", m)) {
                            if let Some(t) = module_target(&it, &root, &root) {
                                let e = InputRef {
                                    role: InputRole::Transform,
                                    target: t,
                                };
                                if !global.entries.contains(&e) {
                                    global.entries.push(e);
                                }
                            }
                        }
                    }
                }
                ConfigVal::Null => {}
                other => r.report_unknown("transform", other),
            }
        }
    }
    // Resolution.
    let mut resolution = ResolutionOverrides {
        condition_requests,
        ..ResolutionOverrides::default()
    };
    if let Some(mv) = r.get("moduleNameMapper") {
        for alt in mv.alternatives() {
            match alt {
                ConfigVal::Obj(o) => {
                    if let Some(why) = &o.open {
                        r.report(
                            "moduleNameMapper",
                            UncertaintyKind::ComputedSpecifier,
                            why.to_string(),
                        );
                    }
                    for (k, v) in &o.entries {
                        let reps: Vec<String> = r
                            .items_of("moduleNameMapper", v)
                            .into_iter()
                            .filter_map(|i| match i {
                                Item::Str(s) => Some(replace_root_dir(&s, &root)),
                                Item::Resolve { spec, from_dir } => {
                                    Some(if crate::common::is_path_request(&spec) {
                                        fsx::join(Path::new(&from_dir), &spec)
                                            .to_string_lossy()
                                            .into_owned()
                                    } else {
                                        spec
                                    })
                                }
                                Item::Regex { .. } => None,
                            })
                            .collect();
                        let re = JsRegex::new(k.clone(), "");
                        if !re.is_supported() {
                            r.report(
                                "moduleNameMapper",
                                UncertaintyKind::UnsupportedSyntax,
                                format!("regex not supported by the matcher: {k}"),
                            );
                        }
                        if resolution.aliases.iter().any(
                            |a| matches!(&a.matcher, AliasMatcher::Regex(x) if x.source() == k),
                        ) {
                            continue;
                        }
                        resolution.aliases.push(Alias {
                            matcher: AliasMatcher::Regex(re),
                            replacements: reps,
                            source: "moduleNameMapper",
                        });
                    }
                }
                ConfigVal::Null => {}
                other => r.report_unknown("moduleNameMapper", other),
            }
        }
    }
    let paths = |r: &mut Reader<'_>, field: &str| -> Option<Vec<PathBuf>> {
        let items = r.items(field)?;
        Some(
            items
                .into_iter()
                .filter_map(|i| match i {
                    Item::Str(s) => Some(fsx::join(&root, &replace_root_dir(&s, &root))),
                    _ => None,
                })
                .collect(),
        )
    };
    resolution.roots = paths(r, "roots").unwrap_or_else(|| vec![root.clone()]);
    // How the config writes each directory it searches: below `<rootDir>` or
    // relative to it without `..` is the same directory of the tree wherever
    // the checkout is.
    let repo = fsx::base().unwrap_or_else(|| PathBuf::from("/"));
    let roots_written: Vec<bool> = match r.items("roots") {
        Some(items) => items
            .into_iter()
            .filter_map(|i| match i {
                Item::Str(s) => Some(crate::location::stays_inside(&repo, &root, &s)),
                _ => None,
            })
            .collect(),
        None => vec![true],
    };
    resolution.module_paths = paths(r, "modulePaths").unwrap_or_default();
    resolution.module_directories = match r.items("moduleDirectories") {
        Some(items) => items
            .into_iter()
            .filter_map(|i| match i {
                Item::Str(s) => Some(replace_root_dir(&s, &root)),
                _ => None,
            })
            .collect(),
        None => vec!["node_modules".into()],
    };
    // Each expression with how the config writes it ([`PatternSpec`]).
    let regexes = |r: &mut Reader<'_>, field: &str| -> Option<Vec<(JsRegex, PatternSpec)>> {
        let items = r.items(field)?;
        let root_s = root.to_string_lossy().into_owned();
        let mut out = Vec::new();
        for i in items {
            let (re, spec) = match i {
                Item::Str(s) => (
                    JsRegex::new(s.replace("<rootDir>", &root_s), ""),
                    PatternSpec::jest_regex(&s),
                ),
                Item::Regex { source, flags } => {
                    let re = JsRegex::new(source, flags);
                    let spec = PatternSpec::plain(&TestPattern::Regex(re.clone()));
                    (re, spec)
                }
                Item::Resolve { .. } => continue,
            };
            if !re.is_supported() {
                r.report(
                    field,
                    UncertaintyKind::UnsupportedSyntax,
                    format!("regex not supported by the matcher: {}", re.source()),
                );
            }
            out.push((re, spec));
        }
        Some(out)
    };
    let module_path_ignore = regexes(r, "modulePathIgnorePatterns").unwrap_or_default();
    resolution.module_path_ignore_patterns = module_path_ignore
        .iter()
        .map(|(re, _)| re.clone())
        .collect();
    resolution.extensions = match r.items("moduleFileExtensions") {
        Some(items) => items
            .into_iter()
            .filter_map(|i| match i {
                Item::Str(s) => Some(s.trim_start_matches('.').to_string()),
                _ => None,
            })
            .collect(),
        None => DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
    };
    // Discovery.
    let mut discovery = TestDiscovery {
        roots: resolution.roots.clone(),
        extensions: resolution.extensions.clone(),
        absolute: true,
        ..TestDiscovery::default()
    };
    // The patterns as the config writes them, in the order of `discovery`'s.
    let (mut include_written, mut exclude_written) = (Vec::new(), Vec::new());
    let test_regex = regexes(r, "testRegex").unwrap_or_default();
    if !test_regex.is_empty() {
        for (re, spec) in test_regex {
            discovery.include.push(TestPattern::Regex(re));
            include_written.push(spec);
        }
        if r.get("testMatch").is_some_and(|v| !v.is_null()) {
            r.report(
                "testMatch",
                UncertaintyKind::ConditionalResolution,
                "both testMatch and testRegex are set (jest rejects this); using testRegex".into(),
            );
        }
    } else {
        let globs: Vec<String> = match r.items("testMatch") {
            Some(items) => items
                .into_iter()
                .filter_map(|i| match i {
                    Item::Str(s) => Some(s),
                    _ => None,
                })
                .collect(),
            None => DEFAULT_TEST_MATCH.iter().map(|s| s.to_string()).collect(),
        };
        for g in globs {
            let (neg, g) = match g.strip_prefix('!') {
                Some(rest) => (true, rest.to_string()),
                None => (false, g),
            };
            // jest escapes the glob characters of `rootDir` before it substitutes it into a
            // `testMatch` pattern (`escapeGlobCharacters`), so a root like `app/(site)` is
            // matched literally.
            let written = PatternSpec::jest_glob(&g);
            let g = crate::location::jest_glob(&g, &root);
            if crate::pattern::glob_to_regex(&g).1 {
                r.report(
                    "testMatch",
                    UncertaintyKind::PatternTooBroad,
                    format!("negated extglob approximated by '*': {g}"),
                );
            }
            if neg {
                discovery.exclude.push(TestPattern::Glob(g));
                exclude_written.push(written);
            } else {
                discovery.include.push(TestPattern::Glob(g));
                include_written.push(written);
            }
        }
    }
    let ignore = regexes(r, "testPathIgnorePatterns").unwrap_or_else(|| {
        let re = JsRegex::new("/node_modules/", "");
        let spec = PatternSpec::plain(&TestPattern::Regex(re.clone()));
        vec![(re, spec)]
    });
    // The haste map ignores modulePathIgnorePatterns: such files are never tests
    // (and no mocks or modules: `unmapped`).
    let first_unmapped = discovery.exclude.len() + ignore.len();
    let unmapped: Vec<usize> =
        (first_unmapped..first_unmapped + module_path_ignore.len()).collect();
    for (re, spec) in ignore.into_iter().chain(module_path_ignore) {
        discovery.exclude.push(TestPattern::Regex(re));
        exclude_written.push(spec);
    }
    // jest matches all of these against the absolute path of a file: the discovery
    // as the config writes it, for whoever keeps the project to check whether
    // the place of the checkout decides something ([`crate::location`]).
    // (`<rootDir>` in the root directory of a project that a config lists inline is
    // the listing config's directory, which is `config_dir` here.)
    let root_dir_plain = match r.string("rootDir") {
        Some(s) => crate::location::stays_inside(&repo, config_dir, &s.replace("<cwd>/", "")),
        None => config_dir.starts_with(&repo),
    };
    let sources = global.config_files.clone();
    let spec_of = |d: &TestDiscovery,
                   include: Vec<PatternSpec>,
                   exclude: Vec<PatternSpec>,
                   unmapped: Vec<usize>| {
        let repo_s = repo.to_string_lossy().into_owned();
        // A pattern with the checkout's path in it: the config's own text when one of
        // its sources spells that path, else computed from where the config is.
        let named = repo_s.len() > 1
            && include
                .iter()
                .chain(&exclude)
                .any(|p| !p.rooted && p.text.contains(repo_s.as_str()));
        let spells_root = named
            && sources
                .iter()
                .any(|f| fsx::read_to_string(f).is_ok_and(|src| src.contains(repo_s.as_str())));
        let roots = d
            .roots
            .iter()
            .enumerate()
            .map(|(i, dir)| (dir.clone(), roots_written.get(i).copied().unwrap_or(true)))
            .collect();
        crate::location::shared(DiscoverySpec::of(Written {
            repo: &repo,
            root_dir: &root,
            root_dir_plain,
            roots,
            include,
            exclude,
            unmapped,
            extensions: d.extensions.clone(),
            absolute: true,
            spells_root,
        }))
    };
    discovery.spec = spec_of(&discovery, include_written, exclude_written, unmapped);
    // A discovery that is not kept exactly: something of it has the checkout's path
    // in its text, or steps out of the config's directory. Whether the config
    // computed that from its own location (`__dirname`) is what a load at the
    // real place cannot say, and what a load as if the repository were at the
    // neutral root says: there the config's location is text that no config
    // spells ([`DiscoverySpec::proved`]). For the config of the file itself (a
    // project that a config lists inline is a part of its value).
    if let (Some(spec), Some(file), true) = (discovery.spec.as_deref(), config_file, whole)
        && !spec.exact
        && repo.parent().is_some()
    {
        let neutral = Path::new(crate::location::NEUTRAL_ROOTS[0]);
        let cfg = jsconfig::as_if(&repo, neutral, || {
            let seen = jsconfig::load(file, load_opts(file));
            let mut cfg = seen
                .value
                .unwrap_or_else(|| ConfigVal::Obj(ConfigObj::default()));
            if let Some(f) = &preset_file
                && let Some(pv) = jsconfig::load(f, LoadOptions::default()).value
            {
                cfg = merge_preset(&cfg, &pv);
            }
            cfg
        });
        let mut seen = Reader::new(cfg, file.to_path_buf(), &[]);
        let dir_there = match config_dir.strip_prefix(&repo) {
            Ok(rest) if rest.as_os_str().is_empty() => neutral.to_path_buf(),
            Ok(rest) => neutral.join(rest),
            Err(_) => config_dir.to_path_buf(),
        };
        let root_there = root_dir_of(&mut seen, &dir_there);
        let f = discovery_fields(&mut seen, &root_there);
        if let Some(proved) = DiscoverySpec::proved(
            &repo,
            &root_there,
            &f.roots,
            &f.include,
            &f.exclude,
            f.unmapped,
            f.extensions,
            &discovery,
        ) {
            discovery.spec = crate::location::shared(proved);
        }
    }
    // Mocks.
    let automock = r.bool("automock").unwrap_or(false);
    // Whether a module a test replaces with a mock stays unloaded
    // ([`crate::mocks`]).
    let esm = r.get("extensionsToTreatAsEsm").is_some_and(|v| {
        !matches!(&v, ConfigVal::Null) && !matches!(&v, ConfigVal::Arr(xs) if xs.is_empty())
    });
    // Anything but an absent or false value counts (a value the evaluator could not
    // read).
    let set = |field: &str| {
        r.get(field)
            .is_some_and(|v| !matches!(&v, ConfigVal::Null | ConfigVal::Bool(false)))
    };
    // jest's own runner by its name is the default.
    let own_runner = r.get("runner").is_some_and(
        |v| matches!(&v, ConfigVal::Str(turborepo_trace_core::eval::Value::Str(s)) if s == "jest-runner"),
    );
    let unreliable = if automock || set("automock") {
        Some("automock: every module is automocked from its source".to_owned())
    } else if set("runner") && !own_runner {
        Some("a custom runner may load modules its own way".to_owned())
    } else if esm {
        Some(
            "extensionsToTreatAsEsm: jest.mock does not replace static imports of ES modules"
                .to_owned(),
        )
    } else if resolver_decides {
        Some("a custom resolver decides which file a request names".to_owned())
    } else {
        crate::mocks::transform_keeps_mocks(r.get("transform").as_ref(), &root)
    };
    let mut mocks = MockPolicy {
        root_mock_dirs: resolution.roots.clone(),
        implicit_package_mocks: true,
        automock,
        extensions: resolution.extensions.clone(),
        haste_mocks: Vec::new(),
        unreliable,
    };
    // The haste map's manual mocks: of the files below the roots, the ones below a
    // `__mocks__` directory, and the walk asks for no other (a file that appears
    // anywhere else is no change for it).
    for root_d in &resolution.roots {
        // (The haste map goes by the whole path: below a root that is itself inside a
        // `__mocks__` directory every file is one.)
        let inside = root_d.components().any(|c| c.as_os_str() == "__mocks__");
        let wanted = if inside { "**" } else { "**/__mocks__/**" };
        fsx::walk_files_matching(root_d, wanted, &mut |p| {
            let s = p.to_string_lossy();
            let Some(idx) = s.find("/__mocks__/") else {
                return;
            };
            if resolution
                .module_path_ignore_patterns
                .iter()
                .any(|re| re.is_match(&s))
            {
                return;
            }
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !mocks.extensions.iter().any(|e| e == ext) {
                return;
            }
            let rel = &s[idx + "/__mocks__/".len()..];
            let name = rel
                .strip_suffix(&format!(".{ext}"))
                .unwrap_or(rel)
                .to_string();
            if !mocks.haste_mocks.iter().any(|(k, _)| *k == name) {
                mocks.haste_mocks.push((name, p.to_path_buf()));
            }
        });
        global.memberships.push(MembershipSpec {
            dir: root_d.clone(),
            pattern: "**/__mocks__/**".into(),
            reason: "manual mocks (jest haste map)".into(),
        });
    }
    mocks.haste_mocks.sort();
    // Never silently zero tests: a config whose discovery can't be read, or matches
    // nothing although it may not have been read exactly, falls back to jest's
    // default `testMatch` under `rootDir`, with an uncertainty. A discovery
    // that was read exactly and matches no file is believed: jest finds no test
    // either (a config for tests that do not exist yet), and inventing tests
    // from the default patterns would turn another runner's files
    // under the same directory into jest tests.
    let exact = !discovery.include.is_empty()
        && matches!(&r.cfg, ConfigVal::Obj(o) if o.open.is_none())
        && loaded.uncertainties.is_empty()
        && extra_unc.is_empty()
        && !r.unc.iter().any(|u| {
            u.field
                .as_deref()
                .is_some_and(|f| f.split(", ").any(|f| DISCOVERY_FIELDS.contains(&f)))
        })
        && discovery
            .include
            .iter()
            .chain(&discovery.exclude)
            .all(|p| match p {
                TestPattern::Glob(g) => crate::pattern::glob_is_exact(g),
                TestPattern::Regex(re) => re.is_supported(),
            });
    if !exact {
        // Which of the two discoveries it is depends on the files that are there:
        // whether the configured one matches any, and whether the default one
        // does. That is a question about every file below the roots, and its
        // answer matters for few of them: a file the configured discovery
        // matches is a test whatever else is there (it is itself a file that is
        // matched), and one that neither matches is none. So the walks are a
        // part of the load of their own ([`fsx::part`]), which the
        // discovery names for whoever keeps the load ([`DiscoveryChoice`]).
        let mut fallback = fallback_discovery(&discovery, &root);
        fallback.spec = spec_of(
            &fallback,
            DEFAULT_TEST_MATCH
                .iter()
                .map(|g| PatternSpec::jest_glob(g))
                .collect(),
            vec![PatternSpec::jest_regex("/node_modules/")],
            Vec::new(),
        );
        // The default discovery searches the directories of the configured one below
        // the same root directory: where those are proved to be the same
        // directories of the tree at every place, so are the default one's.
        if discovery.spec.as_deref().is_some_and(|s| s.exact)
            && let Some(mut spec) = fallback.spec.as_deref().cloned()
            && !spec.exact
        {
            spec.exact = true;
            spec.inexact.clear();
            fallback.spec = crate::location::shared(spec);
        }
        let part = format!("discovery {}", root.display());
        let configured_can_match = !discovery.roots.is_empty() && !discovery.include.is_empty();
        let falls_back = fsx::part(&part, || {
            !finds_a_test(&discovery) && finds_a_test(&fallback)
        });
        if falls_back {
            let (kind, why) = if discovery.include.is_empty() {
                (
                    UncertaintyKind::ComputedSpecifier,
                    "testMatch/testRegex not statically known",
                )
            } else {
                (
                    UncertaintyKind::ConditionalResolution,
                    "testMatch/testRegex/roots match no file",
                )
            };
            r.report(
                "testMatch",
                kind,
                format!("{why}; using jest's default testMatch under rootDir"),
            );
            let configured = std::mem::replace(&mut discovery, fallback);
            discovery.choice = Some(Box::new(DiscoveryChoice {
                other: configured,
                fell_back: true,
                configured_can_match,
                part,
            }));
        } else {
            discovery.choice = Some(Box::new(DiscoveryChoice {
                other: fallback,
                fell_back: false,
                configured_can_match,
                part,
            }));
        }
    }
    if automock {
        r.report(
            "automock",
            UncertaintyKind::UnsupportedSyntax,
            "automock: every imported module is automocked; adjacent __mocks__ apply to the whole \
             closure"
                .into(),
        );
    }
    // Snapshots.
    let snapshots = match r.get("snapshotResolver") {
        Some(v) if !v.is_null() => SnapshotPolicy::Custom {
            resolver: global
                .entries
                .iter()
                .find(|e| e.role == InputRole::SnapshotResolver)
                .map(|e| e.target.clone()),
        },
        _ => SnapshotPolicy::Default,
    };
    // A second config next to this one would change (or break) the setup.
    if let Some(cf) = config_file
        && let Some(dir) = cf.parent()
    {
        for n in CONFIG_NAMES {
            let p = dir.join(n);
            if !p.fx_exists() {
                global.probes.push(p);
            }
        }
    }
    // Name.
    let name = match r.get("displayName") {
        Some(ConfigVal::Str(v)) if v.is_known() => v.as_str().unwrap_or("").to_string(),
        Some(o @ ConfigVal::Obj(_)) => o
            .field("name")
            .and_then(|n| n.as_str().map(str::to_string))
            .unwrap_or_else(|| root.to_string_lossy().into_owned()),
        _ => root.to_string_lossy().into_owned(),
    };
    let mut unc = extra_unc;
    unc.extend(loaded.uncertainties.iter().cloned());
    unc.append(&mut r.unc);
    // Paths the config only hands to the fields read here
    // ([`crate::inputs::ConfigValues`]).
    if let Some(cf) = config_file {
        global.values = config_values(
            &r.cfg,
            MODELLED_FIELDS,
            loaded,
            cf,
            Vec::new(),
            REPORTER_FIELDS,
        );
    }
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
