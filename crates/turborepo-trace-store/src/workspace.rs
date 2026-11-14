//! First-party package discovery (one shard per package).
//!
//! Workspace members come from the root `package.json` `workspaces` field (an
//! array or `{ "packages": [...] }`) and from `pnpm-workspace.yaml`
//! `packages:`, with `!` exclusions. Every walked `package.json` whose
//! directory matches is a package; the repo root is always one. A file belongs
//! to the deepest package root above it.
//!
//! Other languages have their own package boundaries, which need no workspace
//! declaration:
//!
//! | Manifest | Package | Name |
//! |---|---|---|
//! | `pyproject.toml`, else `setup.py` | a Python project | `pypi:<project name>` (`[project]` or `[tool.poetry]`, PEP 503), else `pypi:<dir>` |
//! | `go.mod` | a Go module | `go:<module path>` |
//! | `Cargo.toml` with `[package]` | a crate | `crates:<package name>` |
//!
//! Manifests below `vendor/` or `testdata/` directories are not packages
//! (vendored code and Go test fixtures). A directory that is both a JS package
//! and another language's package stays the JS package (one shard per
//! directory).
//!
//! Package names come from the manifest's `name` (or the root path, `.` for the
//! root). Names must be unique because cross-package edges refer to workspace
//! packages by name; a duplicate gets `#<root>` appended.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use turborepo_trace_core::glob::Glob;

/// One first-party package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageDef {
    /// Unique name.
    pub name: String,
    /// Root relative to the repo root (`""` for the root package).
    pub root: String,
    /// Manifest path relative to the repo root, if any.
    pub manifest: Option<String>,
}

/// The packages of a repository and the file → package assignment.
#[derive(Clone, Debug)]
pub struct Workspace {
    /// Packages, sorted by root; index 0 is the root package.
    pub packages: Vec<PackageDef>,
    by_root: HashMap<String, usize>,
}

impl Workspace {
    /// Discovers packages from the repo root and the walked files.
    pub fn discover(root: &Path, files: &[String]) -> Workspace {
        Self::discover_cached(root, files, &crate::smallfiles::SmallFiles::in_memory())
    }

    /// [`Workspace::discover`], reading manifests through `cache`.
    pub fn discover_cached(
        root: &Path,
        files: &[String],
        cache: &crate::smallfiles::SmallFiles,
    ) -> Workspace {
        Self::discover_with(root, files, cache, &DeclaredMembers)
    }

    /// [`Workspace::discover_cached`] with the JavaScript workspace members
    /// that `members` names (the other languages' packages and the root are
    /// found as always).
    pub fn discover_with(
        root: &Path,
        files: &[String],
        cache: &crate::smallfiles::SmallFiles,
        members: &dyn WorkspaceMembers,
    ) -> Workspace {
        let read_json = |p: &Path| -> Option<serde_json::Value> {
            serde_json::from_slice(&cache.read(p)?).ok()
        };
        let root_manifest = read_json(&root.join("package.json"));
        let mut packages = vec![PackageDef {
            name: root_manifest
                .as_ref()
                .and_then(|m| m.get("name")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| ".".to_owned()),
            root: String::new(),
            manifest: files
                .iter()
                .any(|f| f == "package.json")
                .then(|| "package.json".to_owned()),
        }];
        let member_dirs = members.js_members(root, files, cache);
        let member_dirs: HashSet<&str> = member_dirs.iter().map(String::as_str).collect();
        // In file order: the walked manifests of the member directories.
        let members: Vec<(&String, &str)> = files
            .iter()
            .filter_map(|f| {
                let dir = f.strip_suffix("/package.json")?;
                member_dirs.contains(dir).then_some((f, dir))
            })
            .collect();
        // Member manifests are read in parallel (thousands in big monorepos), in file
        // order.
        use rayon::prelude::*;
        packages.par_extend(members.par_iter().map(|(f, dir)| {
            let name = read_json(&root.join(f))
                .and_then(|m| m.get("name")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| (*dir).to_owned());
            PackageDef {
                name,
                root: (*dir).to_owned(),
                manifest: Some((*f).clone()),
            }
        }));
        // Language package boundaries (see the module docs).
        let mut roots: HashSet<String> = packages.iter().map(|p| p.root.clone()).collect();
        let mut lang: Vec<(String, PackageDef)> = Vec::new();
        for f in files {
            let (dir, file) = match f.rsplit_once('/') {
                Some((d, n)) => (d, n),
                None => ("", f.as_str()),
            };
            if matches!(
                file,
                "pyproject.toml" | "setup.py" | "go.mod" | "Cargo.toml"
            ) && !dir
                .split('/')
                .any(|c| c == "vendor" || c == "testdata" || c == "node_modules")
                && let Some(name) = lang_package_name(cache, &root.join(f), file, dir)
            {
                lang.push((
                    file.to_owned(),
                    PackageDef {
                        name,
                        root: dir.to_owned(),
                        manifest: Some(f.clone()),
                    },
                ));
            }
        }
        // `pyproject.toml` wins over `setup.py` in the same directory.
        lang.sort_by(|a, b| {
            (a.1.root.as_str(), a.0 == "setup.py").cmp(&(b.1.root.as_str(), b.0 == "setup.py"))
        });
        for (_, def) in lang {
            if def.root.is_empty() {
                // The repo root is already a package; name it after the language manifest
                // unless a root `package.json` names it.
                if packages[0].manifest.is_none() {
                    packages[0].name = def.name;
                    packages[0].manifest = def.manifest;
                }
                continue;
            }
            if roots.insert(def.root.clone()) {
                packages.push(def);
            }
        }
        packages[1..].sort_by(|a, b| a.root.cmp(&b.root));
        let mut seen = HashSet::new();
        for p in &mut packages {
            if !seen.insert(p.name.clone()) {
                p.name = format!("{}#{}", p.name, p.root);
                seen.insert(p.name.clone());
            }
        }
        let by_root = packages
            .iter()
            .enumerate()
            .map(|(i, p)| (p.root.clone(), i))
            .collect();
        Workspace { packages, by_root }
    }

