//! The pytest adapter.
//!
//! - **rootdir and config file** (pytest's rules): from the test's directory
//!   upwards, the first directory holding `pytest.ini`, `.pytest.ini` (always a
//!   match), `pyproject.toml` with `[tool.pytest.ini_options]` (or pytest 9's
//!   `[tool.pytest]`), `tox.ini` with `[pytest]` or `setup.cfg` with
//!   `[tool:pytest]`, in that order within a directory. Without a match, the
//!   nearest `pyproject.toml` (pytest ≥ 8.1), then the nearest `setup.py`
//!   directory, else the test's directory. `confcutdir` defaults to the config
//!   file's directory (else the rootdir).
//! - **config inputs**: the config file; `testpaths`, `python_files`,
//!   `norecursedirs`, `pythonpath`, `confcutdir`, and in `addopts` the static
//!   `-p NAME` plugins (`-p no:NAME` disables), `--import-mode`,
//!   `--confcutdir`, `--ignore`, `--ignore-glob`, `--rootdir`.
//! - **discovery**: `testpaths` (globs allowed) or the rootdir, `python_files`
//!   patterns matched on file names, `norecursedirs` and `--ignore` excluded.
//! - **per test** ([`Framework::test_inputs`]): every existing `conftest.py`
//!   from `confcutdir` down to the test's directory (setup files) and a probe
//!   for each missing one; the package `__init__.py` files above the test
//!   (imported with it) and a probe for the first missing one (adding it
//!   changes the test's module name and import root); modules named in
//!   `pytest_plugins` of those conftests and of the test; syrupy snapshots
//!   (`__snapshots__/<test stem>*`).
//! - **`--import-mode`** ([`ImportMode`]): `prepend` (default) inserts each
//!   test's import root (the first directory upwards without `__init__.py`) at
//!   the front of `sys.path`, `append` at the end, `importlib` changes nothing.
//!   [`test_import_roots`] returns them; `pythonpath` entries are project-wide
//!   [`ResolutionOverrides::module_paths`].
//!
//! Roles: conftests are [`InputRole::Conftest`], plugins [`InputRole::Plugin`],
//! package `__init__.py` files [`InputRole::ImportedByTest`]. Each test's
//! import roots are [`TestInputs::import_roots`]; the project named in the
//! rootdir's `pyproject.toml` (`[project]` or `[tool.poetry]` `name`) is the
//! [`crate::PackageRef`] (`pypi`).
//!
//! Files are read through [`crate::fsx`] (the shared dir cache when one is
//! active).

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;

pub use crate::iniconf::{IniValue, ini_section, toml_table};
use crate::{
    Framework,
    common::ModuleResolver,
    fsx,
    fsx::PathFsx,
    inputs::{
        FrameworkInputs, FrameworkUncertainty, GlobalInputs, ImportRoots, InputRef, InputRole,
        MembershipSpec, PackageRef, Project, ResolutionOverrides, Target, TestDiscovery,
        TestInputs, TestPattern,
    },
};

/// Config file names in pytest's lookup order within one directory.
pub const CONFIG_NAMES: &[&str] = &[
    "pytest.ini",
    ".pytest.ini",
    "pyproject.toml",
    "tox.ini",
    "setup.cfg",
];

/// Default `python_files`.
pub const DEFAULT_PYTHON_FILES: &[&str] = &["test_*.py", "*_test.py"];

/// Default `norecursedirs`.
pub const DEFAULT_NORECURSEDIRS: &[&str] = &[
    "*.egg",
    ".*",
    "_darcs",
    "build",
    "CVS",
    "dist",
    "node_modules",
    "venv",
    "{arch}",
];

/// How pytest imports test modules and conftests.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// Insert each module's import root at `sys.path[0]` (default).
    #[default]
    Prepend,
    /// Append it to `sys.path`.
    Append,
    /// Import with `importlib`, without changing `sys.path`.
    Importlib,
}

