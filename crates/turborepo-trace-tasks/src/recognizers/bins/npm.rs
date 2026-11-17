//! npm package binaries: `node_modules/.bin` links and shims, workspace
//! packages' `bin` fields, and dependencies' manifests.
//!
//! Resolution order for a program `name` run from `cwd` (see
//! [`find_installed`]):
//!
//! 1. `node_modules/.bin/<name>` in `cwd` and each ancestor up to the repo root
//!    (or the explicitly written `node_modules/.bin/x` path). A symlink is
//!    followed (npm, yarn classic, bun); a shell shim is parsed (pnpm's `#
//!    cmd-shim-target=` comment, or the `"$basedir/…"` target of cmd-shim /
//!    older pnpm shims).
//! 2. The `bin` fields of every workspace package (pnpm/yarn/npm workspaces),
//!    so internal binaries resolve without an install.
//! 3. The `bin` fields of the task package's and the root's dependencies, found
//!    in `node_modules/<dep>/package.json` (installed, but without a `.bin`
//!    entry).
//!
//! [`declared_dependency`] is the last resort without `node_modules`: a
//! dependency whose name (or unscoped name) equals the program.
//!
//! A resolved target under the repo root and outside `node_modules` is
//! first-party; anything else is a third-party package, named by its
//! `node_modules/<name>` directory.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use turborepo_trace_frameworks::fsx;

use super::{Memo, link_cache};
use crate::{
    registry::RecognizeCx,
    sources::{Workspace, read_json},
    spec::Command,
};

/// A resolved binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// A file of a workspace package (or any first-party file outside
    /// `node_modules`).
    FirstParty {
        /// The bin's target file (absolute, repo-spelled). May not exist (a
        /// build output).
        file: PathBuf,
        /// The package directory owning the file (nearest `package.json`).
        pkg_dir: PathBuf,
        /// How it was found (`.bin`, `workspace bin`, `dependency bin`).
        via: &'static str,
    },
    /// A third-party package.
    ThirdParty {
        /// npm package name.
        package: String,
        /// The bin's target file, when installed.
        file: Option<PathBuf>,
    },
}

/// Steps 1–3 of the module docs. `explicit` is the absolute path of a
/// `node_modules/.bin/x` program written as a path.
///
/// A pure function of the filesystem and `(name, explicit, cwd, repo root,
/// package root)`: memoized per shared dir cache ([`super::RecMemo`], which
/// replays the lookups it recorded), since every command of every task asks
/// again.
pub fn find_installed(
    name: &str,
    explicit: Option<&Path>,
    cmd: &Command,
    cx: &RecognizeCx<'_>,
) -> Option<Found> {
    type Key = (String, Option<PathBuf>, PathBuf, PathBuf, PathBuf);
    static M: OnceLock<super::RecMemo<Key, Option<Found>>> = OnceLock::new();
    M.get_or_init(super::RecMemo::new).get_or(
        (
            name.to_string(),
            explicit.map(Path::to_path_buf),
            cmd.cwd.clone(),
            cx.repo_root.to_path_buf(),
            cx.package_root.to_path_buf(),
        ),
        || find_installed_uncached(name, explicit, cmd, cx),
    )
}

fn find_installed_uncached(
    name: &str,
    explicit: Option<&Path>,
    cmd: &Command,
    cx: &RecognizeCx<'_>,
) -> Option<Found> {
    if let Some(p) = explicit {
        if !fsx::is_file(p) {
            return None;
        }
        return Some(classify(&shim_target(p), cx, ".bin"));
    }
    let dirs = search_dirs(cmd, cx);
    // A `.bin` entry whose target is gone is a stale install: the manifests decide,
    // and it is only used when nothing else matches.
    let mut stale = None;
    for d in &dirs {
        let p = d.join("node_modules/.bin").join(name);
        if fsx::is_file(&p) {
            let t = shim_target(&p);
            if fsx::exists(&t) {
                return Some(classify(&t, cx, ".bin"));
            }
            stale.get_or_insert(t);
        }
    }
    if let Some(f) = workspace_bin(name, cx) {
        return Some(f);
    }
    dependency_bin(name, &dirs, cx)
        .or_else(|| stale.map(|t| classify(&canonical_prefix(&t), cx, ".bin (target missing)")))
}

