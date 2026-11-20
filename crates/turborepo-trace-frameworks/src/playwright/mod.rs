//! The Playwright adapter.
//!
//! - Configs: `playwright.config.{ts,js,mts,mjs,cts,cjs}` in the nearest
//!   ancestor of a spec.
//! - Evaluation (lang-js config evaluator): `testDir`, `testMatch`,
//!   `testIgnore`, `use.baseURL`, `globalSetup`, `globalTeardown`, and
//!   `projects[]` with their own `name`, `testDir`, `testMatch`, `testIgnore`
//!   and `use.baseURL`. A string `testMatch` without `/` matches at any depth
//!   (Playwright prepends `**/`).
//! - Tests: the spec's screenshot directory `<spec>-snapshots/` is a
//!   membership.
//! - Visited routes: [`visits`] (static extraction from specs, helpers and page
//!   objects) is what the selection reads. [`recorded`] (what a recorder saw
//!   the spec request) and [`selection`] (which specs a rebuilt app affects)
//!   are validation tooling: the oracle that the selection from source is
//!   checked against, not a way to select.

pub mod recorded;
pub mod selection;
pub mod visits;

use std::path::{Path, PathBuf};

use turborepo_trace_core::{UncertaintyKind, eval::Value};
use turborepo_trace_lang_js::extract::config::ConfigVal;

use crate::{
    Framework,
    common::{Item, ModuleResolver, Reader},
    fsx,
    inputs::{
        FrameworkInputs, FrameworkUncertainty, GlobalInputs, InputRef, InputRole, MembershipSpec,
        Project, TestDiscovery, TestInputs, TestPattern,
    },
    jsconfig::{self, LoadOptions},
    location::{DiscoverySpec, PatternSpec, Written},
    pattern::JsRegex,
};

/// Config file names.
pub const CONFIG_NAMES: &[&str] = &[
    "playwright.config.ts",
    "playwright.config.js",
    "playwright.config.mts",
    "playwright.config.mjs",
    "playwright.config.cts",
    "playwright.config.cjs",
];

/// Playwright's default `testMatch`.
pub const DEFAULT_TEST_MATCH: &str = "**/*.@(spec|test).?(c|m)[jt]s?(x)";

/// The Playwright adapter.
#[derive(Copy, Clone, Debug, Default)]
pub struct Playwright;

/// A project's base URL, by project name (Playwright-specific data not in
/// [`Project`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseUrls(pub Vec<(String, Option<String>)>);