/// A parsed pytest configuration.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct PytestConfig {
    /// The rootdir.
    pub rootdir: PathBuf,
    /// The config file, if any (a `setup.py` only fixes the rootdir).
    pub config_file: Option<PathBuf>,
    /// Directory above which `conftest.py` files are not loaded.
    pub confcutdir: PathBuf,
    /// `testpaths` (relative to the rootdir, may be globs).
    pub testpaths: Vec<String>,
    /// `python_files` patterns.
    pub python_files: Vec<String>,
    /// `norecursedirs` patterns.
    pub norecursedirs: Vec<String>,
    /// `pythonpath` entries (absolute).
    pub pythonpath: Vec<PathBuf>,
    /// `--import-mode`.
    pub import_mode: ImportMode,
    /// `-p NAME` plugins (module names).
    pub plugins: Vec<String>,
    /// `-p no:NAME`.
    pub disabled_plugins: Vec<String>,
    /// `--ignore` paths (absolute).
    pub ignore: Vec<PathBuf>,
    /// `--ignore-glob` patterns.
    pub ignore_glob: Vec<String>,
    /// Values that could not be read.
    pub uncertainties: Vec<FrameworkUncertainty>,
}

/// The pytest adapter.
#[derive(Copy, Clone, Debug, Default)]
pub struct Pytest;

impl Framework for Pytest {
    fn id(&self) -> &'static str {
        "pytest"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        CONFIG_NAMES
    }

    fn is_config_file(&self, path: &Path) -> bool {
        is_pytest_config(path)
    }

    fn discover(&self, root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fsx::walk_dirs(root, &mut |d| {
            if let Some(c) = CONFIG_NAMES
                .iter()
                .map(|n| d.join(n))
                .find(|p| p.fx_is_file() && is_pytest_config(p))
            {
                out.push(c);
            }
        });
        out
    }

    fn config_for(&self, test: &Path) -> Option<PathBuf> {
        let (rootdir, config) = rootdir_for(test);
        config.or_else(|| {
            let setup = rootdir.join("setup.py");
            setup.fx_is_file().then_some(setup)
        })
    }

    fn load(&self, config: &Path) -> FrameworkInputs {
        let cfg = load_config(config);
        inputs_of(&cfg)
    }

    fn test_inputs(
        &self,
        project: &Project,
        test: &Path,
        src: &[u8],
        _resolver: &dyn ModuleResolver,
    ) -> TestInputs {
        let cfg = match &project.config_file {
            Some(c) => load_config(c),
            None => defaults(&project.root_dir, None),
        };
        test_inputs_of(&cfg, test, src)
    }
}

// ---- Rootdir and config discovery ----

/// Whether `path` is a pytest config file (by name and, for shared files, by
/// section).
pub fn is_pytest_config(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match name {
        "pytest.ini" | ".pytest.ini" => true,
        "pyproject.toml" | "tox.ini" | "setup.cfg" => fsx::read_to_string(path)
            .map(|s| section_of(name, &s).is_some())
            .unwrap_or(false),
        _ => false,
    }
}

/// The pytest section of a config file's text, if present.
fn section_of(name: &str, text: &str) -> Option<Vec<(String, IniValue)>> {
    match name {
        "pyproject.toml" => {
            toml_table(text, "tool.pytest.ini_options").or_else(|| toml_table(text, "tool.pytest"))
        }
        "tox.ini" | "pytest.ini" | ".pytest.ini" => {
            ini_section(text, "pytest").or_else(|| (name != "tox.ini").then(Vec::new))
        }
        "setup.cfg" => ini_section(text, "tool:pytest"),
        _ => None,
    }
}

/// pytest's rootdir and config file for a run on `test` (a file or directory).
pub fn rootdir_for(test: &Path) -> (PathBuf, Option<PathBuf>) {
    let start = if test.fx_is_dir() {
        test.to_path_buf()
    } else {
        test.parent().unwrap_or(Path::new("/")).to_path_buf()
    };
    for dir in start.ancestors() {
        for n in CONFIG_NAMES {
            let p = dir.join(n);
            if p.fx_is_file() && is_pytest_config(&p) {
                return (dir.to_path_buf(), Some(p));
            }
        }
    }
    // pytest >= 8.1: a pyproject.toml without the table still defines the rootdir.
    for dir in start.ancestors() {
        let p = dir.join("pyproject.toml");
        if p.fx_is_file() {
            return (dir.to_path_buf(), Some(p));
        }
    }
    for dir in start.ancestors() {
        if dir.join("setup.py").fx_is_file() {
            return (dir.to_path_buf(), None);
        }
    }
    (start, None)
}

