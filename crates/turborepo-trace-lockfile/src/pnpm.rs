//! `pnpm-lock.yaml`: lockfile versions 6.x (pnpm 8) and 9.0 (pnpm 9 and 10).
//!
//! v6 has one `packages` section keyed by `/name@version(peers)`. v9 splits it
//! into `packages` (keyed by `name@version`: resolution, platform) and
//! `snapshots` (keyed by the full id with peer and patch suffixes:
//! dependencies, optional flag). Both normalize to the same ids
//! (`name@version(peers)`, no leading `/`), so v6 and v9 lockfiles of the same
//! tree diff cleanly.

use std::collections::BTreeMap;

use crate::{
    Error, Result,
    model::{
        CatalogEntry, DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat,
        LockfileMeta, PackageFlags, Platform, Target, join_importer_path,
    },
    util::{FxHashMap, pnpm_patch_hash, split_name_at, strip_paren_suffix},
    yaml::{Node, Parser, YamlError},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Major {
    V6,
    V9,
}

/// Per-`packages` entry data in v9 (joined to snapshots by base key).
#[derive(Default, Clone)]
struct PkgInfo {
    used: bool,
    name: Option<String>,
    version: Option<String>,
    integrity: Option<String>,
    resolution: Option<String>,
    platform: Platform,
}

/// Parses a `pnpm-lock.yaml`.
pub fn parse(src: &str) -> Result<Lockfile> {
    let syntax = |e: YamlError| Error::Syntax {
        format: LockfileFormat::Pnpm,
        line: e.line,
        message: e.message,
    };
    let mut p = Parser::new(src);
    // pnpm may prepend an "env lockfile" document (configDependencies); the main
    // lockfile is the last document.
    let docs = Parser::document_count(src);
    if docs > 1 {
        p.seek_document(docs - 1).map_err(syntax)?;
    }

    let mut version: Option<String> = None;
    // Ids are normalized per major version while streaming, so find the version
    // first (it is normally the first line, but the YAML mapping order is not
    // significant).
    let mut major = prescan_major(src);
    let mut meta = LockfileMeta::default();
    let mut importers: Vec<Importer> = Vec::new();
    let mut root_single = Importer::new(".");
    let mut infos: FxHashMap<String, PkgInfo> = FxHashMap::default();
    let mut packages: Vec<LockedPackage> = Vec::new();
    let mut snapshots: Vec<LockedPackage> = Vec::new();
    let mut bad_version: Option<String> = None;

    p.top_level(|p, key| {
        match key.as_ref() {
            "lockfileVersion" => {
                let v = p.value(0)?;
                let v = v.as_str().unwrap_or("").to_string();
                let maj: u32 = v
                    .split('.')
                    .next()
                    .and_then(|m| m.parse().ok())
                    .unwrap_or(0);
                major = match maj {
                    6..=8 => Major::V6,
                    9 => Major::V9,
                    _ => {
                        bad_version = Some(v.clone());
                        Major::V9
                    }
                };
                version = Some(v);
            }
            "importers" => {
                let mj = major;
                p.entries(0, |path, node| {
                    importers.push(importer(&path, &node, mj));
                    Ok(())
                })?;
            }
            // v6 single-project lockfiles keep the root importer at the top level.
            "dependencies" | "devDependencies" | "optionalDependencies" => {
                let kind = kind_of(&key);
                let node = p.value(0)?;
                push_importer_deps(&mut root_single, &node, kind, major);
            }
            "packages" => {
                if major == Major::V9 {
                    p.entries(0, |k, node| {
                        infos.insert(k.into_owned(), pkg_info(&node));
                        Ok(())
                    })?;
                } else {
                    p.entries(0, |k, node| {
                        let id = k.strip_prefix('/').unwrap_or(&k).to_string();
                        let info = pkg_info(&node);
                        packages.push(package(id, &node, info, Major::V6));
                        Ok(())
                    })?;
                }
            }
            "snapshots" => {
                // Joined with `packages` after the whole document is read (sections may come in
                // any order).
                p.entries(0, |k, node| {
                    snapshots.push(package(
                        k.into_owned(),
                        &node,
                        PkgInfo::default(),
                        Major::V9,
                    ));
                    Ok(())
                })?;
            }
            "overrides" => {
                let node = p.value(0)?;
                meta.overrides = string_map(&node);
            }
            "catalogs" => {
                let node = p.value(0)?;
                for (cat, entries) in node.entries() {
                    let mut m = BTreeMap::new();
                    for (dep, e) in entries.entries() {
                        m.insert(
                            dep.to_string(),
                            CatalogEntry {
                                specifier: e.get_str("specifier").unwrap_or("").to_string(),
                                version: e.get_str("version").unwrap_or("").to_string(),
                            },
                        );
                    }
                    meta.catalogs.insert(cat.to_string(), m);
                }
            }
            "patchedDependencies" => {
                let node = p.value(0)?;
                for (sel, v) in node.entries() {
                    // pnpm 10: `sel: hash`; pnpm 8/9: `sel: {hash, path}`.
                    let hash = v.as_str().or_else(|| v.get_str("hash")).unwrap_or("");
                    meta.patched_dependencies
                        .insert(sel.to_string(), hash.to_string());
                }
            }
            _ => {
                p.value(0)?;
            }
        }
        Ok(())
    })
    .map_err(syntax)?;

    let version = version.ok_or_else(|| Error::Syntax {
        format: LockfileFormat::Pnpm,
        line: 1,
        message: "missing lockfileVersion".into(),
    })?;
    if let Some(v) = bad_version {
        return Err(Error::UnsupportedVersion {
            format: LockfileFormat::Pnpm,
            version: v,
        });
    }
    // v9: fill snapshots from their `packages` entry. Several snapshots (peer
    // variants) share one entry.
    for mut pkg in snapshots {
        if let Some(info) = infos.get_mut(strip_paren_suffix(&pkg.id)) {
            info.used = true;
            apply_info(&mut pkg, info.clone());
        }
        packages.push(pkg);
    }
    // v9 packages without a snapshot (not expected, but keep them rather than
    // drop).
    for (id, info) in infos.into_iter().filter(|(_, i)| !i.used) {
        packages.push(package(id, &Node::Null, info, Major::V9));
    }
    if !root_single.dependencies.is_empty() {
        importers.push(root_single);
    }
    Ok(Lockfile::from_parts(
        LockfileFormat::Pnpm,
        version,
        packages,
        importers,
        meta,
    ))
}

fn prescan_major(src: &str) -> Major {
    for line in src.lines() {
        let Some(rest) = line.strip_prefix("lockfileVersion:").or_else(|| {
            line.strip_prefix("'lockfileVersion':")
                .or_else(|| line.strip_prefix("\"lockfileVersion\":"))
        }) else {
            continue;
        };
        let major = rest
            .trim()
            .trim_matches(['\'', '"'])
            .split('.')
            .next()
            .and_then(|m| m.parse::<u32>().ok());
        return if matches!(major, Some(m) if m < 9) {
            Major::V6
        } else {
            Major::V9
        };
    }
    Major::V9
}

fn kind_of(section: &str) -> DepKind {
    match section {
        "devDependencies" => DepKind::Dev,
        "optionalDependencies" => DepKind::Optional,
        _ => DepKind::Prod,
    }
}

fn string_map(node: &Node<'_>) -> BTreeMap<String, String> {
    node.entries()
        .iter()
        .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string()))
        .collect()
}

