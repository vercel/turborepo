//! Python packaging (deployment phase): the files that ship in a project's
//! wheel or sdist, as a task `py-dist:<dir>` whose memberships are those files.
//!
//! | Configuration | Becomes |
//! |---|---|
//! | `MANIFEST.in` (`include`, `exclude`, `recursive-include`, `recursive-exclude`, `global-include`, `global-exclude`, `graft`, `prune`) | sdist memberships (with excludes, in order) |
//! | setuptools `packages` (lists, `find:`/`find_packages()` with `where`/`include`/`exclude`, auto-discovery of src and flat layouts), `package_dir`, `py_modules`, `package_data`, `include_package_data`, `data_files` (`setup.cfg`, `pyproject.toml` `[tool.setuptools]`, literal `setup.py` arguments) | wheel memberships and inputs; computed `setup.py` arguments are uncertain |
//! | hatch `[tool.hatch.build(.targets.wheel/.sdist)]` `packages`, `include`, `exclude`, `only-include`, `force-include` (default: the detected package for wheels, everything for sdists) | memberships |
//! | poetry `packages` (`include`, `from`), `include`, `exclude` | memberships |
//! | flit, pdm, maturin | the module / `includes` / `python-source` memberships |
//! | sdist metadata files (`README*`, `LICENSE*`, ...) | memberships |
//!
//! The project configs (`pyproject.toml`, `setup.cfg`, `setup.py`,
//! `MANIFEST.in`) are task configs, `setup.py` is also a Python entry (what it
//! imports counts), and `dist/` is the output. The build backend is a toolchain
//! input.

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::util::add_glob,
    registry::Recognizer,
    sources::{
        DiscoveredTask, SourceCx, TaskSource,
        taskfile::repo_rel,
        tox::{Ini, call_args, ini_get, is_kwarg, parse_ini, py_string, split_args},
    },
    spec::{EntryLang, TaskId, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&PyDist]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    Vec::new()
}

const WHEEL: &str = "wheel";
const SDIST: &str = "sdist";

/// Directories setuptools' flat-layout auto-discovery never treats as packages.
const FLAT_EXCLUDE: &[&str] = &[
    "tests",
    "test",
    "docs",
    "doc",
    "examples",
    "example",
    "scripts",
    "script",
    "tools",
    "util",
    "utils",
    "benchmarks",
    "bench",
    "build",
    "dist",
    "venv",
    "env",
    "site-packages",
    "node_modules",
    "tasks",
    "fabfile",
    "debian",
    "conftest",
    "setup",
];

/// A Python literal (as far as `setup.py` arguments go).
#[derive(Clone, Debug, PartialEq)]
pub enum Py {
    /// A string.
    Str(String),
    /// A list, tuple or set.
    List(Vec<Py>),
    /// A dict.
    Dict(Vec<(Py, Py)>),
    /// `True` / `False`.
    Bool(bool),
    /// `None`.
    None,
    /// A call: dotted name and arguments (keyword name, value).
    Call(String, Vec<(Option<String>, Py)>),
    /// A name (resolved against simple module-level assignments).
    Name(String),
    /// Anything else.
    Dyn(String),
}

/// Parses a Python expression into a [`Py`]. Never panics.
pub fn parse_py(s: &str) -> Py {
    parse_py_depth(s.trim(), 0)
}

fn parse_py_depth(s: &str, depth: usize) -> Py {
    if depth > 16 || s.is_empty() {
        return Py::Dyn(s.to_string());
    }
    // `a + b` at the top level (lists or strings).
    let parts = split_top_plus(s);
    if parts.len() > 1 {
        let vals: Vec<Py> = parts.iter().map(|p| parse_py_depth(p, depth + 1)).collect();
        if vals.iter().all(|v| matches!(v, Py::List(_))) {
            return Py::List(
                vals.into_iter()
                    .flat_map(|v| match v {
                        Py::List(l) => l,
                        _ => Vec::new(),
                    })
                    .collect(),
            );
        }
        if vals.iter().all(|v| matches!(v, Py::Str(_))) {
            return Py::Str(
                vals.into_iter()
                    .map(|v| match v {
                        Py::Str(s) => s,
                        _ => String::new(),
                    })
                    .collect(),
            );
        }
        return Py::Dyn(s.to_string());
    }
    // String literals, with implicit concatenation.
    if let Some((first, n)) = py_string(s) {
        let mut v = first;
        let mut rest = s[n..].trim_start();
        while !rest.is_empty() {
            match py_string(rest) {
                Some((x, m)) => {
                    v.push_str(&x);
                    rest = rest[m..].trim_start();
                }
                None => return Py::Dyn(s.to_string()),
            }
        }
        return Py::Str(v);
    }
    let first = s.as_bytes()[0];
    if matches!(first, b'[' | b'(' | b'{') {
        let close = match first {
            b'[' => ']',
            b'(' => ')',
            _ => '}',
        };
        let Some(inner) = call_args(&s[1..]) else {
            return Py::Dyn(s.to_string());
        };
        if s[1 + inner.len()..].trim() != close.to_string() {
            return Py::Dyn(s.to_string());
        }
        let items = split_args(inner);
        if first == b'{' && items.iter().any(|i| dict_colon(i).is_some()) {
            let mut d = Vec::new();
            for it in items {
                let Some(c) = dict_colon(it) else {
                    return Py::Dyn(s.to_string());
                };
                d.push((
                    parse_py_depth(&it[..c], depth + 1),
                    parse_py_depth(&it[c + 1..], depth + 1),
                ));
            }
            return Py::Dict(d);
        }
        if first == b'(' && items.len() == 1 && !inner.trim_end().ends_with(',') {
            return parse_py_depth(items[0], depth + 1);
        }
        return Py::List(
            items
                .into_iter()
                .map(|i| parse_py_depth(i, depth + 1))
                .collect(),
        );
    }
    match s {
        "True" => return Py::Bool(true),
        "False" => return Py::Bool(false),
        "None" => return Py::None,
        _ => {}
    }
    let name: String = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect();
    if !name.is_empty() && !name.as_bytes()[0].is_ascii_digit() {
        let rest = s[name.len()..].trim_start();
        if rest.is_empty() {
            return Py::Name(name);
        }
        if let Some(r) = rest.strip_prefix('(')
            && let Some(args) = call_args(r)
            && r[args.len()..].trim() == ")"
        {
            let mut out = Vec::new();
            for a in split_args(args) {
                if is_kwarg(a) {
                    let (k, v) = a.split_once('=').unwrap_or((a, ""));
                    out.push((Some(k.trim().to_string()), parse_py_depth(v, depth + 1)));
                } else {
                    out.push((None, parse_py_depth(a, depth + 1)));
                }
            }
            return Py::Call(name, out);
        }
    }
    Py::Dyn(s.to_string())
}