/// `p` with its longest existing prefix canonicalized (a missing file below a
/// symlinked package directory).
fn canonical_prefix(p: &Path) -> PathBuf {
    for a in p.ancestors().skip(1) {
        if fsx::exists(a) {
            let rest = p.strip_prefix(a).unwrap_or(Path::new(""));
            return link_cache().0.canonicalize(a).path.join(rest);
        }
    }
    p.to_path_buf()
}

/// A dependency of the task package or the root named like the program
/// (`eslint`, or `@scope/name` for `name`), when nothing is installed.
pub fn declared_dependency(name: &str, cx: &RecognizeCx<'_>) -> Option<Found> {
    for dep in manifest_deps(cx) {
        let unscoped = dep.rsplit('/').next().unwrap_or(&dep);
        if dep == name || unscoped == name {
            return Some(Found::ThirdParty {
                package: dep,
                file: None,
            });
        }
    }
    None
}

/// `cmd.cwd` and its ancestors up to the repo root (the package root's when the
/// cwd is outside the repo).
fn search_dirs(cmd: &Command, cx: &RecognizeCx<'_>) -> Vec<PathBuf> {
    let start: &Path = if cmd.cwd.starts_with(cx.repo_root) {
        &cmd.cwd
    } else {
        cx.package_root
    };
    start
        .ancestors()
        .take_while(|d| d.starts_with(cx.repo_root))
        .map(Path::to_path_buf)
        .collect()
}

/// Where a `.bin` entry leads: the symlink target, the target named by a shell
/// shim, or the file itself.
pub fn shim_target(p: &Path) -> PathBuf {
    let (cache, _) = link_cache();
    if let Some(t) = cache.symlink_target(p) {
        return t.to_path_buf();
    }
    let Ok(bytes) = fsx::read(p) else {
        return p.to_path_buf();
    };
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)]);
    let Some(rel) = parse_shim(&text) else {
        return p.to_path_buf();
    };
    let dir = p.parent().unwrap_or(p);
    let joined = fsx::join(dir, &rel);
    if fsx::exists(&joined) {
        cache.canonicalize(&joined).path
    } else {
        joined
    }
}