fn importer(path: &str, node: &Node<'_>, major: Major) -> Importer {
    let mut imp = Importer::new(path);
    for section in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(deps) = node.get(section) {
            push_importer_deps(&mut imp, deps, kind_of(section), major);
        }
    }
    imp
}

fn push_importer_deps(imp: &mut Importer, deps: &Node<'_>, kind: DepKind, major: Major) {
    for (name, d) in deps.entries() {
        // `{specifier, version}`; very old single-project files map name → version
        // directly.
        let (specifier, reference) = match d {
            Node::Scalar(v) => (None, v.as_ref()),
            _ => (
                d.get_str("specifier").map(str::to_string),
                d.get_str("version").unwrap_or(""),
            ),
        };
        let target = ref_target(&imp.path, name, reference, major);
        imp.dependencies.push(DepEdge {
            name: name.to_string(),
            kind,
            target,
            specifier,
        });
    }
}

/// Converts a dependency reference (`1.0.0(peer@1)`, `link:../x`,
/// `alias@1.0.0`, `/alias@1.0.0`, tarball/git refs) to a target, following
/// pnpm's `refToRelative`.
fn ref_target(base: &str, name: &str, reference: &str, major: Major) -> Target {
    if let Some(rel) = reference.strip_prefix("link:") {
        return Target::Importer(join_importer_path(base, rel));
    }
    if reference.is_empty() {
        return Target::Unresolved(String::new());
    }
    Target::Package(ref_to_id(name, reference, major))
}

fn concat_at(name: &str, reference: &str) -> String {
    let mut s = String::with_capacity(name.len() + 1 + reference.len());
    s.push_str(name);
    s.push('@');
    s.push_str(reference);
    s
}