/// The `:` of a dict item (outside strings and brackets).
fn dict_colon(item: &str) -> Option<usize> {
    let b = item.as_bytes();
    let mut d = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => {
                if let Some((_, n)) = py_string(&item[i..]) {
                    i += n;
                    continue;
                }
                return None;
            }
            b'(' | b'[' | b'{' => d += 1,
            b')' | b']' | b'}' => d -= 1,
            b':' if d == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Splits on top-level `+`.
fn split_top_plus(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut d = 0i32;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => {
                if let Some((_, n)) = py_string(&s[i..]) {
                    i += n;
                    continue;
                }
                return vec![s];
            }
            b'(' | b'[' | b'{' => d += 1,
            b')' | b']' | b'}' => d -= 1,
            b'+' if d == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(s[start..].trim());
    out
}

/// Module-level `NAME = <expr>` assignments of a Python file (paren-balanced,
/// possibly spanning lines).
fn assignments(src: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        i += 1;
        if l.starts_with([' ', '\t', '#']) {
            continue;
        }
        let Some((n, v)) = l.split_once('=') else {
            continue;
        };
        let n = n.trim();
        if n.is_empty()
            || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || v.starts_with('=')
        {
            continue;
        }
        let mut v = v.trim().to_string();
        while crate::sources::tox::paren_depth(&v) > 0 && i < lines.len() {
            v.push('\n');
            v.push_str(lines[i]);
            i += 1;
        }
        out.push((n.to_string(), v));
    }
    out
}

/// The keyword arguments of the `setup(...)` call of a `setup.py`, with
/// module-level names resolved.
pub fn setup_kwargs(src: &str) -> Vec<(String, Py)> {
    let vars = assignments(src);
    let b = src.as_bytes();
    let mut pos = None;
    let mut from = 0;
    while let Some(i) = src[from..].find("setup(") {
        let at = from + i;
        if at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_') {
            pos = Some(at);
        }
        from = at + 6;
    }
    let Some(at) = pos else { return Vec::new() };
    let Some(args) = call_args(&src[at + 6..]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for a in split_args(args) {
        if !is_kwarg(a) {
            continue;
        }
        let (k, v) = a.split_once('=').unwrap_or((a, ""));
        let mut val = parse_py(v);
        for _ in 0..4 {
            match &val {
                Py::Name(n) => match vars.iter().rev().find(|(k, _)| k == n) {
                    Some((_, e)) => val = parse_py(e),
                    None => break,
                },
                _ => break,
            }
        }
        out.push((k.trim().to_string(), val));
    }
    out
}

/// Normalized import name of a project (`My-Pkg` → `my_pkg`).
fn import_name(project: &str) -> String {
    project.to_ascii_lowercase().replace(['-', '.'], "_")
}

/// Converts a gitignore-style pattern (hatch) to globs relative to the project
/// root.
fn gitignore_globs(p: &str) -> Vec<String> {
    let p = p.trim();
    let anchored = p.starts_with('/');
    let p = p.trim_start_matches('/');
    let dir_only = p.ends_with('/');
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        return vec!["**".into()];
    }
    let base = if anchored || p.contains('/') {
        p.to_string()
    } else {
        format!("**/{p}")
    };
    if dir_only {
        vec![format!("{base}/**")]
    } else {
        vec![base.clone(), format!("{base}/**")]
    }
}