impl Framework for Playwright {
    fn id(&self) -> &'static str {
        "playwright"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        CONFIG_NAMES
    }

    fn is_config_file(&self, path: &Path) -> bool {
        let n = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        CONFIG_NAMES.contains(&n)
    }

    fn discover(&self, root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fsx::walk_dirs(root, &mut |d| {
            if let Some(c) = fsx::first_existing(d, CONFIG_NAMES) {
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
        load_config(config).0
    }

    fn test_inputs(
        &self,
        _project: &Project,
        test: &Path,
        _src: &[u8],
        _resolver: &dyn ModuleResolver,
    ) -> TestInputs {
        let name = test.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let dir = test.parent().unwrap_or(Path::new("/"));
        TestInputs {
            test: test.to_path_buf(),
            memberships: vec![MembershipSpec {
                dir: dir.join(format!("{name}-snapshots")),
                pattern: "**".into(),
                reason: "screenshot snapshots".into(),
            }],
            ..TestInputs::default()
        }
    }
}

/// `v` with every string that is one of several known strings (a template with
/// a conditional: `` `*.${KIND === 'e2e' ? 'e2e' : 'spec'}.chromium.ts` ``) as
/// alternatives of those strings, so that each one is read as a pattern.
fn string_alternatives(v: &ConfigVal) -> ConfigVal {
    fn strings(v: &Value, out: &mut Vec<ConfigVal>) {
        match v {
            Value::Alternatives(vs) => vs.iter().for_each(|x| strings(x, out)),
            other => out.push(ConfigVal::Str(other.clone())),
        }
    }
    match v {
        ConfigVal::Str(s @ Value::Alternatives(_)) => {
            let mut out = Vec::new();
            strings(s, &mut out);
            ConfigVal::alts(out)
        }
        ConfigVal::Arr(xs) => ConfigVal::Arr(xs.iter().map(string_alternatives).collect()),
        ConfigVal::Alts(xs) => ConfigVal::alts(xs.iter().map(string_alternatives)),
        other => other.clone(),
    }
}

/// The patterns of `field` (`testMatch`, `testIgnore`): `None` when the config
/// does not set it. Alternatives (a value the environment decides) are united.
/// `Some(vec![])` with an uncertainty on `field` when nothing of the value
/// could be evaluated.
fn patterns(r: &mut Reader<'_>, field: &str, v: Option<ConfigVal>) -> Option<Vec<TestPattern>> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    let items = r.items_of(field, &string_alternatives(&v));
    Some(
        items
            .into_iter()
            .filter_map(|i| match i {
                // Playwright matches a string pattern against the absolute path of a
                // file, with `**/` before it unless it starts with that
                // (`createFileMatcher`).
                Item::Str(s) => Some(TestPattern::Glob(if s.starts_with("**/") {
                    s
                } else {
                    format!("**/{s}")
                })),
                Item::Regex { source, flags } => {
                    Some(TestPattern::Regex(JsRegex::new(source, flags)))
                }
                Item::Resolve { .. } => None,
            })
            .collect(),
    )
}

/// `testMatch` of the reader's object: its patterns, or `default` when it is
/// not set. A `testMatch` of which nothing could be evaluated (reported on the
/// field) must not leave the project without specs: `default` again, next to
/// the report.
fn test_match(r: &mut Reader<'_>, default: &[TestPattern]) -> Vec<TestPattern> {
    let v = r.get("testMatch");
    let unknown = v.as_ref().is_some_and(ConfigVal::has_unknown);
    match patterns(r, "testMatch", v) {
        Some(ps) if !(ps.is_empty() && unknown) => ps,
        _ => default.to_vec(),
    }
}

/// How many levels of helpers a Playwright config follows: the factory of a
/// shared package and what the factory itself imports.
const HELPER_DEPTH: u32 = 2;

/// Loads a config: the inputs and each project's `use.baseURL`.
///
/// A config made by a factory of a workspace package or a local module
/// (`defineConfig(__dirname)`) is evaluated through the factory
/// ([`LoadOptions::follow_factories`]); `defineConfig` of an installed package
/// is the identity. Paths the factory builds from its own `__dirname`
/// (`globalSetup`) are the factory's; `testDir` is relative to the config
/// file's directory, as in Playwright.
pub fn load_config(config: &Path) -> (FrameworkInputs, BaseUrls) {
    let loaded = jsconfig::load(
        config,
        LoadOptions {
            helper_depth: HELPER_DEPTH,
            follow_factories: true,
            ..LoadOptions::default()
        },
    );
    let dir = config.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut inputs = FrameworkInputs {
        framework: "playwright",
        config_file: Some(config.to_path_buf()),
        uncertainties: loaded.uncertainties.clone(),
        ..FrameworkInputs::default()
    };
    let mut base_urls = BaseUrls::default();
    let Some(cfg) = loaded.value.clone() else {
        inputs.uncertainties.push(FrameworkUncertainty {
            kind: UncertaintyKind::UnsupportedSyntax,
            file: config.to_path_buf(),
            line: 0,
            field: None,
            detail: "config exports nothing readable".into(),
        });
        return (inputs, base_urls);
    };
    // A value without fields is not a config: a factory that was not followed
    // (`defineConfig(__dirname)` of a package that is not installed is
    // `__dirname`). Every default applies then, which must not go unsaid.
    if !matches!(
        cfg,
        ConfigVal::Obj(_)
            | ConfigVal::Alts(_)
            | ConfigVal::Unknown(_)
            | ConfigVal::External { .. }
            | ConfigVal::Function(_)
    ) {
        inputs.uncertainties.push(FrameworkUncertainty {
            kind: UncertaintyKind::UnsupportedSyntax,
            file: config.to_path_buf(),
            line: 0,
            field: None,
            detail: "config does not evaluate to an object".into(),
        });
    }
    let mut r = Reader::new(cfg.clone(), config.to_path_buf(), &loaded.src);
    let mut global = GlobalInputs {
        config_files: loaded.files.clone(),
        js_entries: loaded.code_files.clone(),
        ..GlobalInputs::default()
    };
    for (field, role) in [
        ("globalSetup", InputRole::GlobalSetup),
        ("globalTeardown", InputRole::GlobalTeardown),
    ] {
        if let Some(items) = r.items(field) {
            for i in items {
                if let Some(t) = crate::common::module_target(&i, &dir, &dir) {
                    global.entries.push(InputRef { role, target: t });
                }
            }
        }
    }
    // A `testDir` written relative to the config without `..` is the same directory
    // of the tree wherever the checkout is.
    let plain = |s: &str| {
        let repo = fsx::base().unwrap_or_else(|| PathBuf::from("/"));
        crate::location::stays_inside(&repo, &dir, s)
    };
    let (test_dir, test_dir_plain) = match r.string("testDir") {
        Some(s) => (fsx::join(&dir, &s), plain(&s)),
        None => (dir.clone(), true),
    };
    let base_match = test_match(&mut r, &[TestPattern::Glob(DEFAULT_TEST_MATCH.into())]);
    let base_ignore = {
        let v = r.get("testIgnore");
        patterns(&mut r, "testIgnore", v)
    }
    .unwrap_or_default();
    let base_url = r
        .get("use.baseURL")
        .and_then(|v| v.as_str().map(String::from));
    let projects = match r.get("projects") {
        Some(ConfigVal::Arr(xs)) => xs,
        Some(v) if !v.is_null() => {
            r.report_unknown("projects", &v);
            Vec::new()
        }
        _ => Vec::new(),
    };
    // Playwright matches `testMatch` and `testIgnore` against the absolute path of
    // a file (expressions and globs alike), so the discovery is kept as the
    // config writes it for whoever keeps the project to check whether the place
    // of the checkout decides something ([`crate::location`]). Directories
    // called `node_modules` are skipped below `testDir` only.
    let mk = |name: String,
              (td, td_plain): (PathBuf, bool),
              inc: Vec<TestPattern>,
              exc: Vec<TestPattern>| {
        let repo = fsx::base().unwrap_or_else(|| PathBuf::from("/"));
        let repo_s = repo.to_string_lossy().into_owned();
        let written = |ps: &[TestPattern]| -> Vec<PatternSpec> {
            ps.iter().map(PatternSpec::plain).collect()
        };
        let named = repo_s.len() > 1
            && written(&inc)
                .iter()
                .chain(&written(&exc))
                .any(|p| p.text.contains(repo_s.as_str()));
        let spells_root = named
            && global
                .config_files
                .iter()
                .any(|f| fsx::read_to_string(f).is_ok_and(|src| src.contains(repo_s.as_str())));
        let spec = crate::location::shared(DiscoverySpec::of(Written {
            repo: &repo,
            root_dir: &td,
            root_dir_plain: td_plain,
            roots: vec![(td.clone(), td_plain)],
            include: written(&inc),
            exclude: written(&exc),
            unmapped: Vec::new(),
            extensions: Vec::new(),
            absolute: false,
            spells_root,
        }));
        Project {
            name,
            config_file: Some(config.to_path_buf()),
            root_dir: td.clone(),
            global: global.clone(),
            discovery: TestDiscovery {
                roots: vec![td],
                include: inc,
                exclude: exc,
                glob_base: None,
                extensions: Vec::new(),
                absolute: false,
                choice: None,
                spec,
            },
            ..Project::default()
        }
    };
    if projects.is_empty() {
        inputs.projects.push(mk(
            "default".into(),
            (test_dir.clone(), test_dir_plain),
            base_match.clone(),
            base_ignore.clone(),
        ));
        base_urls.0.push(("default".into(), base_url.clone()));
    }
    for (i, p) in projects.iter().enumerate() {
        let mut pr = Reader::new(p.clone(), config.to_path_buf(), &loaded.src);
        let name = pr.string("name").unwrap_or_else(|| format!("project-{i}"));
        let td = match pr.string("testDir") {
            Some(s) => (fsx::join(&dir, &s), plain(&s)),
            None => (test_dir.clone(), test_dir_plain),
        };
        let inc = test_match(&mut pr, &base_match);
        let exc = {
            let v = pr.get("testIgnore");
            patterns(&mut pr, "testIgnore", v)
        }
        .unwrap_or_else(|| base_ignore.clone());
        let bu = pr
            .get("use.baseURL")
            .and_then(|v| v.as_str().map(String::from))
            .or_else(|| base_url.clone());
        let mut proj = mk(name.clone(), td, inc, exc);
        proj.uncertainties = pr.unc;
        inputs.projects.push(proj);
        base_urls.0.push((name, bu));
    }
    inputs.uncertainties.extend(r.unc);
    (inputs, base_urls)
}

/// [`load_config`] and [`discover_specs`] for several configs, side by side, in
/// the order of `configs`.
pub fn load_configs(configs: &[PathBuf]) -> Vec<(FrameworkInputs, BaseUrls, Vec<PathBuf>)> {
    use rayon::prelude::*;
    configs
        .par_iter()
        .map(|c| {
            let (inputs, base_urls) = load_config(c);
            let specs = discover_specs(&inputs);
            (inputs, base_urls, specs)
        })
        .collect()
}

/// Every spec of the config's projects (deduplicated, sorted).
pub fn discover_specs(inputs: &FrameworkInputs) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = inputs
        .projects
        .iter()
        .flat_map(|p| p.discovery.enumerate())
        .collect();
    out.sort();
    out.dedup();
    out
}
