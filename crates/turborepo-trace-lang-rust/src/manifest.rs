//! `Cargo.toml` parsing: package, targets (with cargo's auto-discovery),
//! dependencies, features and the `[workspace]` table, without running `cargo
//! metadata`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use toml::{Table, Value};
use turborepo_trace_core::{
    DirCache,
    fs::{EntryKind, normalize},
};

/// Which dependency table a dependency comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DepKind {
    /// `[dependencies]`.
    Normal,
    /// `[dev-dependencies]`.
    Dev,
    /// `[build-dependencies]`.
    Build,
}

/// One declared dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dep {
    /// The key it is declared under (the crate name used in code, before `-` →
    /// `_`).
    pub key: String,
    /// The real package name (`package = "…"` renames).
    pub package: String,
    /// Table.
    pub kind: DepKind,
    /// `[target.<key>.…]` key, if platform-specific.
    pub target: Option<String>,
    /// `path = "…"`, as written (relative to the declaring manifest, or to the
    /// workspace root for inherited dependencies; see [`Dep::path_base`]).
    pub path: Option<String>,
    /// Directory `path` is relative to.
    pub path_base: PathBuf,
    /// `git = "…"`.
    pub git: Option<String>,
    /// `optional = true`.
    pub optional: bool,
    /// `default-features` (default true).
    pub default_features: bool,
    /// `features = […]`.
    pub features: Vec<String>,
}

/// One declared or discovered target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetDecl {
    /// `name`.
    pub name: Option<String>,
    /// `path`.
    pub path: Option<String>,
    /// `test = false` disables the libtest harness for unit tests.
    pub test: Option<bool>,
    /// `proc-macro = true` (lib only).
    pub proc_macro: bool,
    /// `required-features`.
    pub required_features: Vec<String>,
}

/// The kind of a crate target.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TargetKind {
    /// The library.
    Lib,
    /// A binary.
    Bin,
    /// An integration test.
    Test,
    /// A benchmark.
    Bench,
    /// An example.
    Example,
    /// The build script.
    Build,
}

impl TargetKind {
    /// Lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            TargetKind::Lib => "lib",
            TargetKind::Bin => "bin",
            TargetKind::Test => "test",
            TargetKind::Bench => "bench",
            TargetKind::Example => "example",
            TargetKind::Build => "build-script",
        }
    }

    /// Whether dev-dependencies are linked.
    pub const fn uses_dev_deps(self) -> bool {
        matches!(
            self,
            TargetKind::Test | TargetKind::Bench | TargetKind::Example
        )
    }
}

/// A resolved crate target: a crate root file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrateTarget {
    /// Kind.
    pub kind: TargetKind,
    /// Target name.
    pub name: String,
    /// The crate root file (absolute, normalized).
    pub root: PathBuf,
    /// Whether `cargo test` builds a test harness for it.
    pub tested: bool,
    /// Features it requires.
    pub required_features: Vec<String>,
    /// Whether it is a proc-macro library.
    pub proc_macro: bool,
}

/// `[package] build`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildDecl {
    /// Not set: `build.rs` if it exists.
    Auto,
    /// An explicit path.
    Path(String),
    /// `build = false`.
    Disabled,
}

/// `[workspace]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceDecl {
    /// `members` globs.
    pub members: Vec<String>,
    /// `exclude` paths.
    pub exclude: Vec<String>,
    /// `[workspace.dependencies]`, raw.
    pub dependencies: BTreeMap<String, Value>,
}

/// A parsed `Cargo.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// The directory containing it.
    pub dir: PathBuf,
    /// `[package] name`.
    pub name: Option<String>,
    /// `[package] version` (`None` when inherited or missing).
    pub version: Option<String>,
    /// `[package] workspace` (explicit workspace root, relative).
    pub workspace_path: Option<String>,
    /// `[package] build`.
    pub build: BuildDecl,
    /// `autobins` etc.
    pub autobins: bool,
    /// `autotests`.
    pub autotests: bool,
    /// `autobenches`.
    pub autobenches: bool,
    /// `autoexamples`.
    pub autoexamples: bool,
    /// `[lib]`.
    pub lib: Option<TargetDecl>,
    /// `[[bin]]`.
    pub bins: Vec<TargetDecl>,
    /// `[[test]]`.
    pub tests: Vec<TargetDecl>,
    /// `[[bench]]`.
    pub benches: Vec<TargetDecl>,
    /// `[[example]]`.
    pub examples: Vec<TargetDecl>,
    /// All dependencies (workspace inheritance not yet applied; see
    /// [`Manifest::inherit`]).
    pub deps: Vec<Dep>,
    /// Dependency keys declared with `workspace = true`, by index into `deps`.
    pub inherited: Vec<usize>,
    /// `[features]`.
    pub features: BTreeMap<String, Vec<String>>,
    /// `[workspace]`.
    pub workspace: Option<WorkspaceDecl>,
}