/// Packages found under `where_dir` (directories with `__init__.py`; any
/// directory when `namespaces`), as (dotted name, dir), filtered by
/// include/exclude patterns.
fn find_packages(
    where_dir: &Path,
    include: &[String],
    exclude: &[String],
    namespaces: bool,
) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    fn walk(
        dir: &Path,
        prefix: &str,
        depth: usize,
        namespaces: bool,
        out: &mut Vec<(String, PathBuf)>,
    ) {
        if depth > 8 {
            return;
        }
        let Some(entries) = fsx::read_dir(dir) else {
            return;
        };
        for (p, is_dir) in entries {
            if !is_dir {
                continue;
            }
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if n.starts_with('.') || n == "__pycache__" || fsx::skip_dir(n) || n.contains('-') {
                continue;
            }
            let is_pkg = fsx::is_file(&p.join("__init__.py"));
            if !is_pkg && !namespaces {
                continue;
            }
            let name = if prefix.is_empty() {
                n.to_string()
            } else {
                format!("{prefix}.{n}")
            };
            out.push((name.clone(), p.clone()));
            walk(&p, &name, depth + 1, namespaces, out);
        }
    }
    walk(where_dir, "", 0, namespaces, &mut out);
    out.retain(|(n, _)| {
        (include.is_empty()
            || include
                .iter()
                .any(|g| crate::glob::matches(&g.replace('.', "/"), &n.replace('.', "/"))))
            && !exclude
                .iter()
                .any(|g| crate::glob::matches(&g.replace('.', "/"), &n.replace('.', "/")))
    });
    out.sort();
    out
}

/// `find_packages` arguments: `where`, `include`, `exclude`, namespaces.
type Find = (Vec<String>, Vec<String>, Vec<String>, bool);

/// Setuptools configuration gathered from `pyproject.toml`, `setup.cfg` and
/// `setup.py`.
#[derive(Default)]
struct Setuptools {
    packages: Option<Vec<String>>,
    find: Option<Find>,
    package_dir: Vec<(String, String)>,
    py_modules: Vec<String>,
    package_data: Vec<(String, Vec<String>)>,
    include_package_data: Option<bool>,
    data_files: Vec<String>,
    dynamic: Vec<String>,
}