/// The default configuration for `rootdir`.
fn defaults(rootdir: &Path, config: Option<&Path>) -> PytestConfig {
    PytestConfig {
        rootdir: rootdir.to_path_buf(),
        config_file: config.map(Path::to_path_buf),
        confcutdir: config
            .and_then(Path::parent)
            .unwrap_or(rootdir)
            .to_path_buf(),
        python_files: DEFAULT_PYTHON_FILES
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        norecursedirs: DEFAULT_NORECURSEDIRS
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        ..PytestConfig::default()
    }
}

/// Loads a config file (`pytest.ini`, `.pytest.ini`, `pyproject.toml`,
/// `tox.ini`, `setup.cfg`), or a `setup.py` / directory as a config-less
/// rootdir.
pub fn load_config(path: &Path) -> PytestConfig {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if path.fx_is_dir() {
        return defaults(path, None);
    }
    let dir = path.parent().unwrap_or(Path::new("/"));
    if name == "setup.py" {
        return defaults(dir, None);
    }
    let mut cfg = defaults(dir, Some(path));
    let text = match fsx::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            cfg.uncertainties.push(uncertainty(
                path,
                0,
                None,
                UncertaintyKind::ParseIncomplete,
                format!("cannot read config: {e}"),
            ));
            return cfg;
        }
    };
    let Some(section) = section_of(name, &text) else {
        return cfg;
    };
    let mut addopts: Vec<String> = Vec::new();
    for (key, value) in section {
        let list = value.list();
        match key.as_str() {
            "testpaths" => cfg.testpaths = list,
            "python_files" => cfg.python_files = list,
            "norecursedirs" => cfg.norecursedirs = list,
            "pythonpath" => {
                cfg.pythonpath = list.iter().map(|p| fsx::join(&cfg.rootdir, p)).collect();
            }
            "confcutdir" => {
                if let Some(c) = list.first() {
                    cfg.confcutdir = fsx::join(&cfg.rootdir, c);
                }
            }
            "addopts" => addopts = value.args(),
            "required_plugins" => {}
            _ => {}
        }
    }
    apply_addopts(&mut cfg, &addopts, path, &text);
    cfg
}