/// Why a manifest could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError(pub String);

impl Manifest {
    /// Parses the manifest text of the `Cargo.toml` in `dir`.
    pub fn parse(dir: &Path, text: &str) -> Result<Manifest, ManifestError> {
        let t: Table = toml::from_str(text).map_err(|e| ManifestError(e.to_string()))?;
        let pkg = t
            .get("package")
            .or_else(|| t.get("project"))
            .and_then(Value::as_table);
        let s = |tbl: Option<&Table>, k: &str| -> Option<String> {
            tbl?.get(k)?.as_str().map(str::to_owned)
        };
        let b = |k: &str| -> bool {
            pkg.and_then(|p| p.get(k))
                .and_then(Value::as_bool)
                .unwrap_or(true)
        };
        let build = match pkg.and_then(|p| p.get("build")) {
            Some(Value::String(p)) => BuildDecl::Path(p.clone()),
            Some(Value::Boolean(false)) => BuildDecl::Disabled,
            _ => BuildDecl::Auto,
        };
        let mut m = Manifest {
            dir: dir.to_path_buf(),
            name: s(pkg, "name"),
            version: s(pkg, "version"),
            workspace_path: s(pkg, "workspace"),
            build,
            autobins: b("autobins"),
            autotests: b("autotests"),
            autobenches: b("autobenches"),
            autoexamples: b("autoexamples"),
            lib: t.get("lib").and_then(Value::as_table).map(target_decl),
            bins: target_list(&t, "bin"),
            tests: target_list(&t, "test"),
            benches: target_list(&t, "bench"),
            examples: target_list(&t, "example"),
            deps: Vec::new(),
            inherited: Vec::new(),
            features: BTreeMap::new(),
            workspace: None,
        };
        for (table, kind) in DEP_TABLES {
            if let Some(d) = t.get(*table).and_then(Value::as_table) {
                m.add_deps(d, *kind, None);
            }
        }
        if let Some(targets) = t.get("target").and_then(Value::as_table) {
            for (key, v) in targets {
                let Some(v) = v.as_table() else { continue };
                for (table, kind) in DEP_TABLES {
                    if let Some(d) = v.get(*table).and_then(Value::as_table) {
                        m.add_deps(d, *kind, Some(key.clone()));
                    }
                }
            }
        }
        if let Some(f) = t.get("features").and_then(Value::as_table) {
            for (k, v) in f {
                let items = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                m.features.insert(k.clone(), items);
            }
        }
        if let Some(w) = t.get("workspace").and_then(Value::as_table) {
            let list = |k: &str| -> Vec<String> {
                w.get(k)
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            m.workspace = Some(WorkspaceDecl {
                members: list("members"),
                exclude: list("exclude"),
                dependencies: w
                    .get("dependencies")
                    .and_then(Value::as_table)
                    .map(|d| d.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default(),
            });
        }
        Ok(m)
    }

    fn add_deps(&mut self, table: &Table, kind: DepKind, target: Option<String>) {
        for (key, v) in table {
            let (dep, inherited) = dep_from(key, v, kind, target.clone(), &self.dir);
            if inherited {
                self.inherited.push(self.deps.len());
            }
            self.deps.push(dep);
        }
    }

    /// Applies `workspace = true` inheritance from the workspace root manifest
    /// `ws`: path, git, package, default-features and version come from
    /// `[workspace.dependencies]`; features are the union; `optional` stays the
    /// member's.
    pub fn inherit(&mut self, ws: &Manifest) {
        let Some(decl) = &ws.workspace else { return };
        for &i in &self.inherited {
            let Some(d) = self.deps.get_mut(i) else {
                continue;
            };
            let Some(v) = decl.dependencies.get(&d.key) else {
                continue;
            };
            let (base, _) = dep_from(&d.key, v, d.kind, d.target.clone(), &ws.dir);
            d.package = base.package;
            d.path = base.path;
            d.path_base = ws.dir.clone();
            d.git = base.git;
            d.default_features = base.default_features;
            let mut f = base.features;
            for x in std::mem::take(&mut d.features) {
                if !f.contains(&x) {
                    f.push(x);
                }
            }
            d.features = f;
        }
        self.inherited.clear();
    }

    /// The package name, or the directory name.
    pub fn package_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
    }

    /// Whether this manifest declares a package (not only a virtual workspace).
    pub fn is_package(&self) -> bool {
        self.name.is_some()
    }

    /// The build script path, if any (probing `build.rs` through `fs`).
    pub fn build_script(&self, fs: &DirCache) -> Option<PathBuf> {
        match &self.build {
            BuildDecl::Disabled => None,
            BuildDecl::Path(p) => Some(normalize(&self.dir.join(p))),
            BuildDecl::Auto => {
                let p = self.dir.join("build.rs");
                fs.is_file(&p).then_some(p)
            }
        }
    }

    /// Every target with its crate root, following cargo's target
    /// auto-discovery: `src/lib.rs`, `src/main.rs`, `src/bin/*.rs`,
    /// `src/bin/*/main.rs`, and `tests/`, `benches/`, `examples/` with
    /// `*.rs` and `*/main.rs`, plus explicit `[lib]`, `[[bin]]`, `[[test]]`,
    /// `[[bench]]`, `[[example]]` tables, and the build script. Existence
    /// checks go through `fs` (so missing candidates are recorded as
    /// probes).
    pub fn targets(&self, fs: &DirCache) -> Vec<CrateTarget> {
        let mut out = Vec::new();
        if !self.is_package() {
            return out;
        }
        let pkg = self.package_name();
        let crate_name = pkg.replace('-', "_");
        let dir = &self.dir;
        let file = |rel: &str| -> Option<PathBuf> {
            let p = dir.join(rel);
            fs.is_file(&p).then_some(p)
        };
        // lib
        let lib_path = match self.lib.as_ref().and_then(|l| l.path.as_deref()) {
            Some(p) => Some(normalize(&dir.join(p))),
            None => file("src/lib.rs"),
        };
        if let Some(root) = lib_path {
            let decl = self.lib.clone().unwrap_or_else(empty_decl);
            out.push(CrateTarget {
                kind: TargetKind::Lib,
                name: decl
                    .name
                    .clone()
                    .unwrap_or_else(|| crate_name.clone())
                    .replace('-', "_"),
                root,
                tested: decl.test.unwrap_or(true),
                required_features: Vec::new(),
                proc_macro: decl.proc_macro,
            });
        }
        // bins
        let mut bins = Vec::new();
        for d in &self.bins {
            let name = d.name.clone().unwrap_or_else(|| pkg.clone());
            let root = match &d.path {
                Some(p) => Some(normalize(&dir.join(p))),
                None => file(&format!("src/bin/{name}.rs"))
                    .or_else(|| file(&format!("src/bin/{name}/main.rs")))
                    .or_else(|| (name == pkg).then(|| file("src/main.rs")).flatten()),
            };
            if let Some(root) = root {
                bins.push((name, root, d.clone()));
            }
        }
        if self.autobins {
            let mut auto = Vec::new();
            if let Some(m) = file("src/main.rs") {
                auto.push((pkg.clone(), m));
            }
            auto.extend(discover(fs, &dir.join("src/bin")));
            for (name, root) in auto {
                if !bins.iter().any(|(n, r, _)| *n == name || *r == root) {
                    bins.push((name, root, empty_decl()));
                }
            }
        }
        for (name, root, d) in bins {
            out.push(CrateTarget {
                kind: TargetKind::Bin,
                name,
                root,
                tested: d.test.unwrap_or(true),
                required_features: d.required_features,
                proc_macro: false,
            });
        }
        for (kind, decls, auto, sub) in [
            (
                TargetKind::Example,
                &self.examples,
                self.autoexamples,
                "examples",
            ),
            (TargetKind::Test, &self.tests, self.autotests, "tests"),
            (
                TargetKind::Bench,
                &self.benches,
                self.autobenches,
                "benches",
            ),
        ] {
            let mut list: Vec<(String, PathBuf, TargetDecl)> = Vec::new();
            for d in decls {
                let Some(name) = d.name.clone() else { continue };
                let root = match &d.path {
                    Some(p) => Some(normalize(&dir.join(p))),
                    None => file(&format!("{sub}/{name}.rs"))
                        .or_else(|| file(&format!("{sub}/{name}/main.rs"))),
                };
                if let Some(root) = root {
                    list.push((name, root, d.clone()));
                }
            }
            if auto {
                for (name, root) in discover(fs, &dir.join(sub)) {
                    if !list.iter().any(|(n, r, _)| *n == name || *r == root) {
                        list.push((name, root, empty_decl()));
                    }
                }
            }
            for (name, root, d) in list {
                out.push(CrateTarget {
                    kind,
                    name,
                    root,
                    tested: d.test.unwrap_or(kind == TargetKind::Test),
                    required_features: d.required_features,
                    proc_macro: false,
                });
            }
        }
        if let Some(root) = self.build_script(fs) {
            out.push(CrateTarget {
                kind: TargetKind::Build,
                name: "build-script-build".into(),
                root,
                tested: false,
                required_features: Vec::new(),
                proc_macro: false,
            });
        }
        out
    }
}

const DEP_TABLES: &[(&str, DepKind)] = &[
    ("dependencies", DepKind::Normal),
    ("dev-dependencies", DepKind::Dev),
    ("dev_dependencies", DepKind::Dev),
    ("build-dependencies", DepKind::Build),
    ("build_dependencies", DepKind::Build),
];

fn empty_decl() -> TargetDecl {
    TargetDecl {
        name: None,
        path: None,
        test: None,
        proc_macro: false,
        required_features: Vec::new(),
    }
}

fn target_decl(t: &Table) -> TargetDecl {
    TargetDecl {
        name: t.get("name").and_then(Value::as_str).map(str::to_owned),
        path: t.get("path").and_then(Value::as_str).map(str::to_owned),
        test: t.get("test").and_then(Value::as_bool),
        proc_macro: t
            .get("proc-macro")
            .or_else(|| t.get("proc_macro"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        required_features: t
            .get("required-features")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn target_list(t: &Table, key: &str) -> Vec<TargetDecl> {
    t.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_table)
                .map(target_decl)
                .collect()
        })
        .unwrap_or_default()
}

/// A dependency entry; returns it and whether it says `workspace = true`.
fn dep_from(
    key: &str,
    v: &Value,
    kind: DepKind,
    target: Option<String>,
    dir: &Path,
) -> (Dep, bool) {
    let mut d = Dep {
        key: key.to_owned(),
        package: key.to_owned(),
        kind,
        target,
        path: None,
        path_base: dir.to_path_buf(),
        git: None,
        optional: false,
        default_features: true,
        features: Vec::new(),
    };
    let mut inherited = false;
    if let Some(t) = v.as_table() {
        let s = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_owned);
        if let Some(p) = s("package") {
            d.package = p;
        }
        d.path = s("path");
        d.git = s("git");
        d.optional = t.get("optional").and_then(Value::as_bool).unwrap_or(false);
        d.default_features = t
            .get("default-features")
            .or_else(|| t.get("default_features"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        d.features = t
            .get("features")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        inherited = t.get("workspace").and_then(Value::as_bool) == Some(true);
    }
    (d, inherited)
}

/// `<dir>/*.rs` and `<dir>/*/main.rs`, as `(name, root)`, sorted by name.
fn discover(fs: &DirCache, dir: &Path) -> Vec<(String, PathBuf)> {
    let Some(listing) = fs.list(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in listing.iter() {
        let name = String::from_utf8_lossy(e.name_bytes()).into_owned();
        let p = dir.join(e.name());
        match e.kind() {
            EntryKind::File | EntryKind::Symlink if name.ends_with(".rs") => {
                if fs.is_file(&p) {
                    out.push((name.trim_end_matches(".rs").to_owned(), p));
                }
            }
            EntryKind::Dir | EntryKind::Symlink => {
                let m = p.join("main.rs");
                if fs.is_file(&m) {
                    out.push((name, m));
                }
            }
            _ => {}
        }
    }
    out.sort();
    out
}

/// Whether `rel` (a member path relative to the workspace root) matches the
/// `members` glob `pat` (`*`, `?` and `[…]`-free globs per segment, as cargo's
/// `glob` uses).
pub fn member_glob_matches(pat: &str, rel: &str) -> bool {
    let p: Vec<&str> = pat
        .trim_end_matches('/')
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let r: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    p.len() == r.len()
        && p.iter()
            .zip(&r)
            .all(|(a, b)| seg_matches(a.as_bytes(), b.as_bytes()))
}

fn seg_matches(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => seg_matches(&p[1..], s) || (!s.is_empty() && seg_matches(p, &s[1..])),
        (Some(b'?'), Some(_)) => seg_matches(&p[1..], &s[1..]),
        (Some(a), Some(b)) if a == b => seg_matches(&p[1..], &s[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_packages_deps_and_workspaces() {
        let m = Manifest::parse(
            Path::new("/w/a"),
            r#"
[package]
name = "a-b"
version = "0.1.0"
build = "gen.rs"
autotests = false

[lib]
path = "lib/mod.rs"

[[bin]]
name = "tool"
path = "src/tool.rs"

[dependencies]
serde = { version = "1", features = ["derive"], optional = true }
local = { path = "../local" }
renamed = { package = "real-name", version = "2", default-features = false }
shared = { workspace = true, features = ["x"] }

[target.'cfg(windows)'.dependencies]
winapi = "0.3"

[dev-dependencies]
insta = "1"

[features]
default = ["serde"]
"#,
        )
        .unwrap();
        assert_eq!(m.name.as_deref(), Some("a-b"));
        assert_eq!(m.build, BuildDecl::Path("gen.rs".into()));
        assert!(!m.autotests);
        assert_eq!(m.lib.as_ref().unwrap().path.as_deref(), Some("lib/mod.rs"));
        assert_eq!(m.bins[0].name.as_deref(), Some("tool"));
        let serde = m.deps.iter().find(|d| d.key == "serde").unwrap();
        assert!(serde.optional);
        assert_eq!(serde.features, ["derive"]);
        let r = m.deps.iter().find(|d| d.key == "renamed").unwrap();
        assert_eq!(r.package, "real-name");
        assert!(!r.default_features);
        let w = m.deps.iter().find(|d| d.key == "winapi").unwrap();
        assert_eq!(w.target.as_deref(), Some("cfg(windows)"));
        assert_eq!(
            m.deps.iter().find(|d| d.key == "insta").unwrap().kind,
            DepKind::Dev
        );
        assert_eq!(m.features["default"], ["serde"]);
        assert_eq!(m.inherited.len(), 1);

        let ws = Manifest::parse(
            Path::new("/w"),
            r#"
[workspace]
members = ["a", "crates/*"]
exclude = ["crates/skip"]
[workspace.dependencies]
shared = { path = "crates/shared", features = ["y"], default-features = false }
"#,
        )
        .unwrap();
        assert!(!ws.is_package());
        let mut m2 = m.clone();
        m2.inherit(&ws);
        let s = m2.deps.iter().find(|d| d.key == "shared").unwrap();
        assert_eq!(s.path.as_deref(), Some("crates/shared"));
        assert_eq!(s.path_base, Path::new("/w"));
        assert_eq!(s.features, ["y", "x"]);
        assert!(!s.default_features);
    }

    #[test]
    fn member_globs() {
        assert!(member_glob_matches("crates/*", "crates/core"));
        assert!(!member_glob_matches("crates/*", "crates/core/x"));
        assert!(member_glob_matches("a", "a"));
        assert!(member_glob_matches("./a/", "a"));
        assert!(member_glob_matches("lang-?s", "lang-js"));
    }

    #[test]
    fn bad_toml_is_an_error() {
        assert!(Manifest::parse(Path::new("/x"), "[package\nname=").is_err());
    }
}