fn py_strs(v: &Py) -> Option<Vec<String>> {
    match v {
        Py::List(l) => l
            .iter()
            .map(|x| match x {
                Py::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Py::Str(s) => Some(vec![s.clone()]),
        _ => None,
    }
}

fn toml_strs(v: Option<&toml::Value>) -> Vec<String> {
    match v {
        Some(toml::Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        Some(toml::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn cfg_list(v: &str) -> Vec<String> {
    v.split(['\n', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl Setuptools {
    fn read_pyproject(&mut self, t: &toml::Table) {
        let Some(st) = t.get("tool").and_then(|x| x.get("setuptools")) else {
            return;
        };
        match st.get("packages") {
            Some(toml::Value::Array(a)) => {
                self.packages = Some(
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect(),
                )
            }
            Some(toml::Value::Table(f)) => {
                let find = f.get("find").unwrap_or(&toml::Value::Boolean(true));
                self.find = Some((
                    toml_strs(find.get("where")),
                    toml_strs(find.get("include")),
                    toml_strs(find.get("exclude")),
                    find.get("namespaces")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                ));
            }
            _ => {}
        }
        if let Some(pd) = st.get("package-dir").and_then(|v| v.as_table()) {
            for (k, v) in pd {
                if let Some(s) = v.as_str() {
                    self.package_dir.push((k.clone(), s.to_string()));
                }
            }
        }
        self.py_modules.extend(toml_strs(st.get("py-modules")));
        if let Some(pd) = st.get("package-data").and_then(|v| v.as_table()) {
            for (k, v) in pd {
                self.package_data.push((k.clone(), toml_strs(Some(v))));
            }
        }
        if let Some(b) = st.get("include-package-data").and_then(|v| v.as_bool()) {
            self.include_package_data = Some(b);
        }
        if let Some(df) = st.get("data-files").and_then(|v| v.as_table()) {
            for (_, v) in df {
                self.data_files.extend(toml_strs(Some(v)));
            }
        }
    }

    fn read_cfg(&mut self, ini: &Ini) {
        if let Some(p) = ini_get(ini, "options", "packages") {
            let p = p.trim();
            if p.starts_with("find:") || p.starts_with("find_namespace:") {
                let w = ini_get(ini, "options.packages.find", "where")
                    .map(cfg_list)
                    .unwrap_or_default();
                let inc = ini_get(ini, "options.packages.find", "include")
                    .map(cfg_list)
                    .unwrap_or_default();
                let exc = ini_get(ini, "options.packages.find", "exclude")
                    .map(cfg_list)
                    .unwrap_or_default();
                self.find = Some((w, inc, exc, p.starts_with("find_namespace:")));
            } else {
                self.packages = Some(cfg_list(p));
            }
        }
        if let Some(pd) = ini_get(ini, "options", "package_dir") {
            for l in cfg_list(pd) {
                if let Some((k, v)) = l.split_once('=') {
                    self.package_dir
                        .push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
        if let Some(m) = ini_get(ini, "options", "py_modules") {
            self.py_modules.extend(cfg_list(m));
        }
        if let Some(b) = ini_get(ini, "options", "include_package_data") {
            self.include_package_data = Some(matches!(b.trim(), "True" | "true" | "1" | "yes"));
        }
        for (sec, kv) in ini {
            if sec == "options.package_data" {
                for (k, v) in kv {
                    self.package_data.push((
                        if k == "*" { String::new() } else { k.clone() },
                        cfg_list(v),
                    ));
                }
            }
            if sec == "options.data_files" {
                for (_, v) in kv {
                    self.data_files.extend(cfg_list(v));
                }
            }
        }
    }

    fn read_setup_py(&mut self, src: &str) {
        for (k, v) in setup_kwargs(src) {
            match k.as_str() {
                "packages" => match &v {
                    Py::Call(f, args)
                        if f.ends_with("find_packages")
                            || f.ends_with("find_namespace_packages") =>
                    {
                        let get = |name: &str, pos: usize| {
                            args.iter()
                                .find(|(k, _)| k.as_deref() == Some(name))
                                .or_else(|| args.iter().filter(|(k, _)| k.is_none()).nth(pos))
                                .map(|(_, v)| v.clone())
                        };
                        let w = get("where", 0)
                            .and_then(|v| py_strs(&v))
                            .unwrap_or_default();
                        let exc = get("exclude", 1)
                            .and_then(|v| py_strs(&v))
                            .unwrap_or_default();
                        let inc = get("include", 2)
                            .and_then(|v| py_strs(&v))
                            .unwrap_or_default();
                        self.find = Some((w, inc, exc, f.ends_with("find_namespace_packages")));
                    }
                    other => match py_strs(other) {
                        Some(l) => self.packages = Some(l),
                        None => self.dynamic.push(k.clone()),
                    },
                },
                "package_dir" => match &v {
                    Py::Dict(d) => {
                        for (a, b) in d {
                            if let (Py::Str(a), Py::Str(b)) = (a, b) {
                                self.package_dir.push((a.clone(), b.clone()));
                            } else {
                                self.dynamic.push(k.clone());
                            }
                        }
                    }
                    _ => self.dynamic.push(k.clone()),
                },
                "py_modules" => match py_strs(&v) {
                    Some(l) => self.py_modules.extend(l),
                    None => self.dynamic.push(k.clone()),
                },
                "package_data" => match &v {
                    Py::Dict(d) => {
                        for (a, b) in d {
                            match (a, py_strs(b)) {
                                (Py::Str(a), Some(globs)) => self.package_data.push((
                                    if a == "*" { String::new() } else { a.clone() },
                                    globs,
                                )),
                                _ => self.dynamic.push(k.clone()),
                            }
                        }
                    }
                    _ => self.dynamic.push(k.clone()),
                },
                "include_package_data" => match v {
                    Py::Bool(b) => self.include_package_data = Some(b),
                    _ => self.dynamic.push(k.clone()),
                },
                "data_files" => match &v {
                    Py::List(items) => {
                        for it in items {
                            match it {
                                Py::List(pair) if pair.len() == 2 => match py_strs(&pair[1]) {
                                    Some(f) => self.data_files.extend(f),
                                    None => self.dynamic.push(k.clone()),
                                },
                                _ => self.dynamic.push(k.clone()),
                            }
                        }
                    }
                    _ => self.dynamic.push(k.clone()),
                },
                _ => {}
            }
        }
    }

    /// The directory of package `name` given `package_dir`.
    fn package_path(&self, root: &Path, name: &str) -> PathBuf {
        // The longest `package_dir` key that is a prefix of the name.
        let mut best: Option<(&str, &str)> = None;
        for (k, v) in &self.package_dir {
            let matches = k.is_empty() || name == k || name.starts_with(&format!("{k}."));
            if matches && best.is_none_or(|(bk, _)| k.len() > bk.len()) {
                best = Some((k, v));
            }
        }
        let (k, v) = best.unwrap_or(("", ""));
        let rest = name.strip_prefix(k).unwrap_or(name).trim_start_matches('.');
        let mut p = fsx::join(root, v);
        for seg in rest.split('.').filter(|s| !s.is_empty()) {
            p.push(seg);
        }
        p
    }

    fn apply(
        &self,
        root: &Path,
        project: &str,
        scm: bool,
        pyproject_based: bool,
        spec: &mut TaskSpec,
    ) {
        let mut pkgs: Vec<(String, PathBuf)> = Vec::new();
        if let Some(list) = &self.packages {
            pkgs.extend(list.iter().map(|n| (n.clone(), self.package_path(root, n))));
        } else if let Some((w, inc, exc, ns)) = &self.find {
            let wheres = if w.is_empty() {
                vec![".".to_string()]
            } else {
                w.clone()
            };
            for wd in wheres {
                pkgs.extend(find_packages(&fsx::join(root, &wd), inc, exc, *ns));
            }
        } else if self.py_modules.is_empty() {
            // Auto-discovery: src layout, else flat layout.
            let src = root.join("src");
            if fsx::is_dir(&src) {
                pkgs.extend(find_packages(&src, &[], &[], true));
            } else {
                pkgs.extend(
                    find_packages(root, &[], &[], false)
                        .into_iter()
                        .filter(|(n, _)| {
                            let top = n.split('.').next().unwrap_or(n);
                            !FLAT_EXCLUDE.contains(&top)
                        }),
                );
                // A single-module distribution. A project without a name (at the root of
                // the checkout, whose directory name is no part of the tree) is not asked
                // for `<name>.py`: the build backend takes the modules it finds next to the
                // project file, so every one of them is an input, and so is one that is
                // added.
                let name = import_name(project);
                if !pkgs.is_empty() {
                } else if name.is_empty() {
                    spec.membership(root.to_path_buf(), "*.py", WHEEL);
                } else {
                    let single = root.join(format!("{name}.py"));
                    if fsx::is_file(&single) {
                        spec.input(single);
                    }
                }
            }
        }
        for (_, d) in &pkgs {
            spec.membership(d.clone(), "*.py", WHEEL);
        }
        for m in &self.py_modules {
            spec.input(
                self.package_path(root, "")
                    .join(format!("{}.py", m.replace('.', "/"))),
            );
        }
        for (pkg, globs) in &self.package_data {
            let dirs: Vec<PathBuf> = if pkg.is_empty() {
                pkgs.iter().map(|(_, d)| d.clone()).collect()
            } else {
                vec![self.package_path(root, pkg)]
            };
            for d in dirs {
                for g in globs {
                    add_glob(&d, g, WHEEL, spec);
                }
            }
        }
        // `include_package_data` (default on for pyproject-based configuration): files
        // in packages that the sdist includes. With setuptools-scm that is
        // every tracked file; otherwise MANIFEST.in decides (its memberships
        // are added separately).
        let ipd = self.include_package_data.unwrap_or(pyproject_based);
        if ipd && scm {
            for (_, d) in &pkgs {
                spec.membership(d.clone(), "**", WHEEL);
            }
        }
        for f in &self.data_files {
            add_glob(root, f, WHEEL, spec);
        }
        for d in &self.dynamic {
            spec.uncertain(
                UncertaintyKind::ComputedSpecifier,
                format!("computed `setup()` argument `{d}`"),
            );
        }
    }
}

/// Applies `MANIFEST.in` to `spec` (sdist memberships, excludes in order).
pub fn manifest_in(src: &str, root: &Path, spec: &mut TaskSpec) {
    let mut joined = String::new();
    for l in src.lines() {
        let l = l.split('#').next().unwrap_or("");
        if let Some(x) = l.trim_end().strip_suffix('\\') {
            joined.push_str(x);
            joined.push(' ');
        } else {
            joined.push_str(l);
            joined.push('\n');
        }
    }
    for line in joined.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let Some((&cmd, args)) = w.split_first() else {
            continue;
        };
        match cmd {
            "include" => args.iter().for_each(|p| add_glob(root, p, SDIST, spec)),
            "exclude" => args
                .iter()
                .for_each(|p| add_glob(root, &format!("!{p}"), SDIST, spec)),
            "recursive-include" => {
                if let Some((d, pats)) = args.split_first() {
                    for p in pats {
                        spec.membership(fsx::join(root, d), format!("**/{p}"), SDIST);
                    }
                }
            }
            "recursive-exclude" => {
                if let Some((d, pats)) = args.split_first() {
                    for p in pats {
                        add_glob(root, &format!("!{d}/**/{p}"), SDIST, spec);
                    }
                }
            }
            "global-include" => args
                .iter()
                .for_each(|p| spec.membership(root.to_path_buf(), format!("**/{p}"), SDIST)),
            "global-exclude" => args
                .iter()
                .for_each(|p| add_glob(root, &format!("!**/{p}"), SDIST, spec)),
            "graft" => args
                .iter()
                .for_each(|d| spec.membership(fsx::join(root, d), "**", SDIST)),
            "prune" => args
                .iter()
                .for_each(|d| add_glob(root, &format!("!{d}/**"), SDIST, spec)),
            _ => {}
        }
    }
}

/// Adds hatch-style include/exclude patterns as memberships of `root`.
fn gitignore_memberships(
    root: &Path,
    include: &[String],
    exclude: &[String],
    reason: &str,
    spec: &mut TaskSpec,
) {
    let ex: Vec<String> = exclude.iter().flat_map(|e| gitignore_globs(e)).collect();
    for i in include {
        for g in gitignore_globs(i) {
            // `name/**` for a pattern that names an existing file adds nothing.
            if let Some(f) = g.strip_suffix("/**")
                && !crate::glob::is_glob(f)
                && fsx::is_file(&fsx::join(root, f))
            {
                continue;
            }
            let (prefix, rest) = crate::glob::split_static_prefix(&g);
            let dir = fsx::join(root, prefix);
            if rest.is_empty() {
                if fsx::is_dir(&dir) {
                    let mut m = crate::spec::Membership::new(dir, "**", reason);
                    m.exclude = ex.clone();
                    spec.memberships.push(m);
                } else {
                    spec.input(dir);
                }
            } else {
                let mut m = crate::spec::Membership::new(dir, rest, reason);
                m.exclude = ex.clone();
                spec.memberships.push(m);
            }
        }
    }
}

/// A path that ships as a whole: a directory membership or a file input.
fn add_path(root: &Path, p: &str, reason: &str, spec: &mut TaskSpec) {
    let abs = fsx::join(root, p);
    if fsx::is_dir(&abs) {
        spec.membership(abs, "**", reason);
    } else {
        spec.input(abs);
    }
}

/// The default package of `project` under `root`: `src/<name>`, `<name>`, or
/// the single module `<name>.py` / `src/<name>.py`.
fn default_package(root: &Path, project: &str) -> Option<String> {
    if project.is_empty() {
        return None;
    }
    let n = import_name(project);
    [
        format!("src/{n}"),
        n.clone(),
        format!("src/{n}.py"),
        format!("{n}.py"),
    ]
    .into_iter()
    .find(|c| fsx::exists(&root.join(c)))
}

fn hatch(t: &toml::Table, root: &Path, project: &str, spec: &mut TaskSpec) {
    let build = t
        .get("tool")
        .and_then(|x| x.get("hatch"))
        .and_then(|x| x.get("build"));
    let target = |name: &str| {
        build
            .and_then(|b| b.get("targets"))
            .and_then(|x| x.get(name))
    };
    let get = |tg: Option<&toml::Value>, k: &str| -> Vec<String> {
        let mut v = toml_strs(tg.and_then(|x| x.get(k)));
        if v.is_empty() {
            v = toml_strs(build.and_then(|b| b.get(k)));
        }
        v
    };
    for (name, reason) in [(WHEEL, WHEEL), (SDIST, SDIST)] {
        let tg = target(name);
        let packages = toml_strs(tg.and_then(|x| x.get("packages")));
        let include = get(tg, "include");
        let only = get(tg, "only-include");
        let exclude = get(tg, "exclude");
        for p in &packages {
            add_path(root, p, reason, spec);
        }
        for p in &only {
            add_path(root, p, reason, spec);
        }
        gitignore_memberships(root, &include, &exclude, reason, spec);
        for key in ["force-include"] {
            let fi = tg
                .and_then(|x| x.get(key))
                .or_else(|| build.and_then(|b| b.get(key)))
                .and_then(|v| v.as_table());
            if let Some(fi) = fi {
                for src in fi.keys() {
                    add_path(root, src, reason, spec);
                }
            }
        }
        if packages.is_empty() && include.is_empty() && only.is_empty() {
            if name == SDIST {
                let mut m = crate::spec::Membership::new(
                    root.to_path_buf(),
                    "**",
                    "sdist (hatch default: every file)",
                );
                m.exclude = exclude.iter().flat_map(|e| gitignore_globs(e)).collect();
                spec.memberships.push(m);
            } else {
                match default_package(root, project) {
                    Some(p) => add_path(root, &p, reason, spec),
                    None => spec.membership(
                        root.to_path_buf(),
                        "**/*.py",
                        "wheel (hatch package not detected)",
                    ),
                }
            }
        }
    }
}

fn poetry(t: &toml::Table, root: &Path, project: &str, spec: &mut TaskSpec) {
    let Some(p) = t.get("tool").and_then(|x| x.get("poetry")) else {
        return;
    };
    let mut any = false;
    if let Some(pk) = p.get("packages").and_then(|v| v.as_array()) {
        for e in pk {
            let Some(inc) = e.get("include").and_then(|v| v.as_str()) else {
                continue;
            };
            let from = e.get("from").and_then(|v| v.as_str()).unwrap_or("");
            let base = fsx::join(root, from);
            if crate::glob::is_glob(inc) {
                add_glob(&base, inc, WHEEL, spec);
            } else {
                add_path(&base, inc, WHEEL, spec);
                // `include = "foo"` is also `foo.py`.
                let m = base.join(format!("{inc}.py"));
                if fsx::is_file(&m) {
                    spec.input(m);
                }
            }
            any = true;
        }
    }
    let excl = toml_strs(p.get("exclude"));
    for e in p
        .get("include")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let (path, reason) = match &e {
            toml::Value::String(s) => (s.clone(), SDIST),
            toml::Value::Table(o) => (
                o.get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                match o.get("format") {
                    Some(toml::Value::String(f)) if f == "wheel" => WHEEL,
                    _ => SDIST,
                },
            ),
            _ => continue,
        };
        if !path.is_empty() {
            add_glob(root, &path, reason, spec);
        }
    }
    for e in excl {
        add_glob(root, &format!("!{e}"), WHEEL, spec);
    }
    if !any {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or(project);
        match default_package(root, name) {
            Some(d) => add_path(root, &d, WHEEL, spec),
            None => spec.membership(
                root.to_path_buf(),
                "**/*.py",
                "wheel (poetry package not detected)",
            ),
        }
    }
}

/// `py-dist:<dir>` tasks.
pub struct PyDist;

impl TaskSource for PyDist {
    fn id(&self) -> &'static str {
        "pypackaging"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["pyproject.toml", "setup.cfg", "setup.py", "MANIFEST.in"]
    }

    /// Package discovery (setuptools' auto-discovery, `find:`, the default
    /// package of hatch, pdm and poetry) lists directories and asks which
    /// of their entries hold an `__init__.py`: none of that is a config
    /// file, and the task's memberships are only the packages found. What
    /// the whole discovery asked of the tree is the task's
    /// [`DiscoveredTask::decided_by`], so that a package that comes or goes
    /// (also one whose files no compiled language claims) makes a store
    /// analyze the project again.
    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        crate::sources::with_decided_by(|| self.discover_project(cx))
    }
}

impl PyDist {
    fn discover_project(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let root = &cx.package.dir;
        let pp_path = root.join("pyproject.toml");
        let cfg_path = root.join("setup.cfg");
        let py_path = root.join("setup.py");
        let man_path = root.join("MANIFEST.in");
        let pp_src = fsx::read_to_string(&pp_path).ok();
        let pp: Option<toml::Table> = pp_src.as_deref().and_then(|s| toml::from_str(s).ok());
        let cfg_src = fsx::read_to_string(&cfg_path).ok();
        let cfg = cfg_src.as_deref().map(parse_ini);
        let py_src = fsx::read_to_string(&py_path).ok();
        let manifest = fsx::read_to_string(&man_path).ok();
        let has_build = pp.as_ref().is_some_and(|t| {
            t.contains_key("project")
                || t.contains_key("build-system")
                || t.get("tool").is_some_and(|x| x.get("poetry").is_some())
        });
        let has_cfg_meta = cfg
            .as_ref()
            .is_some_and(|c| c.iter().any(|(s, _)| s == "metadata" || s == "options"));
        if !has_build && py_src.is_none() && !has_cfg_meta {
            return Vec::new();
        }
        let mut spec = TaskSpec::default();
        let mut files = Vec::new();
        for p in [&pp_path, &cfg_path, &py_path, &man_path] {
            if fsx::is_file(p) {
                files.push(p.clone());
            }
        }
        if py_src.is_some() {
            spec.entry(py_path.clone(), EntryLang::Python);
        }
        if pp_src.is_some() && pp.is_none() {
            spec.uncertain(
                UncertaintyKind::ParseIncomplete,
                "unreadable pyproject.toml",
            );
        }
        let empty = toml::Table::new();
        let t = pp.as_ref().unwrap_or(&empty);
        let bs = t.get("build-system");
        let backend = bs
            .and_then(|b| b.get("build-backend"))
            .and_then(|v| v.as_str())
            .unwrap_or("setuptools.build_meta:__legacy__")
            .to_string();
        let requires = toml_strs(bs.and_then(|b| b.get("requires")));
        for r in &requires {
            let n: String = r
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .collect();
            if !n.is_empty() {
                spec.tool("pypi", n);
            }
        }
        let scm = requires
            .iter()
            .any(|r| r.contains("setuptools_scm") || r.contains("setuptools-scm"));
        let project = t
            .get("project")
            .and_then(|p| p.get("name"))
            .or_else(|| {
                t.get("tool")
                    .and_then(|x| x.get("poetry"))
                    .and_then(|p| p.get("name"))
            })
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                cfg.as_ref()
                    .and_then(|c| ini_get(c, "metadata", "name"))
                    .map(str::to_string)
            })
            .or_else(|| {
                py_src.as_deref().and_then(|s| {
                    setup_kwargs(s)
                        .into_iter()
                        .find_map(|(k, v)| match (k.as_str(), v) {
                            ("name", Py::Str(n)) => Some(n),
                            _ => None,
                        })
                })
            })
            .unwrap_or_else(|| {
                // The directory's name, as the build backends fall back to it. Not for the
                // checkout itself: what that directory is called is no part of the tree, so
                // a project without a name at the root has no default package.
                if fsx::is_checkout_dir(root) {
                    return String::new();
                }
                root.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        // sdist metadata files every backend includes.
        for g in [
            "README*", "LICENSE*", "LICENCE*", "COPYING*", "NOTICE*", "AUTHORS*",
        ] {
            spec.membership(root.clone(), g, SDIST);
        }
        let b = backend.split(':').next().unwrap_or("");
        match b {
            "hatchling.build" => hatch(t, root, &project, &mut spec),
            "poetry.core.masonry.api" | "poetry.masonry.api" => {
                poetry(t, root, &project, &mut spec)
            }
            "flit_core.buildapi" | "flit.buildapi" => {
                let module = t
                    .get("tool")
                    .and_then(|x| x.get("flit"))
                    .and_then(|f| f.get("module"))
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&project)
                    .to_string();
                match default_package(root, &module) {
                    Some(d) => add_path(root, &d, WHEEL, &mut spec),
                    None => {
                        spec.membership(root.clone(), "**/*.py", "wheel (flit module not detected)")
                    }
                }
                let sd = t
                    .get("tool")
                    .and_then(|x| x.get("flit"))
                    .and_then(|f| f.get("sdist"));
                for i in toml_strs(sd.and_then(|s| s.get("include"))) {
                    add_glob(root, &i, SDIST, &mut spec);
                }
            }
            "pdm.backend" | "pdm.pep517.api" => {
                let pb = t
                    .get("tool")
                    .and_then(|x| x.get("pdm"))
                    .and_then(|p| p.get("build"));
                let inc = toml_strs(pb.and_then(|p| p.get("includes")));
                let exc = toml_strs(pb.and_then(|p| p.get("excludes")));
                if inc.is_empty() {
                    match default_package(root, &project) {
                        Some(d) => add_path(root, &d, WHEEL, &mut spec),
                        None => spec.membership(
                            root.clone(),
                            "**/*.py",
                            "wheel (pdm package not detected)",
                        ),
                    }
                } else {
                    gitignore_memberships(root, &inc, &exc, WHEEL, &mut spec);
                }
                for i in toml_strs(pb.and_then(|p| p.get("source-includes"))) {
                    add_glob(root, &i, SDIST, &mut spec);
                }
            }
            "maturin" => {
                let m = t.get("tool").and_then(|x| x.get("maturin"));
                let src = m
                    .and_then(|m| m.get("python-source"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                spec.membership(fsx::join(root, src), "**/*.py", WHEEL);
                spec.membership(root.clone(), "**/*.rs", "maturin crate sources");
                spec.config(root.join("Cargo.toml"));
                for i in toml_strs(m.and_then(|m| m.get("include"))) {
                    add_glob(root, &i, WHEEL, &mut spec);
                }
            }
            _ if b.starts_with("setuptools") => {
                let mut st = Setuptools::default();
                if let Some(c) = &cfg {
                    st.read_cfg(c);
                }
                st.read_pyproject(t);
                if let Some(s) = &py_src {
                    st.read_setup_py(s);
                }
                let pyproject_based = t.contains_key("project");
                st.apply(root, &project, scm, pyproject_based, &mut spec);
                if let Some(m) = &manifest {
                    manifest_in(m, root, &mut spec);
                }
                if scm {
                    // setuptools-scm puts every tracked file in the sdist.
                    spec.membership(root.clone(), "**", "sdist (setuptools-scm: tracked files)");
                }
            }
            other => {
                spec.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!("unknown build backend `{other}`"),
                );
            }
        }
        // No `dist/` output: the store would link every task that lists the project
        // root as a consumer of this one (generated-file linking), and nothing
        // reads `dist/`.
        let rel = repo_rel(&cx.workspace.root, root);
        let mut t = DiscoveredTask::new(TaskId::other("py-dist", rel), "dist", root.clone());
        t.spec = spec;
        t.files = files;
        vec![t]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_literals() {
        assert_eq!(parse_py("'a' \"b\""), Py::Str("ab".into()));
        assert_eq!(
            parse_py("['a', ('b',)] + ['c']"),
            Py::List(vec![
                Py::Str("a".into()),
                Py::List(vec![Py::Str("b".into())]),
                Py::Str("c".into())
            ])
        );
        assert!(matches!(parse_py("{'pkg': ['*.json'], '': ['x']}"), Py::Dict(d) if d.len() == 2));
        assert!(
            matches!(parse_py("find_packages(where='src', exclude=['tests*'])"), Py::Call(n, a) if n == "find_packages" && a.len() == 2)
        );
        assert_eq!(parse_py("read('x') + 1"), Py::Dyn("read('x') + 1".into()));
        assert_eq!(parse_py("NAME"), Py::Name("NAME".into()));
    }

    #[test]
    fn setup_py_kwargs() {
        let src = "from setuptools import setup, find_packages\nDATA = {'mypkg': \
                   ['data/*.json']}\nsetup(\n    name='mypkg',\n    \
                   packages=find_packages('src'),\n    package_dir={'': 'src'},\n    \
                   package_data=DATA,\n    data_files=[('etc', ['conf/a.cfg'])],\n    \
                   version=get_version(),\n)\n";
        let kw = setup_kwargs(src);
        let get = |k: &str| kw.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("name"), Some(Py::Str("mypkg".into())));
        assert!(matches!(get("package_data"), Some(Py::Dict(_))));
        assert!(matches!(get("version"), Some(Py::Call(..))));
    }

    #[test]
    fn gitignore_patterns() {
        assert_eq!(gitignore_globs("/src/pkg"), ["src/pkg", "src/pkg/**"]);
        assert_eq!(gitignore_globs("*.json"), ["**/*.json", "**/*.json/**"]);
        assert_eq!(gitignore_globs("data/"), ["**/data/**"]);
    }

    #[test]
    fn parsers_never_panic() {
        let alphabet = b"[](){}'\",:+=_ab\\\nrfTrueNonesetup(include graft prune *";
        let mut x: u64 = 0x5851f42d4c957f2d;
        let mut spec = TaskSpec::default();
        for _ in 0..3000 {
            let mut s = String::new();
            for _ in 0..48 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(alphabet[(x % alphabet.len() as u64) as usize] as char);
            }
            parse_py(&s);
            setup_kwargs(&s);
            manifest_in(&s, Path::new("/r"), &mut spec);
            gitignore_globs(&s);
        }
        for s in [
            "(",
            "[",
            "{",
            "'",
            "setup(",
            "f(",
            "a +",
            "{'a':",
            "(((((((((((((((((((((((",
        ] {
            parse_py(s);
            setup_kwargs(s);
        }
    }
}