/// The target of a shell shim, relative to the shim's directory (or absolute):
/// pnpm's `# cmd-shim-target=<path>` comment, else the first
/// `"$basedir/<path>"` (cmd-shim, older pnpm) that is not a `node` executable
/// probe.
pub fn parse_shim(text: &str) -> Option<String> {
    if !text.starts_with("#!") {
        return None;
    }
    for line in text.lines() {
        if let Some(t) = line.trim().strip_prefix("# cmd-shim-target=") {
            let t = t.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    for marker in ["\"$basedir/", "\"$basedir_abs/"] {
        let mut rest = text;
        while let Some(i) = rest.find(marker) {
            let after = &rest[i + marker.len()..];
            let end = after.find('"').unwrap_or(after.len());
            let t = &after[..end];
            rest = &after[end..];
            if t.is_empty() || matches!(t, "node" | "node.exe") || t.contains('$') {
                continue;
            }
            return Some(t.to_string());
        }
    }
    None
}

/// First-party (under the repo, outside `node_modules`) or third-party.
pub fn classify(target: &Path, cx: &RecognizeCx<'_>, via: &'static str) -> Found {
    let root = cx.repo_root;
    let rel = target
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let real = link_cache().0.canonicalize(root).path;
            target.strip_prefix(&real).ok().map(Path::to_path_buf)
        });
    if let Some(rel) = rel
        && !rel.components().any(|c| c.as_os_str() == "node_modules")
    {
        let file = root.join(rel);
        let pkg_dir = file
            .parent()
            .and_then(|d| {
                d.ancestors()
                    .take_while(|a| a.starts_with(root))
                    .find(|a| fsx::is_file(&a.join("package.json")))
            })
            .unwrap_or(root)
            .to_path_buf();
        return Found::FirstParty { file, pkg_dir, via };
    }
    Found::ThirdParty {
        package: package_of_path(target).unwrap_or_else(|| {
            target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        file: Some(target.to_path_buf()),
    }
}

/// The npm package containing `p` (`…/node_modules/@scope/name/bin/x` →
/// `@scope/name`), preferring the manifest's `name`.
pub fn package_of_path(p: &Path) -> Option<String> {
    for d in p.ancestors().skip(1) {
        let parent = d.parent()?;
        let pname = parent.file_name()?.to_string_lossy();
        let dname = d.file_name()?.to_string_lossy();
        let derived = if pname == "node_modules" {
            dname.into_owned()
        } else if pname.starts_with('@')
            && parent
                .parent()
                .and_then(|g| g.file_name())
                .is_some_and(|g| g == "node_modules")
        {
            format!("{pname}/{dname}")
        } else {
            continue;
        };
        let named = read_json(&d.join("package.json"))
            .and_then(|m| m.get("name").and_then(|n| n.as_str()).map(str::to_string));
        return Some(named.unwrap_or(derived));
    }
    None
}

/// The `bin` map of a manifest: a string (named after the unscoped package
/// name) or an object.
pub fn bin_map(m: &serde_json::Value) -> Vec<(String, String)> {
    match m.get("bin") {
        Some(serde_json::Value::String(s)) => m
            .get("name")
            .and_then(|n| n.as_str())
            .map(|n| vec![(n.rsplit('/').next().unwrap_or(n).to_string(), s.clone())])
            .unwrap_or_default(),
        Some(serde_json::Value::Object(o)) => o
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
            .collect(),
        _ => Vec::new(),
    }
}

/// Dependency names of the task package and the root (all dependency fields),
/// in order.
fn manifest_deps(cx: &RecognizeCx<'_>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut dirs = vec![cx.package_root];
    if cx.repo_root != cx.package_root {
        dirs.push(cx.repo_root);
    }
    for d in dirs {
        let Some(m) = read_json(&d.join("package.json")) else {
            continue;
        };
        for field in [
            "devDependencies",
            "dependencies",
            "optionalDependencies",
            "peerDependencies",
        ] {
            if let Some(o) = m.get(field).and_then(|v| v.as_object()) {
                for k in o.keys() {
                    if !out.contains(k) {
                        out.push(k.clone());
                    }
                }
            }
        }
    }
    out
}

/// Step 3: an installed dependency whose manifest `bin` has `name`.
fn dependency_bin(name: &str, dirs: &[PathBuf], cx: &RecognizeCx<'_>) -> Option<Found> {
    for dep in manifest_deps(cx).into_iter().take(400) {
        for d in dirs {
            let pj = d.join("node_modules").join(&dep).join("package.json");
            let Some(m) = read_json(&pj) else {
                continue;
            };
            if let Some((_, target)) = bin_map(&m).into_iter().find(|(k, _)| k == name) {
                let pkg = pj.parent().unwrap_or(d);
                let file = fsx::join(pkg, &target);
                let real = if fsx::exists(&file) {
                    link_cache().0.canonicalize(&file).path
                } else {
                    file
                };
                return Some(match classify(&real, cx, "dependency bin") {
                    Found::ThirdParty { file, .. } => Found::ThirdParty { package: dep, file },
                    f => f,
                });
            }
            // The nearest installed copy decides.
            break;
        }
    }
    None
}

/// One workspace package's binary.
#[derive(Clone, Debug)]
struct WsBin {
    pkg_dir: PathBuf,
    pkg_name: Option<String>,
    file: PathBuf,
}

/// Every workspace package's `bin` entries, by binary name.
#[derive(Debug, Default)]
struct BinIndex {
    by_name: BTreeMap<String, Vec<WsBin>>,
}

impl BinIndex {
    fn build(root: &Path) -> BinIndex {
        // Built once per shared cache and repo; what it reads is not attributed to the
        // task that happens to build it (the matching manifest is re-read per
        // lookup instead).
        let frame = turborepo_trace_core::record::start();
        let ws = Workspace::discover(root);
        let mut idx = BinIndex::default();
        for p in &ws.packages {
            let Some(m) = read_json(&p.dir.join("package.json")) else {
                continue;
            };
            for (name, target) in bin_map(&m) {
                idx.by_name.entry(name).or_default().push(WsBin {
                    pkg_dir: p.dir.clone(),
                    pkg_name: p.name.clone(),
                    file: fsx::join(&p.dir, &target),
                });
            }
        }
        drop(frame.finish());
        idx
    }
}

/// The workspace bin index of `root`, memoized per dir cache
/// ([`super::link_cache`]).
fn index(root: &Path) -> Arc<BinIndex> {
    static M: OnceLock<Memo<Arc<BinIndex>>> = OnceLock::new();
    M.get_or_init(Memo::new)
        .get_or(root, || Arc::new(BinIndex::build(root)))
        .0
}

/// Builds the workspace bin index of `root` for the active shared dir cache
/// ahead of the first lookup (perf: the index discovers the whole workspace,
/// and every task resolving a binary waits for it). A no-op when it is already
/// built; observations are unaffected (the index is built outside any task's
/// recording, see [`BinIndex::build`]).
pub fn prewarm_index(root: &Path) {
    let _ = index(root);
}

/// Step 2: a workspace package's `bin`. With several candidates, the one the
/// task package depends on wins.
fn workspace_bin(name: &str, cx: &RecognizeCx<'_>) -> Option<Found> {
    let idx = index(cx.repo_root);
    let cands = idx.by_name.get(name)?;
    let deps = manifest_deps(cx);
    let pick = cands
        .iter()
        .find(|b| b.pkg_name.as_ref().is_some_and(|n| deps.contains(n)))
        .or_else(|| cands.iter().find(|b| b.pkg_dir == cx.package_root))
        .unwrap_or(&cands[0]);
    // Record the manifest that defines the bin for this task.
    let _ = fsx::read(pick.pkg_dir.join("package.json"));
    Some(Found::FirstParty {
        file: pick.file.clone(),
        pkg_dir: pick.pkg_dir.clone(),
        via: "workspace bin",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shims() {
        let pnpm10 = "#!/bin/sh\nexec node \"$basedir_abs/../../x/bin/a.js\" \"$@\"\n# \
                      cmd-shim-target=../../x/bin/a.js\n";
        assert_eq!(parse_shim(pnpm10).as_deref(), Some("../../x/bin/a.js"));
        let old = "#!/bin/sh\nif [ -x \"$basedir/node\" ]; then\n  exec \"$basedir/node\"  \
                   \"$basedir/../@changesets/cli/bin.js\" \"$@\"\nfi\n";
        assert_eq!(
            parse_shim(old).as_deref(),
            Some("../@changesets/cli/bin.js")
        );
        assert_eq!(parse_shim("#!/usr/bin/env node\nrequire('x')"), None);
        assert_eq!(parse_shim("console.log(1)"), None);
    }

    #[test]
    fn package_paths() {
        let p = Path::new("/r/node_modules/.pnpm/jest@30/node_modules/jest/bin/jest.js");
        assert_eq!(package_of_path(p).as_deref(), Some("jest"));
        let p = Path::new("/r/node_modules/@scope/tool/cli.js");
        assert_eq!(package_of_path(p).as_deref(), Some("@scope/tool"));
        assert_eq!(package_of_path(Path::new("/r/packages/x/cli.js")), None);
    }

    #[test]
    fn bin_maps() {
        let m = serde_json::json!({"name": "@s/tool", "bin": "cli.js"});
        assert_eq!(bin_map(&m), [("tool".to_string(), "cli.js".to_string())]);
        let m = serde_json::json!({"bin": {"a": "./a.js", "b": 1}});
        assert_eq!(bin_map(&m), [("a".to_string(), "./a.js".to_string())]);
    }
}