    /// The package owning the repo-relative path `rel`: the deepest package
    /// root above it.
    pub fn owner(&self, rel: &str) -> usize {
        let mut cur = rel;
        while let Some(i) = cur.rfind('/') {
            cur = &cur[..i];
            if let Some(&p) = self.by_root.get(cur) {
                return p;
            }
        }
        0
    }

    /// The package with this root, if any.
    pub fn by_root(&self, root: &str) -> Option<usize> {
        self.by_root.get(root).copied()
    }
}

/// Which directories are the JavaScript workspace members of a repository.
///
/// The store finds every other package itself (the root, `pyproject.toml`,
/// `go.mod` and `Cargo.toml` packages). A host that knows the workspace
/// (Turborepo's package graph) supplies the JavaScript members through this
/// trait; [`DeclaredMembers`] reads them from the repository.
pub trait WorkspaceMembers: Send + Sync + std::fmt::Debug {
    /// The member directories, relative to `root` and `/`-separated, given the
    /// walked files (`files`, root-relative). Only directories whose
    /// `package.json` was walked become packages.
    fn js_members(
        &self,
        root: &Path,
        files: &[String],
        cache: &crate::smallfiles::SmallFiles,
    ) -> Vec<String>;
}

/// The default [`WorkspaceMembers`]: the `workspaces` field of the root
/// `package.json` (an array or `{ "packages": [...] }`) and `packages:` of
/// `pnpm-workspace.yaml`, with `!` exclusions, matched against the walked
/// `package.json` files.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeclaredMembers;