fn apply_addopts(cfg: &mut PytestConfig, args: &[String], path: &Path, text: &str) {
    let line = fsx::line_of_field(text.as_bytes(), "addopts");
    let mut i = 0;
    let next = |i: &mut usize| -> Option<String> {
        *i += 1;
        args.get(*i).cloned()
    };
    while i < args.len() {
        let a = args[i].as_str();
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_owned())),
            _ => (a, None),
        };
        match flag {
            "-p" => {
                if let Some(v) = next(&mut i) {
                    plugin(cfg, &v);
                }
            }
            _ if flag.starts_with("-p") && flag.len() > 2 => plugin(cfg, &flag[2..]),
            "--import-mode" => {
                let v = inline.or_else(|| next(&mut i)).unwrap_or_default();
                cfg.import_mode = match v.as_str() {
                    "append" => ImportMode::Append,
                    "importlib" => ImportMode::Importlib,
                    "prepend" => ImportMode::Prepend,
                    other => {
                        cfg.uncertainties.push(uncertainty(
                            path,
                            line,
                            Some("addopts"),
                            UncertaintyKind::UnsupportedSyntax,
                            format!("unknown --import-mode {other:?}"),
                        ));
                        ImportMode::Prepend
                    }
                };
            }
            "--confcutdir" => {
                if let Some(v) = inline.or_else(|| next(&mut i)) {
                    cfg.confcutdir = fsx::join(&cfg.rootdir, &v);
                }
            }
            "--ignore" => {
                if let Some(v) = inline.or_else(|| next(&mut i)) {
                    cfg.ignore.push(fsx::join(&cfg.rootdir, &v));
                }
            }
            "--ignore-glob" => {
                if let Some(v) = inline.or_else(|| next(&mut i)) {
                    cfg.ignore_glob.push(v);
                }
            }
            "--rootdir" => {
                if let Some(v) = inline.or_else(|| next(&mut i)) {
                    if v.contains('$') {
                        cfg.uncertainties.push(uncertainty(
                            path,
                            line,
                            Some("addopts"),
                            UncertaintyKind::EnvDependent,
                            format!("--rootdir {v} depends on the environment"),
                        ));
                    } else {
                        cfg.rootdir = fsx::join(&cfg.rootdir, &v);
                    }
                }
            }
            "-c" | "--config-file" | "-o" | "--override-ini" => {
                cfg.uncertainties.push(uncertainty(
                    path,
                    line,
                    Some("addopts"),
                    UncertaintyKind::UnsupportedSyntax,
                    format!("{flag} in addopts is not modelled"),
                ));
                if inline.is_none() {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

fn plugin(cfg: &mut PytestConfig, name: &str) {
    match name.strip_prefix("no:") {
        Some(n) => cfg.disabled_plugins.push(n.to_owned()),
        None => cfg.plugins.push(name.to_owned()),
    }
}

fn uncertainty(
    file: &Path,
    line: u32,
    field: Option<&str>,
    kind: UncertaintyKind,
    detail: String,
) -> FrameworkUncertainty {
    FrameworkUncertainty {
        kind,
        file: file.to_path_buf(),
        line,
        field: field.map(str::to_owned),
        detail,
    }
}

// ---- Inputs ----

/// The [`FrameworkInputs`] of a configuration.
pub fn inputs_of(cfg: &PytestConfig) -> FrameworkInputs {
    let root = &cfg.rootdir;
    let mut global = GlobalInputs::default();
    if let Some(c) = &cfg.config_file {
        global.config_files.push(c.clone());
        // A config file of higher precedence created next to this one would replace it.
        let name = c.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let dir = c.parent().unwrap_or(root);
        for n in CONFIG_NAMES.iter().take_while(|n| **n != name) {
            let p = dir.join(n);
            if !p.fx_exists() {
                global.probes.push(p);
            }
        }
    }
    global.package = project_package(root);
    for p in &cfg.plugins {
        global.entries.push(InputRef {
            role: InputRole::Plugin,
            target: Target::Module {
                request: p.clone(),
                from_dir: root.clone(),
            },
        });
    }
    let roots: Vec<PathBuf> = if cfg.testpaths.is_empty() {
        vec![root.clone()]
    } else {
        let mut v = Vec::new();
        for t in &cfg.testpaths {
            let abs = fsx::join(root, t);
            if crate::pattern::is_glob(t) {
                // (Python's `glob` follows links.)
                v.extend(fsx::expand_glob(
                    &abs.to_string_lossy(),
                    true,
                    fsx::Links::Follow,
                ));
            } else {
                v.push(abs);
            }
        }
        v
    };
    let mut exclude: Vec<TestPattern> = cfg
        .norecursedirs
        .iter()
        .map(|d| TestPattern::Glob(format!("**/{d}/**")))
        .collect();
    for i in &cfg.ignore {
        if let Ok(rel) = i.strip_prefix(root) {
            let r = rel.to_string_lossy();
            exclude.push(TestPattern::Glob(r.to_string()));
            exclude.push(TestPattern::Glob(format!("{r}/**")));
        }
    }
    exclude.extend(cfg.ignore_glob.iter().map(|g| TestPattern::Glob(g.clone())));
    let project = Project {
        name: root.display().to_string(),
        config_file: cfg.config_file.clone(),
        root_dir: root.clone(),
        global,
        resolution: ResolutionOverrides {
            module_paths: cfg.pythonpath.clone(),
            roots: roots.clone(),
            extensions: vec!["py".into()],
            ..ResolutionOverrides::default()
        },
        discovery: TestDiscovery {
            roots,
            include: cfg
                .python_files
                .iter()
                .map(|p| TestPattern::Glob(format!("**/{p}")))
                .collect(),
            exclude,
            glob_base: Some(root.clone()),
            extensions: vec!["py".into()],
            absolute: false,
            choice: None,
            spec: None,
        },
        uncertainties: cfg.uncertainties.clone(),
        ..Project::default()
    };
    FrameworkInputs {
        framework: "pytest",
        config_file: cfg.config_file.clone(),
        projects: vec![project],
        uncertainties: Vec::new(),
    }
}

/// The directories pytest adds to `sys.path` for `test`: the test's import root
/// (the first directory upwards without `__init__.py`) in front (`prepend`) or
/// at the end (`append`), plus `pythonpath` in front. Returns `(front, back)`.
pub fn test_import_roots(cfg: &PytestConfig, test: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut front = cfg.pythonpath.clone();
    let mut back = Vec::new();
    let basedir = import_root(test);
    match cfg.import_mode {
        ImportMode::Prepend => front.insert(0, basedir),
        ImportMode::Append => back.push(basedir),
        ImportMode::Importlib => {}
    }
    (front, back)
}

/// The Python project rooted at `dir`: `name` and `version` of `[project]` (PEP
/// 621) or `[tool.poetry]` in `dir/pyproject.toml`, canonicalized (PEP 503).
pub fn project_package(dir: &Path) -> Option<PackageRef> {
    let text = fsx::read_to_string(dir.join("pyproject.toml")).ok()?;
    let mut section = "";
    let (mut name, mut version) = (None, None);
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            section = t;
            continue;
        }
        if section != "[project]" && section != "[tool.poetry]" {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        match k.trim() {
            "name" if name.is_none() => name = Some(v.to_owned()),
            "version" if version.is_none() => version = Some(v.to_owned()),
            _ => {}
        }
    }
    Some(PackageRef {
        ecosystem: "pypi",
        name: turborepo_trace_lockfile::requirements::canonicalize_name(&name?),
        version,
        root: dir.to_path_buf(),
    })
}

fn import_root(test: &Path) -> PathBuf {
    let mut dir = test.parent().unwrap_or(Path::new("/"));
    while dir.join("__init__.py").fx_is_file() {
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    dir.to_path_buf()
}

/// Per-test inputs (see the module docs).
pub fn test_inputs_of(cfg: &PytestConfig, test: &Path, src: &[u8]) -> TestInputs {
    let (front, back) = test_import_roots(cfg, test);
    let mut out = TestInputs {
        test: test.to_path_buf(),
        package: project_package(&cfg.rootdir),
        import_roots: ImportRoots { front, back },
        ..TestInputs::default()
    };
    let test_dir = test.parent().unwrap_or(Path::new("/"));
    // conftest.py from confcutdir down to the test's directory.
    let mut dirs: Vec<&Path> = Vec::new();
    for d in test_dir.ancestors() {
        dirs.push(d);
        if d == cfg.confcutdir || !d.starts_with(&cfg.confcutdir) {
            break;
        }
    }
    dirs.reverse();
    let mut plugin_sources: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for d in dirs {
        let c = d.join("conftest.py");
        if c.fx_is_file() {
            out.entries.push(InputRef {
                role: InputRole::Conftest,
                target: Target::Path(c.clone()),
            });
            if let Ok(s) = fsx::read(&c) {
                plugin_sources.push((c, s));
            }
        } else {
            out.probes.push(c);
        }
    }
    // Package __init__.py files above the test; the first missing one is a probe.
    let mut d = test_dir;
    loop {
        let init = d.join("__init__.py");
        if init.fx_is_file() {
            out.entries.push(InputRef {
                role: InputRole::ImportedByTest,
                target: Target::Path(init),
            });
            match d.parent() {
                Some(p) => d = p,
                None => break,
            }
        } else {
            out.probes.push(init);
            break;
        }
    }
    plugin_sources.push((test.to_path_buf(), src.to_vec()));
    for (file, s) in &plugin_sources {
        let (plugins, unknown) = pytest_plugins(s);
        for p in plugins {
            out.entries.push(InputRef {
                role: InputRole::Plugin,
                target: Target::Module {
                    request: p,
                    from_dir: cfg.rootdir.clone(),
                },
            });
        }
        if let Some(line) = unknown {
            out.uncertainties.push(uncertainty(
                file,
                line,
                Some("pytest_plugins"),
                UncertaintyKind::ComputedSpecifier,
                "pytest_plugins is not a literal string or list of strings".into(),
            ));
        }
    }
    // syrupy snapshots: __snapshots__/<stem>.ambr or __snapshots__/<stem>/...
    let snaps = test_dir.join("__snapshots__");
    if snaps.fx_is_dir()
        && let Some(stem) = test.file_stem().and_then(|s| s.to_str())
    {
        out.memberships.push(MembershipSpec {
            dir: snaps,
            pattern: format!("{stem}*"),
            reason: "syrupy snapshots".into(),
        });
    }
    out
}

/// Module names in a top-level `pytest_plugins = ...` assignment: a string or a
/// list/tuple of strings. Returns the names and, if the value is not literal,
/// its 1-based line.
pub fn pytest_plugins(src: &[u8]) -> (Vec<String>, Option<u32>) {
    let text = String::from_utf8_lossy(src);
    let mut out = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let Some(rest) = line.strip_prefix("pytest_plugins") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        if rest.starts_with('=') {
            continue;
        }
        // The value may continue over following lines until the brackets close.
        let start = text
            .lines()
            .take(lineno)
            .map(|l| l.len() + 1)
            .sum::<usize>()
            + (line.len() - rest.len());
        let value = &text[start.min(text.len())..];
        let mut depth = 0i32;
        let mut chars = value.chars().peekable();
        let mut literal = true;
        let mut cur = String::new();
        let mut quote: Option<char> = None;
        while let Some(c) = chars.next() {
            if let Some(q) = quote {
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                    }
                } else if c == q {
                    out.push(std::mem::take(&mut cur));
                    quote = None;
                } else {
                    cur.push(c);
                }
                continue;
            }
            match c {
                '"' | '\'' => quote = Some(c),
                '[' | '(' => depth += 1,
                ']' | ')' => {
                    depth -= 1;
                    if depth <= 0 {
                        break;
                    }
                }
                '\n' if depth == 0 => break,
                '#' => {
                    while let Some(&n) = chars.peek() {
                        if n == '\n' {
                            break;
                        }
                        chars.next();
                    }
                }
                ',' | ' ' | '\t' | '\r' | '\n' => {}
                _ => literal = false,
            }
        }
        let unknown = (!literal).then_some(lineno as u32 + 1);
        return (out, unknown);
    }
    (out, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iniconf::shell_split;

    #[test]
    fn ini_and_toml_readers() {
        let ini = "[metadata]\nname = x\n[tool:pytest]\ntestpaths =\n    tests\n    \
                   integration\naddopts = -p myplug --import-mode=append\n; comment\n";
        let s = ini_section(ini, "tool:pytest").unwrap();
        assert_eq!(s[0].0, "testpaths");
        assert_eq!(s[0].1.list(), ["tests", "integration"]);
        assert_eq!(s[1].1.args(), ["-p", "myplug", "--import-mode=append"]);
        assert!(ini_section(ini, "pytest").is_none());
        let toml = "[project]\nname='x'\n[tool.pytest.ini_options]\ntestpaths = [\n  \"tests\", # \
                    c\n  'more',\n]\naddopts = \"-p 'a b' -q\"\nminversion = 6.0\n[tool.other]\nx \
                    = 1\n";
        let t = toml_table(toml, "tool.pytest.ini_options").unwrap();
        assert_eq!(t[0].1, IniValue::List(vec!["tests".into(), "more".into()]));
        assert_eq!(t[1].1.args(), ["-p", "a b", "-q"]);
        assert_eq!(t[2].1, IniValue::Str("6.0".into()));
        assert!(toml_table(toml, "tool.pytest").is_none());
    }

    #[test]
    fn plugins_in_source() {
        let (p, u) = pytest_plugins(b"import x\npytest_plugins = [\n  'a.b',  # c\n  \"c\",\n]\n");
        assert_eq!(p, ["a.b", "c"]);
        assert_eq!(u, None);
        let (p, _) = pytest_plugins(b"pytest_plugins = 'single'\n");
        assert_eq!(p, ["single"]);
        let (_, u) = pytest_plugins(b"\npytest_plugins = make()\n");
        assert_eq!(u, Some(2));
    }

    #[test]
    fn shell_words() {
        assert_eq!(
            shell_split(r#"-p "x y" a\ b 'c'"#),
            ["-p", "x y", "a b", "c"]
        );
    }
}