fn ref_to_id(name: &str, reference: &str, major: Major) -> String {
    match major {
        Major::V6 => {
            if let Some(r) = reference.strip_prefix('/') {
                return r.to_string();
            }
            let before_paren = strip_paren_suffix(reference);
            if reference.starts_with("file:") || before_paren.contains('/') {
                reference.to_string()
            } else {
                concat_at(name, reference)
            }
        }
        Major::V9 => {
            if reference.starts_with('@') {
                return reference.to_string();
            }
            let Some(at) = reference.find('@') else {
                return concat_at(name, reference);
            };
            let colon = reference.find(':');
            let paren = reference.find('(');
            if colon.is_none_or(|c| at < c) && paren.is_none_or(|p| at < p) {
                reference.to_string()
            } else {
                concat_at(name, reference)
            }
        }
    }
}

fn pkg_info(node: &Node<'_>) -> PkgInfo {
    let mut info = PkgInfo {
        name: node.get_str("name").map(str::to_string),
        version: node.get_str("version").map(str::to_string),
        ..PkgInfo::default()
    };
    if let Some(res) = node.get("resolution") {
        info.integrity = res.get_str("integrity").map(str::to_string);
        info.resolution = if let Some(t) = res.get_str("tarball") {
            Some(t.to_string())
        } else if let Some(d) = res.get_str("directory") {
            Some(format!("directory:{d}"))
        } else if let Some(repo) = res.get_str("repo") {
            Some(format!(
                "git+{repo}#{}",
                res.get_str("commit").unwrap_or("")
            ))
        } else {
            res.get_str("type")
                .map(|t| format!("{t}:{}", res.get_str("path").unwrap_or("")))
        };
    }
    info.platform = Platform {
        os: node.get("os").map(Node::str_list).unwrap_or_default(),
        cpu: node.get("cpu").map(Node::str_list).unwrap_or_default(),
        libc: node.get("libc").map(Node::str_list).unwrap_or_default(),
    };
    info
}

/// Overrides name/version (non-registry packages record them) and fills
/// resolution data.
fn apply_info(pkg: &mut LockedPackage, info: PkgInfo) {
    if let Some(name) = info.name {
        pkg.name = name;
    }
    if let Some(version) = info.version {
        pkg.version = version;
    }
    pkg.integrity = info.integrity;
    pkg.resolution = info.resolution;
    pkg.platform = info.platform;
}

fn package(id: String, node: &Node<'_>, info: PkgInfo, major: Major) -> LockedPackage {
    let base = strip_paren_suffix(&id);
    let (key_name, key_version) = split_name_at(base).unwrap_or((base, ""));
    let mut pkg = LockedPackage::new(id.clone(), key_name, key_version);
    apply_info(&mut pkg, info);
    pkg.patch = pnpm_patch_hash(&id).map(str::to_string);
    for (section, kind) in [
        ("dependencies", DepKind::Prod),
        ("optionalDependencies", DepKind::Optional),
    ] {
        for (dep, r) in node.get(section).map(Node::entries).unwrap_or_default() {
            let reference = r.as_str().unwrap_or("");
            pkg.dependencies.push(DepEdge {
                name: dep.to_string(),
                kind,
                target: ref_target(".", dep, reference, major),
                specifier: None,
            });
        }
    }
    if node.get("optional").is_some_and(Node::is_true) {
        pkg.flags |= PackageFlags::OPTIONAL;
    }
    if node.get("dev").is_some_and(Node::is_true) {
        pkg.flags |= PackageFlags::DEV;
    }
    pkg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs() {
        assert_eq!(
            ref_to_id("a", "1.0.0(b@2.0.0)", Major::V9),
            "a@1.0.0(b@2.0.0)"
        );
        assert_eq!(ref_to_id("a", "b@1.0.0", Major::V9), "b@1.0.0");
        assert_eq!(ref_to_id("a", "@s/b@1.0.0", Major::V9), "@s/b@1.0.0");
        assert_eq!(
            ref_to_id("a", "https://x.example/a.tgz", Major::V9),
            "a@https://x.example/a.tgz"
        );
        assert_eq!(
            ref_to_id("a", "1.0.0(@s/p@1.0.0)", Major::V9),
            "a@1.0.0(@s/p@1.0.0)"
        );
        assert_eq!(
            ref_to_id("a", "1.0.0(b@2.0.0)", Major::V6),
            "a@1.0.0(b@2.0.0)"
        );
        assert_eq!(ref_to_id("a", "/b@1.0.0", Major::V6), "b@1.0.0");
        assert_eq!(
            ref_to_id("a", "github.com/u/r/abc", Major::V6),
            "github.com/u/r/abc"
        );
        assert_eq!(
            ref_target("apps/web", "x", "link:../../packages/x", Major::V9),
            Target::Importer("packages/x".into())
        );
    }
}