impl WorkspaceMembers for DeclaredMembers {
    fn js_members(
        &self,
        root: &Path,
        files: &[String],
        cache: &crate::smallfiles::SmallFiles,
    ) -> Vec<String> {
        let read_json = |p: &Path| -> Option<serde_json::Value> {
            serde_json::from_slice(&cache.read(p)?).ok()
        };
        let root_manifest = read_json(&root.join("package.json"));
        let mut patterns: Vec<String> = Vec::new();
        if let Some(m) = &root_manifest {
            match m.get("workspaces") {
                Some(serde_json::Value::Array(a)) => {
                    patterns.extend(a.iter().filter_map(|v| v.as_str().map(str::to_owned)))
                }
                Some(serde_json::Value::Object(o)) => {
                    if let Some(serde_json::Value::Array(a)) = o.get("packages") {
                        patterns.extend(a.iter().filter_map(|v| v.as_str().map(str::to_owned)));
                    }
                }
                _ => {}
            }
        }
        if let Some(y) = cache.read(&root.join("pnpm-workspace.yaml"))
            && let Ok(y) = std::str::from_utf8(&y)
        {
            patterns.extend(pnpm_packages(y));
        }
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for p in &patterns {
            let p = p.trim().trim_start_matches("./").trim_end_matches('/');
            match p.strip_prefix('!') {
                Some(n) => exclude.push(Glob::new(n.trim_start_matches("./"))),
                None if !p.is_empty() => include.push(Glob::new(p)),
                None => {}
            }
        }
        files
            .iter()
            .filter_map(|f| {
                let dir = f.strip_suffix("/package.json")?;
                let d = dir.as_bytes();
                // A member pattern names directories: match it on its own, without a
                // basename shortcut (`Glob` matches slash-free patterns against the last
                // component).
                let matches = |g: &Glob| {
                    if g.as_str().contains('/') {
                        g.matches(d)
                    } else {
                        !dir.contains('/') && g.matches(d)
                    }
                };
                (include.iter().any(matches) && !exclude.iter().any(matches))
                    .then(|| dir.to_owned())
            })
            .collect()
    }
}

/// A [`WorkspaceMembers`] list that the host gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListedMembers(pub Vec<String>);

impl WorkspaceMembers for ListedMembers {
    fn js_members(
        &self,
        _root: &Path,
        _files: &[String],
        _cache: &crate::smallfiles::SmallFiles,
    ) -> Vec<String> {
        self.0.clone()
    }
}

/// The [`WorkspaceMembers`] of a build ([`crate::GraphOptions::members`]):
/// [`DeclaredMembers`] by default.
#[derive(Clone, Debug)]
pub struct Members(pub std::sync::Arc<dyn WorkspaceMembers>);

impl Default for Members {
    fn default() -> Self {
        Members(std::sync::Arc::new(DeclaredMembers))
    }
}

impl PartialEq for Members {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Members {}

/// The package name a language manifest declares, namespaced by ecosystem;
/// `None` if the file is not a package manifest (a virtual `Cargo.toml`
/// workspace).
fn lang_package_name(
    cache: &crate::smallfiles::SmallFiles,
    path: &Path,
    file: &str,
    dir: &str,
) -> Option<String> {
    let bytes = cache.read(path);
    let text = bytes
        .as_deref()
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or_default();
    let dir_name = || {
        if dir.is_empty() {
            ".".to_owned()
        } else {
            dir.rsplit('/').next().unwrap_or(dir).to_owned()
        }
    };
    match file {
        "go.mod" => {
            let module = turborepo_trace_lockfile::gosum::parse_go_mod(text)
                .ok()
                .and_then(|m| m.module)
                .unwrap_or_else(dir_name);
            Some(format!("go:{module}"))
        }
        "Cargo.toml" => toml_field(text, &["[package]"], "name").map(|n| format!("crates:{n}")),
        "pyproject.toml" => {
            let name =
                toml_field(text, &["[project]", "[tool.poetry]"], "name").unwrap_or_else(dir_name);
            Some(format!(
                "pypi:{}",
                turborepo_trace_lockfile::pep503_name(&name)
            ))
        }
        "setup.py" => Some(format!(
            "pypi:{}",
            turborepo_trace_lockfile::pep503_name(&dir_name())
        )),
        _ => None,
    }
}

/// `key = "value"` in one of the TOML `sections` (line-based; enough for
/// package names).
fn toml_field(text: &str, sections: &[&str], key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = sections.contains(&t);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = t.split_once('=')
            && k.trim() == key
        {
            let v = v.trim();
            let v = v.split('#').next().unwrap_or(v).trim();
            return Some(v.trim_matches(|c| c == '"' || c == '\'').to_owned());
        }
    }
    None
}

/// The `packages:` list of a `pnpm-workspace.yaml` (block sequence or flow
/// sequence).
fn pnpm_packages(yaml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_packages = false;
    for line in yaml.lines() {
        let no_comment = match line.find(" #") {
            Some(i) => &line[..i],
            None if line.trim_start().starts_with('#') => "",
            None => line,
        };
        let t = no_comment.trim();
        if t.is_empty() {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented {
            in_packages = false;
            if let Some(rest) = t.strip_prefix("packages:") {
                let rest = rest.trim();
                if let Some(flow) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    out.extend(flow.split(',').map(unquote).filter(|s| !s.is_empty()));
                } else {
                    in_packages = true;
                }
            }
            continue;
        }
        if in_packages && let Some(item) = t.strip_prefix('-') {
            out.push(unquote(item));
        }
    }
    out
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches(|c| c == '\'' || c == '"').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pnpm_yaml() {
        let y = "packages:\n  - 'apps/*'\n  - \"packages/**\" # all\n  - \
                 '!services/copper'\n\nminimumReleaseAge: 2880\ncatalog:\n  - nope\n";
        assert_eq!(
            pnpm_packages(y),
            ["apps/*", "packages/**", "!services/copper"]
        );
        assert_eq!(pnpm_packages("packages: [a/*, 'b']\n"), ["a/*", "b"]);
    }

    #[test]
    fn language_package_boundaries() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let files = [
            ("pyproject.toml", "[project]\nname = \"My_App\"\n"),
            ("svc/go.mod", "module example.com/svc\n\ngo 1.22\n"),
            ("svc/vendor/x/go.mod", "module x\n"),
            ("svc/testdata/m/go.mod", "module m\n"),
            (
                "crates/a/Cargo.toml",
                "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
            ),
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
            ("legacy/setup.py", "from setuptools import setup\n"),
            ("web/package.json", "{\"name\":\"web\"}"),
            ("web/pyproject.toml", "[project]\nname = \"web-tools\"\n"),
        ];
        for (f, c) in files {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), c).unwrap();
        }
        std::fs::write(r.join("package.json"), r#"{"workspaces":["web"]}"#).unwrap();
        let mut names: Vec<String> = files.iter().map(|(f, _)| f.to_string()).collect();
        names.push("package.json".into());
        let w = Workspace::discover(r, &names);
        let roots: Vec<_> = w
            .packages
            .iter()
            .map(|p| (p.root.as_str(), p.name.as_str()))
            .collect();
        assert_eq!(
            roots,
            [
                ("", "."),
                ("crates/a", "crates:a"),
                ("legacy", "pypi:legacy"),
                ("svc", "go:example.com/svc"),
                ("web", "web"),
            ]
        );
        assert_eq!(w.owner("svc/internal/x.go"), 3);
        assert_eq!(w.owner("svc/vendor/x/y.go"), 3);

        // Without a root package.json the root takes the language manifest's name.
        std::fs::remove_file(r.join("package.json")).unwrap();
        names.pop();
        let w = Workspace::discover(r, &names);
        assert_eq!(w.packages[0].name, "pypi:my-app");
        assert_eq!(w.packages[0].manifest.as_deref(), Some("pyproject.toml"));
    }

    #[test]
    fn discovery_and_owner() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let files = [
            "package.json",
            "pnpm-workspace.yaml",
            "apps/web/package.json",
            "apps/web/src/a.ts",
            "services/copper/package.json",
            "packages/x/package.json",
            "packages/x/test/fixture/package.json",
            "tests/package.json",
        ];
        for f in files {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), "{}").unwrap();
        }
        std::fs::write(r.join("package.json"), r#"{"name":"root"}"#).unwrap();
        std::fs::write(r.join("apps/web/package.json"), r#"{"name":"web"}"#).unwrap();
        std::fs::write(r.join("packages/x/package.json"), r#"{"name":"web"}"#).unwrap();
        std::fs::write(
            r.join("pnpm-workspace.yaml"),
            "packages:\n  - 'apps/*'\n  - 'packages/*'\n  - 'services/*'\n  - \
             '!services/copper'\n  - tests\n",
        )
        .unwrap();
        let files: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        let w = Workspace::discover(r, &files);
        let roots: Vec<_> = w
            .packages
            .iter()
            .map(|p| (p.root.as_str(), p.name.as_str()))
            .collect();
        assert_eq!(
            roots,
            [
                ("", "root"),
                ("apps/web", "web"),
                ("packages/x", "web#packages/x"),
                ("tests", "tests")
            ]
        );
        assert_eq!(w.owner("apps/web/src/a.ts"), 1);
        assert_eq!(w.owner("packages/x/test/fixture/package.json"), 2);
        assert_eq!(w.owner("services/copper/a.ts"), 0);
        assert_eq!(w.owner("x.ts"), 0);
    }
}
